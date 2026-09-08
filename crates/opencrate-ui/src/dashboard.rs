//! Page layout and presentation. Hardware writes stay in App's command flow.

use crate::{
    i18n::{self, t},
    theme::{self, *},
    windows_startup, App,
};
use egui::{pos2, vec2, Align, Color32, FontId, Layout, Rect, RichText, Stroke, Ui};
use opencrate_aura::{
    animation::{self, is_animated, Timeline, MAX_SPEED, MIN_SPEED},
    playback::Settings,
};
use opencrate_core::{EffectMode, RgbColor};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Lighting,
    Fans,
    Power,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Ready,
    Applying,
    Applied,
    Error,
}

impl Activity {
    fn presentation(self, colors: Palette) -> (&'static str, Color32) {
        match self {
            Self::Ready => (t("READY"), colors.muted),
            Self::Applying => (t("APPLYING"), colors.accent),
            Self::Applied => (t("APPLIED"), colors.green),
            Self::Error => (t("NEEDS ATTENTION"), colors.red),
        }
    }
}

pub struct State {
    page: Page,
    logo: egui::TextureHandle,
    pub hex: String,
    timeline: Timeline,
    previous_frame: Instant,
    system_theme: Option<egui::Theme>,
}

impl State {
    pub fn power_visible(&self) -> bool {
        self.page == Page::Power
    }
    #[cfg(feature = "diagnostics")]
    pub fn select_page(&mut self, page: Page) {
        self.page = page;
    }

    pub fn new(ctx: &egui::Context, color: RgbColor) -> Self {
        theme::install(ctx);
        let icon = crate::runtime::icon_from_png(include_bytes!(
            "../../../assets/branding/opencrate-icon-256.png"
        ))
        .expect("embedded OpenCrate brand mark");
        Self {
            page: Page::Lighting,
            logo: ctx.load_texture(
                "opencrate-brand",
                egui::ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                ),
                egui::TextureOptions::LINEAR,
            ),
            hex: color.to_hex(),
            timeline: Timeline::default(),
            previous_frame: Instant::now(),
            system_theme: ctx.system_theme(),
        }
    }
}

