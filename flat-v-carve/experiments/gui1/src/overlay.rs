//! Translucent selection fill and tool/blade marker geometry for the viewport.
//!
//! Everything here is display-only decoration drawn through the same camera
//! uniform as the motion lines. No overlay value feeds planning, verification
//! or export, and the tool radius shown is the simulator's declared geometry.
use crate::compute::Vertex;

pub const SELECT_FILL: [f32; 4] = [1.0, 0.82, 0.25, 0.32];
pub const BLADE_FILL: [f32; 4] = [0.93, 0.96, 1.0, 0.30];
pub const BLADE_LINE: [f32; 4] = [0.93, 0.96, 1.0, 0.95];
const RING: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glyph {
    /// Flat cutting disc of a cylindrical tool.
    Endmill { radius: f32 },
    /// Cone from the cutting tip to the widest declared radius.
    Vbit {
        radius: f32,
        tip_radius: f32,
        slope: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub glyph: Glyph,
    /// Cutting tip in scene coordinates.
    pub tip: [f32; 3],
    /// Stock-top plane the tool shank is drawn up to.
    pub top: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    pub triangles: Vec<Vertex>,
    pub lines: Vec<Vertex>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty() && self.lines.is_empty()
    }
}

/// `selection` is the picked segment; `half_width` is its fill half-width in
/// the same normalized scene units as the vertices.
pub fn build(
    selection: Option<([f32; 3], [f32; 3])>,
    half_width: f32,
    markers: &[Marker],
) -> Overlay {
    let mut overlay = Overlay::default();
    if let Some((a, b)) = selection {
        ribbon(&mut overlay, a, b, half_width.max(1e-5), SELECT_FILL);
    }
    for marker in markers {
        match marker.glyph {
            Glyph::Endmill { radius } => {
                disc(
                    &mut overlay,
                    marker.tip,
                    radius,
                    SELECT_FILL_ENDMILL,
                    BLADE_LINE,
                );
                axis(&mut overlay, marker.tip, marker.top);
            }
            Glyph::Vbit {
                radius,
                tip_radius,
                slope,
            } => {
                let reach = ((marker.top - marker.tip[2]).max(0.) * slope).max(0.);
                let wide = (tip_radius + reach).min(radius).max(tip_radius);
                cone(
                    &mut overlay,
                    marker.tip,
                    tip_radius,
                    wide,
                    BLADE_FILL,
                    BLADE_LINE,
                );
                axis(&mut overlay, marker.tip, marker.top);
            }
        }
    }
    overlay
}

const SELECT_FILL_ENDMILL: [f32; 4] = [0.55, 0.85, 0.95, 0.30];

fn vertex(p: [f32; 3], color: [f32; 4]) -> Vertex {
    Vertex { position: p, color }
}

fn triangle(out: &mut Vec<Vertex>, a: [f32; 3], b: [f32; 3], c: [f32; 3], color: [f32; 4]) {
    out.push(vertex(a, color));
    out.push(vertex(b, color));
    out.push(vertex(c, color));
}

pub fn ribbon(out: &mut Overlay, a: [f32; 3], b: [f32; 3], half: f32, color: [f32; 4]) {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let normal = if length <= 1e-6 {
        [half, 0.]
    } else {
        [-d[1] / length * half, d[0] / length * half]
    };
    let p0 = [a[0] - normal[0], a[1] - normal[1], a[2]];
    let p1 = [a[0] + normal[0], a[1] + normal[1], a[2]];
    let p2 = [b[0] + normal[0], b[1] + normal[1], b[2]];
    let p3 = [b[0] - normal[0], b[1] - normal[1], b[2]];
    triangle(&mut out.triangles, p0, p1, p2, color);
    triangle(&mut out.triangles, p0, p2, p3, color);
}

fn disc(out: &mut Overlay, centre: [f32; 3], radius: f32, fill: [f32; 4], line: [f32; 4]) {
    let vertices = ring(&centre, radius);
    for i in 0..RING {
        triangle(
            &mut out.triangles,
            centre,
            vertices[i],
            vertices[(i + 1) % RING],
            fill,
        );
    }
    outline(out, &vertices, line);
}

