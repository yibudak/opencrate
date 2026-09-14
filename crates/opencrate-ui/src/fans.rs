//! Fan dashboard. Draft edits never write hardware until Apply is pressed.

use crate::{
    dashboard::page_header,
    i18n::{self, t, Message},
    preferences::{FanGroup, Store},
    theme::*,
};
use egui::{pos2, vec2, Align, Align2, FontId, Layout, Rect, RichText, Stroke, Ui};
use opencrate_fan::quick::{self, QuickMode, Setting};
use opencrate_fan::service::{
    critical_temperature, manual_curve, percent, raw_duty, validate_custom, Command, Controller,
    Fan, Point, Target,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Current,
    Profile(i32),
    Manual,
    Curve,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupMode {
    Preset(QuickMode),
    Manual,
}

fn mode_label(mode: QuickMode) -> &'static str {
    if mode == QuickMode::FullBlast {
        "Full speed"
    } else {
        mode.label()
    }
}

struct Draft {
    id: u32,
    mode: Mode,
    duty: u8,
    points: Vec<Point>,
}
impl Draft {
    fn new(fan: &Fan) -> Self {
        Self {
            id: fan.id,
            mode: Mode::Current,
            duty: (percent(fan.duty).round() as u8).max(percent(fan.minimum).ceil() as u8),
            points: fan.curve.clone(),
        }
    }
    fn curve(&self, fan: &Fan) -> Result<Vec<Point>, String> {
        match self.mode {
            Mode::Current => Ok(fan.curve.clone()),
            Mode::Profile(i) => fan
                .profiles
                .iter()
                .find(|p| p.index == i)
                .map(|p| p.curve.clone())
                .ok_or("Profile is unavailable.".into()),
            Mode::Manual => manual_curve(fan, self.duty),
            Mode::Curve => {
                validate_custom(&self.points, fan)?;
                Ok(self.points.clone())
            }
        }
    }
    fn label(&self, fan: &Fan) -> String {
        match self.mode {
            Mode::Current => "Current settings".into(),
            Mode::Profile(i) => fan
                .profiles
                .iter()
                .find(|p| p.index == i)
                .map_or("Unavailable", |p| p.name.as_str())
                .into(),
            Mode::Manual => "Manual speed".into(),
            Mode::Curve => "Custom curve".into(),
        }
    }
}

pub struct State {
    controller: Option<Controller>,
    fans: Vec<Fan>,
    draft: Option<Draft>,
    error: Option<String>,
    message: Message,
    action_success: &'static str,
    action_error: bool,
    busy: bool,
    can_undo: bool,
    selected_group: Option<usize>,
    group_mode: GroupMode,
    group_duty: u8,
    reset_draft: bool,
}

impl State {
    pub fn status(&self, colors: Palette) -> (&str, String, egui::Color32) {
        if let Some(error) = &self.error {
            (
                t("OFFLINE"),
                i18n::f(
                    "Fan control failed: {details}",
                    &[("details", t(error).to_string())],
                ),
                colors.red,
            )
        } else if self.busy {
            (t("APPLYING"), self.message.render(), colors.accent)
        } else if self.action_error {
            (t("NEEDS ATTENTION"), self.message.render(), colors.red)
        } else {
            (t("FANS"), self.message.render(), colors.green)
        }
    }
    pub fn new(ctx: &egui::Context) -> Self {
        let wake = ctx.clone();
        let controller = if crate::runtime::hardware_enabled() {
            Controller::start(move || wake.request_repaint())
        } else {
            Err(std::io::Error::other("Diagnostic mode"))
        };
        let error = controller.as_ref().err().map(ToString::to_string);
        Self {
            controller: controller.ok(),
            fans: Vec::new(),
            draft: None,
            error,
            message: Message::text("Connecting to ASUS fan service…"),
            action_success: "Fan readings refreshed.",
            action_error: false,
            busy: false,
            can_undo: false,
            selected_group: None,
            group_mode: GroupMode::Preset(QuickMode::Standard),
            group_duty: 60,
            reset_draft: false,
        }
    }

