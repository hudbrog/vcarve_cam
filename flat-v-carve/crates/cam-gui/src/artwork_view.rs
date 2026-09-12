//! Display-only filled-region picking and placement candidates. Assignment is
//! a separate document command; pointer selection never changes machining.
use crate::{authoring::Component, pick::Camera};
use cam_core::{geometry::Point, project::v5::GeometryRef, svg::Placement};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GestureMode {
    #[default]
    Select,
    Move,
    Rotate,
    Scale,
}

/// Converts logical viewport points to the setup Z=0 plane. Physical DPI is
/// handled by egui; it must not be applied a second time here.
pub fn setup_point(
    camera: Camera,
    bounds: [f64; 4],
    rect: egui::Rect,
    screen: egui::Pos2,
) -> Point {
    let q = camera.to_ndc(
        [screen.x - rect.center().x, screen.y - rect.center().y],
        [rect.width(), rect.height()],
    );
    let u = q[0] as f64 * camera.aspect as f64 / camera.zoom as f64;
    let v = q[1] as f64 / camera.zoom as f64 / if camera.iso { 0.65 } else { 1. };
    let (s, c) = (camera.yaw as f64).sin_cos();
    let scale = 1.6
        / (bounds[2] - bounds[0])
            .max(bounds[3] - bounds[1])
            .max(0.001);
    Point::new(
        (c * u + s * v) / scale + (bounds[0] + bounds[2]) / 2.,
        (-s * u + c * v) / scale + (bounds[1] + bounds[3]) / 2.,
    )
}
pub fn screen_point(
    camera: Camera,
    bounds: [f64; 4],
    rect: egui::Rect,
    point: Point,
) -> egui::Pos2 {
    let scale = 1.6
        / (bounds[2] - bounds[0])
            .max(bounds[3] - bounds[1])
            .max(0.001);
    let q = camera.ndc([
        ((point.x - (bounds[0] + bounds[2]) / 2.) * scale) as f32,
        ((point.y - (bounds[1] + bounds[3]) / 2.) * scale) as f32,
        0.,
    ]);
    let p = camera.to_points(q, [rect.width(), rect.height()]);
    rect.center() + egui::vec2(p[0], p[1])
}
fn in_ring(point: Point, ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    if ring.len() < 3 {
        return false;
    }
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        if (a[1] > point.y) != (b[1] > point.y)
            && point.x < (b[0] - a[0]) * (point.y - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}
/// Return every coincident candidate, preserving qualified source revisions.
/// Canonical component rings are outer/hole boundaries, so parity excludes holes.
pub fn hit_candidates(components: &[Component], point: Point) -> Vec<GeometryRef> {
    components
        .iter()
        .filter(|c| {
            point.x >= c.bounds[0]
                && point.x <= c.bounds[2]
                && point.y >= c.bounds[1]
                && point.y <= c.bounds[3]
        })
        .filter(|c| c.rings.iter().filter(|ring| in_ring(point, ring)).count() % 2 == 1)
        .map(|c| c.reference.clone())
        .collect()
}
/// Rotation and uniform scale pivot around the page origin chosen in numeric
/// placement (which maps to setup 0,0). Translation changes that page origin
/// through the inverse conversion; it never reinterprets it as setup offset.
pub fn candidate(
    initial: &Placement,
    mode: GestureMode,
    start: Point,
    end: Point,
) -> Result<Placement, String> {
    let mut result = initial.clone();
    match mode {
        GestureMode::Select => {}
        GestureMode::Move => {
            let page_start = initial.to_page(start).map_err(|e| e.to_string())?;
            let page_end = initial.to_page(end).map_err(|e| e.to_string())?;
            result.origin_mm.x -= page_end.x - page_start.x;
            result.origin_mm.y -= page_end.y - page_start.y;
        }
        GestureMode::Rotate => {
            if start.x.hypot(start.y) < 1e-6 || end.x.hypot(end.y) < 1e-6 {
                return Err("Drag away from the rotation pivot".into());
            }
            result.rotation_deg += (end.y.atan2(end.x) - start.y.atan2(start.x)).to_degrees();
        }
        GestureMode::Scale => {
            if start.x.hypot(start.y) < 1e-6 {
                return Err("Drag away from the scale pivot".into());
            }
            result.scale *= end.x.hypot(end.y) / start.x.hypot(start.y);
        }
    }
    result
        .to_setup(Point::new(0., 0.))
        .map_err(|e| e.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_round_trip_and_gesture_use_the_core_placement_convention() {
        let bounds = [-20., -10., 80., 60.];
        let rect = egui::Rect::from_min_size(egui::pos2(210., 150.), egui::vec2(720., 480.));
        for iso in [false, true] {
            for yaw in [0., 0.8, -1.2] {
                let camera = Camera {
                    iso,
                    yaw,
                    zoom: 1.7,
                    aspect: rect.width() / rect.height(),
                };
                let p = Point::new(23.5, 17.2);
                let round =
                    setup_point(camera, bounds, rect, screen_point(camera, bounds, rect, p));
                assert!((round.x - p.x).abs() < 1e-4 && (round.y - p.y).abs() < 1e-4);
            }
        }
        let placement = Placement {
            origin_mm: Point::new(7., -3.),
            scale: 1.8,
            rotation_deg: 37.,
        };
        let page = Point::new(20., 25.);
        let before = placement.to_setup(page).unwrap();
        let moved = candidate(
            &placement,
            GestureMode::Move,
            Point::new(1., 2.),
            Point::new(9., -3.),
        )
        .unwrap();
        let after = moved.to_setup(page).unwrap();
        assert!((after.x - before.x - 8.).abs() < 1e-9 && (after.y - before.y + 5.).abs() < 1e-9);
        let scaled = candidate(
            &placement,
            GestureMode::Scale,
            Point::new(10., 0.),
            Point::new(20., 0.),
        )
        .unwrap();
        assert_eq!(scaled.origin_mm, placement.origin_mm);
        assert_eq!(scaled.scale, 3.6);
        let rotated = candidate(
            &placement,
            GestureMode::Rotate,
            Point::new(10., 0.),
            Point::new(0., 10.),
        )
        .unwrap();
        assert!((rotated.rotation_deg - 127.).abs() < 1e-9);
    }
    #[test]
    fn filled_picking_excludes_holes_and_returns_all_coincident_owners() {
        let job=crate::authoring::import_svg("letters.svg".into(),"<svg xmlns='http://www.w3.org/2000/svg' width='40mm' height='20mm' viewBox='0 0 40 20'><path id='letter-o' fill-rule='evenodd' d='M0 0H20V20H0Z M5 5V15H15V5Z'/><rect id='overlap' x='1' y='1' width='2' height='2'/></svg>".into()).unwrap();
        let (meta, _) = crate::session::execute(
            &mut cam_service::retained::Retained::new(),
            crate::session::Command::Preview {
                job: job.to_json().unwrap(),
            },
        )
        .unwrap();
        let components: Vec<Component> =
            serde_json::from_value(meta.report["gui2"]["components"].clone()).unwrap();
        assert!(hit_candidates(&components, Point::new(10., 10.)).is_empty());
        assert_eq!(hit_candidates(&components, Point::new(2., 18.)).len(), 2);
        assert_eq!(hit_candidates(&components, Point::new(18., 18.)).len(), 1);
    }
}
