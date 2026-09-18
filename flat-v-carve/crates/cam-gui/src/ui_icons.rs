//! Original local line art on a 20-point grid; no font glyph dependencies.
use egui::{Color32, Rect, Stroke};

#[derive(Clone, Copy)]
pub enum Icon {
    Stock,
    Machine,
    Library,
    Settings,
    Artwork,
    Eye,
    Hidden,
    Lock,
    Unlock,
    Face,
    Carve,
    Profile,
    Knife,
    Endmill,
    Vbit,
    Add,
    More,
    Play,
    Pause,
    Start,
    Fit,
    Warning,
    Help,
    Save,
    Undo,
    Redo,
}

pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let p = |x: f32, y: f32| rect.min + egui::vec2(x, y) * (rect.width() / 20.);
    let stroke = Stroke::new(1.5, color);
    let line = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let box_at = |a, b| {
        painter.rect_stroke(
            Rect::from_two_pos(a, b),
            1.,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Stock => {
            line(&[
                (2., 5.),
                (10., 1.),
                (18., 5.),
                (18., 15.),
                (10., 19.),
                (2., 15.),
                (2., 5.),
                (10., 9.),
                (18., 5.),
            ]);
            line(&[(10., 9.), (10., 19.)]);
        }
        Icon::Machine => {
            box_at(p(2., 3.), p(18., 18.));
            line(&[(5., 15.), (15., 15.), (15., 6.), (5., 6.), (5., 15.)]);
            line(&[(10., 6.), (10., 11.)]);
        }
        Icon::Library => {
            for x in [3., 8., 13.] {
                box_at(p(x, 3.), p(x + 4., 18.));
            }
        }
        Icon::Settings => {
            for (y, x) in [(5., 7.), (10., 14.), (15., 6.)] {
                line(&[(2., y), (18., y)]);
                painter.circle_filled(p(x, y), 2., color);
            }
        }
        Icon::Artwork => {
            line(&[
                (4., 1.),
                (12., 1.),
                (17., 6.),
                (17., 19.),
                (4., 19.),
                (4., 1.),
            ]);
            line(&[(12., 1.), (12., 6.), (17., 6.)]);
            line(&[(6., 15.), (9., 11.), (12., 14.), (15., 10.)]);
        }
        Icon::Eye | Icon::Hidden => {
            line(&[
                (1., 10.),
                (5., 6.),
                (10., 4.),
                (15., 6.),
                (19., 10.),
                (15., 14.),
                (10., 16.),
                (5., 14.),
                (1., 10.),
            ]);
            painter.circle_stroke(p(10., 10.), 3., stroke);
            if matches!(icon, Icon::Hidden) {
                line(&[(2., 2.), (18., 18.)]);
            }
        }
        Icon::Lock | Icon::Unlock => {
            box_at(p(4., 9.), p(16., 19.));
            line(&[
                (6., 9.),
                (6., 5.),
                (8., 2.),
                (12., 2.),
                (14., 5.),
                (14., if matches!(icon, Icon::Lock) { 9. } else { 6. }),
            ]);
            line(&[(10., 12.), (10., 16.)]);
        }
        Icon::Face => {
            for y in [3., 8., 13.] {
                line(&[
                    (2., y + 2.),
                    (10., y - 1.),
                    (18., y + 2.),
                    (10., y + 5.),
                    (2., y + 2.),
                ]);
            }
        }
        Icon::Carve | Icon::Vbit => {
            line(&[(3., 4.), (10., 18.), (17., 4.)]);
            if matches!(icon, Icon::Vbit) {
                line(&[(3., 4.), (17., 4.)]);
            } else {
                line(&[(3., 1.), (10., 12.), (17., 1.)]);
            }
        }
        Icon::Profile => {
            line(&[
                (2., 3.),
                (17., 3.),
                (17., 8.),
                (12., 8.),
                (12., 16.),
                (7., 16.),
                (7., 19.),
                (2., 19.),
                (2., 3.),
            ]);
        }
        Icon::Knife => {
            line(&[(6., 2.), (14., 2.), (14., 12.), (6., 19.), (6., 2.)]);
            line(&[(6., 10.), (14., 10.)]);
        }
        Icon::Endmill => {
            box_at(p(7., 1.), p(13., 19.));
            for y in [10., 14., 18.] {
                line(&[(7., y), (13., y - 3.)]);
            }
        }
        Icon::Add => {
            line(&[(3., 10.), (17., 10.)]);
            line(&[(10., 3.), (10., 17.)]);
        }
        Icon::More => {
            for x in [4., 10., 16.] {
                painter.circle_filled(p(x, 10.), 1.5, color);
            }
        }
        Icon::Play => {
            line(&[(5., 2.), (17., 10.), (5., 18.), (5., 2.)]);
        }
        Icon::Pause => {
            line(&[(6., 3.), (6., 17.)]);
            line(&[(14., 3.), (14., 17.)]);
        }
        Icon::Start => {
            line(&[(4., 2.), (4., 18.)]);
            line(&[(16., 3.), (6., 10.), (16., 17.), (16., 3.)]);
        }
        Icon::Fit => {
            for (x, y, sx, sy) in [
                (2., 2., 1., 1.),
                (18., 2., -1., 1.),
                (2., 18., 1., -1.),
                (18., 18., -1., -1.),
            ] {
                line(&[(x, y + sy * 5.), (x, y), (x + sx * 5., y)]);
            }
        }
        Icon::Warning => {
            line(&[(10., 2.), (19., 18.), (1., 18.), (10., 2.)]);
            line(&[(10., 7.), (10., 12.)]);
            painter.circle_filled(p(10., 15.), 1., color);
        }
        Icon::Help => {
            painter.circle_stroke(p(10., 10.), 8., stroke);
            line(&[
                (7., 7.),
                (8., 5.),
                (12., 5.),
                (13., 7.),
                (10., 10.),
                (10., 12.),
            ]);
            painter.circle_filled(p(10., 15.), 0.8, color);
        }
        Icon::Save => {
            box_at(p(2., 2.), p(18., 18.));
            box_at(p(6., 2.), p(14., 8.));
            box_at(p(6., 12.), p(14., 18.));
        }
        Icon::Undo | Icon::Redo => {
            let reverse = matches!(icon, Icon::Redo);
            let pts: Vec<_> = [
                (7., 3.),
                (3., 7.),
                (7., 11.),
                (3., 7.),
                (13., 7.),
                (17., 10.),
                (17., 15.),
            ]
            .into_iter()
            .map(|(x, y)| (if reverse { 20. - x } else { x }, y))
            .collect();
            line(&pts);
        }
    }
}

pub fn show(ui: &mut egui::Ui, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20., 20.), egui::Sense::hover());
    paint(ui.painter(), rect, icon, crate::ui_theme::TEXT);
}

pub fn button(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let response = ui.add(
        egui::Button::new("")
            .min_size(egui::vec2(28., 28.))
            .selected(selected),
    );
    paint(
        ui.painter(),
        Rect::from_center_size(response.rect.center(), egui::vec2(18., 18.)),
        icon,
        ui.visuals().text_color(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}
