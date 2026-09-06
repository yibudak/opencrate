//! Power page. Plan activation is immediate; processor controls use an explicit draft.
use crate::{
    dashboard::page_header,
    i18n::{self, t, Message},
    theme::*,
};
use egui::{vec2, RichText, Ui};
use opencrate_power::{
    service::{Command, Controller},
    *,
};

struct Draft {
    plan: PlanId,
    source: Source,
    controls: Vec<Control>,
    expected: Vec<(Setting, u32)>,
}
impl Draft {
    fn new(snapshot: &Snapshot, source: Source) -> Self {
        let controls = snapshot.cpu[source.index()].controls.clone();
        let expected = values(&controls);
        Self {
            plan: snapshot.active,
            source,
            controls,
            expected,
        }
    }
    fn edit(&self) -> Edit {
        Edit {
            plan: self.plan,
            source: self.source,
            expected: self.expected.clone(),
            values: values(&self.controls),
        }
    }
    fn dirty(&self) -> bool {
        values(&self.controls) != self.expected
    }
    fn stale(&self, snapshot: &Snapshot) -> bool {
        self.plan != snapshot.active
            || self.expected != values(&snapshot.cpu[self.source.index()].controls)
    }
}

pub struct State {
    controller: Option<Controller>,
    snapshot: Option<Snapshot>,
    draft: Option<Draft>,
    selected_plan: Option<PlanId>,
    source: Source,
    error: Option<String>,
    message: Message,
    action_success: &'static str,
    action_error: bool,
    busy: bool,
}
impl State {
    pub fn new(ctx: &egui::Context) -> Self {
        let wake = ctx.clone();
        let result = if crate::runtime::hardware_enabled() {
            Controller::start(move || wake.request_repaint())
        } else {
            Err(std::io::Error::other("Diagnostic mode"))
        };
        let error = result.as_ref().err().map(ToString::to_string);
        Self {
            controller: result.ok(),
            snapshot: None,
            draft: None,
            selected_plan: None,
            source: Source::Ac,
            error,
            message: Message::text("Reading Windows power settings…"),
            action_success: "Windows power settings refreshed.",
            action_error: false,
            busy: false,
        }
    }
    pub fn status(&self) -> (&str, String, egui::Color32) {
        if let Some(error) = &self.error {
            (
                t("UNAVAILABLE"),
                i18n::f(
                    "Power control failed: {details}",
                    &[("details", t(error).to_string())],
                ),
                RED,
            )
        } else if self.busy {
            (t("APPLYING"), self.message.render(), ACCENT)
        } else if self.action_error {
            (t("NEEDS ATTENTION"), self.message.render(), RED)
        } else {
            (t("POWER"), self.message.render(), GREEN)
        }
    }
    pub fn poll(&mut self) {
        let Some(controller) = &self.controller else {
            return;
        };
        while let Some(event) = controller.try_event() {
            let applied = event.action.as_ref().is_some_and(|a| a.is_ok());
            if let Some(action) = event.action {
                self.busy = false;
                self.action_error = action.is_err();
                self.message = match action {
                    Ok(_) => Message::text(self.action_success),
                    Err(error) => {
                        Message::with("Power control failed: {details}", vec![("details", error)])
                    }
                };
            }
            match event.result {
                Ok(snapshot) => {
                    self.error = None;
                    if self.snapshot.is_none() {
                        if !self.action_error && !self.busy {
                            self.message = Message::text("Windows power settings refreshed.");
                        }
                        self.source = snapshot.source.unwrap_or(Source::Ac);
                    }
                    if applied
                        || self
                            .selected_plan
                            .is_none_or(|id| !snapshot.plans.iter().any(|p| p.id == id))
                        || self.snapshot.as_ref().is_some_and(|previous| {
                            self.selected_plan == Some(previous.active)
                                && previous.active != snapshot.active
                        })
                    {
                        self.selected_plan = Some(snapshot.active);
                    }
                    if applied || self.draft.as_ref().is_none_or(|d| !d.dirty()) {
                        self.draft = Some(Draft::new(&snapshot, self.source));
                    }
                    self.snapshot = Some(snapshot);
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
    fn send(&mut self, command: Command) {
        let success = match &command {
            Command::Refresh => "Windows power settings refreshed.",
            Command::Activate { .. } => "Power plan activated.",
            Command::Apply(_) => "Processor settings applied.",
            Command::Undo => "Previous power settings restored.",
            Command::Stop => return,
        };
        match self
            .controller
            .as_ref()
            .ok_or("Power worker is unavailable.".into())
            .and_then(|c| c.send(command))
        {
            Ok(()) => {
                self.busy = true;
                self.action_success = success;
                self.action_error = false;
                self.message = Message::text("Updating Windows power settings…");
            }
            Err(error) => {
                self.action_error = true;
                self.message =
                    Message::with("Power control failed: {details}", vec![("details", error)]);
            }
        }
    }
    pub fn show(&mut self, ui: &mut Ui) {
        page_header(
            ui,
            t("Power"),
            t("Balance everyday efficiency and performance."),
            if self.error.is_some() {
                t("UNAVAILABLE")
            } else if self.snapshot.is_none() {
                t("CONNECTING")
            } else {
                t("WINDOWS CONNECTED")
            },
            if self.error.is_some() { RED } else { GREEN },
        );
        if let Some(error) = &self.error {
            ui.colored_label(
                RED,
                i18n::f("Details: {details}", &[("details", t(error).to_string())]),
            );
            if ui
                .add_enabled(!self.busy, egui::Button::new(t("Refresh power settings")))
                .clicked()
            {
                self.send(Command::Refresh);
            }
        }
        let Some(snapshot) = self.snapshot.clone() else {
            if self.error.is_none() {
                ui.spinner();
            }
            return;
        };
        let enabled = !self.busy && self.error.is_none();
        let mut command = None;
        card().inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal_wrapped(|ui| {
                eyebrow(ui, t("ACTIVE POWER PLAN"));
                badge(
                    ui,
                    snapshot.source.map_or(t("SOURCE UNKNOWN"), |s| {
                        if s == Source::Ac {
                            t("PLUGGED IN")
                        } else {
                            t("ON BATTERY")
                        }
                    }),
                    GREEN,
                );
                if let Some(percent) = snapshot.battery_percent {
                    ui.label(
                        RichText::new(i18n::f(
                            "Battery {percent}%",
                            &[("percent", percent.to_string())],
                        ))
                        .color(MUTED)
                        .size(12.0),
                    );
                }
            });
            ui.label(
                RichText::new(
                    snapshot
                        .plans
                        .iter()
                        .find(|p| p.id == snapshot.active)
                        .map_or(t("Unavailable"), i18n::plan_name),
                )
                .size(25.0)
                .strong(),
            );
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                for (id, label, tip) in [
                    (
                        POWER_SAVER,
                        t("Power saver"),
                        t("Activate the Windows Power saver plan."),
                    ),
                    (
                        BALANCED,
                        t("Balanced"),
                        t("Activate the Windows Balanced plan."),
                    ),
                    (
                        HIGH_PERFORMANCE,
                        t("High performance"),
                        t("Activate the Windows High performance plan."),
                    ),
                ] {
                    let exists = snapshot.plans.iter().any(|p| p.id == id);
                    let active = snapshot.active == id;
                    let button = egui::Button::new(RichText::new(label).color(if active {
                        ACCENT
                    } else {
                        TEXT
                    }))
                    .fill(if active { ACCENT_DIM } else { INPUT })
                    .min_size(vec2(116.0, 36.0));
                    if ui
                        .add_enabled(enabled && exists, button)
                        .on_hover_text(if exists {
                            tip
                        } else {
                            t("This plan is not installed in Windows.")
                        })
                        .clicked()
                        && !active
                    {
                        command = Some(Command::Activate {
                            expected: snapshot.active,
                            plan: id,
                        });
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt("windows_power_plan")
                        .width(235.0)
                        .selected_text(
                            snapshot
                                .plans
                                .iter()
                                .find(|p| Some(p.id) == self.selected_plan)
                                .map_or(t("Choose a plan"), i18n::plan_name),
                        )
                        .show_ui(ui, |ui| {
                            for plan in &snapshot.plans {
                                ui.selectable_value(
                                    &mut self.selected_plan,
                                    Some(plan.id),
                                    i18n::plan_name(plan),
                                );
                            }
                        });
                });
                if ui
                    .add_enabled(
                        enabled && self.selected_plan.is_some_and(|p| p != snapshot.active),
                        egui::Button::new(t("Activate plan")),
                    )
                    .clicked()
                {
                    command = self.selected_plan.map(|plan| Command::Activate {
                        expected: snapshot.active,
                        plan,
                    });
                }
                if ui
                    .add_enabled(
                        enabled && snapshot.can_undo,
                        egui::Button::new(t("Undo last change")),
                    )
                    .clicked()
                {
                    command = Some(Command::Undo);
                }
            });
        });
        ui.add_space(12.0);
        card().inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal_wrapped(|ui| {
                subtitle(ui, t("Processor behavior"));
                let dirty = self.draft.as_ref().is_some_and(|d| d.dirty());
                badge(ui, if dirty { t("UNAPPLIED CHANGES") } else { t("CURRENT SETTINGS") }, if dirty { ACCENT } else { MUTED });
            });
            let old_source = self.source;
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().button_padding.y = 5.0;
                ui.spacing_mut().interact_size.y = 28.0;
                ui.horizontal_wrapped(|ui| {
                    // Preserve a pending draft until explicitly applied or discarded.
                    ui.add_enabled_ui(self.draft.as_ref().is_none_or(|d| !d.dirty()), |ui| {
                        ui.selectable_value(&mut self.source, Source::Ac, t("Plugged in"));
                        ui.selectable_value(&mut self.source, Source::Dc, t("On battery"));
                    }).response.on_hover_text(t("Apply your edits or reload current values before changing the power source."));
                    ui.label(RichText::new(t("Settings for the active Windows plan")).size(12.0).color(MUTED));
                });
            });
            if old_source != self.source { self.draft = Some(Draft::new(&snapshot, self.source)); }
            let Some(draft) = &mut self.draft else { return; };
            let stale = draft.stale(&snapshot);
            if stale { ui.colored_label(ACCENT, t("Windows settings changed. Reload current values to continue.")); }
            ui.add_space(3.0);
            ui.add_enabled_ui(enabled && !stale, |ui| {
                ui.columns(2, |columns| {
                    for (column, key) in columns.iter_mut().zip([Setting::Minimum, Setting::Maximum]) {
                        if let Some(control) = draft.controls.iter_mut().find(|c| c.key == key) { range(column, control); }
                    }
                });
                ui.label(RichText::new(t("These percentages request a processor performance range; they are not watt limits.")).size(12.0).color(MUTED));
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    if let Some(control) = draft.controls.iter_mut().find(|c| c.key == Setting::Boost) {
                        ui.label(t("CPU boost mode"));
                        let text = if let Allowed::Choices(choices) = &control.allowed {
                            choices.iter().find(|(v, _)| *v == control.value).map(|(_, n)| n.clone()).unwrap_or_else(|| i18n::f("Current mode ({mode})", &[("mode", control.value.to_string())]))
                        } else { t("Unavailable").into() };
                        ui.add_enabled_ui(control.write_error.is_none(), |ui| {
                            egui::ComboBox::from_id_salt("cpu_boost_mode").selected_text(t(&text)).width(225.0).show_ui(ui, |ui| {
                                if let Allowed::Choices(choices) = &control.allowed {
                                    for (value, name) in choices { ui.selectable_value(&mut control.value, *value, t(name)); }
                                }
                            });
                        }).response.on_hover_text(control.write_error.as_deref().unwrap_or(t("Controls boost above the processor's nominal performance level.")));
                    }
                });
                if let Some(control) = draft.controls.iter_mut().find(|c| c.key == Setting::EnergyPreference) {
                    range(ui, control);
                    ui.label(RichText::new(t("0% favors performance · 100% favors energy savings")).size(12.0).color(MUTED));
                }
            });
            let dirty = draft.dirty();
            let edit = draft.edit();
            let validation = validate(&snapshot.cpu[self.source.index()].controls, &edit);
            if dirty && !stale {
                if let Err(error) = &validation { ui.colored_label(RED, i18n::f("Details: {details}", &[("details", t(error).to_string())])); }
            }
            ui.add_space(5.0);
            let mut reload = false;
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(enabled && dirty && !stale && validation.is_ok(), egui::Button::new(RichText::new(t("Apply processor settings")).color(BACKGROUND).strong()).fill(ACCENT)).clicked() {
                    command = Some(Command::Apply(edit));
                }
                if ui.add_enabled(enabled && (dirty || stale), egui::Button::new(t("Reload current values"))).clicked() { reload = true; }
            });
            if reload { self.draft = Some(Draft::new(&snapshot, self.source)); }
            for error in &snapshot.cpu[self.source.index()].unavailable { ui.label(RichText::new(error).size(12.0).color(MUTED)); }
        });
        ui.add_space(8.0);
        ui.label(RichText::new(t("Power settings stay active after quitting OpenCrate and restarting Windows. Undo is available for the last change in this session.")).size(12.0).color(MUTED));
        ui.label(
            RichText::new(t(
                "Windows power mode and firmware can influence the resulting CPU behavior.",
            ))
            .size(12.0)
            .color(MUTED),
        );
        if let Some(command) = command {
            self.send(command);
        }
    }
}

