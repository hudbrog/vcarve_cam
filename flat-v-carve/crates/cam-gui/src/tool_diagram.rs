//! Dimension-driven cutter outlines. No inferred shaft, holder or flute shape.
use crate::ui_theme as theme;
use cam_core::tool_library::{LibraryGeometry, LibraryTool};
use egui::{Align2, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

/// A diagram is schematic and uses the parsed draft; partial raw text remains
/// in the editor. The optional shaft is drawn only when its extent is known.
pub fn show(ui: &mut egui::Ui, tool: &LibraryTool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(144., 136.), Sense::hover());
    crate::app::observe_control("Tool geometry diagram", rect);
    let painter = ui.painter_at(rect);
    let stroke = Stroke::new(1.5, theme::TEXT);
    let font = FontId::proportional(11.);
    let text = |at: Pos2, align, value: String| {
        painter.text(at, align, value, font.clone(), theme::MUTED);
    };
    text(
        rect.center_top(),
        Align2::CENTER_TOP,
        "Schematic · mm".into(),
    );
    if let LibraryGeometry::DragKnife(g) = &tool.geometry {
        if !g.blade_offset_mm.is_finite()
            || g.blade_offset_mm <= 0.
            || !g.max_cut_depth_mm.is_finite()
            || g.max_cut_depth_mm <= 0.
        {
            text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Complete geometry".into(),
            );
            return;
        }
        let pivot = rect.left_center() + Vec2::new(32., -6.);
        let tip = pivot + Vec2::new(76., 0.);
        painter.circle_stroke(pivot, 5., stroke);
        painter.line_segment([pivot, tip], Stroke::new(1.5, theme::ACCENT));
        painter.circle_filled(tip, 3., theme::ACCENT);
        text(
            pivot - Vec2::new(0., 14.),
            Align2::CENTER_BOTTOM,
            "Pivot".into(),
        );
        text(
            tip - Vec2::new(0., 14.),
            Align2::CENTER_BOTTOM,
            "Tip".into(),
        );
        text(
            rect.center() + Vec2::new(0., 18.),
            Align2::CENTER_TOP,
            format!("Offset {}", g.blade_offset_mm),
        );
        text(
            rect.center_bottom() - Vec2::new(0., 3.),
            Align2::CENTER_BOTTOM,
            format!("Cut depth {}", g.max_cut_depth_mm),
        );
        return;
    }
    let (diameter, cutting, tip, taper) = match &tool.geometry {
        LibraryGeometry::Endmill(g) => (g.diameter_mm, g.cutting_length_mm, g.diameter_mm, 0.),
        LibraryGeometry::Vbit(g) => (
            g.max_cutting_diameter_mm,
            g.cutting_height_mm,
            g.tip_diameter_mm,
            (g.max_cutting_diameter_mm - g.tip_diameter_mm)
                / (2. * (g.included_angle_deg.to_radians() / 2.).tan()),
        ),
        LibraryGeometry::DragKnife(_) => unreachable!(),
    };
    if !diameter.is_finite()
        || !cutting.is_finite()
        || diameter <= 0.
        || cutting <= 0.
        || !taper.is_finite()
        || taper < 0.
        || !tip.is_finite()
        || tip < 0.
        || tip > diameter
    {
        text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Complete geometry".into(),
        );
        return;
    }
    let extent = tool
        .assembly
        .stickout_mm
        .filter(|s| s.is_finite() && *s >= cutting)
        .unwrap_or(cutting);
    let widest = diameter.max(
        tool.assembly
            .shaft_diameter_mm
            .filter(|s| s.is_finite() && *s > 0.)
            .unwrap_or(diameter),
    );
    let scale = (78. / extent).min(60. / widest);
    let bottom = rect.center() + Vec2::new(-10., 38.);
    let point = |x: f64, z: f64| bottom + Vec2::new((x * scale) as f32, -(z * scale) as f32);
    let shoulder = taper.min(cutting);
    let top_diameter = if taper > cutting {
        tip + (diameter - tip) * cutting / taper
    } else {
        diameter
    };
    let vertices = vec![
        point(-tip / 2., 0.),
        point(tip / 2., 0.),
        point(top_diameter / 2., shoulder),
        point(top_diameter / 2., cutting),
        point(-top_diameter / 2., cutting),
        point(-top_diameter / 2., shoulder),
    ];
    painter.add(Shape::convex_polygon(vertices, theme::SELECTION, stroke));
    if let Some(shaft) = tool
        .assembly
        .shaft_diameter_mm
        .filter(|s| s.is_finite() && *s > 0.)
        && extent > cutting
    {
        painter.rect_stroke(
            Rect::from_min_max(point(-shaft / 2., extent), point(shaft / 2., cutting)),
            0.,
            stroke,
            egui::StrokeKind::Inside,
        );
    }
    let measure_x = rect.right() - 17.;
    let top = point(0., cutting).y;
    painter.line_segment(
        [Pos2::new(measure_x, top), Pos2::new(measure_x, bottom.y)],
        Stroke::new(1., theme::MUTED),
    );
    for y in [top, bottom.y] {
        painter.line_segment(
            [Pos2::new(measure_x - 3., y), Pos2::new(measure_x + 3., y)],
            stroke,
        );
    }
    text(
        Pos2::new(rect.right() - 1., (top + bottom.y) / 2.),
        Align2::RIGHT_CENTER,
        format!("{cutting}"),
    );
    text(
        rect.center_bottom() - Vec2::new(0., 16.),
        Align2::CENTER_BOTTOM,
        format!("Max Ø {diameter}"),
    );
    if let LibraryGeometry::Vbit(g) = &tool.geometry {
        text(
            rect.center_bottom(),
            Align2::CENTER_BOTTOM,
            format!("{}° · tip Ø {}", g.included_angle_deg, g.tip_diameter_mm),
        );
    } else if let Some(stickout) = tool.assembly.stickout_mm {
        text(
            rect.center_bottom(),
            Align2::CENTER_BOTTOM,
            format!("Stickout {stickout}"),
        );
    }
}
