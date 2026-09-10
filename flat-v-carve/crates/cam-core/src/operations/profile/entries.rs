//! Entry and lead geometry for profiles (plan section 10.4, slice E3):
//! tangent line/arc leads blending into and out of the compensated contour,
//! built on the explicit scrap side and checked against the retained
//! geometry of every selected contour. Arcs are linearized within a share of
//! the motion tolerance (native G2/G3 output is later work), and a lead that
//! cannot fit is a located error — never silently shortened or dropped.
use crate::geometry::Point;

/// Unit direction of a non-degenerate segment.
pub(crate) fn edge_dir(a: Point, b: Point) -> Option<Point> {
    let len = a.distance(b);
    (len > 1e-12).then(|| Point::new((b.x - a.x) / len, (b.y - a.y) / len))
}

fn rot90ccw(v: Point) -> Point {
    Point::new(-v.y, v.x)
}

/// Unit normal pointing at the scrap side of a loop traveled along `t`.
///
/// Canonical loops are CCW, so the loop interior lies on the feed-left side
/// when traveling forward. The retained material is inside the compensated
/// loop exactly when the contour is cut on its outside (plan section 10.2);
/// the scrap side is the opposite normal.
pub(crate) fn scrap_normal(t: Point, retained_inside_loop: bool, forward: bool) -> Point {
    let retained_left = forward == retained_inside_loop;
    if retained_left {
        // Scrap on the right of travel.
        Point::new(t.y, -t.x)
    } else {
        rot90ccw(t)
    }
}

/// The configured shape of one lead (plan section 10.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LeadShape {
    TangentLine { length_mm: f64 },
    TangentArc { radius_mm: f64, sweep_rad: f64 },
}

impl LeadShape {
    pub(crate) fn path_len(&self) -> f64 {
        match *self {
            Self::TangentLine { length_mm } => length_mm,
            Self::TangentArc {
                radius_mm,
                sweep_rad,
            } => radius_mm * sweep_rad,
        }
    }

    /// Polyline of a lead-in: it ends at `seam` arriving along `t` (the
    /// travel direction) and bulges toward the scrap side. Arc leads need
    /// the scrap-side normal; line leads ignore it.
    pub(crate) fn lead_in_polyline(
        &self,
        seam: Point,
        t: Point,
        scrap: Option<Point>,
        tolerance: f64,
    ) -> Vec<Point> {
        match *self {
            Self::TangentLine { length_mm } => {
                vec![
                    Point::new(seam.x - t.x * length_mm, seam.y - t.y * length_mm),
                    seam,
                ]
            }
            Self::TangentArc {
                radius_mm,
                sweep_rad,
            } => {
                // Departing the seam backwards along the travel tangent and
                // reversing keeps the arrival tangent `t` while the arc
                // bulges to the scrap side.
                let scrap = scrap.expect("arc leads require a retained side");
                let mut points = arc_points(
                    seam,
                    Point::new(-t.x, -t.y),
                    scrap,
                    radius_mm,
                    sweep_rad,
                    tolerance,
                );
                points.reverse();
                points
            }
        }
    }

    /// Polyline of a lead-out: it departs `attach` along `t` and bulges
    /// toward the scrap side.
    pub(crate) fn lead_out_polyline(
        &self,
        attach: Point,
        t: Point,
        scrap: Option<Point>,
        tolerance: f64,
    ) -> Vec<Point> {
        match *self {
            Self::TangentLine { length_mm } => {
                vec![
                    attach,
                    Point::new(attach.x + t.x * length_mm, attach.y + t.y * length_mm),
                ]
            }
            Self::TangentArc {
                radius_mm,
                sweep_rad,
            } => {
                let scrap = scrap.expect("arc leads require a retained side");
                arc_points(attach, t, scrap, radius_mm, sweep_rad, tolerance)
            }
        }
    }
}

