use super::*;
use crate::{
    stock::{removed_depth_at, vbit_removed_depth_at},
    target::ReachabilityOptions,
};

#[cfg(test)]
mod tests;

struct Sweep<'a> {
    endmill: bool,
    motion: &'a Motion,
    bounds: Bounds,
}
struct Cell {
    bounds: Bounds,
    depth: usize,
    sweeps: Vec<usize>,
}
struct Leaf {
    area: f64,
    target: Interval,
    actual: Interval,
}
struct Evaluation {
    target: Interval,
    actual: Interval,
    errors: [Interval; 5],
    reachability_cells: usize,
}
fn overlap(a: Bounds, b: Bounds) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}
fn include(a: &mut Bounds, b: Bounds) {
    a.min.x = a.min.x.min(b.min.x);
    a.min.y = a.min.y.min(b.min.y);
    a.max.x = a.max.x.max(b.max.x);
    a.max.y = a.max.y.max(b.max.y);
}
fn area(b: Bounds) -> f64 {
    (b.max.x - b.min.x) * (b.max.y - b.min.y)
}
fn children(b: Bounds) -> Vec<Bounds> {
    let p = b.min.lerp(b.max, 0.5);
    let w = b.max.x - b.min.x;
    let h = b.max.y - b.min.y;
    let pairs = if w > 1.5 * h {
        vec![
            (Point::new(p.x, b.min.y), b.max),
            (b.min, Point::new(p.x, b.max.y)),
        ]
    } else if h > 1.5 * w {
        vec![
            (Point::new(b.min.x, p.y), b.max),
            (b.min, Point::new(b.max.x, p.y)),
        ]
    } else {
        vec![
            (p, b.max),
            (Point::new(b.min.x, p.y), Point::new(p.x, b.max.y)),
            (Point::new(p.x, b.min.y), Point::new(b.max.x, p.y)),
            (b.min, p),
        ]
    };
    pairs
        .into_iter()
        .map(|(min, max)| Bounds { min, max })
        .collect()
}

