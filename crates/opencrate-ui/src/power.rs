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
use std::collections::VecDeque;

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
    monitor: Option<telemetry::Monitor>,
    reading: telemetry::Reading,
    history: VecDeque<telemetry::Reading>,
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
            monitor: None,
            reading: telemetry::Reading::default(),
            history: VecDeque::new(),
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
    pub fn status(&self, colors: Palette) -> (&str, String, egui::Color32) {
        if let Some(error) = &self.error {
            (
                t("UNAVAILABLE"),
                i18n::f(
                    "Power control failed: {details}",
                    &[("details", t(error).to_string())],
                ),
                colors.red,
            )
        } else if self.busy {
            (t("APPLYING"), self.message.render(), colors.accent)
        } else if self.action_error {
            (t("NEEDS ATTENTION"), self.message.render(), colors.red)
        } else {
            (t("POWER"), self.message.render(), colors.green)
        }
    }
    pub fn poll(&mut self, visible: bool) {
        if let Some(monitor) = &mut self.monitor {
            monitor.set_active(visible);
            if let Some(reading) = monitor.take() {
                if self.history.len() == 60 {
                    self.history.pop_front();
                }
                self.history.push_back(reading.clone());
                self.reading = reading;
            }
        }
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
                        self.source = if snapshot.has_battery {
                            snapshot.source.unwrap_or(Source::Ac)
                        } else {
                            Source::Ac
                        };
                    }
                    if !snapshot.has_battery && self.source != Source::Ac {
                        self.source = Source::Ac;
                        self.draft = Some(Draft::new(&snapshot, Source::Ac));
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
    fn show_telemetry(&self, ui: &mut Ui) {
        let colors = palette(ui);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
        card(ui).inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                subtitle(ui, t("Live CPU metrics"));
                badge(ui, if self.reading.fresh() { t("LIVE") } else { t("WAITING FOR DATA") },
                    if self.reading.fresh() { colors.green } else { colors.muted });
                if !self.reading.cpu_name.is_empty() {
                    ui.label(RichText::new(&self.reading.cpu_name).size(12.0).color(colors.muted));
                }
            });
            ui.add_space(8.0);
            type Metric = (&'static str, &'static str, fn(&telemetry::Reading) -> Option<f64>);
            let metrics: [Metric; 4] = [
                ("CPU package power", "W", |r| r.power_w),
                ("CPU temperature", "°C", |r| r.temperature_c),
                ("CPU frequency", "GHz", |r| r.frequency_mhz.map(|v| v / 1000.0)),
                ("CPU usage", "%", |r| r.load_percent),
            ];
            let columns = if ui.available_width() < 560.0 { 2 } else { 4 };
            for group in metrics.chunks(columns) {
                ui.columns(columns, |uis| {
                    for (ui, (label, unit, value)) in uis.iter_mut().zip(group) {
                        ui.label(RichText::new(t(label)).size(12.0).color(colors.muted));
                        let current = self.reading.fresh().then(|| value(&self.reading)).flatten();
                        ui.label(RichText::new(current.map_or_else(|| "—".into(), |v| {
                            if *unit == "GHz" { format!("{v:.2} {unit}") } else { format!("{v:.1} {unit}") }
                        })).size(24.0).strong());
                        sparkline(ui, &self.history, *value);
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("Updates every second · last 60 seconds")).size(11.0).color(colors.muted));
                if let Some(provider) = self.reading.provider {
                    ui.label(RichText::new(provider).size(11.0).color(colors.muted));
                }
            });
            if self.reading.windows_frequency {
                ui.label(RichText::new(t("Frequency is estimated from Windows performance counters.")).size(11.0).color(colors.muted));
            } else if self.reading.frequency_mhz.is_some() {
                ui.label(RichText::new(t("Frequency is the average reported CPU core clock.")).size(11.0).color(colors.muted));
            }
            if self.reading.power_w.is_none() || self.reading.temperature_c.is_none() {
                ui.add_space(4.0);
                ui.label(t("For CPU power and temperature, run LibreHardwareMonitor or OpenHardwareMonitor as administrator with WMI enabled. OpenCrate connects automatically."));
                ui.hyperlink_to(t("Get LibreHardwareMonitor"), "https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases");
            }
            if let Some(error) = &self.reading.error {
                ui.collapsing(t("Sensor details"), |ui| { ui.label(error); });
            }
        });
    }
    fn show_ryzen(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        card(ui).inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            subtitle(ui, t("AMD Ryzen tuning"));
            ui.label(t("PBO, Curve Optimizer and Eco Mode are configured in Ryzen Master or your UEFI/BIOS. AMD's public SDK provides monitoring only; direct tuning is unavailable here."));
            ui.add_space(6.0);
            egui::Grid::new("ryzen_controls").num_columns(2).spacing(vec2(20.0, 6.0)).show(ui, |ui| {
                for (label, description) in [
                    ("Precision Boost Overdrive", "Adjust PPT, TDC and EDC limits in Ryzen Master."),
                    ("Curve Optimizer", "Tune the per-core voltage/frequency curve in Ryzen Master."),
                    ("AMD Eco Mode", "Use AMD's CPU power limits in Ryzen Master; Windows Quiet is a separate policy."),
                ] {
                    ui.label(RichText::new(t(label)).strong());
                    ui.add(egui::Label::new(t(description)).wrap());
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for (label, value, unit) in [
                    ("Core voltage", self.reading.voltage_v, "V"),
                    ("TDC", self.reading.tdc_a, "A"),
                    ("EDC", self.reading.edc_a, "A"),
                ] {
                    if let Some(value) = value.filter(|_| self.reading.fresh()) {
                        ui.label(RichText::new(format!("{}: {value:.2} {unit}", t(label))).size(12.0).color(colors.muted));
                    }
                }
            });
            if let Some(path) = &self.reading.ryzen_master {
                if ui.button(t("Open Ryzen Master")).clicked() {
                    if let Err(error) = launch_ryzen_master(path) {
                        self.message = Message::with("Power control failed: {details}", vec![("details", error)]);
                        self.action_error = true;
                    }
                }
            } else {
                ui.hyperlink_to(t("Get Ryzen Master"), "https://www.amd.com/en/products/software/ryzen-master.html");
            }
        });
    }
    fn send(&mut self, command: Command) {
        let success = match &command {
            Command::Refresh => "Windows power settings refreshed.",
            Command::Activate { .. } => "Power plan activated.",
            Command::ActivateUltimate { .. } => "Power plan activated.",
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
        if self.monitor.is_none() && crate::runtime::hardware_enabled() && !cfg!(test) {
            let wake = ui.ctx().clone();
            match telemetry::Monitor::start(move || wake.request_repaint()) {
                Ok(monitor) => self.monitor = Some(monitor),
                Err(error) => self.reading.error = Some(error.to_string()),
            }
        }
        let colors = palette(ui);
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
            if self.error.is_some() {
                colors.red
            } else {
                colors.green
            },
        );
        if let Some(error) = &self.error {
            ui.colored_label(
                colors.red,
                i18n::f("Details: {details}", &[("details", t(error).to_string())]),
            );
            if ui
                .add_enabled(!self.busy, egui::Button::new(t("Refresh power settings")))
                .clicked()
            {
                self.send(Command::Refresh);
            }
        }
        self.show_telemetry(ui);
        ui.add_space(12.0);
        let Some(snapshot) = self.snapshot.clone() else {
            if self.error.is_none() {
                ui.spinner();
            }
            return;
        };
        let enabled = !self.busy && self.error.is_none();
        let mut command = None;
        card(ui).inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal_wrapped(|ui| {
                eyebrow(ui, t("ACTIVE POWER PLAN"));
                if snapshot.has_battery {
                badge(
                    ui,
                    snapshot.source.map_or(t("SOURCE UNKNOWN"), |s| {
                        if s == Source::Ac {
                            t("PLUGGED IN")
                        } else {
                            t("ON BATTERY")
                        }
                    }),
                    colors.green,
                );
                if let Some(percent) = snapshot.battery_percent {
                    ui.label(
                        RichText::new(i18n::f(
                            "Battery {percent}%",
                            &[("percent", percent.to_string())],
                        ))
                        .color(colors.muted)
                        .size(12.0),
                    );
                }
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
                    (
                        snapshot.ultimate_plan.unwrap_or(ULTIMATE_PERFORMANCE),
                        t("Ultimate performance"),
                        t("Activate Ultimate Performance, installing the Windows template if needed."),
                    ),
                ] {
                    let ultimate = id == snapshot.ultimate_plan.unwrap_or(ULTIMATE_PERFORMANCE);
                    let exists = snapshot.plans.iter().any(|p| p.id == id);
                    let active = snapshot.active == id;
                    let button = egui::Button::new(RichText::new(label).color(if active {
                        colors.accent
                    } else {
                        colors.text
                    }))
                    .fill(if active {
                        colors.accent_dim
                    } else {
                        colors.input
                    })
                    .min_size(vec2(116.0, 36.0));
                    if ui
                        .add_enabled(enabled && (exists || ultimate), button)
                        .on_hover_text(if exists || ultimate {
                            tip
                        } else {
                            t("This plan is not installed in Windows.")
                        })
                        .clicked()
                        && !active
                    {
                        command = Some(if ultimate { Command::ActivateUltimate { expected: snapshot.active } } else { Command::Activate {
                            expected: snapshot.active,
                            plan: id,
                        } });
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
        card(ui).inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal_wrapped(|ui| {
                subtitle(ui, t("CPU performance"));
                let dirty = self.draft.as_ref().is_some_and(|d| d.dirty());
                badge(ui, if dirty { t("UNAPPLIED CHANGES") } else { t("CURRENT SETTINGS") }, if dirty { colors.accent } else { colors.muted });
            });
            let old_source = self.source;
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().button_padding.y = 5.0;
                ui.spacing_mut().interact_size.y = 28.0;
                ui.horizontal_wrapped(|ui| {
                    // Preserve a pending draft until explicitly applied or discarded.
                    if snapshot.has_battery { ui.add_enabled_ui(self.draft.as_ref().is_none_or(|d| !d.dirty()), |ui| {
                        ui.selectable_value(&mut self.source, Source::Ac, t("Plugged in"));
                        ui.selectable_value(&mut self.source, Source::Dc, t("On battery"));
                    }).response.on_hover_text(t("Apply your edits or reload current values before changing the power source.")); }
                    ui.label(RichText::new(t("Settings for the active Windows plan")).size(12.0).color(colors.muted));
                });
            });
            if old_source != self.source { self.draft = Some(Draft::new(&snapshot, self.source)); }
            let Some(draft) = &mut self.draft else { return; };
            let stale = draft.stale(&snapshot);
            if stale { ui.colored_label(colors.accent, t("Windows settings changed. Reload current values to continue.")); }
            ui.add_space(3.0);
            ui.add_enabled_ui(enabled && !stale, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for preset in CpuPreset::ALL {
                        let candidate = preset.draft(&snapshot.cpu[self.source.index()].controls);
                        let selected = candidate.as_ref().is_ok_and(|c| values(c) == values(&draft.controls));
                        if ui.add_enabled(candidate.is_ok(), egui::Button::new(t(preset.label())).selected(selected))
                            .on_hover_text(candidate.as_ref().err().map_or(t(preset.description()), |e| t(e))).clicked() {
                            draft.controls = candidate.unwrap();
                        }
                    }
                });
                ui.label(RichText::new(t("Choose a preset, review its values below, then apply. These are Windows policies."))
                    .size(12.0).color(colors.muted));
                egui::CollapsingHeader::new(t("Advanced Windows settings")).default_open(false).show(ui, |ui| {
                ui.columns(2, |columns| {
                    for (column, key) in columns.iter_mut().zip([Setting::Minimum, Setting::Maximum]) {
                        if let Some(control) = draft.controls.iter_mut().find(|c| c.key == key) { range(column, control); }
                    }
                });
                ui.label(RichText::new(t("These percentages request a processor performance range; they are not watt limits.")).size(12.0).color(colors.muted));
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
                    ui.label(RichText::new(t("0% favors performance · 100% favors energy savings")).size(12.0).color(colors.muted));
                }
                });
            });
            let dirty = draft.dirty();
            let edit = draft.edit();
            let validation = validate(&snapshot.cpu[self.source.index()].controls, &edit);
            if dirty && !stale {
                if let Err(error) = &validation { ui.colored_label(colors.red, i18n::f("Details: {details}", &[("details", t(error).to_string())])); }
            }
            ui.add_space(5.0);
            let mut reload = false;
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(enabled && dirty && !stale && validation.is_ok(), egui::Button::new(RichText::new(t("Apply processor settings")).color(colors.on_accent).strong()).fill(colors.accent)).clicked() {
                    command = Some(Command::Apply(edit));
                }
                if ui.add_enabled(enabled && (dirty || stale), egui::Button::new(t("Reload current values"))).clicked() { reload = true; }
            });
            if reload { self.draft = Some(Draft::new(&snapshot, self.source)); }
            for error in &snapshot.cpu[self.source.index()].unavailable { ui.label(RichText::new(error).size(12.0).color(colors.muted)); }
        });
        if self.reading.is_ryzen() {
            ui.add_space(12.0);
            self.show_ryzen(ui);
        }
        ui.add_space(8.0);
        ui.label(RichText::new(t("Power settings stay active after quitting OpenCrate and restarting Windows. Undo is available for the last change in this session.")).size(12.0).color(colors.muted));
        ui.label(
            RichText::new(t(
                "Windows power mode and firmware can influence the resulting CPU behavior.",
            ))
            .size(12.0)
            .color(colors.muted),
        );
        if let Some(command) = command {
            self.send(command);
        }
    }
}

