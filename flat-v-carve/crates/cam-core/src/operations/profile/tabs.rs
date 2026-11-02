//! Tab geometry for profiles (plan section 10.3): the protected bridge is
//! defined on the source contour and dilated into a centerline interval
//! where the cutter must stay at or above the tab top, so the full-height
//! bridge survives every deep rough and finish pass. Rectangular tabs only;
//! ramped shoulders arrive with their own slice and are diagnosed, never
//! silently substituted. Automatic placement excludes the seam neighborhood
//! occupied by the entry, ramp and leads (plan section 10.3).
use crate::{
    contours::ResolvedAnchor,
    geometry::{Diagnostic, Point, Result},
    project::{TabPlacement, TabSettings, TabShape},
};

use super::entries::edge_dir;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("profile")
}

/// One resolved tab on a compensated loop, in arc-length coordinates along
/// the executed travel direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TabSpan {
    /// Minimum full-height bridge interval.
    pub bridge: (f64, f64),
    /// Dilated centerline interval where `z >= top_z` (cutter radius plus
    /// numerical margin at both ends).
    pub restricted: (f64, f64),
    pub top_z: f64,
}

/// A continuous placement span, ending at the seam or an abrupt corner.
#[derive(Clone, Copy, Debug)]
struct PlacementRun {
    start: f64,
    end: f64,
    curved: bool,
}

/// Flattened curves consist of short edges. Join their gradual turns, while
/// keeping tabs away from corners of 30 degrees or more. The actual cutter
/// clearance is checked against the resulting bridge geometry below.
fn placement_runs(
    vertices: &[Point],
    source: &[Point],
    offset: f64,
    margin: f64,
) -> Vec<PlacementRun> {
    let n = vertices.len();
    if n < 2 {
        return vec![];
    }
    let corners: Vec<_> = (0..source.len())
        .filter_map(|i| {
            let incoming = edge_dir(source[(i + source.len() - 1) % source.len()], source[i])?;
            let outgoing = edge_dir(source[i], source[(i + 1) % source.len()])?;
            (incoming.x * outgoing.x + incoming.y * outgoing.y <= (std::f64::consts::PI / 6.).cos())
                .then_some(source[i])
        })
        .collect();
    let mut cumulative = vec![0f64];
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        cumulative.push(cumulative[i] + a.distance(b));
    }
    let mut runs = vec![];
    let mut run_first_edge = 0usize;
    let mut curved = false;
    for edge in 0..n {
        let next = (edge + 1) % n;
        let a = vertices[edge];
        let b = vertices[next];
        let len = a.distance(b);
        let dir = Point::new((b.x - a.x) / len, (b.y - a.y) / len);
        let continues = edge + 1 < n && {
            let c = vertices[(edge + 2) % n];
            let len_next = b.distance(c);
            let dir_next = Point::new((c.x - b.x) / len_next, (c.y - b.y) / len_next);
            let dot = dir.x * dir_next.x + dir.y * dir_next.y;
            let turning = (dir.x * dir_next.y - dir.y * dir_next.x).abs() >= 1e-9;
            // Offset fillets around sharp SOURCE corners are not smooth
            // source curves. Do not join them into an eligible tab span.
            let at_corner = corners
                .iter()
                .any(|corner| corner.distance(b) <= offset + 4. * margin);
            // Integer offsetting can insert grid-sized connector edges with
            // misleading tangents. They do not turn a smooth source into a
            // corner; the spatial footprint check still sees every edge.
            let continues =
                (dot > (std::f64::consts::PI / 6.).cos() || len <= margin || len_next <= margin)
                    && !at_corner;
            curved |= continues && turning;
            continues
        };
        if !continues {
            runs.push(PlacementRun {
                start: cumulative[run_first_edge],
                end: cumulative[edge + 1],
                curved,
            });
            run_first_edge = edge + 1;
            curved = false;
        }
    }
    runs
}