fn evaluate(
    ctx: &Context<'_>,
    cell: &Cell,
    sweeps: &[Sweep<'_>],
    options: &VerificationOptions,
    all_clear: bool,
) -> Result<Evaluation> {
    let p = cell.bounds.min.lerp(cell.bounds.max, 0.5);
    let radius = p.distance(cell.bounds.max) + ctx.reserve;
    let slope = ctx.target.angle().slope();
    let cap = ctx.target.depth_cap().mm();
    let variation = radius / slope;
    let distance = ctx.target.boundary().sample(p)?;
    let target_at = (distance.signed_distance_mm / slope).clamp(0., cap);
    let (distance_lower, distance_upper) = ctx.target.boundary().box_bounds_with_center_sample(
        cell.bounds.min,
        cell.bounds.max,
        distance,
    )?;
    let target = Interval::positive(
        (distance_lower / slope).clamp(0., cap),
        (distance_upper / slope).clamp(0., cap),
    );
    let mut actual_at: f64 = 0.;
    let mut actual = Interval::default();
    let mut best_vbit = None;
    let mut best_vbit_depth = 0.;
    for &i in &cell.sweeps {
        let sweep = &sweeps[i];
        let moves = std::slice::from_ref(sweep.motion);
        let (at, lower, upper) = if sweep.endmill {
            let r = ctx.mill.radius().mm();
            let at = removed_depth_at(moves, r, p)?;
            let lower = if r > radius {
                removed_depth_at(moves, r - radius, p)?
            } else {
                0.
            };
            (at, lower, removed_depth_at(moves, r + radius, p)?)
        } else {
            let at = vbit_removed_depth_at(moves, &ctx.vbit, p)?;
            if at > best_vbit_depth {
                best_vbit_depth = at;
                best_vbit = Some(i);
            }
            let deepest = sweep.motion.start.depth().max(sweep.motion.end.depth());
            (at, (at - variation).max(0.), (at + variation).min(deepest))
        };
        actual_at = actual_at.max(at);
        actual.lower = actual.lower.max((lower - ctx.reserve).max(0.));
        actual.upper = actual.upper.max(upper + ctx.reserve);
    }
    let mut residual_upper = (target.upper - actual.lower).max(0.);
    // A single linear V-bit sweep has a concave *unclamped* depth surface.
    // Where all four corners are cut, its minimum is at a corner. Subtracting
    // it from distance to any boundary segment (a convex target upper roof)
    // is convex, so the maximum is also at a corner. The roof remains valid
    // beyond segment endpoints and across changes of the nearest feature.
    if let Some(i) = best_vbit {
        let corners = [
            cell.bounds.min,
            Point::new(cell.bounds.max.x, cell.bounds.min.y),
            cell.bounds.max,
            Point::new(cell.bounds.min.x, cell.bounds.max.y),
        ];
        let depths = corners
            .map(|p| vbit_removed_depth_at(std::slice::from_ref(sweeps[i].motion), &ctx.vbit, p))
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        if depths.iter().all(|d| *d > ctx.reserve) {
            actual.lower = actual
                .lower
                .max(depths.iter().copied().fold(f64::INFINITY, f64::min) - ctx.reserve);
            residual_upper = residual_upper.min((target.upper - actual.lower).max(0.));
            // An improving segment-distance roof must be within slope *
            // (corner depth + current residual bound) of every corner. Edges
            // outside this expanded box cannot improve the existing bound.
            // Round the search outward before BVH pruning.
            let padding = (depths.iter().copied().fold(0., f64::max) + residual_upper) * slope
                + 16. * ctx.reserve * (1. + slope);
            ctx.target.boundary().visit_segments(
                Point::new(cell.bounds.min.x - padding, cell.bounds.min.y - padding),
                Point::new(cell.bounds.max.x + padding, cell.bounds.max.y + padding),
                |edge| {
                    let upper = corners
                        .iter()
                        .zip(&depths)
                        .map(|(&p, &d)| edge.distance(p) / slope - d + 4. * ctx.reserve)
                        .fold(0., f64::max);
                    residual_upper = residual_upper.min(upper);
                },
            )?;
        }
    }
    let tip = ctx.vbit.tip_radius().mm();
    let mut reach_cells = 0;
    let (h, h_at) = if tip == 0. {
        // The ideal pointed cutter can attain T everywhere; this correlation
        // proves zero unreachable residue without subtracting two loose bounds.
        (
            target,
            Interval::positive(target_at - ctx.reserve, target_at + ctx.reserve),
        )
    } else if target.upper == 0. {
        (Interval::default(), Interval::default())
    } else if distance_lower >= tip + cap * slope {
        (Interval::positive(cap, cap), Interval::positive(cap, cap))
    } else if all_clear && residual_upper <= ctx.tolerance.min(ctx.detail) {
        // Recorded admissible removal is itself a reachability lower witness.
        (
            Interval::positive(actual.lower, target.upper),
            Interval::positive(actual_at - ctx.reserve, target_at + ctx.reserve),
        )
    } else {
        let query_options = ReachabilityOptions {
            depth_tolerance_mm: ctx.tolerance / 8.,
            max_cells: options.reachability_max_cells,
        };
        let r = ctx.target.vbit_reachability(&ctx.vbit, p, query_options)?;
        reach_cells += r.evaluated_cells;
        let mut at = Interval::positive(r.reachable_depth_lower_mm, r.reachable_depth_upper_mm);
        // A V-bit whose flat tip is no larger than the endmill radius can
        // reproduce every feasible endmill removal by translating its center.
        if tip > ctx.mill.radius().mm() {
            let e = ctx
                .target
                .endmill_reachability(&ctx.mill, ctx.allowance, p, query_options)?;
            reach_cells += e.evaluated_cells;
            at.maximum(Interval::positive(
                e.reachable_depth_lower_mm,
                e.reachable_depth_upper_mm,
            ));
        }
        (
            Interval::positive(
                (at.lower - variation - ctx.reserve).max(0.),
                (at.upper + variation + ctx.reserve).min(target.upper),
            ),
            at,
        )
    };
    let overcut = Interval::positive(
        actual_at - target_at - 2. * ctx.reserve,
        if all_clear {
            ctx.reserve
        } else {
            (actual.upper - target.lower).max(0.)
        },
    );
    let missed = Interval::positive(
        h_at.lower - actual_at - ctx.reserve,
        (h.upper - actual.lower).min(residual_upper),
    );
    let unreachable = if tip == 0. {
        Interval::default()
    } else {
        Interval::positive(
            target_at - h_at.upper - ctx.reserve,
            (target.upper - h.lower)
                .min(tip / slope)
                .min(if all_clear { residual_upper } else { cap })
                .max(0.),
        )
    };
    let floor = if target.upper < cap {
        Interval::default()
    } else {
        Interval::positive(
            if target_at == cap { missed.lower } else { 0. },
            missed.upper,
        )
    };
    let other = if target.lower == cap {
        Interval::default()
    } else {
        Interval::positive(
            if target_at < cap { missed.lower } else { 0. },
            missed.upper,
        )
    };
    let residual = Interval::positive(target_at - actual_at - 2. * ctx.reserve, residual_upper);
    Ok(Evaluation {
        target,
        actual,
        errors: [overcut, floor, unreachable, other, residual],
        reachability_cells: reach_cells,
    })
}

fn bands(
    leaves: &[Leaf],
    critical: &[f64],
    tolerance: f64,
    limit: usize,
) -> (Vec<DepthBand>, bool) {
    let mut depths = critical.to_vec();
    depths.sort_by(f64::total_cmp);
    depths.dedup();
    // Height-field leaves enclose the entire continuous sweep. Their occupancy
    // bounds below hold throughout any closed band; motion endpoint depths do
    // not need separate bands. Retain the complete range when a budget ends.
    let mut limited = depths.len() - 1 > limit;
    if limited {
        depths = vec![depths[0], *depths.last().unwrap()];
    }
    loop {
        let split = depths.windows(2).position(|w| {
            w[1] - w[0] > tolerance
                && leaves.iter().any(|l| {
                    l.target.upper >= w[0] && l.target.lower <= w[1]
                        || l.actual.upper >= w[0] && l.actual.lower <= w[1]
                })
        });
        let Some(i) = split else {
            break;
        };
        if depths.len() > limit {
            limited = true;
            break;
        }
        depths.insert(i + 1, (depths[i] + depths[i + 1]) / 2.);
    }
    let result = depths
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let mut nominal = Interval::default();
            let mut removed = Interval::default();
            let mut residual = Interval::default();
            let mut overcut = Interval::default();
            for l in leaves {
                let t_all = l.target.lower >= b;
                let t_any = l.target.upper >= a;
                let a_all = l.actual.lower >= b;
                let a_any = l.actual.upper >= a;
                if t_all {
                    nominal.lower += l.area;
                }
                if t_any {
                    nominal.upper += l.area;
                }
                if a_all {
                    removed.lower += l.area;
                }
                if a_any {
                    removed.upper += l.area;
                }
                if t_all && !a_any {
                    residual.lower += l.area;
                }
                if t_any && !a_all {
                    residual.upper += l.area;
                }
                if a_all && !t_any {
                    overcut.lower += l.area;
                }
                if a_any && !t_all {
                    overcut.upper += l.area;
                }
            }
            let reserve = 32. * f64::EPSILON * (leaves.len() as f64 + 1.);
            for b in [&mut nominal, &mut removed, &mut residual, &mut overcut] {
                b.lower = (b.lower * (1. - reserve)).max(0.);
                b.upper *= 1. + reserve;
            }
            DepthBand {
                from_depth_mm: a,
                to_depth_mm: b,
                nominal_area_mm2: nominal,
                removed_area_mm2: removed,
                residual_area_mm2: residual,
                overcut_area_mm2: overcut,
            }
        })
        .collect();
    (result, limited)
}