fn range(ui: &mut Ui, control: &mut Control) {
    ui.label(RichText::new(t(control.key.label())).strong().size(13.0));
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 78.0).clamp(70.0, 440.0);
        ui.spacing_mut().interact_size.y = 24.0;
        ui.spacing_mut().button_padding.y = 3.0;
        ui.add_enabled_ui(control.write_error.is_none(), |ui| {
            if let Allowed::Range { min, max, step } = control.allowed {
                let response = ui.add(
                    egui::Slider::new(&mut control.value, min..=max)
                        .suffix("%")
                        .step_by(step as f64)
                        .clamping(egui::SliderClamping::Never),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::slider(
                        ui.is_enabled(),
                        control.value.into(),
                        t(control.key.label()),
                    )
                });
            }
        })
        .response
        .on_hover_text(
            control
                .write_error
                .as_deref()
                .unwrap_or(t("Edit the value, then apply processor settings.")),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            plans: vec![],
            active: BALANCED,
            cpu: [
                CpuSettings {
                    controls: vec![Control {
                        key: Setting::Maximum,
                        value: 100,
                        allowed: Allowed::Range {
                            min: 0,
                            max: 100,
                            step: 1,
                        },
                        write_error: None,
                    }],
                    unavailable: vec![],
                },
                CpuSettings::default(),
            ],
            source: Some(Source::Ac),
            battery_percent: None,
            can_undo: false,
        }
    }
    #[test]
    fn draft_edits_are_isolated_and_external_changes_are_detected() {
        let mut snapshot = snapshot();
        let mut draft = Draft::new(&snapshot, Source::Ac);
        draft.controls[0].value = 90;
        assert!(draft.dirty());
        assert!(!draft.stale(&snapshot));
        assert_eq!(snapshot.cpu[0].controls[0].value, 100);
        snapshot.cpu[0].controls[0].value = 80;
        assert!(draft.stale(&snapshot));
        assert!(validate(&snapshot.cpu[0].controls, &draft.edit()).is_err());
    }
}