/// Resolve this loop's tab spans from the settings. `perimeter` is the loop
/// length; `anchor_offset` is the compensation distance of this centerline
/// from the source contour (cutter radius for a final pass, radius plus the
/// allowance when roughing) used to match manual anchors; `cutter_radius` is
/// the cutter disc radius whose dilation protects the bridge. `exclude` are
/// arc intervals around the seam occupied by entry/lead motion: automatic
/// placements never bridge there (manual anchors keep precedence and are
/// checked against the leads by the caller).
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_spans(
    settings: &TabSettings,
    vertices: &[Point],
    source: &[Point],
    perimeter: f64,
    anchor_offset: f64,
    cutter_radius: f64,
    margin: f64,
    tab_top_z: f64,
    anchors: &[ResolvedAnchor],
    contour_id: &str,
    exclude: &[(f64, f64)],
) -> Result<Vec<TabSpan>> {
    if settings.shape != TabShape::Rectangular {
        return Err(error(
            "PROFILE_TAB_SHAPE_UNSUPPORTED",
            "ramped tab shoulders ship with their own slice; configure rectangular tabs",
        ));
    }
    let width = settings
        .width_mm
        .ok_or_else(|| error("MISSING_MACHINING_SETTING", "tab width is required"))?;
    let runs = placement_runs(vertices, source, anchor_offset, margin);
    // Dilate the bridge by the cutter radius (plus margin) at both ends: the
    // cutter disc must clear the protected volume, not just its center.
    let dilation = cutter_radius + margin;

    // Eligible bridge-start windows: bridges fit inside continuous spans with
    // their restricted interval clear of run ends and excluded arcs.
    let mut bridges: Vec<(f64, f64)> = vec![];
    match &settings.placement {
        TabPlacement::Automatic { count, spacing_mm } => {
            let mut windows: Vec<(f64, f64)> = vec![];
            for run in &runs {
                // Curves need extra room: arc distance can exceed the chord
                // clearance. This reserve is only a placement heuristic;
                // the footprint sweep below establishes protection.
                let reserve = if run.curved { 2. * dilation } else { dilation };
                let mut candidates = vec![(run.start + reserve, run.end - reserve - width)];
                for &(excluded_start, excluded_end) in exclude {
                    // A bridge start `a` is invalid when its restricted
                    // interval [a - dilation, a + width + dilation] reaches
                    // into the excluded arc.
                    let mut next = vec![];
                    for (lo, hi) in candidates {
                        let cut_lo = excluded_start - width - reserve;
                        let cut_hi = excluded_end + reserve;
                        if cut_lo > lo {
                            let end = hi.min(cut_lo);
                            if end >= lo - 1e-9 {
                                next.push((lo, end));
                            }
                        }
                        if cut_hi < hi {
                            let start = lo.max(cut_hi);
                            if hi >= start - 1e-9 {
                                next.push((start, hi));
                            }
                        }
                    }
                    candidates = next;
                }
                windows.extend(candidates.into_iter().filter(|&(lo, hi)| hi >= lo - 1e-9));
            }
            match count {
                Some(count) => {
                    // Even distribution round-robin over eligible windows; a
                    // window hosts at most `capacity` tabs with at least one
                    // bridge-width gap between them.
                    let gap = spacing_mm.unwrap_or(width);
                    let capacity =
                        |lo: f64, hi: f64| 1 + ((hi - lo) / (width + gap)).floor() as usize;
                    let total: usize = windows.iter().map(|&(lo, hi)| capacity(lo, hi)).sum();
                    if total < *count as usize {
                        return Err(error(
                            "PROFILE_TAB_NO_SPACE",
                            format!(
                                "requested {count} tabs on contour '{contour_id}' but only {total} fit its available spans \
                                 (excluding the entry/lead neighborhood); reduce the count or width, or use manual placement",
                            ),
                        ));
                    }
                    let mut assigned = vec![0usize; windows.len()];
                    let mut placed = 0usize;
                    'outer: loop {
                        for (slot, &(lo, hi)) in windows.iter().enumerate() {
                            if placed >= *count as usize {
                                break 'outer;
                            }
                            if assigned[slot] < capacity(lo, hi) {
                                assigned[slot] += 1;
                                placed += 1;
                            }
                        }
                    }
                    for (slot, &(lo, hi)) in windows.iter().enumerate() {
                        let k = assigned[slot];
                        for j in 0..k {
                            let start = if runs.len() == 1 && runs[0].curved && exclude.is_empty() {
                                // Equal angular/arc sectors on a smooth closed
                                // loop, including the gap across its seam.
                                ((j as f64 + 0.5) * perimeter / k as f64 - width / 2.).clamp(lo, hi)
                            } else if k == 1 {
                                // Center a lone tab on its span.
                                (lo + hi) / 2.
                            } else {
                                lo + j as f64 * (hi - lo) / (k - 1) as f64
                            };
                            bridges.push((start, start + width));
                        }
                    }
                }
                None => {
                    let spacing = spacing_mm.ok_or_else(|| {
                        error(
                            "MISSING_MACHINING_SETTING",
                            "automatic tab placement needs a count or a spacing",
                        )
                    })?;
                    for &(lo, hi) in &windows {
                        let mut start = lo;
                        while start <= hi + 1e-9 {
                            bridges.push((start, start + width));
                            start += spacing;
                        }
                    }
                    if bridges.is_empty() {
                        return Err(error(
                            "PROFILE_TAB_NO_SPACE",
                            format!(
                                "no tab of width {width} mm fits the available spans of contour '{contour_id}' at spacing {spacing} mm"
                            ),
                        ));
                    }
                }
            }
        }
        TabPlacement::Manual { anchors: _ } => {
            // Map source anchors to the nearest point of the compensated
            // polyline. Its flattened edges need not be exactly parallel to
            // the source tangent, especially at a curved offset's vertices.
            for anchor in anchors {
                let (bridge_start, distance) = project_arc(vertices, anchor.point);
                let perp_err = (distance - anchor_offset).abs();
                if perp_err > margin.max(1e-6) * 4. {
                    return Err(error(
                        "PROFILE_TAB_NO_SPACE",
                        format!(
                            "tab anchor on contour '{contour_id}' lies {} mm away from the compensated path; reattach it",
                            perp_err
                        ),
                    ));
                }
                let bridge_end = bridge_start + width;
                if !runs.iter().any(|run| {
                    bridge_start - dilation >= run.start - 1e-9
                        && bridge_end + dilation <= run.end + 1e-9
                }) {
                    return Err(error(
                        "PROFILE_TAB_NO_SPACE",
                        format!(
                            "the tab anchored on contour '{contour_id}' needs {:.2} mm clear of corners and the entry but runs out of room; \
                             move the anchor or shorten the tab",
                            width + 2. * dilation,
                        ),
                    ));
                }
                bridges.push((bridge_start, bridge_end));
            }
        }
    }

    // Keep each requested bridge's own footprint. Depth pieces combine
    // overlapping protection without inventing an unverified bridge across
    // the gap between two curved footprints.
    bridges.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("finite arcs"));
    let mut spans: Vec<TabSpan> = vec![];
    for (start, end) in bridges {
        let footprint = span_footprint(vertices, perimeter, (start, end), dilation, margin)
            .ok_or_else(|| error("PROFILE_TAB_NO_SPACE", "tab bridge has no usable path"))?;
        let restricted = protected_interval(vertices, &footprint, dilation)
            .expect("the bridge intersects its own path");
        if (restricted.0 <= 1e-9 || restricted.1 >= perimeter - 1e-9)
            && protected_interval(&[vertices[0]], &footprint, cutter_radius + margin / 2.).is_some()
        {
            return Err(error(
                "PROFILE_TAB_NO_SPACE",
                format!(
                    "tab on contour '{contour_id}' reaches the entry; move the tab or choose another start"
                ),
            ));
        }
        spans.push(TabSpan {
            bridge: (start, end),
            restricted,
            top_z: tab_top_z,
        });
    }
    let mut intervals: Vec<_> = spans.iter().map(|s| s.restricted).collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut end: f64 = 0.;
    let mut restricted_total = 0.;
    for (a, b) in intervals {
        restricted_total += (b - a.max(end)).max(0.);
        end = end.max(b);
    }
    if restricted_total >= perimeter * 0.999 {
        return Err(error(
            "PROFILE_TAB_NO_SPACE",
            format!(
                "tab placements on contour '{contour_id}' consume the whole contour; \
                 reduce the count or width",
            ),
        ));
    }
    Ok(spans)
}

