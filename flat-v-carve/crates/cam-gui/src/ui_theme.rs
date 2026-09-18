//! Shared presentation tokens. Dimensions are logical egui points.
use egui::{Color32, FontId, Stroke, TextStyle};

pub const HEADER: Color32 = Color32::from_rgb(44, 55, 65);
pub const PANEL: Color32 = Color32::from_rgb(237, 242, 246);
pub const SURFACE: Color32 = Color32::from_rgb(248, 250, 252);
pub const DIVIDER: Color32 = Color32::from_rgb(197, 208, 217);
pub const TEXT: Color32 = Color32::from_rgb(34, 49, 63);
pub const MUTED: Color32 = Color32::from_rgb(85, 103, 121);
pub const SELECTION: Color32 = Color32::from_rgb(183, 230, 233);
pub const ACCENT: Color32 = Color32::from_rgb(22, 142, 152);
pub const PRIMARY: Color32 = Color32::from_rgb(41, 188, 195);
pub const MODE: Color32 = Color32::from_rgb(255, 166, 76);
pub const WARNING: Color32 = Color32::from_rgb(152, 87, 16);
pub const ERROR: Color32 = Color32::from_rgb(176, 42, 35);
pub const SUCCESS: Color32 = Color32::from_rgb(27, 112, 77);
pub const ROW: f32 = 28.;
pub const INSPECTOR: f32 = 344.;
pub const NAVIGATOR: f32 = 248.;

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = Color32::WHITE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.weak_text_color = Some(MUTED);
    style.visuals.selection.bg_fill = SELECTION;
    style.visuals.selection.stroke = Stroke::new(1., ACCENT);
    style.visuals.slider_trailing_fill = true;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1., DIVIDER);
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(221, 233, 238);
    style.visuals.widgets.active.bg_fill = SELECTION;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(3);
        widget.bg_stroke = Stroke::new(1., DIVIDER);
        widget.fg_stroke = Stroke::new(1., TEXT);
    }
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.5, ACCENT);
    for (kind, size) in [
        (TextStyle::Body, 14.),
        (TextStyle::Button, 14.),
        (TextStyle::Small, 12.),
        (TextStyle::Heading, 18.),
    ] {
        style.text_styles.insert(kind, FontId::proportional(size));
    }
    style.spacing.item_spacing = egui::vec2(8., 6.);
    style.spacing.button_padding = egui::vec2(9., 5.);
    style.spacing.interact_size.y = ROW;
    ctx.set_style(style);
}