/// Arc departing `attach` along `depart_tangent`, curving toward
/// `center_side` (the center lies that way), swept by `sweep_rad` and
/// linearized within `tolerance` chord error (plan section 8.4).
fn arc_points(
    attach: Point,
    depart_tangent: Point,
    center_side: Point,
    radius: f64,
    sweep_rad: f64,
    tolerance: f64,
) -> Vec<Point> {
    let center = Point::new(
        attach.x + center_side.x * radius,
        attach.y + center_side.y * radius,
    );
    let phi0 = (attach.y - center.y).atan2(attach.x - center.x);
    // Direction of travel for increasing angle: rot90ccw of the radius
    // vector; pick the sweep sign whose departure matches the tangent.
    let increasing = rot90ccw(Point::new(attach.x - center.x, attach.y - center.y));
    let sign = if increasing.x * depart_tangent.x + increasing.y * depart_tangent.y >= 0. {
        1.
    } else {
        -1.
    };
    // Chord error e over step angle a satisfies e = r(1 - cos(a/2)); bound a
    // by 2*acos(1 - e/r), spending at most half the motion tolerance here so
    // import/offset/rounding keep their share. Large error budgets fall back
    // to quarter-turn steps.
    let chord_budget = (tolerance / 2.).max(1e-9);
    let step = if chord_budget >= radius {
        std::f64::consts::FRAC_PI_2
    } else {
        2. * (1. - chord_budget / radius).clamp(-1., 1.).acos()
    };
    let steps = ((sweep_rad / step).ceil() as usize).max(1);
    let mut points = vec![attach];
    for index in 1..=steps {
        let phi = phi0 + sign * sweep_rad * index as f64 / steps as f64;
        points.push(Point::new(
            center.x + radius * phi.cos(),
            center.y + radius * phi.sin(),
        ));
    }
    points
}

pub(crate) fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = Point::new(b.x - a.x, b.y - a.y);
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq <= 1e-24 {
        return p.distance(a);
    }
    let t = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len_sq;
    let t = t.clamp(0., 1.);
    p.distance(Point::new(a.x + ab.x * t, a.y + ab.y * t))
}

fn on_segment(p: Point, a: Point, b: Point) -> bool {
    p.x <= a.x.max(b.x) + 1e-12
        && p.x >= a.x.min(b.x) - 1e-12
        && p.y <= a.y.max(b.y) + 1e-12
        && p.y >= a.y.min(b.y) - 1e-12
}

fn orientation(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    if ((o1 > 0.) != (o2 > 0.)) && ((o3 > 0.) != (o4 > 0.)) {
        return true;
    }
    (o1.abs() <= 1e-12 && on_segment(c, a, b))
        || (o2.abs() <= 1e-12 && on_segment(d, a, b))
        || (o3.abs() <= 1e-12 && on_segment(a, c, d))
        || (o4.abs() <= 1e-12 && on_segment(b, c, d))
}

fn segments_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if segments_intersect(a, b, c, d) {
        return 0.;
    }
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

/// Why a lead violates the retained geometry of one contour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LeadViolation {
    /// The centerline enters the retained region itself.
    InsideRetained,
    /// The cutter disc comes closer than its radius to the retained
    /// boundary (with the rasterization allowance).
    TooClose { distance_mm: f64 },
}

/// Check one lead polyline against the retained side of one selected
/// contour ring. `retained_inside_ring` is true for contours cut on their
/// outside (the part or island interior is retained) and false for inside
/// cuts (the surrounding material is retained). The compensated loop itself
/// runs exactly one radius from the ring, so `slop` absorbs offset-grid
/// quantization; on-contour selections skip their own ring (the cut is
/// intentionally on the line).
pub(crate) fn lead_violation(
    points: &[Point],
    ring: &[Point],
    ring_contains: impl Fn(&[Point], Point) -> bool,
    retained_inside_ring: bool,
    radius: f64,
    slop: f64,
) -> Option<LeadViolation> {
    let ring_edges = |i: usize| (ring[i], ring[(i + 1) % ring.len()]);
    for pair in points.windows(2) {
        let p = pair[0];
        let q = pair[1];
        let p_inside = ring_contains(ring, p);
        let q_inside = ring_contains(ring, q);
        let enters_retained = if retained_inside_ring {
            p_inside || q_inside
        } else {
            !p_inside || !q_inside
        };
        if enters_retained {
            return Some(LeadViolation::InsideRetained);
        }
        for index in 0..ring.len() {
            let (c, d) = ring_edges(index);
            if segments_intersect(p, q, c, d) {
                return Some(LeadViolation::InsideRetained);
            }
            let distance = segments_distance(p, q, c, d);
            if distance < radius - slop {
                return Some(LeadViolation::TooClose {
                    distance_mm: distance,
                });
            }
        }
    }
    None
}