/// One commanded-depth interval of a traversal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthPiece {
    pub start: f64,
    pub end: f64,
    pub z: f64,
    pub on_tab: bool,
}

/// Commanded depth profile `z(s) = max(pass_z, tab_top)` split exactly at
/// envelope boundaries (plan section 10.3). Passes at or above every tab top
/// return one unsplit piece.
pub(crate) fn depth_pieces(spans: &[TabSpan], pass_z: f64, perimeter: f64) -> Vec<DepthPiece> {
    let deep_spans: Vec<&TabSpan> = spans
        .iter()
        .filter(|span| pass_z < span.top_z - 1e-9)
        .collect();
    if deep_spans.is_empty() {
        return vec![DepthPiece {
            start: 0.,
            end: perimeter,
            z: pass_z,
            on_tab: false,
        }];
    }
    let mut pieces = vec![DepthPiece {
        start: 0.,
        end: perimeter,
        z: pass_z,
        on_tab: false,
    }];
    for span in deep_spans {
        let (a, b) = span.restricted;
        let mut next = vec![];
        for piece in pieces {
            if piece.end <= a || piece.start >= b || piece.on_tab {
                next.push(piece);
                continue;
            }
            if piece.start < a {
                next.push(DepthPiece {
                    start: piece.start,
                    end: a,
                    z: pass_z,
                    on_tab: false,
                });
            }
            next.push(DepthPiece {
                start: a.max(piece.start),
                end: b.min(piece.end),
                z: span.top_z,
                on_tab: true,
            });
            if piece.end > b {
                next.push(DepthPiece {
                    start: b,
                    end: piece.end,
                    z: pass_z,
                    on_tab: false,
                });
            }
        }
        pieces = next;
    }
    pieces.retain(|p| p.end > p.start + 1e-12);
    pieces
}

