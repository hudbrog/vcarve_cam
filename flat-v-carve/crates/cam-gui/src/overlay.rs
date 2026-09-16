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

/// A cutter where the animation put it, drawn from the simulator's own tool
/// geometry. The body is generated from [`crate::sim::ToolSpec::profile`], so
/// the drawn reach at any height is the reach the field removes there; nothing
/// above the cutting portion is drawn until a tool states a shaft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker<'a> {
    pub tool: crate::sim::ToolSpec,
    /// Normalized scene units per millimetre: the same factor the motion
    /// vertices were written with, so the body is drawn to scale.
    pub scale: f32,
    /// Cutting tip in normalized scene coordinates.
    pub tip: [f32; 3],
    /// Stock-top plane the tool axis is drawn up to.
    pub top: f32,
    /// Blade heading in degrees CCW from +X. Knives only: it rotates the blade
    /// about the holder pivot, which is how a corner swivel reads on screen.
    pub heading_deg: Option<f32>,
    /// How the tool is held: the shaft above the cutter and its stickout.
    pub assembly: cam_core::project::ToolAssembly,
    /// The machine's holder body, resolved from the catalogue. Nothing is drawn
    /// from it unless the assembly states where its bottom face sits.
    pub holder: Option<&'a [cam_core::post::HolderSegment]>,
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
/// the same normalized scene units as the vertices. `trail` is the part of the
/// move the cutter has already made, drawn so the drawn path ends where the
/// tool is instead of at the next motion boundary.
pub fn build(
    selection: Option<([f32; 3], [f32; 3])>,
    trail: Option<([f32; 3], [f32; 3])>,
    half_width: f32,
    markers: &[Marker<'_>],
) -> Overlay {
    let mut overlay = Overlay::default();
    if let Some((a, b)) = selection {
        ribbon(&mut overlay, a, b, half_width.max(1e-5), SELECT_FILL);
    }
    if let Some((a, b)) = trail {
        ribbon(&mut overlay, a, b, half_width.max(1e-5), TRAIL_LINE);
    }
    for marker in markers {
        draw_cutter(&mut overlay, marker);
    }
    overlay
}

/// One cutter body: a surface of revolution for a mill, a blade plate for a
/// knife. Both are placed by the tip and drawn to the same scale as the paths.
fn draw_cutter(out: &mut Overlay, marker: &Marker<'_>) {
    match marker.tool {
        crate::sim::ToolSpec::Knife {
            offset,
            max_cut_depth,
        } => {
            blade(out, marker, offset as f32, max_cut_depth as f32);
            assembly(out, marker, marker.tip[2]);
        }
        tool => {
            let Some(profile) = tool.profile() else {
                return;
            };
            // The cutter's own profile, in absolute scene heights.
            let turned: Vec<(f32, f32)> = profile
                .iter()
                .map(|(height, radius)| {
                    (
                        marker.tip[2] + *height as f32 * marker.scale,
                        *radius as f32 * marker.scale,
                    )
                })
                .collect();
            turned_body(out, marker, &turned, true);
            axis(out, marker.tip, marker.top);
            let cutter_top = turned.last().map_or(marker.tip[2], |(z, _)| *z);
            assembly(out, marker, cutter_top);
        }
    }
}

/// The rest of the assembly: the shaft above the cutter, and the machine's
/// holder standing on the stickout. A tool that states no stickout says nothing
/// about where a holder sits, so none is drawn — the axis line carries the eye
/// instead of a body in the wrong place.
fn assembly(out: &mut Overlay, marker: &Marker<'_>, body_top: f32) {
    let scale = marker.scale;
    let shaft_radius = marker
        .assembly
        .shaft_diameter_mm
        .map(|d| (d / 2.) as f32 * scale);
    let Some(stickout) = marker.assembly.stickout_mm else {
        // No stickout: the shaft's top is unknown, so it is drawn only as far as
        // the stock top, and not at all when that is below the cutter's own top.
        if let Some(radius) = shaft_radius
            && marker.top > body_top + 1e-4
        {
            let shaft = [(body_top, radius), (marker.top, radius)];
            turned_body_with(out, marker, &shaft, false, SHAFT_FILL, SHAFT_LINE);
        }
        return;
    };
    let base = marker.tip[2] + stickout as f32 * scale;
    if let Some(radius) = shaft_radius {
        let shaft = [(body_top, radius), (base.max(body_top), radius)];
        turned_body_with(out, marker, &shaft, false, SHAFT_FILL, SHAFT_LINE);
    }
    let Some(segments) = marker.holder else {
        return;
    };
    let mut z = base;
    for (index, segment) in segments.iter().enumerate() {
        let next = z + segment.height_mm as f32 * scale;
        let body = [
            (z, segment.lower_diameter_mm as f32 / 2. * scale),
            (next, segment.upper_diameter_mm as f32 / 2. * scale),
        ];
        // The holder's own bottom face is drawn; the joints above it are not, so
        // a stack of segments reads as one body.
        turned_body_with(out, marker, &body, index == 0, HOLDER_FILL, HOLDER_LINE);
        z = next;
    }
}

/// Revolve an absolute `(z, radius)` profile into a closed body: quads between
/// consecutive rings and an outline on each ring so the silhouette reads.
fn turned_body(out: &mut Overlay, marker: &Marker<'_>, profile: &[(f32, f32)], cap: bool) {
    turned_body_with(out, marker, profile, cap, SELECT_FILL_ENDMILL, BLADE_LINE);
}

fn turned_body_with(
    out: &mut Overlay,
    marker: &Marker<'_>,
    profile: &[(f32, f32)],
    cap: bool,
    fill: [f32; 4],
    line: [f32; 4],
) {
    let ring = |z: f32, radius: f32| -> Vec<[f32; 3]> {
        (0..RING)
            .map(|i| {
                let angle = i as f32 / RING as f32 * std::f32::consts::TAU;
                [
                    marker.tip[0] + radius * angle.cos(),
                    marker.tip[1] + radius * angle.sin(),
                    z,
                ]
            })
            .collect()
    };
    let rings: Vec<Vec<[f32; 3]>> = profile
        .iter()
        .map(|(z, radius)| ring(*z, *radius))
        .collect();
    for pair in rings.windows(2) {
        for i in 0..RING {
            let j = (i + 1) % RING;
            triangle(&mut out.triangles, pair[0][i], pair[0][j], pair[1][j], fill);
            triangle(&mut out.triangles, pair[0][i], pair[1][j], pair[1][i], fill);
        }
    }
    for ring in &rings {
        outline(out, ring, line);
    }
    let Some((base, radius)) = profile.first().copied() else {
        return;
    };
    if cap && radius > 0. {
        let centre = [marker.tip[0], marker.tip[1], base];
        for i in 0..RING {
            triangle(
                &mut out.triangles,
                centre,
                rings[0][i],
                rings[0][(i + 1) % RING],
                fill,
            );
        }
    }
}

/// A drag knife: the blade stands from the cutting tip to the holder pivot
/// along the modeled heading, `max_cut_depth` tall. The pivot is where the
/// programmed position is, and the tip is `blade_offset` ahead of it, so the
/// drawn blade turns exactly as the modeled heading does.
fn blade(out: &mut Overlay, marker: &Marker, offset: f32, depth: f32) {
    let heading = marker.heading_deg.unwrap_or(0.).to_radians();
    let (sin, cos) = heading.sin_cos();
    let reach = offset * marker.scale;
    let pivot = [
        marker.tip[0] + reach * cos,
        marker.tip[1] + reach * sin,
        marker.tip[2],
    ];
    let rise = depth.max(0.) * marker.scale;
    let top = |p: [f32; 3]| [p[0], p[1], p[2] + rise.max(1e-4)];
    let quad = |out: &mut Overlay, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
        triangle(&mut out.triangles, a, b, c, BLADE_FILL);
        triangle(&mut out.triangles, a, c, d, BLADE_FILL);
        for (p, q) in [(a, b), (b, c), (c, d), (d, a)] {
            out.lines.push(vertex(p, BLADE_LINE));
            out.lines.push(vertex(q, BLADE_LINE));
        }
    };
    // Two faces of the same plate, offset across the heading so the blade has a
    // visible thickness in the view instead of being a zero-area polygon.
    let thickness = (reach * 0.06).max(1e-4);
    let across = [-sin * thickness, cos * thickness];
    for side in [-1., 1.] {
        let shift = [across[0] * side, across[1] * side];
        let at = |p: [f32; 3]| [p[0] + shift[0], p[1] + shift[1], p[2]];
        quad(
            out,
            at(marker.tip),
            at(pivot),
            at(top(pivot)),
            at(top(marker.tip)),
        );
    }
    // The pivot is the programmed point; a small ring marks it without claiming
    // a holder dimension the tool does not state.
    let mark = ring(&pivot, (reach * 0.25).max(1e-4));
    outline(out, &mark, BLADE_LINE);
}