fn cone(
    out: &mut Overlay,
    tip: [f32; 3],
    tip_radius: f32,
    wide: f32,
    fill: [f32; 4],
    line: [f32; 4],
) {
    let top = [tip[0], tip[1], tip[2]];
    let outer = ring(&top, wide.max(1e-5));
    for i in 0..RING {
        // Apex fan plus the flat tip face, so a pointed and a finite-tip bit
        // stay visually distinct.
        triangle(
            &mut out.triangles,
            [tip[0], tip[1], tip[2]],
            outer[i],
            outer[(i + 1) % RING],
            fill,
        );
    }
    if tip_radius > 0. {
        let inner = ring(&top, tip_radius);
        for i in 0..RING {
            triangle(
                &mut out.triangles,
                inner[i],
                outer[i],
                outer[(i + 1) % RING],
                fill,
            );
            triangle(
                &mut out.triangles,
                inner[i],
                outer[(i + 1) % RING],
                inner[(i + 1) % RING],
                fill,
            );
        }
    }
    outline(out, &outer, line);
}

fn ring(centre: &[f32; 3], radius: f32) -> Vec<[f32; 3]> {
    (0..RING)
        .map(|i| {
            let angle = i as f32 / RING as f32 * std::f32::consts::TAU;
            [
                centre[0] + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
                centre[2],
            ]
        })
        .collect()
}

fn outline(out: &mut Overlay, ring: &[[f32; 3]], color: [f32; 4]) {
    for i in 0..ring.len() {
        out.lines.push(vertex(ring[i], color));
        out.lines.push(vertex(ring[(i + 1) % ring.len()], color));
    }
}

fn axis(out: &mut Overlay, tip: [f32; 3], top: f32) {
    out.lines.push(vertex(tip, BLADE_LINE));
    out.lines.push(vertex([tip[0], tip[1], top], BLADE_LINE));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finite(overlay: &Overlay) -> bool {
        overlay
            .triangles
            .iter()
            .chain(&overlay.lines)
            .all(|v| v.position.iter().all(|c| c.is_finite()) && v.color[3] > 0.)
    }

    #[test]
    fn ribbon_is_perpendicular_with_the_requested_half_width() {
        let mut overlay = Overlay::default();
        ribbon(&mut overlay, [0., 0., 0.], [1., 0., 0.], 0.05, SELECT_FILL);
        assert_eq!(overlay.triangles.len(), 6);
        for vertex in &overlay.triangles {
            assert!(finite(&overlay));
            assert_eq!(vertex.position[1].abs(), 0.05);
            assert!(vertex.color[3] < 1.);
        }
    }

    #[test]
    fn degenerate_selection_stays_finite() {
        let mut overlay = Overlay::default();
        ribbon(
            &mut overlay,
            [0.2, 0.3, 0.],
            [0.2, 0.3, 0.],
            0.04,
            SELECT_FILL,
        );
        assert_eq!(overlay.triangles.len(), 6);
        assert!(finite(&overlay));
    }

    #[test]
    fn markers_draw_endmill_and_vbit_geometry() {
        let overlay = build(
            None,
            0.02,
            &[
                Marker {
                    glyph: Glyph::Endmill { radius: 0.03 },
                    tip: [0., 0., -0.1],
                    top: 0.02,
                },
                Marker {
                    glyph: Glyph::Vbit {
                        radius: 0.06,
                        tip_radius: 0.01,
                        slope: 1.,
                    },
                    tip: [0.3, 0.2, -0.05],
                    top: 0.02,
                },
            ],
        );
        assert!(finite(&overlay));
        // 24 fan triangles per marker plus the finite V-bit tip band.
        assert!(overlay.triangles.len() >= 24 * 2 * 3);
        assert_eq!(overlay.lines.len() % 2, 0);
        // The shank is drawn up to the stock top for both markers.
        assert!(
            overlay
                .lines
                .iter()
                .any(|v| (v.position[2] - 0.02).abs() < 1e-6)
        );
    }

    #[test]
    fn vbit_reach_is_clamped_to_the_declared_radius() {
        let mut overlay = Overlay::default();
        cone(
            &mut overlay,
            [0., 0., 0.],
            0.01,
            0.06,
            BLADE_FILL,
            BLADE_LINE,
        );
        let max = overlay
            .lines
            .iter()
            .map(|v| (v.position[0].powi(2) + v.position[1].powi(2)).sqrt())
            .fold(0., f32::max);
        assert!((max - 0.06).abs() < 1e-5);
    }
}