/// Point on the closed travel-order loop at arc length `arc` (wrapped into
/// `[0, perimeter)`).
pub(crate) fn point_at_arc(vertices: &[Point], perimeter: f64, arc: f64) -> Point {
    let arc = arc.rem_euclid(perimeter.max(1e-12));
    let n = vertices.len();
    let mut walked = 0f64;
    for index in 0..n {
        let a = vertices[index];
        let b = vertices[(index + 1) % n];
        let len = a.distance(b);
        if arc <= walked + len {
            let t = if len > 1e-12 {
                (arc - walked) / len
            } else {
                0.
            };
            return Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        }
        walked += len;
    }
    vertices[0]
}

/// Unit travel direction at an arc position: the edge containing it, or the
/// outgoing edge when the position lands exactly on a vertex.
pub(crate) fn tangent_at_arc(vertices: &[Point], arc: f64) -> Option<Point> {
    let n = vertices.len();
    let mut walked = 0f64;
    for index in 0..n {
        let a = vertices[index];
        let b = vertices[(index + 1) % n];
        let len = a.distance(b);
        if arc <= walked + len {
            return edge_dir(a, b).or_else(|| edge_dir(b, vertices[(index + 2) % n]));
        }
        walked += len;
    }
    edge_dir(vertices[0], vertices[1 % n])
}

/// Project a source point to arc length on the compensated travel loop.
pub(crate) fn project_arc(vertices: &[Point], point: Point) -> (f64, f64) {
    let mut walked = 0.;
    let mut best = (0., f64::INFINITY);
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        let len = a.distance(b);
        if len <= 1e-12 {
            continue;
        }
        let t = (((point.x - a.x) * (b.x - a.x) + (point.y - a.y) * (b.y - a.y)) / (len * len))
            .clamp(0., 1.);
        let distance = point.distance(a.lerp(b, t));
        if distance < best.1 {
            best = (walked + t * len, distance);
        }
        walked += len;
    }
    best
}

/// Choose a tab-free seam between manual anchors when the user has not
/// pinned the entry. In particular, an anchor at source fraction zero must
/// not also force the tool to plunge through that tab.
pub(crate) fn manual_seam(vertices: &[Point], anchors: &[ResolvedAnchor], width: f64) -> Point {
    let perimeter: f64 = (0..vertices.len())
        .map(|i| vertices[i].distance(vertices[(i + 1) % vertices.len()]))
        .sum();
    let mut arcs: Vec<_> = anchors
        .iter()
        .map(|a| project_arc(vertices, a.point).0)
        .collect();
    arcs.sort_by(f64::total_cmp);
    let (start, gap) = arcs
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = if i + 1 < arcs.len() {
                arcs[i + 1]
            } else {
                arcs[0] + perimeter
            };
            (start + width, end - start - width)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("nonempty anchors");
    point_at_arc(vertices, perimeter, start + gap / 2.)
}

