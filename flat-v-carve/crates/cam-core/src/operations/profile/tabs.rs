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

/// A maximal straight span of the loop (collinear edges merged).
#[derive(Clone, Copy, Debug)]
struct StraightRun {
    start: f64,
    end: f64,
    origin: Point,
    dir: Point,
}

impl StraightRun {
    fn len(&self) -> f64 {
        self.end - self.start
    }
}

/// Extract maximal straight runs of a closed travel-order loop. Runs never
/// wrap the seam: the loop start is a vertex, so the closing edge ends a run.
fn straight_runs(vertices: &[Point]) -> Vec<StraightRun> {
    let n = vertices.len();
    if n < 2 {
        return vec![];
    }
    let mut cumulative = vec![0f64];
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        cumulative.push(cumulative[i] + a.distance(b));
    }
    let mut runs: Vec<StraightRun> = vec![];
    let mut run_first_edge = 0usize;
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
            // Same direction within float noise: collinear, not a reversal.
            (dir.x * dir_next.y - dir.y * dir_next.x).abs() < 1e-9
                && dir.x * dir_next.x + dir.y * dir_next.y > 0.
        };
        if !continues {
            let first = vertices[run_first_edge];
            let run_dir = {
                let second = vertices[(run_first_edge + 1) % n];
                let l = first.distance(second);
                Point::new((second.x - first.x) / l, (second.y - first.y) / l)
            };
            runs.push(StraightRun {
                start: cumulative[run_first_edge],
                end: cumulative[edge + 1],
                origin: first,
                dir: run_dir,
            });
            run_first_edge = edge + 1;
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
    let runs = straight_runs(vertices);
    // Dilate the bridge by the cutter radius (plus margin) at both ends: the
    // cutter disc must clear the protected volume, not just its center.
    let dilation = cutter_radius + margin;

    // Eligible bridge-start windows: bridges fit inside straight runs with
    // their restricted interval clear of run ends and excluded arcs.
    let mut bridges: Vec<(f64, f64)> = vec![];
    match &settings.placement {
        TabPlacement::Automatic { count, spacing_mm } => {
            let mut windows: Vec<(f64, f64)> = vec![];
            for run in &runs {
                let mut candidates = vec![(run.start + dilation, run.end - dilation - width)];
                for &(excluded_start, excluded_end) in exclude {
                    // A bridge start `a` is invalid when its restricted
                    // interval [a - dilation, a + width + dilation] reaches
                    // into the excluded arc.
                    let mut next = vec![];
                    for (lo, hi) in candidates {
                        let cut_lo = excluded_start - width - dilation;
                        let cut_hi = excluded_end + dilation;
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
                                "requested {count} tabs on contour '{contour_id}' but only {total} fit its straight spans \
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
                            let start = if k == 1 {
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
                                "no tab of width {width} mm fits the straight spans of contour '{contour_id}' at spacing {spacing} mm"
                            ),
                        ));
                    }
                }
            }
        }
        TabPlacement::Manual { anchors: _ } => {
            // Manual anchors were resolved by the caller against the source
            // contour; map each onto this loop's parallel straight run. The
            // projection is local to the run; placements are stored in loop
            // arc coordinates.
            for anchor in anchors {
                let mut best: Option<(usize, f64, f64)> = None; // (run, along, perp error)
                for (index, run) in runs.iter().enumerate() {
                    let vx = anchor.point.x - run.origin.x;
                    let vy = anchor.point.y - run.origin.y;
                    let along = vx * run.dir.x + vy * run.dir.y;
                    let perp = vx * (-run.dir.y) + vy * run.dir.x;
                    // The anchor must sit roughly one compensation distance
                    // from the parallel centerline run and address its interior.
                    let parallel =
                        (anchor.tangent.x * run.dir.y - anchor.tangent.y * run.dir.x).abs() < 1e-6;
                    if !parallel || !(0. ..=run.len()).contains(&along) {
                        continue;
                    }
                    let err = (perp.abs() - anchor_offset).abs();
                    if best.is_none_or(|(_, _, best_err)| err < best_err) {
                        best = Some((index, along, err));
                    }
                }
                let Some((run_index, along, perp_err)) = best else {
                    return Err(error(
                        "PROFILE_TAB_NO_SPACE",
                        format!(
                            "tab anchor on contour '{contour_id}' does not address a straight span of its compensated path; \
                             move it onto a sufficiently long straight edge",
                        ),
                    ));
                };
                if perp_err > margin.max(1e-6) * 4. {
                    return Err(error(
                        "PROFILE_TAB_NO_SPACE",
                        format!(
                            "tab anchor on contour '{contour_id}' lies {} mm away from the compensated path; reattach it",
                            perp_err
                        ),
                    ));
                }
                let run = runs[run_index];
                let bridge_start = run.start + along;
                let bridge_end = bridge_start + width;
                if bridge_start - dilation < run.start - 1e-9
                    || bridge_end + dilation > run.end + 1e-9
                {
                    return Err(error(
                        "PROFILE_TAB_NO_SPACE",
                        format!(
                            "the tab anchored on contour '{contour_id}' needs {:.2} mm of straight span but runs out of room; \
                             move the anchor or shorten the tab",
                            width + 2. * dilation,
                        ),
                    ));
                }
                bridges.push((bridge_start, bridge_end));
            }
        }
    }

    // Overlapping footprints merge conservatively; placements that would
    // consume the whole contour are rejected outright.
    bridges.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("finite arcs"));
    let mut spans: Vec<TabSpan> = vec![];
    for (start, end) in bridges {
        let restricted = (start - dilation, end + dilation);
        match spans.last_mut() {
            Some(last) if restricted.0 <= last.restricted.1 => {
                last.bridge.1 = last.bridge.1.max(end);
                last.restricted.1 = last.restricted.1.max(restricted.1);
            }
            _ => spans.push(TabSpan {
                bridge: (start, end),
                restricted,
                top_z: tab_top_z,
            }),
        }
    }
    let restricted_total: f64 = spans.iter().map(|s| s.restricted.1 - s.restricted.0).sum();
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

/// Setup-space quad of a resolved bridge's protected cross-section: the
/// cutter corridor over the bridge interval, i.e. the material that must
/// survive every deep pass. Bridges live on straight spans in this release,
/// so the quad is an exact rectangle; previews draw exactly this geometry
/// instead of relying on display-grid resolution (plan section 15.3).
pub(crate) fn span_footprint(
    vertices: &[Point],
    perimeter: f64,
    bridge: (f64, f64),
    half_width: f64,
) -> Option<Vec<(f64, f64)>> {
    let a = point_at_arc(vertices, perimeter, bridge.0);
    let b = point_at_arc(vertices, perimeter, bridge.1);
    let dir = edge_dir(a, b)?;
    let normal = Point::new(-dir.y, dir.x);
    Some(vec![
        (a.x + normal.x * half_width, a.y + normal.y * half_width),
        (b.x + normal.x * half_width, b.y + normal.y * half_width),
        (b.x - normal.x * half_width, b.y - normal.y * half_width),
        (a.x - normal.x * half_width, a.y - normal.y * half_width),
    ])
}
