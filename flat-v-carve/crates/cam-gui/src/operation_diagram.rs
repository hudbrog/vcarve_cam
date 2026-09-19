//! Original procedural authoring diagrams, drawn from committed job values.
//! These illustrate geometry and conventions; they are never generated paths.
use crate::ui_theme as theme;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

pub fn face(ui: &mut egui::Ui, preview: &cam_core::operations::face::FaceEntryPreview) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 152.), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3., theme::SURFACE);
    let envelope = preview.envelope;
    let coverage = preview.coverage;
    let scale = ((rect.width() - 32.) as f64 / envelope.width_mm).min(116. / envelope.length_mm);
    if !scale.is_finite() || scale <= 0. {
        return;
    }
    let center = rect.center() - Vec2::new(0., 6.);
    let point = |x: f64, y: f64| {
        Pos2::new(
            center.x + ((x - envelope.min_x_mm - envelope.width_mm / 2.) * scale) as f32,
            center.y - ((y - envelope.min_y_mm - envelope.length_mm / 2.) * scale) as f32,
        )
    };
    let bounds = |r: cam_core::project::RectXY| {
        Rect::from_two_pos(
            point(r.min_x_mm, r.min_y_mm),
            point(r.min_x_mm + r.width_mm, r.min_y_mm + r.length_mm),
        )
    };
    painter.rect_stroke(
        bounds(envelope),
        0.,
        Stroke::new(1., theme::WARNING),
        StrokeKind::Inside,
    );
    painter.rect_filled(bounds(coverage), 0., theme::SELECTION);
    painter.rect_stroke(
        bounds(coverage),
        0.,
        Stroke::new(1.5, theme::ACCENT),
        StrokeKind::Inside,
    );
    if let Some(area) = preview.area {
        painter.rect_stroke(
            bounds(area),
            0.,
            Stroke::new(1., theme::TEXT),
            StrokeKind::Inside,
        );
    }
    // A true-size footprint centered in the coverage, deliberately separate
    // from the entry-axis coordinate. It does not invent an actual raster row.
    let low = if preview.axis == 'X' {
        coverage.min_x_mm
    } else {
        coverage.min_y_mm
    };
    let radius = (low - preview.pass_low_mm).max(0.);
    let footprint = point(
        coverage.min_x_mm + coverage.width_mm / 2.,
        coverage.min_y_mm + coverage.length_mm / 2.,
    );
    painter.circle_stroke(
        footprint,
        (radius * scale) as f32,
        Stroke::new(1.5, theme::TEXT),
    );
    let entries: Vec<_> = if let Some(invalid) = preview.inside_coverage {
        vec![invalid]
    } else {
        let span = preview.entries.iter().copied().fold(None, |span, value| {
            Some(match span {
                None => (value, value),
                Some((low, high)) => (f64::min(low, value), f64::max(high, value)),
            })
        });
        match span {
            Some((low, high)) if (low - high).abs() > 1e-9 => vec![low, high],
            Some((value, _)) => vec![value],
            None => vec![],
        }
    };
    for entry in entries {
        let ends = if preview.axis == 'X' {
            [
                point(entry, envelope.min_y_mm),
                point(entry, envelope.min_y_mm + envelope.length_mm),
            ]
        } else {
            [
                point(envelope.min_x_mm, entry),
                point(envelope.min_x_mm + envelope.width_mm, entry),
            ]
        };
        painter.line_segment(
            ends,
            Stroke::new(
                2.,
                if preview.inside_coverage.is_some() {
                    theme::ERROR
                } else {
                    theme::MODE
                },
            ),
        );
    }
    painter.text(
        rect.left_bottom() + Vec2::new(8., -6.),
        Align2::LEFT_BOTTOM,
        format!("+X right · +Y up · pass axis {}", preview.axis),
        FontId::proportional(11.),
        theme::MUTED,
    );
    ui.small("Teal: coverage with margins · outer outline: allowed travel · circle: cutter size · orange: entry coordinate.");
    if preview.inside_coverage.is_some() {
        ui.colored_label(
            theme::ERROR,
            "Red entry is inside the pass span and needs correction.",
        );
    }
    ui.small("Committed values · cutter size only").on_hover_text("Top view of committed values. The circle illustrates the cutter's size, not a generated path or planned position.");
}

pub fn knife(ui: &mut egui::Ui, heading: Option<f64>, offset: Option<f64>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 138.), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3., theme::SURFACE);
    let pivot = rect.center() - Vec2::new(12., 4.);
    let axis = Stroke::new(1., theme::DIVIDER);
    painter.arrow(pivot, Vec2::new(62., 0.), axis);
    painter.arrow(pivot, Vec2::new(0., -50.), axis);
    painter.text(
        pivot + Vec2::new(67., 0.),
        Align2::LEFT_CENTER,
        "+X",
        FontId::proportional(11.),
        theme::MUTED,
    );
    painter.text(
        pivot + Vec2::new(0., -53.),
        Align2::CENTER_BOTTOM,
        "+Y",
        FontId::proportional(11.),
        theme::MUTED,
    );
    if let Some(heading) = heading.filter(|n| n.is_finite()) {
        let angle = heading.to_radians();
        let tip = pivot + Vec2::new(angle.cos() as f32, -angle.sin() as f32) * 44.;
        painter.line_segment([pivot, tip], Stroke::new(6., theme::TEXT));
        painter.line_segment([pivot, tip], Stroke::new(3., Color32::WHITE));
        painter.circle_filled(tip, 4., theme::ACCENT);
        painter.text(
            rect.left_bottom() + Vec2::new(8., -7.),
            Align2::LEFT_BOTTOM,
            format!("Heading {heading}° · counterclockwise from +X"),
            FontId::proportional(11.),
            theme::TEXT,
        );
    } else {
        painter.text(
            rect.left_bottom() + Vec2::new(8., -7.),
            Align2::LEFT_BOTTOM,
            "Set initial heading to show blade alignment",
            FontId::proportional(11.),
            theme::MUTED,
        );
    }
    painter.circle_filled(pivot, 4., theme::MODE);
    ui.small(format!(
        "Orange pivot → teal tip · white blade · offset {} · schematic",
        offset
            .map(|v| format!("{v} mm"))
            .unwrap_or_else(|| "unset".into())
    ));
    ui.small("Simulator traces: orange pivot, cyan intended tip, green emitted replay. Initial heading is the physical blade alignment.");
}
