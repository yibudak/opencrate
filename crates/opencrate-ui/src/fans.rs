//! Fan dashboard. Draft edits never write hardware until Apply is pressed.

use crate::{
    dashboard::page_header,
    i18n::{self, t, Message},
    theme::*,
};
use eframe::egui::{self, pos2, vec2, Align, Align2, FontId, Layout, Rect, RichText, Stroke, Ui};
use opencrate_fan::quick::{self, QuickMode};
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
}

impl State {
    pub fn status(&self) -> (&str, String, egui::Color32) {
        if let Some(error) = &self.error {
            (
                t("OFFLINE"),
                i18n::f(
                    "Fan control failed: {details}",
                    &[("details", t(error).to_string())],
                ),
                RED,
            )
        } else if self.busy {
            (t("APPLYING"), self.message.render(), ACCENT)
        } else if self.action_error {
            (t("NEEDS ATTENTION"), self.message.render(), RED)
        } else {
            (t("FANS"), self.message.render(), GREEN)
        }
    }
    pub fn new(ctx: &egui::Context) -> Self {
        let wake = ctx.clone();
        let controller = Controller::start(move || wake.request_repaint());
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
                    }
                    self.fans = fans;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn send(&mut self, command: Command) {
        let success = match &command {
            Command::Refresh => "Fan readings refreshed.",
            Command::Apply {
                target: Target::Restore,
                ..
            }
            | Command::UndoQuick => "Previous fan settings restored.",
            Command::Apply { .. } => "Fan settings applied.",
            Command::Quick(_) => "All fan settings applied.",
            Command::Stop => return,
        };
        let refreshing = matches!(command, Command::Refresh);
        let quick_action = matches!(command, Command::Quick(_) | Command::UndoQuick);
        let result = self
            .controller
            .as_ref()
            .ok_or("Fan worker is unavailable.".into())
            .and_then(|c| c.send(command));
        match result {
            Ok(()) => {
                if quick_action {
                    if let Some(draft) = &mut self.draft {
                        draft.mode = Mode::Current;
                    }
                }
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

    pub fn show(&mut self, ui: &mut Ui) {
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
            if self.error.is_some() { RED } else { GREEN },
        );
        if let Some(error) = &self.error {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                subtitle(ui, t("Fan service unavailable"));
                ui.colored_label(RED, i18n::f("Details: {details}", &[("details", t(error).to_string())]));
                ui.label(RichText::new(t("OpenCrate uses the installed ASUS fan service. Previous readings are paused until it responds.")).color(MUTED));
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
                        .selectable_label(selected == Some(fan.id), t(&fan.name))
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
        let Some(draft) = &mut self.draft else {
            return;
        };
        let Some(fan) = self.fans.iter().find(|f| f.id == draft.id).cloned() else {
            return;
        };
        let enabled = self.error.is_none() && !self.busy && fan.writable;
        if !fan.writable {
            ui.colored_label(RED, t("This fan uses an unsupported curve or RPM control mode. Readings remain available."));
        }
        let mut command = None;
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                subtitle(ui, t(&fan.name));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(if self.error.is_none() { i18n::f("{duty}% output", &[("duty", format!("{:.0}", percent(fan.duty)))]) } else { t("Reading paused").into() }).color(ACCENT));
                });
            });
            ui.add_space(6.0);
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label(t("Cooling mode"));
                    let previous = draft.mode;
                    egui::ComboBox::from_id_salt("fan_mode").selected_text(t(&draft.label(&fan))).width(210.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.mode, Mode::Current, t("Current settings"));
                        for profile in &fan.profiles { ui.selectable_value(&mut draft.mode, Mode::Profile(profile.index), t(&profile.name)); }
                        ui.selectable_value(&mut draft.mode, Mode::Manual, t("Manual speed"));
                        ui.selectable_value(&mut draft.mode, Mode::Curve, t("Custom curve"));
                    });
                    if previous != draft.mode && draft.mode == Mode::Curve {
                        draft.points = fan.curve.clone();
                        let critical = critical_temperature(&fan.curve);
                        for p in &mut draft.points { p.duty = p.duty.max(fan.minimum); p.temperature = p.temperature.clamp(20, critical); }
                        if let Some(p) = draft.points.last_mut() { p.duty = 255; }
                    }
                });
                match draft.mode {
                    Mode::Manual => {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(t("Fan speed"));
                            ui.label(RichText::new(format!("{}%", draft.duty)).color(ACCENT).strong());
                        });
                        ui.spacing_mut().slider_width = ui.available_width() - 8.0;
                        let response = ui.scope(|ui| {
                            ui.spacing_mut().interact_size.y = 22.0;
                            ui.add(egui::Slider::new(&mut draft.duty, (percent(fan.minimum).ceil() as u8)..=100).show_value(false))
                        }).inner;
                        response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(draft.duty), t("Fan speed")));
                        ui.label(RichText::new(i18n::f("Holds the selected speed at lower temperatures, then rises to 100% at {temperature} °C.", &[("temperature", critical_temperature(&fan.curve).to_string())])).small().color(MUTED));
                    },
                    Mode::Curve => {
                        ui.label(RichText::new(t("Edit the temperature and speed at each point. Changes apply together.")).small().color(MUTED));
                        egui::Grid::new("fan_curve_points").num_columns(3).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                            eyebrow(ui, t("POINT")); eyebrow(ui, t("TEMPERATURE")); eyebrow(ui, t("FAN SPEED")); ui.end_row();
                            let last = draft.points.len() - 1;
                            for (i, point) in draft.points.iter_mut().enumerate() {
                                ui.label(format!("{:02}", i + 1));
                                let r = ui.add(egui::DragValue::new(&mut point.temperature).range(20..=critical_temperature(&fan.curve)).suffix(" °C"));
                                r.widget_info(|| egui::WidgetInfo::slider(true, f64::from(point.temperature), i18n::f("Point {number} temperature", &[("number", (i+1).to_string())])));
                                let mut duty = percent(point.duty).round() as u8;
                                let r = ui.add_enabled(i != last, egui::DragValue::new(&mut duty).range((percent(fan.minimum).ceil() as u8)..=100).suffix("%"));
                                r.widget_info(|| egui::WidgetInfo::slider(i != last, f64::from(duty), i18n::f("Point {number} fan speed", &[("number", (i+1).to_string())])));
                                if r.changed() { point.duty = raw_duty(duty).unwrap_or(255); }
                                ui.end_row();
                            }
                        });
                    },
                    Mode::Current => { ui.label(RichText::new(t("Live controller curve. Choose a mode to make changes.")).color(MUTED)); },
                    Mode::Profile(_) => { ui.label(RichText::new(t("Temperature-based profile supplied by your ASUS fan controller.")).color(MUTED)); },
                }
            });
            ui.add_space(12.0);
            let curve = draft.curve(&fan);
            let preview = if draft.mode == Mode::Curve { &draft.points } else { curve.as_ref().unwrap_or(&fan.curve) };
            curve_chart(ui, &fan.curve, preview, if draft.mode == Mode::Manual { 112.0 } else { 140.0 });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("Preview")).small().color(ACCENT));
                ui.label(RichText::new(t("Current curve")).small().color(MUTED));
                ui.label(RichText::new(i18n::f("Minimum manual speed: {duty}%", &[("duty", format!("{:.0}", percent(fan.minimum).ceil()))])).small().color(MUTED));
            });
            if let Err(error) = &curve { ui.colored_label(RED, i18n::f("Details: {details}", &[("details", t(error).to_string())])); }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let changed = curve.as_ref().is_ok_and(|c| *c != fan.curve);
                if ui.add_enabled(enabled && changed && draft.mode != Mode::Current,
                    egui::Button::new(RichText::new(t("Apply to this fan")).color(BACKGROUND).strong()).fill(ACCENT).min_size(vec2(160.0, 40.0))).clicked() {
                    let target = match draft.mode { Mode::Profile(i) => Target::Profile(i), _ => Target::Custom(curve.clone().unwrap()) };
                    command = Some(Command::Apply { id: fan.id, target });
                }
                if ui.add_enabled(enabled && fan.can_restore, egui::Button::new(t("Restore original")).min_size(vec2(140.0, 40.0))).clicked() {
                    command = Some(Command::Apply { id: fan.id, target: Target::Restore });
                    draft.mode = Mode::Current;
                }
            });
        });
        if let Some(command) = command {
            self.send(command);
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(t("Active in the tray · Original curves restored on Quit"))
                .small()
                .color(MUTED),
        );
        ui.label(RichText::new(t("ASUS fan service required. Output is duty percentage; RPM readings are unavailable.")).small().color(MUTED));
    }

    fn quick_controls(&mut self, ui: &mut Ui) {
        let enabled = self.error.is_none() && !self.busy;
        let full_blast = self
            .fans
            .iter()
            .all(|f| f.curve.iter().all(|p| p.duty == 255));
        let mut command = None;
        eyebrow(ui, t("ALL FANS · ONE CLICK"));
        ui.horizontal_wrapped(|ui| {
            for mode in [
                QuickMode::FullBlast,
                QuickMode::Silent,
                QuickMode::Standard,
                QuickMode::Turbo,
            ] {
                let preflight = quick::plan(&self.fans, mode);
                let mut button = egui::Button::new(t(mode.label())).min_size(vec2(80.0, 36.0));
                if mode == QuickMode::FullBlast {
                    button = egui::Button::new(
                        RichText::new(if full_blast {
                            t("Full Blast · ON")
                        } else {
                            t("Full Blast")
                        })
                        .strong()
                        .color(BACKGROUND),
                    )
                    .fill(ACCENT)
                    .min_size(vec2(120.0, 36.0));
                }
                let response = ui.add_enabled(
                    enabled
                        && preflight.as_ref().is_ok_and(|p| !p.is_empty())
                        && !(mode == QuickMode::FullBlast && full_blast),
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
                    Ok(_) => {
                        response.on_hover_text(if mode == QuickMode::FullBlast {
                            t("Immediately set every fan to 100%. Use Undo quick action to return.")
                                .into()
                        } else {
                            i18n::f(
                                "Apply {profile} to every fan immediately.",
                                &[("profile", t(mode.label()).to_string())],
                            )
                        });
                    }
                }
            }
            if ui
                .add_enabled(
                    enabled && self.can_undo,
                    egui::Button::new(t("Undo quick action")).min_size(vec2(138.0, 36.0)),
                )
                .on_hover_text(t(
                    "Restore the exact settings from before the last quick action.",
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
    let frame = egui::Frame::new()
        .fill(if selected { ACCENT_DIM } else { SURFACE })
        .stroke(Stroke::new(1.0_f32, if selected { ACCENT } else { BORDER }))
        .corner_radius(12)
        .inner_margin(12);
    let result = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(t(&fan.name)).size(13.0).strong());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::hover());
            icon(ui, Icon::Fan, rect, if selected { ACCENT } else { MUTED });
            ui.label(
                RichText::new(if live {
                    format!("{:.0}%", percent(fan.duty))
                } else {
                    "—".into()
                })
                .size(25.0)
                .strong(),
            );
        });
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

fn curve_chart(ui: &mut Ui, current: &[Point], preview: &[Point], height: f32) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(34.0, 10.0), rect.max - vec2(16.0, 24.0));
    let position = |temperature: f32, duty: f32| {
        pos2(
            plot.left() + temperature / 100.0 * plot.width(),
            plot.bottom() - duty / 100.0 * plot.height(),
        )
    };
    let p = ui.painter();
    for value in [0, 50, 100] {
        let y = position(0.0, value as f32).y;
        p.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0_f32, BORDER),
        );
        p.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{value}%"),
            FontId::proportional(9.0),
            MUTED,
        );
    }
    for value in [0, 20, 40, 60, 80, 100] {
        let x = position(value as f32, 0.0).x;
        p.text(
            pos2(x, plot.bottom() + 14.0),
            Align2::CENTER_CENTER,
            format!("{value}°"),
            FontId::proportional(9.0),
            MUTED,
        );
    }
    for (points, color, width) in [(current, MUTED, 1.5_f32), (preview, ACCENT, 2.5_f32)] {
        if points.is_empty() {
            continue;
        }
        let mut line = vec![position(0.0, percent(points[0].duty))];
        line.extend(
            points
                .iter()
                .map(|v| position(f32::from(v.temperature), percent(v.duty))),
        );
        line.push(position(100.0, percent(points.last().unwrap().duty)));
        p.add(egui::Shape::line(line, Stroke::new(width, color)));
    }
    for point in preview {
        p.circle_filled(
            position(f32::from(point.temperature), percent(point.duty)),
            4.0,
            ACCENT,
        );
    }
}
