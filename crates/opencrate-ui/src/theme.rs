//! Shared visual language for the hardware control pages.

use crate::i18n::t;
use egui::{pos2, vec2, Color32, FontId, Rect, RichText, Stroke, Ui};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        t(match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        })
    }

    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        });
        // Keep the title bar aligned with the preference, including System mode.
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match self {
            Self::System => egui::SystemTheme::SystemDefault,
            Self::Light => egui::SystemTheme::Light,
            Self::Dark => egui::SystemTheme::Dark,
        }));
        ctx.request_repaint();
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub on_accent: Color32,
    pub green: Color32,
    pub red: Color32,
    hover: Color32,
}

impl Palette {
    pub fn for_theme(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Dark => Self {
                background: Color32::from_rgb(17, 19, 23),
                sidebar: Color32::from_rgb(21, 23, 27),
                surface: Color32::from_rgb(27, 30, 35),
                input: Color32::from_rgb(34, 38, 44),
                border: Color32::from_rgb(48, 53, 61),
                text: Color32::from_rgb(238, 239, 242),
                muted: Color32::from_rgb(151, 158, 171),
                accent: Color32::from_rgb(244, 177, 64),
                accent_dim: Color32::from_rgb(53, 43, 28),
                on_accent: Color32::from_rgb(17, 19, 23),
                green: Color32::from_rgb(113, 207, 161),
                red: Color32::from_rgb(248, 132, 137),
                hover: Color32::from_rgb(45, 48, 55),
            },
            egui::Theme::Light => Self {
                background: Color32::from_rgb(245, 246, 248),
                sidebar: Color32::from_rgb(237, 239, 242),
                surface: Color32::WHITE,
                input: Color32::from_rgb(240, 242, 245),
                border: Color32::from_rgb(207, 212, 220),
                text: Color32::from_rgb(30, 34, 42),
                muted: Color32::from_rgb(91, 100, 114),
                accent: Color32::from_rgb(147, 86, 8),
                accent_dim: Color32::from_rgb(255, 237, 207),
                on_accent: Color32::WHITE,
                green: Color32::from_rgb(26, 116, 75),
                red: Color32::from_rgb(183, 45, 62),
                hover: Color32::from_rgb(226, 230, 236),
            },
        }
    }
}

pub fn palette(ui: &Ui) -> Palette {
    Palette::for_theme(if ui.visuals().dark_mode {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    })
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "opencrate-cjk".into(),
        egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/NotoSansSC-Regular.ttf"
        ))
        .into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("opencrate-cjk".into());
    }
    ctx.set_fonts(fonts);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
}

fn style(theme: egui::Theme) -> egui::Style {
    let colors = Palette::for_theme(theme);
    let mut style = egui::Style {
        text_styles: [
            (egui::TextStyle::Heading, FontId::proportional(28.0)),
            (egui::TextStyle::Body, FontId::proportional(14.0)),
            (egui::TextStyle::Button, FontId::proportional(14.0)),
            (egui::TextStyle::Small, FontId::proportional(12.0)),
            (egui::TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into(),
        visuals: theme.default_visuals(),
        ..Default::default()
    };
    style.spacing.item_spacing = vec2(10.0, 10.0);
    style.spacing.button_padding = vec2(14.0, 9.0);
    style.spacing.interact_size = vec2(40.0, 36.0);
    style.spacing.combo_height = 360.0;
    style.spacing.slider_rail_height = 4.0;
    let v = &mut style.visuals;
    v.override_text_color = Some(colors.text);
    v.weak_text_color = Some(colors.muted);
    v.panel_fill = colors.background;
    v.window_fill = colors.surface;
    v.extreme_bg_color = colors.background;
    v.text_edit_bg_color = Some(colors.input);
    v.faint_bg_color = colors.input;
    v.window_stroke = Stroke::new(1.0_f32, colors.border);
    v.window_corner_radius = 12.into();
    v.menu_corner_radius = 10.into();
    v.selection.bg_fill = colors.accent_dim;
    v.selection.stroke = Stroke::new(1.0_f32, colors.accent);
    v.hyperlink_color = colors.accent;
    v.warn_fg_color = colors.accent;
    v.error_fg_color = colors.red;
    v.slider_trailing_fill = true;
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, colors.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors.text);
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        widget.corner_radius = 7.into();
        widget.bg_fill = colors.input;
        widget.weak_bg_fill = colors.input;
        widget.bg_stroke = Stroke::new(1.0_f32, colors.border);
        widget.fg_stroke = Stroke::new(1.5_f32, colors.text);
    }
    v.widgets.hovered.bg_fill = colors.hover;
    v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, colors.muted);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, colors.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, colors.text);
    style
}

