//! Explicit entries and leads, checked against the full pocket cutter envelope.
use super::{
    planner::Builder,
    verify::{self, error},
};
use crate::{
    geometry::{BoundaryQuery, Point, Region, Result},
    motion::Position,
    project::{LeadSpec, v5::PocketEntry},
    toolpath::{ArcMove, Interpolation, MotionPurpose, PlannedMotion},
};

#[derive(Clone)]
struct Lead {
    from: Point,
    to: Point,
    interpolation: Interpolation,
    feed: f64,
}

fn rotate(v: Point, angle: f64) -> Point {
    let (s, c) = angle.sin_cos();
    Point::new(c * v.x - s * v.y, s * v.x + c * v.y)
}
fn sum(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}
fn diff(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

fn lead(
    query: &BoundaryQuery,
    spec: &LeadSpec,
    seam: Point,
    tangent: Point,
    incoming: bool,
    clearance: f64,
    e: f64,
) -> Result<Option<Lead>> {
    let (length, feed) = match spec {
        LeadSpec::None => return Ok(None),
        LeadSpec::TangentLine {
            length_mm,
            feed_mm_min,
        } => (length_mm.unwrap(), feed_mm_min.unwrap()),
        LeadSpec::TangentArc {
            radius_mm,
            sweep_deg,
            feed_mm_min,
        } => {
            let radius = radius_mm.unwrap();
            let sweep = sweep_deg.unwrap().to_radians();
            // Try both tangent circles. Only the one contained in the pocket
            // is admissible; hole and outer boundaries share this rule.
            for clockwise in [false, true] {
                let sign = if clockwise { -1. } else { 1. };
                let v = Point::new(sign * tangent.y * radius, -sign * tangent.x * radius);
                let center = diff(seam, v);
                let other = sum(
                    center,
                    rotate(v, sign * if incoming { -sweep } else { sweep }),
                );
                let (from, to) = if incoming {
                    (other, seam)
                } else {
                    (seam, other)
                };
                let arc = ArcMove { center, clockwise };
                if arc_inside(query, arc, from, to, clearance, e)? {
                    return Ok(Some(Lead {
                        from,
                        to,
                        interpolation: Interpolation::ArcFeed(arc),
                        feed: feed_mm_min.unwrap(),
                    }));
                }
            }
            return Err(error(
                "POCKET_LEAD_NO_FIT",
                "Requested tangent arc lead cannot fit inside the pocket",
            ));
        }
    };
    let other = sum(
        seam,
        Point::new(
            tangent.x * length * if incoming { -1. } else { 1. },
            tangent.y * length * if incoming { -1. } else { 1. },
        ),
    );
    let (from, to) = if incoming {
        (other, seam)
    } else {
        (seam, other)
    };
    if !verify::segment_inside(query, from, to, clearance)? {
        return Err(error(
            "POCKET_LEAD_NO_FIT",
            "Requested tangent line lead cannot fit inside the pocket",
        ));
    }
    Ok(Some(Lead {
        from,
        to,
        interpolation: Interpolation::LinearFeed,
        feed,
    }))
}

fn arc_inside(
    query: &BoundaryQuery,
    arc: ArcMove,
    from: Point,
    to: Point,
    clearance: f64,
    e: f64,
) -> Result<bool> {
    let radius = arc
        .radius(from)
        .ok_or_else(|| error("POCKET_ARC", "degenerate entry/lead arc"))?;
    let sweep = arc.sweep_rad(from, to).unwrap();
    let angle = (2. * (1. - (e / radius / 4.).min(0.5)).acos()).min(std::f64::consts::FRAC_PI_2);
    let count = (sweep / angle).ceil();
    if !count.is_finite() || count > 16384. {
        return Err(error(
            "POCKET_VERIFICATION_LIMIT",
            "entry/lead arc enclosure budget exceeded",
        ));
    }
    let count = count.max(1.) as usize;
    let sagitta = radius * (1. - (sweep / count as f64 / 2.).cos());
    let mut a = from;
    for i in 1..=count {
        let b = if i == count {
            to
        } else {
            arc.point_at(from, to, i as f64 / count as f64).unwrap()
        };
        if !verify::segment_inside(query, a, b, clearance + sagitta)? {
            return Ok(false);
        }
        a = b;
    }
    Ok(true)
}

/// Polygon visibility routing is bounded separately from motion generation.
/// It is used only for the feed connection from a helix to a loop/lead.
fn corridor(
    region: &Region,
    query: &BoundaryQuery,
    start: Point,
    end: Point,
    clearance: f64,
) -> Result<Vec<Point>> {
    if verify::segment_inside(query, start, end, clearance)? {
        return Ok(vec![end]);
    }
    let mut nodes = vec![start, end];
    nodes.extend(region.rings_mm().into_iter().flatten());
    if nodes.len() > 2048 {
        return Err(error(
            "POCKET_ROUTING_LIMIT",
            "entry corridor exceeds visibility-node budget",
        ));
    }
    let mut predecessor = vec![None; nodes.len()];
    predecessor[0] = Some(0);
    let mut queue = std::collections::VecDeque::from([0]);
    let mut checks = 0;
    while let Some(i) = queue.pop_front() {
        for j in 1..nodes.len() {
            if predecessor[j].is_some() {
                continue;
            }
            checks += 1;
            if checks > 100_000 {
                return Err(error(
                    "POCKET_ROUTING_LIMIT",
                    "entry corridor exceeds visibility-test budget",
                ));
            }
            if verify::segment_inside(query, nodes[i], nodes[j], clearance)? {
                predecessor[j] = Some(i);
                if j == 1 {
                    let mut path = vec![end];
                    let mut at = i;
                    while at != 0 {
                        path.push(nodes[at]);
                        at = predecessor[at].unwrap();
                    }
                    path.reverse();
                    return Ok(path);
                }
                queue.push_back(j);
            }
        }
    }
    Err(error(
        "POCKET_ENTRY_NO_ROUTE",
        "Cannot connect the requested helix to this cutting run",
    ))
}

/// Start at a straight edge midpoint so either lead has a defined tangent.
pub(super) fn seam(points: &mut Vec<Point>) {
    let index = (0..points.len())
        .max_by(|&a, &b| {
            points[a]
                .distance(points[(a + 1) % points.len()])
                .total_cmp(&points[b].distance(points[(b + 1) % points.len()]))
        })
        .unwrap();
    points.rotate_left(index);
    let middle = points[0].lerp(points[1], 0.5);
    points.rotate_left(1);
    points.insert(0, middle);
}

/// Prefer the closest straight-edge interior, retaining a defined tangent.
/// Leads are still checked in full; the original seam is the fallback.
fn nearby_seam(points: &mut Vec<Point>, from: Point) {
    let candidate = (0..points.len())
        .filter_map(|i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            let d = diff(b, a);
            let length2 = d.x * d.x + d.y * d.y;
            if length2 <= 1e-18 {
                return None;
            }
            let v = diff(from, a);
            let t = ((v.x * d.x + v.y * d.y) / length2).clamp(0.01, 0.99);
            let p = a.lerp(b, t);
            Some((i, p, from.distance(p)))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2));
    if let Some((i, p, _)) = candidate {
        let count = points.len();
        points.rotate_left((i + 1) % count);
        points.insert(0, p);
    }
}