    pub fn poll(&mut self) {
        let Some(controller) = &self.controller else {
            return;
        };
        while let Some(event) = controller.try_event() {
            if let Some(action) = event.action {
                self.busy = false;
                self.action_error = action.is_err();
                self.reset_draft &= action.is_ok();
                self.message = match action {
                    Ok(_) => Message::text(self.action_success),
                    Err(error) => {
                        Message::with("Fan control failed: {details}", vec![("details", error)])
                    }
                };
            }
            match event.result {
                Ok(snapshot) => {
                    if self.fans.is_empty() && !self.action_error && !self.busy {
                        self.message = Message::text("Fan readings refreshed.");
                    }
                    let fans = snapshot.fans;
                    self.can_undo = snapshot.can_undo;
                    self.error = None;
                    if self
                        .draft
                        .as_ref()
                        .is_none_or(|d| !fans.iter().any(|f| f.id == d.id))
                    {
                        self.draft = fans.first().map(Draft::new);
                    } else if self.reset_draft && !self.busy {
                        self.draft = fans
                            .iter()
                            .find(|f| Some(f.id) == self.draft.as_ref().map(|d| d.id))
                            .map(Draft::new);
                    }
                    if !self.busy {
                        self.reset_draft = false;
                    }
                    self.fans = fans;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn send(&mut self, command: Command) {
        // One user action creates exactly one command, even if multiple widgets
        // were rendered before the first click set busy in this frame.
        if self.busy {
            return;
        }
        let success = match &command {
            Command::Refresh => "Fan readings refreshed.",
            Command::Apply {
                target: Target::Restore,
                ..
            }
            | Command::UndoQuick => "Previous fan settings restored.",
            Command::Apply { .. } => "Fan settings applied.",
            Command::Quick(_) => "All fan settings applied.",
            Command::ApplyGroup { .. } => "Group settings applied once.",
            Command::Stop => return,
        };
        let refreshing = matches!(command, Command::Refresh);
        let selected = self.draft.as_ref().map(|d| d.id);
        let reset_draft = match &command {
            Command::Apply { id, .. } => selected == Some(*id),
            Command::ApplyGroup { ids, .. } => selected.is_some_and(|id| ids.contains(&id)),
            Command::Quick(_) | Command::UndoQuick => true,
            Command::Refresh | Command::Stop => false,
        };
        let result = self
            .controller
            .as_ref()
            .ok_or("Fan worker is unavailable.".into())
            .and_then(|c| c.send(command));
        match result {
            Ok(()) => {
                self.reset_draft = reset_draft;
                self.busy = true;
                self.action_success = success;
                self.action_error = false;
                self.message = Message::text(if refreshing {
                    "Reconnecting to ASUS fan service…"
                } else {
                    "Applying fan settings…"
                });
            }
            Err(e) => {
                self.action_error = true;
                self.message = Message::with("Fan control failed: {details}", vec![("details", e)]);
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, store: &mut Store) {
        let colors = palette(ui);
        page_header(
            ui,
            t("Fans"),
            t("Find your balance between quiet and cool."),
            if self.error.is_some() {
                t("OFFLINE")
            } else if self.fans.is_empty() {
                t("CONNECTING")
            } else {
                t("ASUS CONNECTED")
            },
            if self.error.is_some() {
                colors.red
            } else {
                colors.green
            },
        );
        if let Some(error) = &self.error {
            card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                subtitle(ui, t("Fan service unavailable"));
                ui.colored_label(colors.red, i18n::f("Details: {details}", &[("details", t(error).to_string())]));
                ui.label(RichText::new(t("OpenCrate uses the installed ASUS fan service. Previous readings are paused until it responds.")).color(colors.muted));
            });
            if ui
                .add_enabled(!self.busy, egui::Button::new(t("Retry connection")))
                .clicked()
            {
                self.send(Command::Refresh);
            }
        }
        if self.fans.is_empty() {
            if self.error.is_none() {
                ui.spinner();
                ui.label(t("Reading fan headers and cooling profiles…"));
            }
            return;
        }
        self.quick_controls(ui);
        self.group_controls(ui, store);
        let selected = self.draft.as_ref().map(|d| d.id);
        let mut choose = None;
        let count = self.fans.len();
        if ui.available_width() >= 690.0 && count <= 4 {
            ui.columns(count, |columns| {
                for (ui, fan) in columns.iter_mut().zip(&self.fans) {
                    if fan_tile(ui, fan, selected == Some(fan.id), self.error.is_none()) {
                        choose = Some(fan.id);
                    }
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                for fan in &self.fans {
                    if ui
                        .selectable_label(
                            selected == Some(fan.id),
                            format!(
                                "{} · {}",
                                t(&fan.name),
                                telemetry(fan, self.error.is_none())
                            ),
                        )
                        .clicked()
                    {
                        choose = Some(fan.id);
                    }
                }
            });
        }
        if let Some(id) = choose {
            self.draft = self.fans.iter().find(|f| f.id == id).map(Draft::new);
        }
        ui.add_space(12.0);
        let group_ids = self.group_ids(store);
        let group_name = self.group_name(store);
        let Some(draft) = &mut self.draft else {
            return;
        };
        let Some(fan) = self.fans.iter().find(|f| f.id == draft.id).cloned() else {
            return;
        };
        let enabled = self.error.is_none() && !self.busy && fan.writable;
        if !fan.writable {
            ui.colored_label(colors.red, t("This fan uses an unsupported curve or RPM control mode. Readings remain available."));
        }
        let mut command = None;
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                subtitle(ui, t(&fan.name));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(telemetry(&fan, self.error.is_none())).color(colors.accent));
                });
            });
            ui.add_space(6.0);
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label(t("Cooling mode"));
                    let previous = draft.mode;
                    let previous_curve = draft.curve(&fan).unwrap_or_else(|_| fan.curve.clone());
                    egui::ComboBox::from_id_salt("fan_mode").selected_text(t(&draft.label(&fan))).width(210.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.mode, Mode::Current, t("Current settings"));
                        for profile in &fan.profiles { ui.selectable_value(&mut draft.mode, Mode::Profile(profile.index), t(&profile.name)); }
                        ui.selectable_value(&mut draft.mode, Mode::Manual, t("Manual speed"));
                        ui.selectable_value(&mut draft.mode, Mode::Curve, t("Custom curve"));
                    });
                    if previous != draft.mode && draft.mode == Mode::Curve {
                        draft.points = editable_curve(&previous_curve, &fan);
                    }
                });
                match draft.mode {
                    Mode::Manual => {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(t("Fan speed"));
                            ui.label(RichText::new(format!("{}%", draft.duty)).color(colors.accent).strong());
                        });
                        ui.spacing_mut().slider_width = ui.available_width() - 8.0;
                        let response = ui.scope(|ui| {
                            ui.spacing_mut().interact_size.y = 22.0;
                            ui.add(egui::Slider::new(&mut draft.duty, (percent(fan.minimum).ceil() as u8)..=100).show_value(false))
                        }).inner;
                        response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(draft.duty), t("Fan speed")));
                        ui.label(RichText::new(i18n::f("Holds the selected speed at lower temperatures, then rises to 100% at {temperature} °C.", &[("temperature", critical_temperature(&fan.curve).to_string())])).small().color(colors.muted));
                    },
                    Mode::Curve => {
                        ui.label(RichText::new(t("Drag points on the chart or enter exact values below. Apply when ready.")).small().color(colors.muted));
                        ui.horizontal_wrapped(|ui| {
                            ui.label(t("Start from"));
                            if ui.button(t("Current curve")).clicked() {
                                draft.points = editable_curve(&fan.curve, &fan);
                            }
                            for profile in &fan.profiles {
                                if ui.button(t(&profile.name)).clicked() {
                                    draft.points = editable_curve(&profile.curve, &fan);
                                }
                            }
                            if ui.button(t("Linear ramp")).clicked() {
                                draft.points = linear_curve(&draft.points, &fan);
                            }
                        });
                        curve_chart(ui, &fan.curve, &mut draft.points, &fan, true, 230.0);
                        egui::Grid::new("fan_curve_points").num_columns(3).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                            eyebrow(ui, t("POINT")); eyebrow(ui, t("TEMPERATURE")); eyebrow(ui, t("FAN SPEED")); ui.end_row();
                            let last = draft.points.len() - 1;
                            for i in 0..draft.points.len() {
                                let (min_t, max_t, min_d, max_d) = point_bounds(&draft.points, i, &fan);
                                let point = &mut draft.points[i];
                                ui.label(format!("{:02}", i + 1));
                                let r = ui.add(egui::DragValue::new(&mut point.temperature).range(min_t..=max_t).suffix(" °C"));
                                r.widget_info(|| egui::WidgetInfo::slider(true, f64::from(point.temperature), i18n::f("Point {number} temperature", &[("number", (i+1).to_string())])));
                                let mut duty = percent(point.duty).round() as u8;
                                let r = ui.add_enabled(i != last, egui::DragValue::new(&mut duty).range((percent(min_d).round() as u8)..=(percent(max_d).round() as u8)).suffix("%"));
                                r.widget_info(|| egui::WidgetInfo::slider(i != last, f64::from(duty), i18n::f("Point {number} fan speed", &[("number", (i+1).to_string())])));
                                if r.changed() { point.duty = raw_duty(duty).unwrap_or(255).clamp(min_d, max_d); }
                                ui.end_row();
                            }
                        });
                        ui.label(RichText::new(i18n::f("This controller uses {count} points. The last point stays at 100% for cooling protection.", &[("count", draft.points.len().to_string())])).small().color(colors.muted));
                    },
                    Mode::Current => { ui.label(RichText::new(t("Live controller curve. Choose a mode to make changes.")).color(colors.muted)); },
                    Mode::Profile(_) => { ui.label(RichText::new(t("Temperature-based profile supplied by your ASUS fan controller.")).color(colors.muted)); },
                }
            });
            ui.add_space(12.0);
            let curve = draft.curve(&fan);
            if draft.mode != Mode::Curve {
                let mut preview = curve.as_ref().unwrap_or(&fan.curve).clone();
                curve_chart(ui, &fan.curve, &mut preview, &fan, false, 160.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("Preview")).small().color(colors.accent));
                ui.label(RichText::new(t("Current curve")).small().color(colors.muted));
                ui.label(RichText::new(i18n::f("Minimum manual speed: {duty}%", &[("duty", format!("{:.0}", percent(fan.minimum).ceil()))])).small().color(colors.muted));
            });
            if let Err(error) = &curve { ui.colored_label(colors.red, i18n::f("Details: {details}", &[("details", t(error).to_string())])); }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let changed = curve.as_ref().is_ok_and(|c| *c != fan.curve);
                if ui.add_enabled(enabled && changed && draft.mode != Mode::Current,
                    egui::Button::new(RichText::new(t("Apply to this fan")).color(colors.on_accent).strong()).fill(colors.accent).min_size(vec2(160.0, 40.0))).clicked() {
                    let target = match draft.mode { Mode::Profile(i) => Target::Profile(i), _ => Target::Custom(curve.clone().unwrap()) };
                    command = Some(Command::Apply { id: fan.id, target });
                }
                if ui.add_enabled(enabled && fan.can_restore, egui::Button::new(t("Restore original")).min_size(vec2(140.0, 40.0))).clicked() {
                    command = Some(Command::Apply { id: fan.id, target: Target::Restore });
                }
            });
            if draft.mode != Mode::Current {
                let setting = match draft.mode {
                    Mode::Profile(i) => fan.profiles.iter().find(|p| p.index == i)
                        .and_then(|p| [QuickMode::Silent, QuickMode::Standard, QuickMode::Turbo].into_iter().find(|m| m.label() == p.name))
                        .map(Setting::Preset),
                    Mode::Manual => Some(Setting::Manual(draft.duty)),
                    Mode::Curve => curve.as_ref().ok().map(|c| Setting::Curve(c.clone())),
                    Mode::Current => None,
                };
                if let Some(setting) = setting {
                    let plan = quick::plan_group(&self.fans, &group_ids, &setting);
                    let response = ui.add_enabled(enabled && plan.as_ref().is_ok_and(|p| !p.is_empty()),
                        egui::Button::new(i18n::f("Apply this mode to {group}", &[("group", group_name.clone())])));
                    if response.clicked() {
                        command = Some(Command::ApplyGroup { ids: group_ids.clone(), setting });
                    }
                    if let Err(error) = plan { ui.label(RichText::new(t(&error)).small().color(colors.muted)); }
                }
            }
        });
        if let Some(command) = command {
            self.send(command);
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(t("Active in the tray · Original curves restored on Quit"))
                .small()
                .color(colors.muted),
        );
        ui.label(RichText::new(t("Readings update every 2 seconds. % is the controller's duty output; RPM is measured fan speed.")).small().color(colors.muted));
    }

    fn group_ids(&self, store: &Store) -> Vec<u32> {
        self.selected_group
            .and_then(|i| store.preferences.fan_groups.get(i))
            .map_or_else(
                || self.fans.iter().map(|f| f.id).collect(),
                |g| g.members.clone(),
            )
    }

    fn group_name(&self, store: &Store) -> String {
        self.selected_group
            .and_then(|i| store.preferences.fan_groups.get(i))
            .map_or_else(
                || t("All fans").to_string(),
                |g| {
                    if g.name.trim().is_empty() {
                        t("Unnamed group").into()
                    } else {
                        g.name.clone()
                    }
                },
            )
    }

    fn group_controls(&mut self, ui: &mut Ui, store: &mut Store) {
        let colors = palette(ui);
        let enabled = self.error.is_none() && !self.busy;
        let mut changed = false;
        let mut command = None;
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                subtitle(ui, t("Fan groups"));
                egui::ComboBox::from_id_salt("fan_group").selected_text(self.group_name(store))
                    .width(180.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.selected_group, None, t("All fans"));
                        for (i, group) in store.preferences.fan_groups.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_group, Some(i), if group.name.trim().is_empty() { t("Unnamed group") } else { &group.name });
                        }
                    });
                if ui.button(t("New group")).clicked() {
                    self.selected_group = Some(store.preferences.fan_groups.len());
                    store.preferences.fan_groups.push(FanGroup {
                        name: i18n::f("Fan group {number}", &[("number", (store.preferences.fan_groups.len() + 1).to_string())]),
                        members: self.fans.iter().map(|f| f.id).collect(),
                    });
                    changed = true;
                }
            });
            if let Some(index) = self.selected_group {
                let mut remove = false;
                if let Some(group) = store.preferences.fan_groups.get_mut(index) {
                    ui.horizontal(|ui| {
                        ui.label(t("Group name"));
                        changed |= ui.add(egui::TextEdit::singleline(&mut group.name).desired_width(180.0).char_limit(60)).changed();
                        remove = ui.button(t("Delete group")).clicked();
                    });
                    ui.horizontal_wrapped(|ui| {
                        for fan in &self.fans {
                            let mut member = group.members.contains(&fan.id);
                            if ui.checkbox(&mut member, t(&fan.name)).changed() {
                                group.members.retain(|id| *id != fan.id);
                                if member { group.members.push(fan.id); }
                                changed = true;
                            }
                        }
                        // Keep disconnected members until explicitly removed.
                        let missing = group.members.iter().copied().filter(|id| !self.fans.iter().any(|f| f.id == *id)).collect::<Vec<_>>();
                        for id in missing {
                            let mut member = true;
                            if ui.checkbox(&mut member, i18n::f("Unavailable fan ({id})", &[("id", id.to_string())])).changed() {
                                group.members.retain(|saved| *saved != id);
                                changed = true;
                            }
                        }
                    });
                } else {
                    self.selected_group = None;
                }
                if remove {
                    store.preferences.fan_groups.remove(index);
                    self.selected_group = None;
                    changed = true;
                }
            }
            let ids = self.group_ids(store);
            ui.label(RichText::new(i18n::f("{count} fans selected · Group membership is saved; cooling modes apply only when clicked.", &[("count", ids.len().to_string())])).small().color(colors.muted));
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Cooling mode"));
                let label = match self.group_mode { GroupMode::Preset(m) => mode_label(m), GroupMode::Manual => "Manual speed" };
                egui::ComboBox::from_id_salt("group_cooling_mode").selected_text(t(label)).show_ui(ui, |ui| {
                    for mode in [QuickMode::Silent, QuickMode::Standard, QuickMode::Turbo, QuickMode::FullBlast] {
                        ui.selectable_value(&mut self.group_mode, GroupMode::Preset(mode), t(mode_label(mode)));
                    }
                    ui.selectable_value(&mut self.group_mode, GroupMode::Manual, t("Manual speed"));
                });
                let minimum = self.fans.iter().filter(|f| ids.contains(&f.id)).map(|f| percent(f.minimum).ceil() as u8).max().unwrap_or(1);
                if self.group_mode == GroupMode::Manual {
                    self.group_duty = self.group_duty.max(minimum);
                    ui.add(egui::DragValue::new(&mut self.group_duty).range(minimum..=100).suffix("%"));
                }
                let setting = match self.group_mode { GroupMode::Preset(m) => Setting::Preset(m), GroupMode::Manual => Setting::Manual(self.group_duty) };
                let plan = quick::plan_group(&self.fans, &ids, &setting);
                if ui.add_enabled(enabled && plan.as_ref().is_ok_and(|p| !p.is_empty()),
                    egui::Button::new(RichText::new(t("Apply to group")).strong().color(colors.on_accent)).fill(colors.accent)).clicked() {
                    command = Some(Command::ApplyGroup { ids, setting });
                }
                match plan {
                    Err(error) => { ui.label(RichText::new(t(&error)).small().color(colors.muted)); }
                    Ok(changes) if changes.is_empty() => { ui.label(RichText::new(t("Already applied")).small().color(colors.muted)); }
                    _ => {}
                }
            });
            ui.label(RichText::new(t("To share a custom curve, edit a fan below and apply its mode to this group.")).small().color(colors.muted));
        });
        if changed {
            store.changed();
        }
        if let Some(command) = command {
            self.send(command);
        }
        ui.add_space(12.0);
    }

    fn quick_controls(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        let enabled = self.error.is_none() && !self.busy;
        let mut command = None;
        eyebrow(ui, t("QUICK ACTIONS · ALL FANS"));
        ui.label(RichText::new(t("Each button applies once to all fans. The controller keeps that curve until you change or undo it.")).small().color(colors.muted));
        ui.horizontal_wrapped(|ui| {
            for mode in [
                QuickMode::FullBlast,
                QuickMode::Silent,
                QuickMode::Standard,
                QuickMode::Turbo,
            ] {
                let preflight = quick::plan(&self.fans, mode);
                let button = egui::Button::new(i18n::f(
                    "Apply {mode}",
                    &[("mode", t(mode_label(mode)).to_string())],
                ))
                .min_size(vec2(80.0, 36.0));
                let response = ui.add_enabled(
                    enabled && preflight.as_ref().is_ok_and(|p| !p.is_empty()),
                    button,
                );
                if response.clicked() {
                    command = Some(Command::Quick(mode));
                }
                match preflight {
                    Err(error) => {
                        response.on_disabled_hover_text(i18n::f(
                            "Details: {details}",
                            &[("details", t(&error).to_string())],
                        ));
                    }
                    Ok(changes) => {
                        response
                            .on_disabled_hover_text(t("Already applied"))
                            .on_hover_text(if changes.is_empty() {
                                t("Already applied").to_string()
                            } else {
                                i18n::f(
                                    "Apply {profile} to every fan immediately.",
                                    &[("profile", t(mode_label(mode)).to_string())],
                                )
                            });
                    }
                }
            }
            if ui
                .add_enabled(
                    enabled && self.can_undo,
                    egui::Button::new(t("Undo last group action")).min_size(vec2(138.0, 36.0)),
                )
                .on_hover_text(t(
                    "Restore the exact settings from before the last quick or group action.",
                ))
                .clicked()
            {
                command = Some(Command::UndoQuick);
            }
        });
        ui.add_space(10.0);
        if let Some(command) = command {
            self.send(command);
        }
    }
}