fn maxima(b: &ErrorBounds) -> [Interval; 4] {
    [
        b.overcut_mm,
        b.floor_ridge_mm,
        b.unreachable_detail_mm,
        b.other_reachable_residual_mm,
    ]
}
pub(super) fn verify(
    ctx: &Context<'_>,
    endmill: &[Motion],
    vbit: &[Motion],
    options: &VerificationOptions,
    places: Option<usize>,
    findings: Vec<Finding>,
) -> Result<StockVerification> {
    let mut first = run(ctx, endmill, vbit, options, places, findings.clone(), None)?;
    if first.status != VerificationStatus::Passed
        || first.maximum_error_uncertainty_mm <= ctx.tolerance
    {
        return Ok(first);
    }
    if first.evaluated_cells >= options.max_cells {
        first.status = VerificationStatus::Inconclusive;
        if first.findings.len() < options.max_findings {
            first.findings.push(Finding {
                code: "M5_MAXIMUM_UNCERTAINTY".into(),
                status: VerificationStatus::Inconclusive,
                message: "no cell budget remains to tighten maximum-error uncertainty".into(),
                location: first.domain.min,
                cell: None,
                motion_id: None,
                measured_mm: None,
                limit_mm: Some(ctx.tolerance),
            });
        } else {
            first.omitted_findings += 1;
        }
        return Ok(first);
    }
    let mut remaining = options.clone();
    remaining.max_cells = options
        .max_cells
        .saturating_sub(first.evaluated_cells)
        .max(1);
    let caps = maxima(&first.bounds).map(|b| b.lower + ctx.tolerance * 0.99);
    let mut refined = run(ctx, endmill, vbit, &remaining, places, findings, Some(caps))?;
    refined.evaluated_cells += first.evaluated_cells;
    refined.reachability_cells += first.reachability_cells;
    for (new, old) in [
        &mut refined.bounds.overcut_mm,
        &mut refined.bounds.floor_ridge_mm,
        &mut refined.bounds.unreachable_detail_mm,
        &mut refined.bounds.other_reachable_residual_mm,
    ]
    .into_iter()
    .zip(maxima(&first.bounds))
    {
        new.lower = new.lower.max(old.lower);
    }
    refined.maximum_error_uncertainty_mm = maxima(&refined.bounds)
        .into_iter()
        .map(|b| b.upper - b.lower)
        .fold(0., f64::max);
    if refined.status == VerificationStatus::Passed
        && refined.maximum_error_uncertainty_mm > ctx.tolerance
    {
        refined.status = VerificationStatus::Inconclusive;
        refined.findings.push(Finding {
            code: "M5_MAXIMUM_UNCERTAINTY".into(),
            status: VerificationStatus::Inconclusive,
            message:
                "maximum-error enclosure remains wider than the requested verification tolerance"
                    .into(),
            location: refined.domain.min,
            cell: None,
            motion_id: None,
            measured_mm: None,
            limit_mm: Some(ctx.tolerance),
        });
    }
    Ok(refined)
}