pub(crate) const SELECT_FILL_ENDMILL: [f32; 4] = [0.55, 0.85, 0.95, 0.30];
/// The shaft is part of the tool body but not part of the cut, so it is drawn
/// dimmer than the cutting portion.
const SHAFT_FILL: [f32; 4] = [0.62, 0.66, 0.72, 0.22];
const SHAFT_LINE: [f32; 4] = [0.78, 0.82, 0.88, 0.75];
/// The holder belongs to the machine, not the tool: a warmer, heavier tone.
const HOLDER_FILL: [f32; 4] = [0.78, 0.72, 0.55, 0.30];
const HOLDER_LINE: [f32; 4] = [0.92, 0.88, 0.72, 0.9];
/// The in-flight move's cut part: the same colour the cutting paths use, so it
/// reads as the path the tool is making rather than a selection.
pub(crate) const TRAIL_LINE: [f32; 4] = [1., 0.62, 0.2, 0.85];

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

/// A small screen-independent marker for one candidate anchor: a square with a
/// pale outline so it stays visible over stock, paths and generated bridges.
pub fn candidate_marker(out: &mut Overlay, centre: [f32; 3], fill: [f32; 4]) {
    let half = 0.022;
    let corners = [
        [centre[0] - half, centre[1] - half, centre[2]],
        [centre[0] + half, centre[1] - half, centre[2]],
        [centre[0] + half, centre[1] + half, centre[2]],
        [centre[0] - half, centre[1] + half, centre[2]],
    ];
    triangle(&mut out.triangles, corners[0], corners[1], corners[2], fill);
    triangle(&mut out.triangles, corners[0], corners[2], corners[3], fill);
    for index in 0..4 {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        out.lines.push(vertex(a, BLADE_LINE));
        out.lines.push(vertex(b, BLADE_LINE));
    }
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

    /// S1: the part of the in-flight move the cutter has already made is drawn,
    /// so the drawn path ends where the tool is instead of jumping a whole
    /// motion ahead of it when the move starts.
    #[test]
    fn a_trail_is_drawn_for_the_part_of_the_move_already_cut() {
        let mut overlay = build(None, Some(([0., 0., 0.], [0.5, 0., 0.])), 0.02, &[]);
        assert_eq!(overlay.triangles.len(), 6, "one ribbon for the trail");
        assert!(finite(&overlay));
        // Nothing to draw is nothing drawn: no selection, no trail, no marker.
        overlay = build(None, None, 0.02, &[]);
        assert!(overlay.is_empty());
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

    fn marker(
        tool: crate::sim::ToolSpec,
        tip: [f32; 3],
        heading_deg: Option<f32>,
    ) -> Marker<'static> {
        Marker {
            tool,
            scale: 1.,
            tip,
            top: 0.,
            heading_deg,
            assembly: Default::default(),
            holder: None,
        }
    }

    #[test]
    fn markers_draw_the_declared_cutter_bodies() {
        let endmill = crate::sim::ToolSpec::Endmill {
            diameter: 6.,
            cutting_length: 12.,
        };
        let vbit = crate::sim::ToolSpec::Vbit {
            angle: 90.,
            tip: 0.4,
            diameter: 6.,
            height: 3.,
        };
        let overlay = build(
            None,
            None,
            0.02,
            &[
                marker(endmill, [0., 0., -0.1], None),
                marker(vbit, [0.3, 0.2, -0.05], None),
            ],
        );
        assert!(finite(&overlay));
        // Each body is a ring of quads per profile segment, plus its outline.
        assert!(overlay.triangles.len() >= 24 * 6);
        assert_eq!(overlay.lines.len() % 2, 0);
        // The tool axis is drawn up to the stock top for both markers, so a
        // short cutter still reads as held.
        assert!(
            overlay
                .lines
                .iter()
                .any(|v| (v.position[2] - 0.).abs() < 1e-6)
        );
        // The endmill body stops at its cutting length: 12 mm above the tip at
        // z = -0.1 in scene units of one per millimetre.
        let top = overlay
            .triangles
            .iter()
            .map(|v| v.position[2])
            .fold(f32::MIN, f32::max);
        assert!((top - 11.9).abs() < 1e-3, "endmill body height {top}");
    }

    /// S2/D6: the drawn body reaches exactly as far as the removal does. A
    /// single plunge is cut into a field, the removed footprint is measured, and
    /// the radius of the profile at the same depth has to match it.
    #[test]
    fn a_cutter_body_reaches_exactly_as_far_as_the_removal() {
        use crate::sim::{Field, Interpolation, Motion, Stock};

        let stock = Stock {
            x0: -6.,
            y0: -6.,
            x1: 6.,
            y1: 6.,
            thickness_mm: 6.,
        };
        let cases = [
            crate::sim::ToolSpec::Endmill {
                diameter: 6.,
                cutting_length: 12.,
            },
            crate::sim::ToolSpec::Vbit {
                angle: 90.,
                tip: 0.4,
                diameter: 6.,
                height: 3.,
            },
        ];
        for tool in cases {
            for depth in [0.5_f64, 1.4, 3.] {
                let plunge = Motion {
                    kind: "cut".into(),
                    tool: 0,
                    stage: 0,
                    interpolation: Interpolation::Feed,
                    feed_mm_min: Some(100.),
                    arc: None,
                    x0: 0.,
                    y0: 0.,
                    z0: 0.,
                    x1: 0.,
                    y1: 0.,
                    z1: -depth,
                };
                let mut field = Field::new(stock, &[tool], 0.1).unwrap();
                field.apply(&plunge, 0., 1.).unwrap();
                // Widest cell the plunge actually removed, in millimetres from
                // the tool axis, with half a cell for the cell's own extent.
                let mut measured: f64 = 0.;
                for row in 0..field.rows {
                    for col in 0..field.cols {
                        let (removed, _, _) = field.cell_at(col, row);
                        if removed == 0 {
                            continue;
                        }
                        let x = stock.x0 + (col as f64 + 0.5) * field.cell;
                        let y = stock.y0 + (row as f64 + 0.5) * field.cell;
                        measured = measured.max((x * x + y * y).sqrt() + field.cell * 0.5);
                    }
                }
                let expected = tool
                    .radius_at_depth(depth)
                    .expect("a mill has a cutting radius");
                assert!(
                    (measured - expected).abs() <= field.cell,
                    "{tool:?} at {depth} mm: the cut reaches {measured:.3} mm, \
                     the drawn body reaches {expected:.3} mm"
                );
                // And the profile itself agrees with the envelope at its own
                // heights, so the drawn silhouette is the cutting silhouette.
                for (height, radius) in tool.profile().expect("a mill has a profile") {
                    let envelope = tool.radius_at_depth(height).unwrap();
                    assert!(
                        (radius - envelope).abs() < 1e-9,
                        "{tool:?} at {height} mm: body {radius} vs envelope {envelope}"
                    );
                }
            }
        }
    }

    /// The knife has no surface of revolution, so its equivalent claim is the
    /// one the plan's section 12.5 makes: the blade stands from the programmed
    /// pivot to `blade_offset` along the modeled heading.
    #[test]
    fn the_knife_blade_stands_along_the_modeled_heading() {
        let tool = crate::sim::ToolSpec::Knife {
            offset: 2.,
            max_cut_depth: 1.5,
        };
        assert!(tool.profile().is_none(), "a knife is not a revolved body");
        // Tip at the origin means the pivot is two millimetres along +X, and
        // turning the heading through 90 degrees moves it to +Y.
        let along_x = build(None, None, 0.02, &[marker(tool, [0., 0., -0.5], Some(0.))]);
        let pivot = |overlay: &Overlay| {
            overlay
                .lines
                .iter()
                .map(|v| [v.position[0], v.position[1]])
                .fold([f32::MIN, f32::MIN], |a, b| {
                    [a[0].max(b[0]), a[1].max(b[1])]
                })
        };
        // The plate is drawn with a little thickness and the pivot carries a
        // ring, so the reach lands within a fraction of a millimetre of the
        // modeled offset rather than exactly on it.
        let x = pivot(&along_x);
        assert!(
            (x[0] - 2.).abs() < 0.6 && x[0] >= 2.,
            "blade reaches {x:?} for heading 0"
        );
        assert!(x[1].abs() < 0.6, "heading 0 keeps the blade on the X axis");
        let along_y = build(None, None, 0.02, &[marker(tool, [0., 0., -0.5], Some(90.))]);
        let y = pivot(&along_y);
        assert!(
            (y[1] - 2.).abs() < 0.6 && y[1] >= 2.,
            "blade reaches {y:?} for heading 90°"
        );
        assert!(
            y[0].abs() < 0.6,
            "heading 90° keeps the blade on the Y axis"
        );
        // The blade is as tall as the tool says it can cut.
        let highest = along_x
            .triangles
            .iter()
            .map(|v| v.position[2])
            .fold(f32::MIN, f32::max);
        assert!((highest - 1.).abs() < 1e-3, "blade top at {highest}");
    }

    /// S3: the assembly is drawn only where the tool says it is. The holder's
    /// bottom face sits on the stickout and its widest ring is the catalogue
    /// nut; a tool that states no stickout gets no holder, because nothing says
    /// where one would be.
    #[test]
    fn the_assembly_is_drawn_only_where_the_tool_states_it() {
        let holder = cam_core::post::HolderSelection::catalogue("er20")
            .body()
            .expect("a catalogue holder");
        let whole = Marker {
            tool: crate::sim::ToolSpec::Endmill {
                diameter: 6.,
                cutting_length: 12.,
            },
            scale: 1.,
            tip: [0., 0., -2.],
            top: 0.,
            heading_deg: None,
            assembly: cam_core::project::ToolAssembly {
                shaft_diameter_mm: Some(6.),
                stickout_mm: Some(30.),
            },
            holder: Some(&holder),
        };
        let widest = |overlay: &Overlay| {
            overlay
                .lines
                .iter()
                .map(|v| (v.position[0].powi(2) + v.position[1].powi(2)).sqrt())
                .fold(0., f32::max)
        };
        let overlay = build(None, None, 0.02, &[whole]);
        assert!(finite(&overlay));
        // The holder's bottom face lands on the stickout: 30 mm above the tip.
        assert!(
            overlay
                .lines
                .iter()
                .any(|v| (v.position[2] - 28.).abs() < 1e-3),
            "holder base at 28 (tip -2 + stickout 30)"
        );
        // ER20's nut is 25 mm across, so the widest ring is 12.5 either side.
        assert!(
            (widest(&overlay) - 12.5).abs() < 1e-3,
            "widest ring {}",
            widest(&overlay)
        );

        // A holder with no stickout is not drawn: only the cutter is, because
        // the shaft's own top is unknown and the stock top is below it.
        let nowhere = Marker {
            assembly: cam_core::project::ToolAssembly {
                shaft_diameter_mm: Some(6.),
                stickout_mm: None,
            },
            ..whole
        };
        let overlay = build(None, None, 0.02, &[nowhere]);
        assert!(
            (widest(&overlay) - 3.).abs() < 1e-3,
            "cutter only: {}",
            widest(&overlay)
        );
        assert!(
            !overlay
                .lines
                .iter()
                .any(|v| (v.position[2] - 28.).abs() < 1e-3),
            "no holder without a stickout"
        );

        // With neither value stated, the cutter and its axis are all there is.
        let bare = Marker {
            assembly: Default::default(),
            holder: Some(&holder),
            ..whole
        };
        let overlay = build(None, None, 0.02, &[bare]);
        assert!((widest(&overlay) - 3.).abs() < 1e-3);
        assert!(finite(&overlay));
    }
}
