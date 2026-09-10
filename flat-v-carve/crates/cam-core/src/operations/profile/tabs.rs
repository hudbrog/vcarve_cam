//! Tab geometry for profiles (plan section 10.3): the protected bridge is
//! defined on the source contour and dilated into a centerline interval
//! where the cutter must stay at or above the tab top, so the full-height
//! bridge survives every deep rough and finish pass. Rectangular tabs only;
//! ramped shoulders arrive with their own slice and are diagnosed, never
//! silently substituted.
use crate::{
    contours::ResolvedAnchor,
    geometry::{Diagnostic, Point, Result},
    project::{TabPlacement, TabSettings, TabShape},
};

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
/// length; `expected_offset` is the compensation distance of the centerline
/// from the source contour (cutter radius for the rough pass).
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_spans(
    settings: &TabSettings,
    vertices: &[Point],
    perimeter: f64,
    expected_offset: f64,
    margin: f64,
    tab_top_z: f64,
    anchors: &[ResolvedAnchor],
    contour_id: &str,
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
    let dilation = expected_offset + margin;

    let mut bridges: Vec<(f64, f64)> = vec![];
    match &settings.placement {
        TabPlacement::Automatic { count, spacing_mm } => {
            let eligible: Vec<(usize, f64, f64)> = runs
                .iter()
                .enumerate()
                .filter_map(|(index, run)| {
                    let lo = run.start + dilation;
                    let hi = run.end - dilation - width;
                    (hi >= lo).then_some((index, lo, hi)) // bridge [a, a+width] fits
                })
                .collect();
            match count {
                Some(count) => {
                    // Even distribution round-robin over eligible runs; a
                    // run hosts at most `capacity` tabs with at least one
                    // bridge-width gap between them.
                    let gap = spacing_mm.unwrap_or(width);
                    let capacity =
                        |lo: f64, hi: f64| 1 + ((hi - lo) / (width + gap)).floor() as usize;
                    let total: usize = eligible.iter().map(|(_, lo, hi)| capacity(*lo, *hi)).sum();
                    if total < *count as usize {
                        return Err(error(
                            "PROFILE_TAB_NO_SPACE",
                            format!(
                                "requested {count} tabs on contour '{contour_id}' but only {total} fit its straight spans; \
                                 reduce the count, width, or use manual placement",
                            ),
                        ));
                    }
                    let mut assigned = vec![0usize; eligible.len()];
                    let mut placed = 0usize;
                    'outer: loop {
                        for (slot, (_, lo, hi)) in eligible.iter().enumerate() {
                            if placed >= *count as usize {
                                break 'outer;
                            }
                            if assigned[slot] < capacity(*lo, *hi) {
                                assigned[slot] += 1;
                                placed += 1;
                            }
                        }
                    }
                    for (slot, (_, lo, hi)) in eligible.iter().enumerate() {
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
                    for (_, lo, hi) in &eligible {
                        let mut start = *lo;
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
                    let err = (perp.abs() - expected_offset).abs();
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