pub fn card(ui: &Ui) -> egui::Frame {
    let colors = palette(ui);
    egui::Frame::new()
        .fill(colors.surface)
        .stroke(Stroke::new(1.0_f32, colors.border))
        .corner_radius(14)
        .inner_margin(22)
}

pub fn theme_selector(ui: &mut Ui, preference: &mut ThemePreference) -> bool {
    let previous = *preference;
    let colors = palette(ui);
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        subtitle(ui, t("Appearance"));
        ui.label(
            RichText::new(t(
                "Choose a theme. System follows your Windows app mode automatically.",
            ))
            .color(colors.muted),
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for choice in ThemePreference::ALL {
                ui.selectable_value(preference, choice, choice.label());
            }
        });
    });
    *preference != previous
}

pub fn eyebrow(ui: &mut Ui, label: &str) {
    let colors = palette(ui);
    ui.label(
        RichText::new(t(label))
            .size(10.0)
            .strong()
            .color(colors.muted),
    );
}

pub fn subtitle(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(t(label)).size(17.0).strong());
}

pub fn badge(ui: &mut Ui, label: &str, color: Color32) {
    let label = t(label);
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(10.0), color);
    let (rect, response) =
        ui.allocate_exact_size(text.size() + vec2(18.0, 10.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    ui.painter()
        .rect_filled(rect, 5, color.gamma_multiply(0.13));
    ui.painter().galley(rect.min + vec2(9.0, 5.0), text, color);
}

#[derive(Clone, Copy)]
pub enum Icon {
    Lighting,
    Fan,
    Power,
    Settings,
}

pub fn icon(ui: &Ui, kind: Icon, rect: Rect, color: Color32) {
    let colors = palette(ui);
    let p = ui.painter();
    let c = rect.center();
    let r = rect.width() * 0.39;
    let stroke = Stroke::new(1.6_f32, color);
    match kind {
        Icon::Lighting => {
            p.circle_stroke(c, r * 0.5, stroke);
            for n in 0..8 {
                let a = n as f32 * std::f32::consts::TAU / 8.0;
                let v = vec2(a.cos(), a.sin());
                p.line_segment([c + v * r * 0.77, c + v * r], stroke);
            }
        }
        Icon::Fan => {
            p.circle_stroke(c, r * 0.2, stroke);
            for n in 0..3 {
                let a = n as f32 * std::f32::consts::TAU / 3.0;
                let v = vec2(a.cos(), a.sin());
                p.circle_stroke(c + v * r * 0.59, r * 0.39, stroke);
            }
        }
        Icon::Power => {
            let points: Vec<_> = (0..=32)
                .map(|i| {
                    let a = -1.05 + i as f32 / 32.0 * 5.24;
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
            p.line_segment([c + vec2(0.0, -r * 1.13), c + vec2(0.0, -r * 0.1)], stroke);
        }
        Icon::Settings => {
            for (i, offset) in [-0.5, 0.5, -0.2].into_iter().enumerate() {
                let y = c.y + (i as f32 - 1.0) * r * 0.8;
                p.line_segment([pos2(c.x - r, y), pos2(c.x + r, y)], stroke);
                p.circle(pos2(c.x + offset * r, y), r * 0.22, colors.sidebar, stroke);
            }
        }
    }
}

pub fn toggle(ui: &mut Ui, value: &mut bool, label: &str) -> egui::Response {
    let colors = palette(ui);
    let (rect, mut response) = ui.allocate_exact_size(vec2(40.0, 23.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    let amount = ui.ctx().animate_bool(response.id, *value);
    let fill = if *value { colors.accent } else { colors.border };
    let stroke = if response.hovered() || response.has_focus() {
        Stroke::new(1.0_f32, colors.text)
    } else {
        Stroke::NONE
    };
    ui.painter()
        .rect(rect, 12, fill, stroke, egui::StrokeKind::Inside);
    ui.painter().circle_filled(
        pos2(rect.left() + 11.5 + 17.0 * amount, rect.center().y),
        8.0,
        if *value {
            colors.on_accent
        } else {
            colors.muted
        },
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_frame(ctx: &egui::Context, system: Option<egui::Theme>, expected: egui::Theme) {
        let _ = ctx.run(
            egui::RawInput {
                system_theme: system,
                ..Default::default()
            },
            |ctx| {
                assert_eq!(ctx.theme(), expected);
                let colors = Palette::for_theme(expected);
                assert_eq!(ctx.style().visuals.panel_fill, colors.background);
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert_eq!(palette(ui).text, colors.text);
                    assert_eq!(card(ui).fill, colors.surface);
                    assert_eq!(ui.visuals().selection.bg_fill, colors.accent_dim);
                });
            },
        );
    }

    #[test]
    fn system_changes_and_manual_overrides_update_the_entire_palette() {
        use egui::Theme::{Dark, Light};
        let ctx = egui::Context::default();
        install(&ctx);
        ThemePreference::System.apply(&ctx);
        for system in [Light, Dark, Light] {
            check_frame(&ctx, Some(system), system);
        }
        for (preference, expected) in [
            (ThemePreference::Dark, Dark),
            (ThemePreference::Light, Light),
        ] {
            preference.apply(&ctx);
            for system in [Dark, Light, Dark] {
                check_frame(&ctx, Some(system), expected);
            }
        }
        // The latest system theme is used immediately when removing an override.
        ThemePreference::System.apply(&ctx);
        assert_eq!(ctx.theme(), Dark);
        check_frame(&ctx, Some(Light), Light);
        check_frame(&ctx, None, ctx.options(|o| o.fallback_theme));
    }

    #[test]
    fn installation_preserves_saved_preference_and_native_theme_commands() {
        for (preference, native) in [
            (ThemePreference::System, egui::SystemTheme::SystemDefault),
            (ThemePreference::Light, egui::SystemTheme::Light),
            (ThemePreference::Dark, egui::SystemTheme::Dark),
        ] {
            let ctx = egui::Context::default();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                preference.apply(ctx);
                let selected = ctx.options(|o| o.theme_preference);
                install(ctx);
                assert_eq!(ctx.options(|o| o.theme_preference), selected);
            });
            assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(
                |command| matches!(command, egui::ViewportCommand::SetTheme(theme) if *theme == native)
            ));
        }
    }

    #[test]
    fn both_palettes_keep_small_text_and_primary_actions_readable() {
        fn luminance(color: Color32) -> f32 {
            let linear = [color.r(), color.g(), color.b()].map(|v| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            });
            linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
        }
        fn contrast(foreground: Color32, background: Color32) {
            let a = luminance(foreground);
            let b = luminance(background);
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(
                ratio >= 4.5,
                "{foreground:?} on {background:?}: {ratio:.2}:1"
            );
        }
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let colors = Palette::for_theme(theme);
            for background in [
                colors.background,
                colors.sidebar,
                colors.surface,
                colors.input,
            ] {
                for foreground in [
                    colors.text,
                    colors.muted,
                    colors.accent,
                    colors.green,
                    colors.red,
                ] {
                    contrast(foreground, background);
                }
            }
            contrast(colors.on_accent, colors.accent);
            contrast(colors.accent, colors.accent_dim);
        }
    }
}