fn fan_tile(ui: &mut Ui, fan: &Fan, selected: bool, live: bool) -> bool {
    let colors = palette(ui);
    let frame = egui::Frame::new()
        .fill(if selected {
            colors.accent_dim
        } else {
            colors.surface
        })
        .stroke(Stroke::new(
            1.0_f32,
            if selected {
                colors.accent
            } else {
                colors.border
            },
        ))
        .corner_radius(12)
        .inner_margin(12);
    let result = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(t(&fan.name)).size(13.0).strong());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::hover());
            icon(
                ui,
                Icon::Fan,
                rect,
                if selected {
                    colors.accent
                } else {
                    colors.muted
                },
            );
            ui.label(
                RichText::new(if live {
                    fan.rpm
                        .map_or_else(|| "— RPM".into(), |rpm| format!("{rpm} RPM"))
                } else {
                    "—".into()
                })
                .size(22.0)
                .strong(),
            );
        });
        ui.label(
            RichText::new(if live {
                i18n::f(
                    "{duty}% output",
                    &[("duty", format!("{:.0}", percent(fan.duty)))],
                )
            } else {
                t("Reading paused").into()
            })
            .color(colors.muted),
        );
        if live && fan.rpm.is_none() {
            ui.label(
                RichText::new(t("RPM sensor unavailable"))
                    .small()
                    .color(colors.muted),
            );
        }
    });
    let response = ui.interact(
        result.response.rect,
        ui.id().with(fan.id),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            t(&fan.name),
        )
    });
    response.clicked()
}