impl App {
    pub fn render_dashboard(&mut self, ctx: &egui::Context) {
        if self.ui.system_theme != ctx.system_theme() {
            self.ui.system_theme = ctx.system_theme();
            // Windows also rethemes the title bar when its app mode changes.
            self.store.preferences.theme.apply(ctx);
        }
        let colors = Palette::for_theme(ctx.theme());
        self.sidebar(ctx);
        egui::TopBottomPanel::bottom("application_status")
            .frame(
                egui::Frame::new()
                    .fill(colors.sidebar)
                    .inner_margin(egui::Margin::symmetric(28, 14)),
            )
            .show(ctx, |ui| {
                let (label, status, color) = if self.ui.page == Page::Fans {
                    self.fans.status(colors)
                } else if self.ui.page == Page::Power {
                    self.power.status(colors)
                } else {
                    let (label, color) = self.activity.presentation(colors);
                    (label, self.lighting_status(), color)
                };
                ui.horizontal_wrapped(|ui| {
                    badge(ui, label, color);
                    ui.label(
                        RichText::new(status)
                            .size(12.0)
                            .color(if color == colors.red {
                                colors.red
                            } else {
                                colors.muted
                            }),
                    );
                });
                if self.ui.page != Page::Settings {
                    for error in [&self.store.error, &self.startup_error]
                        .into_iter()
                        .flatten()
                    {
                        ui.colored_label(
                            colors.red,
                            i18n::f("Details: {details}", &[("details", t(error).to_string())]),
                        );
                    }
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(colors.background).inner_margin(28))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("page", self.ui.page as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match self.ui.page {
                            Page::Lighting => self.lighting_page(ui),
                            Page::Fans => self.fans.show(ui),
                            Page::Power => self.power.show(ui),
                            Page::Settings => self.settings_page(ui),
                        }
                    });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let colors = Palette::for_theme(ctx.theme());
        egui::SidePanel::left("navigation")
            .exact_width(192.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(colors.sidebar).inner_margin(16))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new((self.ui.logo.id(), vec2(38.0, 38.0))).corner_radius(9),
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new("OpenCrate").size(20.0).strong());
                        ui.label(
                            RichText::new(t("HARDWARE CONTROL"))
                                .size(8.0)
                                .color(colors.muted),
                        );
                    });
                });
                ui.add_space(34.0);
                eyebrow(ui, t("WORKSPACE"));
                ui.add_space(4.0);
                self.nav_item(ui, Page::Lighting, t("Lighting"), Icon::Lighting, false);
                self.nav_item(ui, Page::Fans, t("Fans"), Icon::Fan, false);
                self.nav_item(ui, Page::Power, t("Power"), Icon::Power, false);
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.label(
                        RichText::new(format!("{} {}", t("VERSION"), env!("CARGO_PKG_VERSION")))
                            .size(10.0)
                            .color(colors.muted),
                    );
                    ui.add_space(12.0);
                    self.nav_item(ui, Page::Settings, t("Settings"), Icon::Settings, false);
                    if self.updates.available()
                        && ui
                            .button(RichText::new(t("Update available")).color(colors.accent))
                            .clicked()
                    {
                        self.ui.page = Page::Settings;
                    }
                });
            });
    }

    fn nav_item(&mut self, ui: &mut Ui, page: Page, label: &str, symbol: Icon, soon: bool) {
        let colors = palette(ui);
        let label = t(label);
        let selected = self.ui.page == page;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
        });
        let color = if selected {
            colors.accent
        } else if response.hovered() {
            colors.text
        } else {
            colors.muted
        };
        if selected || response.hovered() || response.has_focus() {
            ui.painter().rect(
                rect,
                8,
                if selected {
                    colors.accent_dim
                } else {
                    colors.input
                },
                if response.has_focus() {
                    Stroke::new(1.0_f32, colors.accent)
                } else {
                    Stroke::NONE
                },
                egui::StrokeKind::Inside,
            );
        }
        if selected {
            ui.painter().rect_filled(
                Rect::from_min_size(rect.left_center() + vec2(0.0, -9.0), vec2(3.0, 18.0)),
                2,
                colors.accent,
            );
        }
        icon(
            ui,
            symbol,
            Rect::from_center_size(rect.left_center() + vec2(23.0, 0.0), vec2(22.0, 22.0)),
            color,
        );
        ui.painter().text(
            rect.left_center() + vec2(45.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(14.0),
            color,
        );
        if soon {
            ui.painter().text(
                rect.right_center() - vec2(10.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                t("SOON"),
                FontId::proportional(8.0),
                colors.muted,
            );
        }
        if response.clicked() {
            self.ui.page = page;
        }
    }

    fn lighting_page(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        page_header(
            ui,
            t("Lighting"),
            t("Set the mood for your entire setup."),
            t("SYNC"),
            colors.accent,
        );
        self.preview_card(ui);
        ui.add_space(8.0);
        if ui.available_width() >= 690.0 {
            ui.columns(2, |columns| {
                self.effect_card(&mut columns[0]);
                self.adjustments_card(&mut columns[1]);
            });
        } else {
            self.effect_card(ui);
            ui.add_space(8.0);
            self.adjustments_card(ui);
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            let valid_color = !self.mode.takes_color() || self.ui.hex.parse::<RgbColor>().is_ok();
            let enabled = self.lighting.is_some() && valid_color;
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(
                        RichText::new(t("Apply changes"))
                            .strong()
                            .color(colors.on_accent),
                    )
                    .fill(colors.accent)
                    .stroke(Stroke::NONE)
                    .min_size(vec2(162.0, 42.0)),
                )
                .clicked()
            {
                self.apply(self.mode, self.color);
            }
            if ui
                .add_enabled(
                    self.lighting.is_some(),
                    egui::Button::new(t("Lights off")).min_size(vec2(105.0, 42.0)),
                )
                .clicked()
            {
                self.mode = EffectMode::Off;
                self.apply(self.mode, self.color);
            }
            let pending = !self
                .requested
                .is_some_and(|settings| self.matches_controls(settings));
            ui.label(
                RichText::new(if !valid_color {
                    t("Enter a valid hex color.")
                } else if pending {
                    t("Preview has unapplied changes")
                } else if self.activity == Activity::Applying {
                    t("Applying your changes…")
                } else {
                    t("Your lighting is up to date")
                })
                .size(12.0)
                .color(if pending || !valid_color {
                    colors.accent
                } else {
                    colors.muted
                }),
            );
        });
    }

    fn effect_card(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(255.0);
            subtitle(ui, t("Effect & color"));
            ui.add_space(2.0);
            egui::ComboBox::from_id_salt("lighting_effect")
                .width(ui.available_width())
                .selected_text(t(self.mode.display_name()))
                .show_ui(ui, |ui| {
                    for mode in EffectMode::all() {
                        ui.selectable_value(&mut self.mode, *mode, t(mode.display_name()));
                    }
                });
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 34.0), Layout::top_down(Align::Min), |ui| {
                ui.set_min_height(34.0);
                ui.label(RichText::new(effect_description(self.mode)).size(12.0).color(colors.muted));
            });
            ui.add_space(2.0);
            ui.separator();
            if self.mode.takes_color() {
                ui.label(RichText::new(t("Color")).strong());
                ui.horizontal(|ui| {
                    let mut rgb = [self.color.r, self.color.g, self.color.b];
                    ui.scope(|ui| {
                        ui.spacing_mut().interact_size = vec2(48.0, 36.0);
                        if ui.color_edit_button_srgb(&mut rgb).on_hover_text(t("Open color picker")).changed() {
                            self.color = RgbColor::new(rgb[0], rgb[1], rgb[2]);
                            self.ui.hex = self.color.to_hex();
                        }
                    });
                    ui.label(RichText::new("#").monospace().color(colors.muted));
                    let response = ui.add(egui::TextEdit::singleline(&mut self.ui.hex)
                        .id_salt("hex_color").font(egui::TextStyle::Monospace)
                        .desired_width(92.0).char_limit(7).hint_text("RRGGBB"));
                    if response.changed() {
                        if let Ok(color) = self.ui.hex.parse::<RgbColor>() {
                            self.color = color;
                        }
                    }
                    if response.lost_focus() && self.ui.hex.parse::<RgbColor>().is_ok() {
                        self.ui.hex = self.color.to_hex();
                    }
                    ui.label(RichText::new(t("HEX")).size(10.0).color(colors.muted));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (label, rgb) in [
                        (t("White"), [255, 255, 255]), (t("Amber"), [255, 177, 64]),
                        (t("Red"), [255, 48, 64]), (t("Pink"), [255, 77, 166]),
                        (t("Violet"), [150, 95, 255]), (t("Blue"), [50, 120, 255]),
                        (t("Cyan"), [45, 220, 230]), (t("Green"), [95, 220, 133]),
                    ] {
                        let color = RgbColor::new(rgb[0], rgb[1], rgb[2]);
                        let (rect, response) = ui.allocate_exact_size(vec2(29.0, 29.0), egui::Sense::click());
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton,
                            true, self.color == color, label));
                        if self.color == color || response.hovered() || response.has_focus() {
                            ui.painter().circle_stroke(rect.center(), 13.0, Stroke::new(1.0_f32, colors.text));
                        }
                        ui.painter().circle(rect.center(), 9.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]), Stroke::new(0.75_f32, colors.border));
                        if response.on_hover_text(label).clicked() {
                            self.color = color;
                            self.ui.hex = color.to_hex();
                        }
                    }
                });
            } else {
                ui.add_space(6.0);
                ui.label(RichText::new(if self.mode == EffectMode::Off { t("Lights out") } else { t("Automatic palette") }).strong());
                ui.label(RichText::new(if self.mode == EffectMode::Off {
                    t("Choose another effect to bring your lighting back.")
                } else { t("This effect creates its own colors. Adjust its brightness and speed to make it yours.") }).size(13.0).color(colors.muted));
            }
        });
    }

    fn adjustments_card(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        let mut brightness_changed = false;
        let mut speed_changed = false;
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(255.0);
            ui.spacing_mut().interact_size.y = 22.0;
            subtitle(ui, t("Fine-tune"));
            ui.add_space(4.0);
            ui.add_enabled_ui(self.mode != EffectMode::Off, |ui| {
                value_heading(ui, t("Brightness"), &format!("{}%", self.brightness));
                ui.spacing_mut().slider_width = ui.available_width();
                ui.visuals_mut().selection.bg_fill = colors.accent;
                let response =
                    ui.add(egui::Slider::new(&mut self.brightness, 0..=100).show_value(false));
                response.widget_info(|| {
                    egui::WidgetInfo::slider(
                        ui.is_enabled(),
                        self.brightness.into(),
                        t("Brightness"),
                    )
                });
                brightness_changed = response.changed();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t("Dim")).size(11.0).color(colors.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(t("Reset to 100%")).clicked() {
                            self.brightness = 100;
                            brightness_changed = true;
                        }
                    });
                });
            });
            ui.separator();
            ui.add_enabled_ui(is_animated(self.mode), |ui| {
                value_heading(ui, t("Animation speed"), &format!("{:.2}×", self.speed));
                ui.spacing_mut().slider_width = ui.available_width();
                ui.visuals_mut().selection.bg_fill = colors.accent;
                let response = ui.add(
                    egui::Slider::new(&mut self.speed, MIN_SPEED..=MAX_SPEED)
                        .logarithmic(true)
                        .show_value(false),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::slider(ui.is_enabled(), self.speed, t("Animation speed"))
                });
                speed_changed = response.changed();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("0.25× — 4×").size(11.0).color(colors.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(t("Reset to 1×")).clicked() {
                            self.speed = 1.0;
                            speed_changed = true;
                        }
                    });
                });
            });
            ui.label(
                RichText::new(if self.mode == EffectMode::Off {
                    t("Choose an effect to adjust your lighting.")
                } else if !is_animated(self.mode) {
                    t("Brightness is live after Apply. Static has no animation.")
                } else {
                    t("Changes are live after Apply. Effects keep running in the tray.")
                })
                .size(11.0)
                .color(colors.muted),
            );
        });
        if brightness_changed || speed_changed {
            if let Some(mut settings) = self.requested.filter(|s| s.mode == self.mode) {
                if brightness_changed {
                    settings.brightness = self.brightness;
                }
                if speed_changed && is_animated(settings.mode) {
                    settings.speed = self.speed;
                }
                self.submit(settings);
            }
        }
    }

    fn settings_page(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        page_header(
            ui,
            t("Settings"),
            t("Make OpenCrate fit your routine."),
            t("PREFERENCES"),
            colors.muted,
        );
        let mut autostart = self.autostart;
        self.updates_card(ui);
        ui.add_space(12.0);
        let mut preferences_changed = false;
        let mut language = self.store.preferences.language;
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            subtitle(ui, t("Application language"));
            ui.label(
                RichText::new(t(
                    "Choose the display language. Changes take effect immediately.",
                ))
                .color(colors.muted),
            );
            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("application_language")
                .selected_text(language.name())
                .width(230.0)
                .show_ui(ui, |ui| {
                    for choice in i18n::Language::ALL {
                        ui.selectable_value(&mut language, choice, choice.name());
                    }
                });
        });
        if language != self.store.preferences.language {
            self.change_language(language, ui.ctx());
        }
        ui.add_space(12.0);
        if theme_selector(ui, &mut self.store.preferences.theme) {
            self.store.preferences.theme.apply(ui.ctx());
            preferences_changed = true;
        }
        ui.add_space(12.0);
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            subtitle(ui, t("Startup & restore"));
            ui.add_space(8.0);
            if setting_row(
                ui,
                t("Launch with Windows"),
                t("Open OpenCrate automatically when you sign in."),
                &mut autostart,
            ) {
                match windows_startup::set_autostart(autostart) {
                    Ok(()) => {
                        self.autostart = autostart;
                        self.startup_error = None;
                    }
                    Err(error) => {
                        self.startup_error =
                            Some(format!("Could not change Windows startup: {error}"))
                    }
                }
            }
            ui.separator();
            preferences_changed |= setting_row(
                ui,
                t("Restore last lighting"),
                t("Restore your last applied effect, color, brightness and speed on launch."),
                &mut self.store.preferences.restore_lighting,
            );
            if !self.store.preferences.restore_lighting {
                self.startup_restore = None;
            }
            ui.separator();
            ui.add_enabled_ui(self.autostart, |ui| {
                preferences_changed |= setting_row(
                    ui,
                    t("Start in the tray"),
                    t("Keep the window hidden when OpenCrate starts with Windows."),
                    &mut self.store.preferences.start_in_tray,
                );
            });
            if !self.autostart {
                ui.label(
                    RichText::new(t("Enable Launch with Windows to use Start in the tray."))
                        .size(12.0)
                        .color(colors.muted),
                );
            }
        });
        if preferences_changed {
            self.store.changed();
            ui.ctx().request_repaint_after(Duration::from_millis(400));
        }
        ui.add_space(8.0);
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            subtitle(ui, t("Close to tray"));
            ui.label(RichText::new(t("Closing the window keeps lighting and fan control running. Use Show OpenCrate in the tray menu to return, or Quit to exit.")).color(colors.muted));
            ui.add_space(2.0);
            ui.label(RichText::new(t("When you quit, lighting switches to a built-in effect. Dimmed multicolor animations remain as a static color.")).size(12.0).color(colors.muted));
            ui.label(RichText::new(t("Fan changes are temporary. Quit restores the curves that were active before your changes; fan settings are not restored on Windows startup.")).size(12.0).color(colors.muted));
        });
        ui.add_space(14.0);
        subtitle(ui, t("About"));
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((self.ui.logo.id(), vec2(40.0, 40.0))).corner_radius(8));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new(concat!("OpenCrate ", env!("CARGO_PKG_VERSION"))).strong());
                ui.label(
                    RichText::new(t("An open-source alternative to Armoury Crate. RGB lighting, fan and power controls."))
                        .size(12.0)
                        .color(colors.muted),
                );
            });
        });
        ui.label(
            RichText::new(t("OpenCrate is an independent project. It is not affiliated with, supported or endorsed by ASUS."))
                .size(12.0)
                .color(colors.muted),
        );
        for error in [&self.startup_error, &self.store.error]
            .into_iter()
            .flatten()
        {
            ui.colored_label(
                colors.red,
                i18n::f("Details: {details}", &[("details", t(error).to_string())]),
            );
        }
    }

    fn matches_controls(&self, settings: Settings) -> bool {
        settings.mode == self.mode
            && (!self.mode.takes_color() || settings.color == self.color)
            && (self.mode == EffectMode::Off || settings.brightness == self.brightness)
            && (!is_animated(self.mode) || settings.speed == self.speed)
    }

    fn preview_card(&mut self, ui: &mut Ui) {
        let colors = palette(ui);
        let now = Instant::now();
        self.ui
            .timeline
            .advance(now - self.ui.previous_frame, self.speed);
        self.ui.previous_frame = now;
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 158.0), egui::Sense::hover());
            let right = Rect::from_min_max(pos2(rect.right() - 182.0, rect.top()), rect.max);
            let left = Rect::from_min_max(rect.min, pos2(right.left() - 12.0, rect.bottom()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                eyebrow(ui, t("EFFECT PREVIEW"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(t(self.mode.display_name()))
                        .size(25.0)
                        .strong(),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(if self.mode == EffectMode::Off {
                        t("Lighting is off").to_string()
                    } else if is_animated(self.mode) {
                        i18n::f(
                            "{brightness}% brightness / {speed}× speed",
                            &[
                                ("brightness", self.brightness.to_string()),
                                ("speed", format!("{:.2}", self.speed)),
                            ],
                        )
                    } else {
                        i18n::f(
                            "{color} / {brightness}% brightness",
                            &[
                                ("color", self.color.to_string()),
                                ("brightness", self.brightness.to_string()),
                            ],
                        )
                    })
                    .size(13.0)
                    .color(colors.muted),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(t("Choose an effect, then apply it to your lights."))
                        .size(12.0)
                        .color(colors.muted),
                );
            });
            let mut leds = [RgbColor::BLACK; 48];
            animation::render(
                self.mode,
                self.color,
                self.ui.timeline.seconds(),
                self.brightness,
                &mut leds,
            );
            let center = right.center();
            let p = ui.painter();
            for radius in [45.0, 64.0, 76.0] {
                p.circle_stroke(
                    center,
                    radius,
                    Stroke::new(1.0_f32, colors.border.gamma_multiply(0.6)),
                );
            }
            // Keep changing LEDs in their own mesh so the software renderer can
            // reuse the rest of the dashboard instead of rasterizing it again.
            let led_painter = p
                .clone()
                .with_layer_id(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::new("lighting-preview-leds"),
                ))
                .with_clip_rect(right.intersect(ui.clip_rect()));
            for (index, led) in leds.into_iter().enumerate() {
                let angle =
                    index as f32 / 48.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                let position = center + vec2(angle.cos(), angle.sin()) * 56.0;
                // Display LED intensity in the preview's sRGB color space.
                let color = Color32::from(egui::Rgba::from_rgb(
                    led.r as f32 / 255.0,
                    led.g as f32 / 255.0,
                    led.b as f32 / 255.0,
                ));
                led_painter.circle_filled(position, 11.0, color.gamma_multiply(0.07));
                led_painter.circle_filled(position, 6.5, color.gamma_multiply(0.15));
                led_painter.circle(position, 3.2, color, Stroke::new(0.5_f32, colors.border));
            }
            p.text(
                center - vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                t("SYNC"),
                FontId::proportional(14.0),
                colors.text,
            );
            p.text(
                center + vec2(0.0, 13.0),
                egui::Align2::CENTER_CENTER,
                t("ALL LIGHTS"),
                FontId::proportional(8.0),
                colors.muted,
            );
        });
        // Keep this decorative preview at 20 Hz. Hardware playback has its own
        // cadence and continues independently when this page is not being drawn.
        if is_animated(self.mode)
            && ui
                .ctx()
                .input(|i| i.focused && i.viewport().minimized != Some(true))
        {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

pub(crate) fn page_header(ui: &mut Ui, title: &str, description: &str, tag: &str, color: Color32) {
    let colors = palette(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.heading(RichText::new(t(title)).strong());
            ui.label(RichText::new(t(description)).color(colors.muted));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            badge(ui, tag, color)
        });
    });
    ui.add_space(16.0);
}