fn run(
    ctx: &Context<'_>,
    endmill: &[Motion],
    vbit: &[Motion],
    options: &VerificationOptions,
    places: Option<usize>,
    mut findings: Vec<Finding>,
    caps: Option<[f64; 4]>,
) -> Result<StockVerification> {
    let mut timing = crate::timing::Timer::new("M5 continuous");
    let (clear_count, motion_findings) = super::motion::check(ctx, endmill, vbit, places);
    timing.lap("motion checks");
    findings.extend(motion_findings);
    let all_clear = clear_count
        == endmill
            .iter()
            .chain(vbit)
            .filter(|m| m.kind.cutting())
            .count();
    let mut domain = Bounds::of(ctx.target.region()).unwrap();
    let mut sweeps = vec![];
    let cap = ctx.target.depth_cap().mm();
    let mut critical = vec![0., cap, (cap - ctx.ridge).max(0.)];
    let mut maximum_depth = cap;
    for (is_endmill, moves) in [(true, endmill), (false, vbit)] {
        for m in moves {
            if !m.start.finite() || !m.end.finite() {
                return Err(error(
                    "VERIFICATION_COORDINATE",
                    "nonfinite motion cannot be bounded",
                ));
            }
            ctx.target.region().grid().quantize(m.start.xy())?;
            ctx.target.region().grid().quantize(m.end.xy())?;
            if !m.kind.cutting() {
                continue;
            }
            let radius = if is_endmill {
                ctx.mill.radius().mm()
            } else {
                ctx.vbit.tip_radius().mm()
                    + m.start.depth().max(m.end.depth()) * ctx.vbit.angle().slope()
            } + ctx.reserve;
            let bounds = Bounds {
                min: Point::new(
                    m.start.x.min(m.end.x) - radius,
                    m.start.y.min(m.end.y) - radius,
                ),
                max: Point::new(
                    m.start.x.max(m.end.x) + radius,
                    m.start.y.max(m.end.y) + radius,
                ),
            };
            include(&mut domain, bounds);
            sweeps.push(Sweep {
                endmill: is_endmill,
                motion: m,
                bounds,
            });
            maximum_depth = maximum_depth.max(m.start.depth()).max(m.end.depth());
        }
    }
    critical.push(maximum_depth);
    timing.lap("prepare sweeps");
    let mut result_status = findings
        .iter()
        .fold(VerificationStatus::Passed, |s, f| status(s, f.status));
    let mut omitted = findings.len().saturating_sub(options.max_findings);
    findings.truncate(options.max_findings);
    let mut queue = vec![Cell {
        bounds: domain,
        depth: 0,
        sweeps: (0..sweeps.len()).collect(),
    }];
    let mut leaves = vec![];
    let mut evaluated = 0;
    let mut unresolved = 0;
    let mut deepest = 0;
    let mut reachability_cells = 0;
    let mut bounds = ErrorBounds::default();
    for f in &findings {
        if f.code == "M5_OVERCUT"
            && let Some(measured) = f.measured_mm
        {
            bounds.overcut_mm.lower = bounds.overcut_mm.lower.max(measured.lower);
        }
    }
    let limits = [ctx.tolerance, ctx.ridge, ctx.detail, ctx.tolerance];
    let mut acceptance = caps.map_or(limits, |c| std::array::from_fn(|i| c[i].min(limits[i])));
    let codes = [
        "M5_OVERCUT",
        "M5_FLOOR_RIDGE",
        "M5_UNREACHABLE_DETAIL",
        "M5_REACHABLE_RESIDUAL",
    ];
    while let Some(cell) = queue.pop() {
        let p = cell.bounds.min.lerp(cell.bounds.max, 0.5);
        // Unprocessed cells keep full conservative bounds when the budget ends.
        let budget = evaluated >= options.max_cells;
        let evaluation = if budget {
            let maximum = critical.iter().copied().fold(cap, f64::max) + ctx.reserve;
            Evaluation {
                target: Interval::positive(0., cap),
                actual: Interval::positive(0., maximum),
                errors: [
                    Interval::positive(0., maximum),
                    Interval::positive(0., cap),
                    Interval::positive(0., cap),
                    Interval::positive(0., cap),
                    Interval::positive(0., cap),
                ],
                reachability_cells: 0,
            }
        } else {
            evaluated += 1;
            evaluate(ctx, &cell, &sweeps, options, all_clear)?
        };
        deepest = deepest.max(cell.depth);
        reachability_cells += evaluation.reachability_cells;
        if caps.is_some() {
            for i in 0..4 {
                acceptance[i] = acceptance[i]
                    .max(evaluation.errors[i].lower + ctx.tolerance * 0.99)
                    .min(limits[i]);
            }
        }
        let failure = (0..4).find(|&i| evaluation.errors[i].lower > limits[i]);
        let ambiguous = budget || (0..4).any(|i| evaluation.errors[i].upper > acceptance[i]);
        if failure.is_none()
            && ambiguous
            && !budget
            && cell.depth < options.max_depth
            && evaluated + queue.len() + 4 <= options.max_cells
            && p.x > cell.bounds.min.x
            && p.x < cell.bounds.max.x
            && p.y > cell.bounds.min.y
            && p.y < cell.bounds.max.y
        {
            // Fixed quadrant order and depth-first traversal keep artifacts stable.
            for b in children(cell.bounds) {
                queue.push(Cell {
                    bounds: b,
                    depth: cell.depth + 1,
                    sweeps: cell
                        .sweeps
                        .iter()
                        .copied()
                        .filter(|&i| overlap(b, sweeps[i].bounds))
                        .collect(),
                });
            }
            continue;
        }
        let finding = if let Some(i) = failure {
            Some(Finding { code:codes[i].into(), status:VerificationStatus::Failed,
                message:"an independent point witness exceeds the criterion; upper bound covers the whole cell".into(),
                location:p,cell:Some(cell.bounds),motion_id:None,measured_mm:Some(evaluation.errors[i]),limit_mm:Some(limits[i]) })
        } else if ambiguous {
            unresolved += 1;
            let i = (0..4)
                .find(|&i| evaluation.errors[i].upper > limits[i])
                .unwrap_or(3);
            Some(Finding {
                code: "M5_UNRESOLVED_CELL".into(),
                status: VerificationStatus::Inconclusive,
                message: format!(
                    "{} bound remains unresolved at the cell, depth, or arithmetic limit",
                    codes[i]
                ),
                location: p,
                cell: Some(cell.bounds),
                motion_id: None,
                measured_mm: Some(evaluation.errors[i]),
                limit_mm: Some(limits[i]),
            })
        } else {
            None
        };
        if let Some(f) = finding {
            result_status = status(result_status, f.status);
            if findings.len() < options.max_findings {
                findings.push(f);
            } else {
                omitted += 1;
            }
        }
        bounds.overcut_mm.maximum(evaluation.errors[0]);
        bounds.floor_ridge_mm.maximum(evaluation.errors[1]);
        bounds.unreachable_detail_mm.maximum(evaluation.errors[2]);
        bounds
            .other_reachable_residual_mm
            .maximum(evaluation.errors[3]);
        bounds.total_residual_mm.maximum(evaluation.errors[4]);
        let size = area(cell.bounds);
        bounds.residual_volume_mm3.lower +=
            size * (evaluation.target.lower - evaluation.actual.upper).max(0.);
        bounds.residual_volume_mm3.upper += size * evaluation.errors[4].upper;
        bounds.overcut_volume_mm3.lower +=
            size * (evaluation.actual.lower - evaluation.target.upper).max(0.);
        bounds.overcut_volume_mm3.upper += size * evaluation.errors[0].upper;
        leaves.push(Leaf {
            area: size,
            target: evaluation.target,
            actual: evaluation.actual,
        });
    }
    timing.lap("adaptive cells");
    let (depth_bands, limited) = bands(&leaves, &critical, ctx.tolerance, options.max_depth_bands);
    timing.lap("depth bands");
    let sum_reserve = 32. * f64::EPSILON * (leaves.len() as f64 + 1.);
    for b in [
        &mut bounds.residual_volume_mm3,
        &mut bounds.overcut_volume_mm3,
    ] {
        b.lower = (b.lower * (1. - sum_reserve)).max(0.);
        b.upper *= 1. + sum_reserve;
    }
    if limited {
        result_status = status(result_status, VerificationStatus::Inconclusive);
        let f = Finding { code:"M5_DEPTH_BAND_LIMIT".into(),status:VerificationStatus::Inconclusive,
            message:"depth event/refinement budget exhausted; remaining bands retain their full area bounds".into(),
            location:domain.min,cell:None,motion_id:None,measured_mm:None,limit_mm:None };
        if findings.len() < options.max_findings {
            findings.push(f);
        } else {
            omitted += 1;
        }
    }
    Ok(StockVerification {
        status:result_status,domain,verification_tolerance_mm:ctx.tolerance,floor_ridge_limit_mm:ctx.ridge,detail_residual_limit_mm:ctx.detail,
        arithmetic_reserve_mm:ctx.reserve,source_geometry_depth_error_mm:ctx.source_error,
        checked_motion_count:endmill.len()+vbit.len(),analytically_clear_motion_count:clear_count,
        evaluated_cells:evaluated,terminal_cells:leaves.len(),unresolved_cells:unresolved,maximum_refinement_depth:deepest,reachability_cells,
        maximum_error_uncertainty_mm:maxima(&bounds).into_iter().map(|b|b.upper-b.lower).fold(0.,f64::max),
        bounds,depth_bands,findings,omitted_findings:omitted,
        limitations:vec![
            "Bounds apply to the rebuilt normalized polygon target, modeled cutters, and recorded linear XYZ motions. Source flattening/snap depth error is reported separately; this is not a topology proof for arbitrary SVG artwork.".into(),
            "Analytical distance queries use explicit floating-point reserves, not directed-rounding interval arithmetic. Preview polygon Boolean results are not used as acceptance evidence.".into(),
            "Depth-band areas and residual volumes retain spatial uncertainty. A pass bounds maximum finish errors; it does not claim that area or volume uncertainty has a requested dimensional tolerance.".into(),
            "Fixture/holder clearance, machine dynamics, feeds suitability, and hidden tool-change motions are outside this geometric contract. Numeric formatting is checked by explicit rounded-coordinate verification or the M6 emitted-program reader.".into(),
        ],
    })
}