fn telemetry(fan: &Fan, live: bool) -> String {
    if !live {
        return t("Reading paused").into();
    }
    let rpm = fan.rpm.map_or_else(
        || t("RPM sensor unavailable").into(),
        |rpm| format!("{rpm} RPM"),
    );
    format!("{rpm} · {:.0}%", percent(fan.duty))
}

fn editable_curve(points: &[Point], fan: &Fan) -> Vec<Point> {
    let mut result = points.to_vec();
    let critical = critical_temperature(&fan.curve);
    let mut previous_t = 20;
    let mut previous_d = fan.minimum;
    for point in &mut result {
        point.temperature = point.temperature.clamp(previous_t, critical);
        point.duty = point.duty.max(previous_d);
        previous_t = point.temperature;
        previous_d = point.duty;
    }
    if let Some(last) = result.last_mut() {
        last.duty = 255;
    }
    result
}

fn linear_curve(points: &[Point], fan: &Fan) -> Vec<Point> {
    let points = editable_curve(points, fan);
    let Some(first) = points.first() else {
        return points;
    };
    let last = points.last().unwrap();
    let n = points.len().saturating_sub(1).max(1);
    (0..points.len())
        .map(|i| Point {
            temperature: first.temperature
                + ((usize::from(last.temperature - first.temperature) * i) / n) as u8,
            duty: first.duty + ((usize::from(255 - first.duty) * i) / n) as u8,
        })
        .collect()
}