fn value_heading(ui: &mut Ui, label: &str, value: &str) {
    let colors = palette(ui);
    ui.horizontal(|ui| {
        ui.label(t(label));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).monospace().color(colors.accent));
        });
    });
}

fn setting_row(ui: &mut Ui, title: &str, description: &str, value: &mut bool) -> bool {
    let colors = palette(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        let text_width = ui.available_width() - 60.0;
        ui.allocate_ui_with_layout(vec2(text_width, 62.0), Layout::top_down(Align::Min), |ui| {
            ui.set_min_width(text_width);
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(t(title)).strong());
            ui.label(RichText::new(t(description)).size(13.0).color(colors.muted));
        });
        changed = toggle(ui, value, t(title)).changed();
    });
    changed
}

fn effect_description(mode: EffectMode) -> &'static str {
    match mode {
        EffectMode::Off => t("Switch off all synced lights."),
        EffectMode::Static => t("A solid color across your entire setup."),
        EffectMode::Breathing => t("Your chosen color gently fades in and out."),
        EffectMode::Flashing => t("Your chosen color flashes on and off."),
        EffectMode::SpectrumCycle => t("Cycle smoothly through the color spectrum."),
        EffectMode::Rainbow => t("A moving rainbow of colors across your lights."),
        EffectMode::SpectrumCycleBreathing => t("Soft breathing that changes color over time."),
        EffectMode::ChaseFade => t("A moving band of color with a fading trail."),
        EffectMode::SpectrumCycleChaseFade => t("A color-changing chase with a fading trail."),
        EffectMode::Chase => t("A crisp band of your chosen color moves along the lights."),
        EffectMode::SpectrumCycleChase => t("A moving band that cycles through colors."),
        EffectMode::SpectrumCycleWave => t("A flowing wave that shifts through the spectrum."),
        EffectMode::ChaseRainbowPulse => t("Rainbow colors chase and pulse along the lights."),
        EffectMode::RainbowFlicker => t("A rainbow with softly sparkling highlights."),
        EffectMode::GentleTransition => t("Slow, subtle transitions between colors."),
        EffectMode::WavePropagation => t("Waves of color spread out from the center."),
        EffectMode::WavePropagationPause => t("Expanding color waves with a pause between them."),
    }
}