fn launch_ryzen_master(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if code <= 32 {
            return Err(format!(
                "Could not open Ryzen Master (Windows error {code})."
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("Ryzen Master requires Windows.".into())
    }
}

fn sparkline(
    ui: &mut Ui,
    history: &VecDeque<telemetry::Reading>,
    metric: fn(&telemetry::Reading) -> Option<f64>,
) {
    let colors = palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::hover());
    let now = std::time::Instant::now();
    let series: Vec<_> = history
        .iter()
        .filter_map(|r| {
            let age = now.saturating_duration_since(r.sampled_at?).as_secs_f32();
            (age <= 60.0).then_some((age, metric(r)))
        })
        .collect();
    let max = series.iter().filter_map(|(_, v)| *v).fold(1.0, f64::max);
    let mut previous: Option<(f32, egui::Pos2)> = None;
    for (age, value) in series {
        if let Some(value) = value {
            let point = egui::pos2(
                rect.right() - rect.width() * age / 60.0,
                rect.bottom() - rect.height() * (value / max) as f32,
            );
            if let Some((last_age, last_point)) = previous {
                if last_age - age < 5.0 {
                    ui.painter().line_segment(
                        [last_point, point],
                        egui::Stroke::new(1.5_f32, colors.accent),
                    );
                }
            }
            previous = Some((age, point));
        } else {
            previous = None;
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
            has_battery: false,
            ultimate_plan: None,
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

    fn preview_state(has_battery: bool) -> State {
        let mut snapshot = snapshot();
        snapshot.has_battery = has_battery;
        snapshot.battery_percent = has_battery.then_some(72);
        snapshot.ultimate_plan = Some(OPENCRATE_ULTIMATE);
        snapshot.plans = [
            (BALANCED, "Balanced"),
            (HIGH_PERFORMANCE, "High performance"),
            (POWER_SAVER, "Power saver"),
            (OPENCRATE_ULTIMATE, "Ultimate performance"),
        ]
        .into_iter()
        .map(|(id, name)| Plan {
            id,
            name: name.into(),
        })
        .collect();
        snapshot.cpu[0].controls = Setting::ALL
            .into_iter()
            .zip([5, 100, 3, 50])
            .map(|(key, value)| Control {
                key,
                value,
                write_error: None,
                allowed: if key == Setting::Boost {
                    Allowed::Choices(vec![
                        (0, "Disabled".into()),
                        (1, "Enabled".into()),
                        (2, "Aggressive".into()),
                        (3, "Efficient enabled".into()),
                    ])
                } else {
                    Allowed::Range {
                        min: 0,
                        max: 100,
                        step: 1,
                    }
                },
            })
            .collect();
        snapshot.cpu[1] = snapshot.cpu[0].clone();
        let reading = telemetry::Reading {
            cpu_name: "AMD Ryzen 5 7600 6-Core Processor".into(),
            power_w: Some(45.3),
            temperature_c: Some(62.5),
            frequency_mhz: Some(4875.0),
            load_percent: Some(22.0),
            provider: Some("LibreHardwareMonitor"),
            sampled_at: Some(std::time::Instant::now()),
            ..Default::default()
        };
        let history = (0..60)
            .map(|i| telemetry::Reading {
                sampled_at: Some(
                    std::time::Instant::now() - std::time::Duration::from_secs(59 - i),
                ),
                power_w: Some(40.0 + (i as f64 * 0.3).sin() * 8.0),
                ..reading.clone()
            })
            .collect();
        State {
            monitor: None,
            reading,
            history,
            controller: None,
            draft: Some(Draft::new(&snapshot, Source::Ac)),
            selected_plan: Some(BALANCED),
            snapshot: Some(snapshot),
            source: Source::Ac,
            error: None,
            message: Message::text("Windows power settings refreshed."),
            action_success: "Windows power settings refreshed.",
            action_error: false,
            busy: false,
        }
    }

    fn rendered_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    rendered_text(shape, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn desktop_hides_source_controls_and_laptop_keeps_them() {
        for has_battery in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let mut state = preview_state(has_battery);
            let mut text = Vec::new();
            for _ in 0..3 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(950.0, 1400.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| state.show(ui));
                    },
                );
                text.clear();
                for shape in output.shapes {
                    rendered_text(&shape.shape, &mut text);
                }
            }
            assert_eq!(text.iter().any(|s| s == t("Plugged in")), has_battery);
            assert_eq!(text.iter().any(|s| s == t("On battery")), has_battery);
            assert_eq!(text.iter().any(|s| s == t("PLUGGED IN")), has_battery);
            assert!(text.iter().any(|s| s == t("Ultimate performance")));
            assert!(text.iter().any(|s| s == t("AMD Ryzen tuning")));
        }
    }

    #[test]
    #[ignore = "Writes local UI previews when explicitly requested"]
    fn render_power_previews() {
        use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
        let output = std::path::PathBuf::from(
            std::env::var_os("OPENCRATE_POWER_PREVIEW").expect("preview directory"),
        );
        std::fs::create_dir_all(&output).unwrap();
        for (language, name) in [
            (i18n::Language::English, "en"),
            (i18n::Language::Turkish, "tr"),
            (i18n::Language::Chinese, "zh-CN"),
        ] {
            i18n::set_language(language);
            for width in [900, 560] {
                let ctx = egui::Context::default();
                crate::theme::install(&ctx);
                ctx.set_visuals(egui::Visuals::dark());
                let mut state = preview_state(false);
                let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Rgba);
                let height = 1400;
                let mut buffer = vec![0u8; width * height * 4];
                for _ in 0..3 {
                    let frame = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                vec2(width as f32, height as f32),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default()
                                .frame(
                                    egui::Frame::new()
                                        .fill(Palette::for_theme(ctx.theme()).background)
                                        .inner_margin(24),
                                )
                                .show(ctx, |ui| state.show(ui));
                        },
                    );
                    let primitives = ctx.tessellate(frame.shapes, frame.pixels_per_point);
                    renderer.render(
                        &mut BufferMutRef::new(
                            bytemuck::cast_slice_mut(&mut buffer),
                            width,
                            height,
                        ),
                        &primitives,
                        &frame.textures_delta,
                        frame.pixels_per_point,
                    );
                }
                image::save_buffer(
                    output.join(format!("power-{name}-{width}.png")),
                    &buffer,
                    width as u32,
                    height as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
        i18n::set_language(i18n::Language::English);
    }
}