fn point_bounds(points: &[Point], index: usize, fan: &Fan) -> (u8, u8, u8, u8) {
    let previous = index.checked_sub(1).map(|i| points[i]);
    let next = points.get(index + 1);
    let critical = critical_temperature(&fan.curve);
    let max_t = next.map_or(critical, |p| p.temperature.clamp(20, critical));
    let min_t = previous.map_or(20, |p| p.temperature).clamp(20, max_t);
    let min_d = if next.is_none() {
        255
    } else {
        previous.map_or(fan.minimum, |p| p.duty)
    };
    let max_d = next.map_or(255, |p| p.duty).max(min_d);
    (min_t, max_t, min_d, max_d)
}

fn move_point(points: &mut [Point], index: usize, fan: &Fan, temperature: u8, duty: u8) {
    let (min_t, max_t, min_d, max_d) = point_bounds(points, index, fan);
    points[index] = Point {
        temperature: temperature.clamp(min_t, max_t),
        duty: duty.clamp(min_d, max_d),
    };
}

fn curve_chart(
    ui: &mut Ui,
    current: &[Point],
    preview: &mut [Point],
    fan: &Fan,
    editable: bool,
    height: f32,
) {
    let colors = palette(ui);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(40.0, 20.0), rect.max - vec2(18.0, 28.0));
    let position = |temperature: f32, duty: f32| {
        pos2(
            plot.left() + (temperature - 20.0) / 80.0 * plot.width(),
            plot.bottom() - duty / 100.0 * plot.height(),
        )
    };
    let mut hovered = None;
    if editable {
        for i in 0..preview.len() {
            let point = preview[i];
            let center = position(f32::from(point.temperature), percent(point.duty));
            let response = ui.interact(
                Rect::from_center_size(center, vec2(22.0, 22.0)),
                ui.id().with(("curve_point", fan.id, i)),
                egui::Sense::drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    f64::from(percent(point.duty)),
                    i18n::f(
                        "Point {number} fan speed",
                        &[("number", (i + 1).to_string())],
                    ),
                )
            });
            if response.dragged() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let temperature = (20.0 + (pointer.x - plot.left()) / plot.width() * 80.0)
                        .round()
                        .clamp(20.0, 100.0) as u8;
                    let duty = ((plot.bottom() - pointer.y) / plot.height() * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8;
                    move_point(preview, i, fan, temperature, raw_duty(duty).unwrap_or(255));
                }
            }
            if response.hovered() || response.dragged() {
                hovered = Some(i);
            }
            response
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text(format!(
                    "{} °C · {:.0}%",
                    preview[i].temperature,
                    percent(preview[i].duty)
                ));
        }
    }
    let p = ui.painter();
    for value in [0, 25, 50, 75, 100] {
        let y = position(0.0, value as f32).y;
        p.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0_f32, colors.border),
        );
        p.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{value}%"),
            FontId::proportional(9.0),
            colors.muted,
        );
    }
    for value in [20, 40, 60, 80, 100] {
        let x = position(value as f32, 0.0).x;
        p.text(
            pos2(x, plot.bottom() + 14.0),
            Align2::CENTER_CENTER,
            format!("{value}°"),
            FontId::proportional(9.0),
            colors.muted,
        );
    }
    for (points, color, width) in [
        (current, colors.muted, 1.5_f32),
        (&*preview, colors.accent, 2.5_f32),
    ] {
        if points.is_empty() {
            continue;
        }
        let mut line = vec![position(20.0, percent(points[0].duty))];
        line.extend(
            points
                .iter()
                .map(|v| position(f32::from(v.temperature.max(20)), percent(v.duty))),
        );
        line.push(position(100.0, percent(points.last().unwrap().duty)));
        p.add(egui::Shape::line(line, Stroke::new(width, color)));
    }
    for (i, point) in preview.iter().enumerate() {
        let center = position(f32::from(point.temperature.max(20)), percent(point.duty));
        p.circle_filled(center, if editable { 7.0 } else { 4.0 }, colors.accent);
        if editable {
            p.text(
                center,
                Align2::CENTER_CENTER,
                (i + 1).to_string(),
                FontId::proportional(9.0),
                colors.on_accent,
            );
        }
        if hovered == Some(i) {
            p.text(
                pos2(
                    center.x.clamp(plot.left() + 38.0, plot.right() - 38.0),
                    center.y - 14.0,
                ),
                Align2::CENTER_BOTTOM,
                format!("{} °C · {:.0}%", point.temperature, percent(point.duty)),
                FontId::proportional(12.0),
                colors.text,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencrate_fan::service::Profile;

    fn fan() -> Fan {
        let curve = [30, 50, 70, 85]
            .into_iter()
            .zip([51, 102, 179, 255])
            .map(|(temperature, duty)| Point { temperature, duty })
            .collect::<Vec<_>>();
        Fan {
            id: 1,
            name: "CPU Fan".into(),
            duty: 51,
            rpm: Some(780),
            minimum: 51,
            profiles: vec![Profile {
                index: 2,
                name: "Silent".into(),
                curve: curve.clone(),
            }],
            curve,
            can_restore: false,
            writable: true,
        }
    }

    #[test]
    fn curve_dragging_clamps_neighbors_minimum_and_final_full_speed() {
        let fan = fan();
        let mut points = fan.curve.clone();
        move_point(&mut points, 1, &fan, 100, 0);
        assert_eq!(
            points[1],
            Point {
                temperature: 70,
                duty: 51
            }
        );
        validate_custom(&points, &fan).unwrap();
        move_point(&mut points, 3, &fan, 10, 1);
        assert_eq!(
            points[3],
            Point {
                temperature: 70,
                duty: 255
            }
        );
        validate_custom(&points, &fan).unwrap();
        let ramp = linear_curve(&points, &fan);
        validate_custom(&ramp, &fan).unwrap();
        assert_eq!(ramp[0], points[0]);
        assert_eq!(ramp[3], points[3]);
    }

    #[test]
    fn external_thermal_limit_changes_do_not_panic_when_editing_old_draft() {
        let mut fan = fan();
        let mut points = fan.curve.clone();
        fan.curve.last_mut().unwrap().temperature = 60;
        move_point(&mut points, 3, &fan, 85, 128);
        assert_eq!(points.last().unwrap().temperature, 60);
        // Remaining stale points must be fixed before Apply can be enabled.
        assert!(validate_custom(&points, &fan).is_err());
        validate_custom(&editable_curve(&points, &fan), &fan).unwrap();
    }

    #[test]
    fn rpm_telemetry_distinguishes_a_stopped_fan_missing_sensor_and_offline() {
        let mut fan = fan();
        assert!(telemetry(&fan, true).contains("780 RPM"));
        fan.rpm = Some(0);
        assert!(telemetry(&fan, true).contains("0 RPM"));
        fan.rpm = None;
        assert!(!telemetry(&fan, true).contains("0 RPM"));
        assert!(!telemetry(&fan, false).contains("20%"));
    }

    #[test]
    #[ignore = "Writes hardware-free fan UI previews to OPENCRATE_FAN_PREVIEW_OUTPUT"]
    fn render_fan_editor_preview() {
        use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
        let output_dir = std::path::PathBuf::from(
            std::env::var_os("OPENCRATE_FAN_PREVIEW_OUTPUT").expect("preview output directory"),
        );
        std::fs::create_dir_all(&output_dir).unwrap();
        for (name, language, theme, width) in [
            (
                "fans-en-dark",
                i18n::Language::English,
                ThemePreference::Dark,
                1000,
            ),
            (
                "fans-tr-light",
                i18n::Language::Turkish,
                ThemePreference::Light,
                1000,
            ),
            (
                "fans-tr-compact",
                i18n::Language::Turkish,
                ThemePreference::Dark,
                680,
            ),
        ] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            theme.apply(&ctx);
            i18n::set_language(language);
            let scratch = tempfile::tempdir().unwrap();
            let mut store = Store::load_path(scratch.path().join("settings.json"));
            store.preferences.fan_groups.push(FanGroup {
                name: "Case fans".into(),
                members: vec![2, 3],
            });
            let fans = (1..=3)
                .map(|id| {
                    let mut fan = fan();
                    fan.id = id;
                    if id > 1 {
                        fan.name = format!("Chassis Fan {}", id - 1);
                    }
                    fan.rpm = match id {
                        1 => Some(780),
                        2 => Some(0),
                        _ => None,
                    };
                    fan
                })
                .collect::<Vec<_>>();
            let mut draft = Draft::new(&fans[0]);
            draft.mode = Mode::Curve;
            let mut state = State {
                controller: None,
                fans,
                draft: Some(draft),
                error: None,
                message: Message::text("Fan readings refreshed."),
                action_success: "Fan readings refreshed.",
                action_error: false,
                busy: false,
                can_undo: false,
                selected_group: Some(0),
                group_mode: GroupMode::Preset(QuickMode::Silent),
                group_duty: 60,
                reset_draft: false,
            };
            let height = 1580;
            let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Rgba);
            let mut pixels = vec![0u8; width * height * 4];
            for _ in 0..3 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            pos2(0.0, 0.0),
                            vec2(width as f32, height as f32),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                state.show(ui, &mut store);
                            });
                        });
                    },
                );
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                renderer.render(
                    &mut BufferMutRef::new(bytemuck::cast_slice_mut(&mut pixels), width, height),
                    &primitives,
                    &output.textures_delta,
                    output.pixels_per_point,
                );
            }
            image::save_buffer(
                output_dir.join(format!("{name}.png")),
                &pixels,
                width as u32,
                height as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        i18n::set_language(i18n::Language::English);
    }
}
