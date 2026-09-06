//! Shared visual language for the hardware control pages.

use crate::i18n::t;
use egui::{pos2, vec2, Color32, FontId, Rect, RichText, Stroke, Ui};

pub const BACKGROUND: Color32 = Color32::from_rgb(17, 19, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(21, 23, 27);
pub const SURFACE: Color32 = Color32::from_rgb(27, 30, 35);
pub const INPUT: Color32 = Color32::from_rgb(34, 38, 44);
pub const BORDER: Color32 = Color32::from_rgb(48, 53, 61);
pub const TEXT: Color32 = Color32::from_rgb(238, 239, 242);
pub const MUTED: Color32 = Color32::from_rgb(151, 158, 171);
pub const ACCENT: Color32 = Color32::from_rgb(244, 177, 64);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(53, 43, 28);
pub const GREEN: Color32 = Color32::from_rgb(113, 207, 161);
pub const RED: Color32 = Color32::from_rgb(248, 132, 137);

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
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::proportional(28.0)),
        (egui::TextStyle::Body, FontId::proportional(14.0)),
        (egui::TextStyle::Button, FontId::proportional(14.0)),
        (egui::TextStyle::Small, FontId::proportional(12.0)),
        (egui::TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = vec2(10.0, 10.0);
    style.spacing.button_padding = vec2(14.0, 9.0);
    style.spacing.interact_size = vec2(40.0, 36.0);
    style.spacing.combo_height = 360.0;
    style.spacing.slider_rail_height = 4.0;
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(MUTED);
    v.panel_fill = BACKGROUND;
    v.window_fill = SURFACE;
    v.extreme_bg_color = BACKGROUND;
    v.text_edit_bg_color = Some(INPUT);
    v.faint_bg_color = INPUT;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = 12.into();
    v.menu_corner_radius = 10.into();
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = ACCENT;
    v.error_fg_color = RED;
    v.slider_trailing_fill = true;
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        widget.corner_radius = 7.into();
        widget.bg_fill = INPUT;
        widget.weak_bg_fill = INPUT;
        widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = Stroke::new(1.5_f32, TEXT);
    }
    v.widgets.hovered.bg_fill = Color32::from_rgb(45, 48, 55);
    v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, MUTED);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, TEXT);
    ctx.set_style(style);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(14)
        .inner_margin(22)
}

pub fn eyebrow(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(t(label)).size(10.0).strong().color(MUTED));
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
                p.circle(pos2(c.x + offset * r, y), r * 0.22, SIDEBAR, stroke);
            }
        }
    }
}

pub fn toggle(ui: &mut Ui, value: &mut bool, label: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(40.0, 23.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    let amount = ui.ctx().animate_bool(response.id, *value);
    let fill = if *value { ACCENT } else { BORDER };
    let stroke = if response.hovered() || response.has_focus() {
        Stroke::new(1.0_f32, TEXT)
    } else {
        Stroke::NONE
    };
    ui.painter()
        .rect(rect, 12, fill, stroke, egui::StrokeKind::Inside);
    ui.painter().circle_filled(
        pos2(rect.left() + 11.5 + 17.0 * amount, rect.center().y),
        8.0,
        if *value { BACKGROUND } else { MUTED },
    );
    response
}
