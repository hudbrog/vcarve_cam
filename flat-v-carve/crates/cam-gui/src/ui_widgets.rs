//! Bounded shared form primitives. No document parsing or mutation lives here.
use crate::{
    ui_icons::{self, Icon},
    ui_theme as theme,
};
use egui::{RichText, Sense};

pub fn section(ui: &mut egui::Ui, title: &str, icon: Option<Icon>) {
    ui.add_space(8.);
    ui.separator();
    ui.horizontal(|ui| {
        ui.set_min_height(theme::ROW);
        if let Some(icon) = icon {
            ui_icons::show(ui, icon);
        }
        ui.label(RichText::new(title).size(14.).strong());
        crate::app::help::icon(ui, title);
    });
}

pub fn scope(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::SELECTION)
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.).color(theme::TEXT));
        });
}

pub fn number_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    text: &mut String,
    unit: &str,
    help: &str,
) -> egui::Response {
    let narrow = ui.available_width() < 260.;
    let edit = |ui: &mut egui::Ui, label_id: egui::Id, width: f32, text: &mut String| {
        let response = ui
            .add_sized(
                [width, theme::ROW],
                egui::TextEdit::singleline(text)
                    .id(id)
                    .char_limit(128)
                    .hint_text("Unset")
                    .horizontal_align(egui::Align::Max),
            )
            .labelled_by(label_id);
        ui.add_sized(
            [52., theme::ROW],
            egui::Label::new(RichText::new(unit).size(12.).color(theme::MUTED))
                .wrap_mode(egui::TextWrapMode::Extend),
        );
        crate::app::help::icon(ui, help);
        response
    };
    if narrow {
        let label = ui.label(label);
        ui.horizontal(|ui| edit(ui, label.id, (ui.available_width() - 92.).max(48.), text))
            .inner
    } else {
        ui.horizontal(|ui| {
            ui.set_min_height(theme::ROW);
            let label_width = (ui.available_width() * 0.42).clamp(104., 154.);
            let response = ui.add_sized([label_width, theme::ROW], egui::Label::new(label).wrap());
            edit(ui, response.id, (ui.available_width() - 92.).max(56.), text)
        })
        .inner
    }
}

/// Quiet table headings; callers supply columns, stable identities and editing.
pub fn table_header(ui: &mut egui::Ui, labels: &[&str]) {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .inner_margin(6.)
        .show(ui, |ui| {
            ui.columns(labels.len(), |columns| {
                for (column, label) in columns.iter_mut().zip(labels) {
                    column.strong(*label);
                }
            });
        });
}

/// A bounded navigation row. Its siblings own toggles and menus separately.
pub fn navigation_row(
    ui: &mut egui::Ui,
    label: &str,
    detail: Option<&str>,
    icon: Icon,
    selected: bool,
) -> egui::Response {
    let height = if detail.is_some() { 46. } else { 32. };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(1.), height),
        Sense::click(),
    );
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            3.,
            if selected {
                theme::SELECTION
            } else {
                theme::DIVIDER.gamma_multiply(0.5)
            },
        );
    }
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(3., height)),
            0.,
            theme::ACCENT,
        );
    }
    let icon_rect = egui::Rect::from_center_size(
        rect.left_center() + egui::vec2(15., 0.),
        egui::vec2(18., 18.),
    );
    ui_icons::paint(ui.painter(), icon_rect, icon, theme::TEXT);
    let text_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(30., 0.),
        rect.max - egui::vec2(4., 0.),
    );
    let paint_text = |text: &str, y: f32, size: f32, color| {
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.into(),
            egui::FontId::proportional(size),
            color,
        );
        job.wrap.max_width = text_rect.width().max(1.);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().with_clip_rect(text_rect).galley(
            egui::pos2(text_rect.left(), y),
            galley,
            color,
        );
    };
    paint_text(
        label,
        rect.top() + if detail.is_some() { 5. } else { 8. },
        14.,
        theme::TEXT,
    );
    if let Some(detail) = detail {
        paint_text(detail, rect.top() + 26., 12., theme::MUTED);
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response.on_hover_text(match detail {
        Some(detail) => format!("{label}\n{detail}"),
        None => label.into(),
    })
}

/// Schematic setup Z diagram; coordinates and labels use the actual stock datum.
pub fn stock_datum(
    ui: &mut egui::Ui,
    thickness: Option<f64>,
    clearance: Option<f64>,
    bottom_zero: bool,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 94.), Sense::hover());
    let p = ui.painter();
    let stock = egui::Rect::from_min_size(rect.min + egui::vec2(12., 31.), egui::vec2(94., 36.));
    p.rect_filled(stock, 1., egui::Color32::from_rgb(216, 193, 148));
    p.rect_stroke(
        stock,
        1.,
        egui::Stroke::new(1., theme::MUTED),
        egui::StrokeKind::Inside,
    );
    let zero = if bottom_zero {
        stock.bottom()
    } else {
        stock.top()
    };
    p.line_segment(
        [
            egui::pos2(stock.left() - 4., zero),
            egui::pos2(stock.right() + 5., zero),
        ],
        egui::Stroke::new(2., theme::ACCENT),
    );
    let value = |v: Option<f64>| {
        v.map(|v| format!("{v:.2} mm"))
            .unwrap_or_else(|| "unset".into())
    };
    let top = if bottom_zero {
        thickness
            .map(|t| format!("Top +{t:.2} mm"))
            .unwrap_or_else(|| "Top unset".into())
    } else {
        "Top Z0".into()
    };
    let bottom = if bottom_zero {
        "Bottom Z0".into()
    } else {
        thickness
            .map(|t| format!("Bottom −{t:.2} mm"))
            .unwrap_or_else(|| "Bottom unset".into())
    };
    for (y, text) in [
        (rect.top() + 9., format!("Clearance {}", value(clearance))),
        (stock.top() + 3., top),
        (stock.bottom() + 3., bottom),
    ] {
        p.text(
            egui::pos2(stock.right() + 12., y),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(12.),
            theme::TEXT,
        );
    }
    p.line_segment(
        [
            egui::pos2(stock.left(), rect.top() + 15.),
            egui::pos2(stock.right(), rect.top() + 15.),
        ],
        egui::Stroke::new(1., theme::MUTED),
    );
    crate::app::observe_control("Stock datum diagram", rect);
}