/// Setup-space strip around the bridge, following all flattened curve
/// vertices. The first half is its left boundary in travel order; the second
/// half is the right boundary in reverse order. Straight bridges remain quads.
pub(crate) fn span_footprint(
    vertices: &[Point],
    perimeter: f64,
    bridge: (f64, f64),
    half_width: f64,
    tolerance: f64,
) -> Option<Vec<(f64, f64)>> {
    let mut points = vec![point_at_arc(vertices, perimeter, bridge.0)];
    let mut arc = 0.;
    for i in 0..vertices.len() {
        if arc > bridge.0 + 1e-9 && arc < bridge.1 - 1e-9 {
            points.push(vertices[i]);
        }
        arc += vertices[i].distance(vertices[(i + 1) % vertices.len()]);
    }
    let end = point_at_arc(vertices, perimeter, bridge.1);
    points.push(end);
    // Remove offset-grid jitter before constructing normals. RDP bounds each
    // discarded point by the motion tolerance; the cutter path itself is
    // unchanged and is checked against this explicit footprint below.
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut pending = vec![(0, points.len() - 1)];
    while let Some((a, b)) = pending.pop() {
        if let Some((index, distance)) = (a + 1..b)
            .map(|i| {
                (
                    i,
                    super::entries::point_segment_distance(points[i], points[a], points[b]),
                )
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            && distance > tolerance
        {
            keep[index] = true;
            pending.extend([(a, index), (index, b)]);
        }
    }
    let points: Vec<_> = points
        .into_iter()
        .zip(keep)
        .filter_map(|(point, keep)| keep.then_some(point))
        .collect();
    let mut left = vec![];
    let mut right = vec![];
    for (i, &point) in points.iter().enumerate() {
        let before = edge_dir(points[i.saturating_sub(1)], point);
        let after = edge_dir(point, points[(i + 1).min(points.len() - 1)]);
        let (dx, dy) = match (before, after) {
            (Some(a), Some(b)) => {
                // Intersection of the two parallel offsets (miter join).
                let denominator = 1. + a.x * b.x + a.y * b.y;
                if denominator <= 1e-9 {
                    return None;
                }
                (-(a.y + b.y) / denominator, (a.x + b.x) / denominator)
            }
            (Some(a), None) | (None, Some(a)) => (-a.y, a.x),
            _ => return None,
        };
        left.push((point.x + dx * half_width, point.y + dy * half_width));
        right.push((point.x - dx * half_width, point.y - dy * half_width));
    }
    // A strip wider than a tight bend can fold over itself. Reject a folded
    // cell rather than applying convex clipping to invalid geometry.
    for i in 0..left.len() - 1 {
        let cell = [left[i], left[i + 1], right[i + 1], right[i]];
        let mut sign: f64 = 0.;
        for j in 0..4 {
            let (a, b, c) = (cell[j], cell[(j + 1) % 4], cell[(j + 2) % 4]);
            let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
            if cross.abs() < 1e-12 || sign * cross < 0. {
                return None;
            }
            sign = cross.signum();
        }
    }
    left.extend(right.into_iter().rev());
    Some(left)
}

/// Arc interval where the cutter can touch a protected strip. Each strip
/// cell is convex. Clip every loop edge against its outward-offset half
/// planes: this contains the rounded cutter dilation, so even nearby bends
/// cannot cut into the bridge. Using XY clearance instead of adding a radius
/// to arc length is essential on curves, where chords are shorter than arcs.
pub(crate) fn protected_interval(
    vertices: &[Point],
    footprint: &[(f64, f64)],
    radius: f64,
) -> Option<(f64, f64)> {
    let half = footprint.len() / 2;
    let mut interval: Option<(f64, f64)> = None;
    let mut walked = 0.;
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        let len = a.distance(b);
        for j in 0..half.saturating_sub(1) {
            let cell = [
                footprint[j],
                footprint[j + 1],
                footprint[footprint.len() - 2 - j],
                footprint[footprint.len() - 1 - j],
            ];
            let area: f64 = (0..4)
                .map(|k| {
                    let c = cell[k];
                    let d = cell[(k + 1) % 4];
                    c.0 * d.1 - c.1 * d.0
                })
                .sum();
            let sign = area.signum();
            let mut lo: f64 = 0.;
            let mut hi: f64 = 1.;
            for k in 0..4 {
                let c = cell[k];
                let d = cell[(k + 1) % 4];
                let dx = d.0 - c.0;
                let dy = d.1 - c.1;
                let value = sign * (dx * (a.y - c.1) - dy * (a.x - c.0)) + radius * dx.hypot(dy);
                let slope = sign * (dx * (b.y - a.y) - dy * (b.x - a.x));
                if slope.abs() < 1e-12 {
                    if value < 0. {
                        hi = -1.;
                        break;
                    }
                } else if slope > 0. {
                    lo = lo.max(-value / slope);
                } else {
                    hi = hi.min(-value / slope);
                }
            }
            if lo <= hi {
                let start = walked + lo * len;
                let end = walked + hi * len;
                interval = Some(match interval {
                    Some((first, last)) => (first.min(start), last.max(end)),
                    None => (start, end),
                });
            }
        }
        walked += len;
    }
    interval
}