pub(super) struct Run<'a> {
    pub region: &'a Region,
    pub centers: &'a Region,
    pub points: &'a [Point],
    pub radius: f64,
    pub clearance_radius: f64,
    pub top: f64,
    pub z: f64,
    pub clearance_z: f64,
    pub feed: f64,
    pub purpose: MotionPurpose,
}

pub(super) fn cut_run(b: &mut Builder, run: Run) -> Result<()> {
    let Run {
        region,
        centers,
        points,
        radius,
        clearance_radius,
        top,
        z,
        clearance_z,
        feed,
        purpose,
    } = run;
    let query = BoundaryQuery::new(region);
    let e = region.grid().tolerance_mm();
    let mut linked_points = None;
    if b.cursor.z == z {
        let mut aim = b.cursor.xy();
        let mut best_distance = f64::INFINITY;
        // The second projection accounts for the incoming lead's displacement,
        // so a tangent lead does not needlessly turn a radial bridge diagonal.
        for _ in 0..2 {
            let mut nearby = points.to_vec();
            nearby_seam(&mut nearby, aim);
            let p = nearby[0];
            let d = diff(nearby[1], p);
            let len = p.distance(nearby[1]);
            let t = Point::new(d.x / len, d.y / len);
            let Ok(incoming) = lead(&query, &b.settings.lead_in, p, t, true, clearance_radius, e)
            else {
                break;
            };
            let at = incoming.as_ref().map_or(p, |lead| lead.from);
            aim = sum(b.cursor.xy(), diff(p, at));
            let distance = b.cursor.xy().distance(at);
            if distance < best_distance
                && verify::short_link(&b.motions, b.cursor, Position::new(at, z), b.settings, e)
                && verify::segment_inside(&query, b.cursor.xy(), at, clearance_radius)?
                && lead(
                    &query,
                    &b.settings.lead_out,
                    p,
                    t,
                    false,
                    clearance_radius,
                    e,
                )
                .is_ok()
            {
                best_distance = distance;
                linked_points = Some(nearby);
            }
        }
    }
    let points = linked_points.as_deref().unwrap_or(points);
    let start = points[0];
    let delta = diff(points[1], start);
    let length = start.distance(points[1]);
    let tangent = Point::new(delta.x / length, delta.y / length);
    let incoming = lead(
        &query,
        &b.settings.lead_in,
        start,
        tangent,
        true,
        clearance_radius,
        e,
    )?;
    let outgoing = lead(
        &query,
        &b.settings.lead_out,
        start,
        tangent,
        false,
        clearance_radius,
        e,
    )?;
    let entry_end = incoming.as_ref().map_or(start, |lead| lead.from);
    let entry_feed = match b.settings.entry {
        PocketEntry::Plunge => b.settings.assignment.plunge_feed_mm_min.unwrap(),
        PocketEntry::Ramp { feed_mm_min, .. } | PocketEntry::Helix { feed_mm_min, .. } => {
            feed_mm_min.unwrap()
        }
    };
    let linked = b.cursor.z == z
        && verify::short_link(
            &b.motions,
            b.cursor,
            Position::new(entry_end, z),
            b.settings,
            e,
        )
        && verify::segment_inside(&query, b.cursor.xy(), entry_end, clearance_radius)?;
    if linked {
        b.feed(
            Position::new(entry_end, z),
            MotionPurpose::Approach,
            feed.min(entry_feed),
        )?;
    } else {
        b.rapid(Position::new(b.cursor.xy(), clearance_z))?;
        match b.settings.entry {
            PocketEntry::Plunge => {
                let surface = verify::endpoint_floor(entry_history(b), entry_end, top).max(z);
                approach(b, entry_end, surface, top, clearance_z, entry_feed)?;
                b.feed(
                    Position::new(entry_end, z),
                    MotionPurpose::Entry,
                    b.settings.assignment.plunge_feed_mm_min.unwrap(),
                )?;
            }
            PocketEntry::Ramp {
                max_angle_deg,
                feed_mm_min,
            } => {
                // An out-and-back ramp returns exactly to the lead's start. Both
                // traverses obey the angle and half-stepdown descent bound.
                let other = points[1];
                let length = entry_end.distance(other);
                if length <= 4. * e
                    || !verify::segment_inside(&query, entry_end, other, clearance_radius)?
                {
                    return Err(error(
                        "POCKET_RAMP_NO_FIT",
                        "No contained linear ramp fits the requested run",
                    ));
                }
                let surface = verify::path_floor(
                    entry_history(b),
                    &[(entry_end, other, Interpolation::LinearFeed)],
                    top,
                )
                .max(z);
                let drop = (length * max_angle_deg.unwrap().to_radians().tan())
                    .min(b.settings.assignment.max_stepdown_mm.unwrap() / 2.);
                let count = if surface > z {
                    ((surface - z) / (2. * drop)).ceil().max(1.)
                } else {
                    0.
                };
                if !count.is_finite() || count > (b.settings.limits.max_motions / 2) as f64 {
                    return Err(error(
                        "PLANNING_RESOURCE_LIMIT",
                        "Pocket ramp exceeds motion budget",
                    ));
                }
                let count = count as usize * 2;
                approach(b, entry_end, surface, top, clearance_z, entry_feed)?;
                for i in 1..=count {
                    b.feed(
                        Position::new(
                            if i % 2 == 0 { entry_end } else { other },
                            if i == count {
                                z
                            } else {
                                surface + (z - surface) * i as f64 / count as f64
                            },
                        ),
                        MotionPurpose::Entry,
                        feed_mm_min.unwrap(),
                    )?;
                }
                // Establish a level corridor so the next layer can start its ramp
                // at this floor, rather than repeating the descent through air.
                if surface > z {
                    for point in [other, entry_end] {
                        b.feed(
                            Position::new(point, z),
                            MotionPurpose::Entry,
                            feed_mm_min.unwrap(),
                        )?;
                    }
                }
            }
            PocketEntry::Helix {
                radius_mm,
                max_angle_deg,
                feed_mm_min,
            } => {
                let hr = radius_mm.unwrap();
                if hr > radius - 4. * e {
                    return Err(error(
                        "POCKET_HELIX_CORE",
                        "Helix center-path radius must be smaller than cutter radius to avoid an uncut central plug",
                    ));
                }
                let available = region.erode(clearance_radius + hr + 8. * e)?;
                let mut candidates: Vec<_> = available.rings_mm().into_iter().flatten().collect();
                candidates.sort_by(|a, b| a.distance(entry_end).total_cmp(&b.distance(entry_end)));
                let mut selected = None;
                for center in candidates.into_iter().take(64) {
                    let end = Point::new(center.x + hr, center.y);
                    if let Ok(path) = corridor(centers, &query, end, entry_end, clearance_radius) {
                        selected = Some((center, end, path));
                        break;
                    }
                }
                let (center, end, path) = selected.ok_or_else(|| {
                    error(
                        "POCKET_HELIX_NO_FIT",
                        "Requested helix cannot fit or reach this run within the pocket",
                    )
                })?;
                let opposite = Point::new(center.x - hr, center.y);
                let interpolation = Interpolation::ArcFeed(ArcMove {
                    center,
                    clockwise: false,
                });
                let surface = verify::path_floor(
                    entry_history(b),
                    &[
                        (end, opposite, interpolation),
                        (opposite, end, interpolation),
                    ],
                    top,
                )
                .max(z);
                let pitch =
                    (std::f64::consts::TAU * hr * max_angle_deg.unwrap().to_radians().tan())
                        .min(b.settings.assignment.max_stepdown_mm.unwrap());
                let turns = if surface > z {
                    ((surface - z) / pitch).ceil().max(1.)
                } else {
                    0.
                };
                if !turns.is_finite() || turns > (b.settings.limits.max_motions / 2) as f64 {
                    return Err(error(
                        "PLANNING_RESOURCE_LIMIT",
                        "Pocket helix exceeds motion budget",
                    ));
                }
                let halves = turns as usize * 2;
                approach(b, end, surface, top, clearance_z, entry_feed)?;
                for i in 1..=halves {
                    b.push(
                        Position::new(
                            if i % 2 == 0 { end } else { opposite },
                            if i == halves {
                                z
                            } else {
                                surface + (z - surface) * i as f64 / halves as f64
                            },
                        ),
                        Interpolation::ArcFeed(ArcMove {
                            center,
                            clockwise: false,
                        }),
                        MotionPurpose::Entry,
                        Some(feed_mm_min.unwrap()),
                    )?;
                }
                // A level revolution establishes the whole entry floor before
                // leaving the helix. The descending circle alone cannot do that.
                if surface > z {
                    for point in [opposite, end] {
                        b.push(
                            Position::new(point, z),
                            Interpolation::ArcFeed(ArcMove {
                                center,
                                clockwise: false,
                            }),
                            MotionPurpose::Entry,
                            Some(feed_mm_min.unwrap()),
                        )?;
                    }
                }
                for point in path {
                    b.feed(
                        Position::new(point, z),
                        MotionPurpose::Entry,
                        feed_mm_min.unwrap(),
                    )?;
                }
            }
        }
    }
    if let Some(lead) = incoming {
        b.push(
            Position::new(lead.to, z),
            lead.interpolation,
            MotionPurpose::LeadIn,
            Some(lead.feed),
        )?;
    }
    for point in points.iter().skip(1).chain(points.first()) {
        b.feed(Position::new(*point, z), purpose, feed)?;
    }
    if let Some(lead) = outgoing {
        b.push(
            Position::new(lead.to, z),
            lead.interpolation,
            MotionPurpose::LeadOut,
            Some(lead.feed),
        )?;
    }
    Ok(())
}

fn approach(
    b: &mut Builder,
    at: Point,
    surface: f64,
    top: f64,
    clearance: f64,
    feed: f64,
) -> Result<()> {
    b.rapid(Position::new(at, clearance))?;
    b.rapid(Position::new(at, top))?;
    // Even an established column is traversed at feed, never a below-surface rapid.
    b.feed(Position::new(at, surface), MotionPurpose::Approach, feed)
}

fn entry_history<'a>(b: &'a Builder<'_>) -> &'a [PlannedMotion] {
    // General entry checks deliberately scope cleared-column evidence to the
    // current stage. Start a finish stage from the top, then reuse its own cuts.
    let start = b.motions.partition_point(|m| m.stage_id != b.stage);
    &b.motions[start..]
}
