use super::{Context, MedialAxis, error};
use crate::{
    geometry::{BooleanOp, Diagnostic, Point, Region, Result},
    model::{Depth, Length},
    motion::Motion,
    pocket::{EndmillPlan, PlanStatus},
    stock::{
        SliceRemoval, StockQuery, capsule_bounds, vbit_removal_at_slice, vbit_removal_at_slices,
    },
    svg::Bounds,
    target::{CenterSet, CenterSetStatus, ReachabilityOptions, ReachabilityStatus},
};
use serde::Serialize;
use std::collections::BTreeSet;
#[derive(Clone, Debug, Serialize)]
pub struct QualitySample {
    pub point: Point,
    pub nominal_depth_mm: f64,
    pub endmill_removed_mm: f64,
    pub vbit_removed_mm: f64,
    pub removed_mm: f64,
    pub residual_mm: f64,
    pub allowed_ridge_mm: f64,
    pub unavoidable_residual_lower_mm: f64,
    pub unavoidable_residual_upper_mm: f64,
    pub missed_reachable_mm: f64,
    pub missed_reachable_upper_mm: f64,
    pub reachability_resolved: bool,
    pub best_tip_center: Point,
}
#[derive(Clone, Debug, Serialize)]
pub struct CombinedSlice {
    pub depth_mm: f64,
    pub nominal_section: Region,
    pub removal: SliceRemoval,
    pub remaining_target: Region,
    pub possible_overcut: Region,
}
#[derive(Clone, Debug, Serialize)]
pub struct CombinedAnalysis {
    pub status: PlanStatus,
    pub medial_axis: MedialAxis,
    pub slices: Vec<CombinedSlice>,
    pub samples: Vec<QualitySample>,
    pub accessible_floor: Region,
    pub floor_check_depth_mm: f64,
    pub floor_numerical_depth_budget_mm: f64,
    pub missing_floor_beyond_tolerance: Region,
    pub max_sampled_residual_mm: f64,
    pub max_sampled_missed_reachable_mm: f64,
    pub max_sampled_unavoidable_residual_mm: f64,
    pub minimum_center_margin_lower_bound_mm: Option<f64>,
    pub finish_paths_expected: usize,
    pub finish_paths_executed: usize,
    pub pruned_air_paths: usize,
    pub cleanup_paths: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub meaning: String,
    pub limitations: Vec<String>,
}
pub(super) struct Samples {
    pub samples: Vec<QualitySample>,
    pub limited: bool,
}

pub(super) struct SampleReuse {
    samples: Samples,
    floor: Option<CombinedSlice>,
    prefix_len: usize,
    prefix_hash: String,
}

pub(super) struct FloorCleanup {
    pub(super) slice: CombinedSlice,
    pub(super) points: Vec<Point>,
}

pub(super) fn cleanup(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
) -> Result<(Samples, FloorCleanup)> {
    // Both inspect the same immutable stock and neither adds motions. Running
    // the floor reconstruction beside the sample workers shortens the serial
    // cleanup stage; sample errors retain their original reporting priority.
    if moves.len() < 4096 || std::thread::available_parallelism().map_or(1, usize::from) < 2 {
        return Ok((
            sample(ctx, endmill, moves)?,
            floor_cleanup(ctx, endmill, moves)?,
        ));
    }
    std::thread::scope(|scope| {
        let floor = scope.spawn(|| floor_cleanup(ctx, endmill, moves));
        let quality = sample(ctx, endmill, moves);
        let floor = floor
            .join()
            .map_err(|_| error("CLEANUP_WORKER_PANIC", "floor cleanup worker failed"));
        Ok((quality?, floor??))
    })
}

impl SampleReuse {
    pub(super) fn new(
        samples: Samples,
        moves: &[Motion],
        floor: Option<CombinedSlice>,
    ) -> Result<Self> {
        Ok(Self {
            samples,
            floor,
            prefix_len: moves.len(),
            prefix_hash: super::hash(&moves)?,
        })
    }
    fn recover(self, moves: &[Motion]) -> Result<Option<(Samples, Option<CombinedSlice>)>> {
        let Some(prefix) = moves.get(..self.prefix_len) else {
            return Ok(None);
        };
        if super::hash(&prefix)? != self.prefix_hash {
            return Ok(None);
        }
        // Exact motion identity preserves the slice polygons and contributing
        // IDs. Appended duplicates may reuse point samples below, but their
        // slice IDs and polygon reconstruction must be computed afresh.
        if moves.len() == self.prefix_len {
            return Ok(Some((self.samples, self.floor)));
        }
        // Appended forward duplicates have exactly the same analytic removal
        // and sample witnesses. Anything new falls back to full sampling.
        let key = |m: &Motion| {
            [
                m.start.x.to_bits(),
                m.start.y.to_bits(),
                m.start.z.to_bits(),
                m.end.x.to_bits(),
                m.end.y.to_bits(),
                m.end.z.to_bits(),
            ]
        };
        let mut seen = std::collections::HashSet::new();
        for m in prefix.iter().filter(|m| m.kind.cutting()) {
            seen.insert(key(m));
            if seen.len() > 131072 {
                return Ok(None);
            }
        }
        if moves[self.prefix_len..]
            .iter()
            .filter(|m| m.kind.cutting())
            .all(|m| seen.contains(&key(m)))
        {
            Ok(Some((self.samples, None)))
        } else {
            Ok(None)
        }
    }
}
pub(super) fn sample(ctx: &Context, endmill: &EndmillPlan, moves: &[Motion]) -> Result<Samples> {
    let _timing = crate::timing::Timer::new("quality samples");
    let bounds = Bounds::of(ctx.target.region()).unwrap();
    let step = ctx.settings.quality_sample_spacing_mm;
    let nx = ((bounds.max.x - bounds.min.x) / step).ceil().max(1.);
    let ny = ((bounds.max.y - bounds.min.y) / step).ceil().max(1.);
    if nx * ny > ctx.settings.max_quality_samples as f64 {
        return Ok(Samples {
            samples: vec![],
            limited: true,
        });
    }
    let mut points = vec![];
    let mut keys = BTreeSet::new();
    let mut limited = false;
    let grid = ctx.target.region().grid();
    let mut add = |p: Point| -> Result<()> {
        let q = grid.quantize(p)?;
        if keys.insert((q.x, q.y)) {
            if points.len() >= ctx.settings.max_quality_samples {
                limited = true;
            } else {
                points.push(p);
            }
        }
        Ok(())
    };
    for iy in 0..ny as usize {
        for ix in 0..nx as usize {
            add(Point::new(
                bounds.min.x + (ix as f64 + 0.5) * (bounds.max.x - bounds.min.x) / nx,
                bounds.min.y + (iy as f64 + 0.5) * (bounds.max.y - bounds.min.y) / ny,
            ))?;
        }
    }
    for m in moves.iter().filter(|m| m.kind.cutting()) {
        for t in [0., 0.5, 1.] {
            add(m.start.lerp(m.end, t).xy())?;
        }
    }
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    let samples = sample_points(ctx, endmill, moves, &points, workers)?;
    Ok(Samples { samples, limited })
}

fn sample_points(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
    points: &[Point],
    workers: usize,
) -> Result<Vec<QualitySample>> {
    let endmill_stock = StockQuery::endmill(&endmill.motions, ctx.mill.radius().mm())?;
    let vbit_stock = StockQuery::vbit(moves, &ctx.tool)?;
    // Queries are immutable; each worker owns its samples. Contiguous chunks
    // and ordered joins preserve both sample order and the first reported error.
    let workers = workers.max(1).min(points.len().div_ceil(4096).max(1));
    if workers == 1 {
        return evaluate_samples(ctx, endmill, &endmill_stock, &vbit_stock, points);
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = points
            .chunks(points.len().div_ceil(workers))
            .map(|chunk| {
                let endmill_stock = &endmill_stock;
                let vbit_stock = &vbit_stock;
                scope
                    .spawn(move || evaluate_samples(ctx, endmill, endmill_stock, vbit_stock, chunk))
            })
            .collect();
        let batches = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| error("QUALITY_WORKER_PANIC", "quality sample worker failed"))?
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(batches.into_iter().flatten().collect())
    })
}

fn evaluate_samples(
    ctx: &Context,
    endmill: &EndmillPlan,
    endmill_stock: &StockQuery<'_>,
    vbit_stock: &StockQuery<'_>,
    points: &[Point],
) -> Result<Vec<QualitySample>> {
    let mut samples = vec![];
    for &p in points {
        let nominal = ctx.target.nominal_depth(p)?.mm();
        if nominal == 0. {
            continue;
        }
        let ea = endmill_stock.removed_depth(p)?;
        let va = vbit_stock.removed_depth(p)?;
        let actual = ea.max(va);
        let ridge = if nominal == ctx.target.depth_cap().mm() {
            ctx.ridge
        } else {
            0.
        };
        let (mut lower, mut upper, mut best, mut resolved) = (
            if ctx.tool.tip_radius().mm() > 0. {
                actual.min(nominal)
            } else {
                nominal
            },
            nominal,
            p,
            true,
        );
        if ctx.tool.tip_radius().mm() > 0. && nominal - actual > ctx.tolerance / 2. {
            let options = ReachabilityOptions {
                depth_tolerance_mm: ctx.tolerance / 4.,
                max_cells: ctx.settings.reachability_max_cells,
            };
            let r = ctx.target.vbit_reachability(&ctx.tool, p, options)?;
            lower = r.reachable_depth_lower_mm.max(ea);
            upper = r.reachable_depth_upper_mm.max(ea);
            best = r.best_tip_center;
            resolved = r.status == ReachabilityStatus::Resolved;
            if ctx.tool.tip_radius().mm() > ctx.mill.radius().mm() {
                let r = ctx.target.endmill_reachability(
                    &ctx.mill,
                    Length::new(endmill.job.operation.wall_allowance_mm.unwrap())?,
                    p,
                    options,
                )?;
                lower = lower.max(r.reachable_depth_lower_mm);
                upper = upper.max(r.reachable_depth_upper_mm);
                resolved &= r.status == ReachabilityStatus::Resolved;
            }
        }
        samples.push(QualitySample {
            point: p,
            nominal_depth_mm: nominal,
            endmill_removed_mm: ea,
            vbit_removed_mm: va,
            removed_mm: actual,
            residual_mm: (nominal - actual).max(0.),
            allowed_ridge_mm: ridge,
            unavoidable_residual_lower_mm: (nominal - upper).max(0.),
            unavoidable_residual_upper_mm: (nominal - lower).max(0.),
            missed_reachable_mm: (lower - actual).max(0.),
            missed_reachable_upper_mm: (upper - actual).max(0.),
            reachability_resolved: resolved,
            best_tip_center: best,
        });
    }
    Ok(samples)
}
fn combined_slice(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
    depth: f64,
) -> Result<CombinedSlice> {
    let vbit = vbit_removal_at_slice(ctx.target.region().grid(), moves, &ctx.tool, depth)?;
    combined_slice_with_vbit(ctx, endmill, depth, &vbit)
}
fn combined_slice_with_vbit(
    ctx: &Context,
    endmill: &EndmillPlan,
    depth: f64,
    v: &SliceRemoval,
) -> Result<CombinedSlice> {
    let mut timing = crate::timing::Timer::new("combined stock slice");
    let e = super::endmill_slice(ctx, endmill, depth)?;
    timing.lap("endmill");
    let lower = e.lower.boolean(BooleanOp::Union, &v.lower)?;
    let upper = e.upper.boolean(BooleanOp::Union, &v.upper)?;
    timing.lap("merge tools");
    let nominal = ctx.target.section_area(Depth::new(depth)?)?;
    timing.lap("nominal section");
    let remaining = nominal.boolean(BooleanOp::Difference, &lower)?;
    let overcut = upper.boolean(BooleanOp::Difference, &nominal)?;
    let mut ids = e.contributing_motion_ids;
    ids.extend(v.contributing_motion_ids.iter().copied());
    Ok(CombinedSlice {
        depth_mm: depth,
        nominal_section: nominal,
        remaining_target: remaining,
        possible_overcut: overcut,
        removal: SliceRemoval {
            depth_mm: depth,
            lower,
            upper,
            contributing_motion_ids: ids,
            capsule_radial_error_mm: e.capsule_radial_error_mm.max(v.capsule_radial_error_mm),
        },
    })
}
fn accessible_area(access: &CenterSet, radius: f64) -> Result<Region> {
    let mut area = access.area.dilate(radius)?;
    if radius > 0. {
        for s in &access.contact_segments {
            area = area.boolean(
                BooleanOp::Union,
                &capsule_bounds(area.grid(), s.start, s.end, radius)?.upper,
            )?;
        }
        for &p in &access.contact_points {
            area = area.boolean(
                BooleanOp::Union,
                &capsule_bounds(area.grid(), p, p, radius)?.upper,
            )?;
        }
    }
    Ok(area)
}

fn endmill_accessible_floor(ctx: &Context, endmill: &EndmillPlan, cap: f64) -> Result<Region> {
    // A freshly planned/authenticated complete layer has no unresolved or
    // exact-fit contacts, so its recorded accessible area is the same query.
    if let Some(layer) =
        endmill.analysis.layers.iter().find(|l| {
            l.depth_mm == cap && matches!(l.status, PlanStatus::Complete | PlanStatus::Empty)
        })
    {
        return Ok(layer.accessible_floor.clone());
    }
    let access = ctx.target.endmill_centers(
        &ctx.mill,
        Depth::new(cap)?,
        Length::new(endmill.job.operation.wall_allowance_mm.unwrap())?,
    )?;
    accessible_area(&access, ctx.mill.radius().mm())
}

fn stock_slices(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
    depths: &[f64],
) -> Result<Vec<CombinedSlice>> {
    let vbit = vbit_removal_at_slices(ctx.target.region().grid(), moves, &ctx.tool, depths)?;
    // Each slice owns its unions. Bound concurrency/memory and retain depth
    // order (including which error is reported), independent of scheduling.
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8)
        .min(depths.len());
    if workers <= 1 {
        return depths
            .iter()
            .zip(&vbit)
            .map(|(&d, v)| combined_slice_with_vbit(ctx, endmill, d, v))
            .collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let batches = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let next = &next;
                let vbit = &vbit;
                scope.spawn(move || {
                    // A shallow slice is much more expensive than a deep one.
                    // Let the first available worker take the next depth rather
                    // than assigning the ninth slice behind the slowest worker.
                    let mut results = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&depth) = depths.get(i) else {
                            break;
                        };
                        results.push((i, combined_slice_with_vbit(ctx, endmill, depth, &vbit[i])));
                    }
                    results
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| error("STOCK_WORKER_PANIC", "stock reconstruction worker failed"))
            })
            .collect::<Result<Vec<_>>>()
    })?;
    let mut slices: Vec<_> = batches.into_iter().flatten().collect();
    slices.sort_by_key(|(i, _)| *i);
    slices.into_iter().map(|(_, slice)| slice).collect()
}

fn floor_cleanup(ctx: &Context, endmill: &EndmillPlan, moves: &[Motion]) -> Result<FloorCleanup> {
    let cap = ctx.target.depth_cap().mm();
    let access = ctx.target.vbit_centers(&ctx.tool, Depth::new(cap)?)?;
    let floor = combined_slice(
        ctx,
        endmill,
        moves,
        (cap - ctx.ridge - ctx.tolerance / 2.).max(0.),
    )?;
    let missing = accessible_area(&access, ctx.tool.tip_radius().mm())?
        .erode(ctx.tolerance)?
        .boolean(BooleanOp::Difference, &floor.removal.lower)?;
    let mut points = vec![];
    for component in missing.components().into_iter().take(16) {
        let bounds = Bounds::of(&component).unwrap();
        let query = crate::geometry::BoundaryQuery::new(&component);
        let center = bounds.min.lerp(bounds.max, 0.5);
        let witness = if query.sample(center)?.location == crate::geometry::PointLocation::Inside {
            center
        } else {
            let ring = component.rings_mm().remove(0);
            let a = ring[0];
            let b = ring[1];
            let length = a.distance(b);
            let mut witness = a.lerp(b, 0.5);
            for i in 0..16 {
                let d = ctx.guard / 2f64.powi(i);
                let p = Point::new(
                    witness.x - (b.y - a.y) * d / length,
                    witness.y + (b.x - a.x) * d / length,
                );
                if query.sample(p)?.location == crate::geometry::PointLocation::Inside {
                    witness = p;
                    break;
                }
            }
            witness
        };
        let r = ctx.target.vbit_reachability(
            &ctx.tool,
            witness,
            ReachabilityOptions {
                depth_tolerance_mm: ctx.tolerance / 4.,
                max_cells: ctx.settings.reachability_max_cells,
            },
        )?;
        points.push(r.best_tip_center);
    }
    Ok(FloorCleanup {
        slice: floor,
        points,
    })
}
pub(super) fn analyze(
    ctx: &Context,
    endmill: &EndmillPlan,
    checked: super::verify::CheckedMotions<'_>,
    axis: MedialAxis,
    reuse: Option<SampleReuse>,
) -> Result<CombinedAnalysis> {
    let _timing = crate::timing::Timer::new("combined analysis");
    let moves = checked.motions();
    let minimum = checked.minimum_margin();
    let (quality, reused_floor) = match reuse.map(|r| r.recover(moves)).transpose()?.flatten() {
        Some(evidence) => evidence,
        None => (sample(ctx, endmill, moves)?, None),
    };
    let cap = ctx.target.depth_cap().mm();
    let floor_check = (cap - ctx.ridge - ctx.tolerance / 2.).max(0.);
    let mut depths = (1..=ctx.settings.stock_slices)
        .map(|i| i as f64 * cap / ctx.settings.stock_slices as f64)
        .collect::<Vec<_>>();
    depths.push(floor_check);
    depths.sort_by(f64::total_cmp);
    depths.dedup();
    let retained = reused_floor.and_then(|floor| {
        depths
            .iter()
            .position(|&d| d == floor.depth_mm)
            .map(|i| (i, floor))
    });
    if let Some((i, _)) = &retained {
        depths.remove(*i);
    }
    let mut slices = stock_slices(ctx, endmill, moves, &depths)?;
    if let Some((i, floor)) = retained {
        slices.insert(i, floor);
    }
    let access = ctx.target.vbit_centers(&ctx.tool, Depth::new(cap)?)?;
    let accessible = accessible_area(&access, ctx.tool.tip_radius().mm())?.boolean(
        BooleanOp::Union,
        &endmill_accessible_floor(ctx, endmill, cap)?,
    )?;
    let floor = slices.iter().find(|l| l.depth_mm == floor_check).unwrap();
    let missing = accessible
        .erode(ctx.tolerance)?
        .boolean(BooleanOp::Difference, &floor.removal.lower)?;
    let mut diagnostics = vec![];
    let mut uncertain = quality.limited || access.status == CenterSetStatus::Unresolved;
    if quality.limited {
        diagnostics.push(error(
            "QUALITY_SAMPLE_LIMIT",
            "requested sample lattice or path witnesses exceed the budget; quality is unresolved",
        ));
    }
    if access.status == CenterSetStatus::Unresolved {
        diagnostics.extend(access.diagnostics.clone());
        diagnostics.push(error(
            "VBIT_CENTER_UNRESOLVED",
            "full-depth V-bit center geometry is unresolved",
        ));
    }
    if !missing.rings().is_empty() {
        diagnostics.push(error("INCOMPLETE_VBIT_FLOOR","actual combined sweeps leave accessible floor above the allowed ridge depth and XY tolerance"));
    }
    if slices
        .iter()
        .any(|s| !s.possible_overcut.rings().is_empty())
    {
        uncertain = true;
        diagnostics.push(error(
            "COMBINED_SWEEP_UNCERTAINTY",
            "outer sweep polygons extend outside a nominal slice",
        ));
    }
    let samples = &quality.samples;
    if samples
        .iter()
        .any(|s| s.removed_mm > s.nominal_depth_mm + ctx.tolerance)
    {
        diagnostics.push(error(
            "COMBINED_OVERCUT",
            "sampled actual removal exceeds nominal target",
        ));
    }
    if samples
        .iter()
        .any(|s| s.unavoidable_residual_lower_mm > ctx.detail + ctx.tolerance)
    {
        diagnostics.push(error(
            "DETAIL_RESIDUAL_LIMIT",
            "proved cutter-limited detail exceeds the requested residual limit at reported points",
        ));
    }
    if samples
        .iter()
        .any(|s| s.missed_reachable_mm > s.allowed_ridge_mm + ctx.tolerance)
    {
        diagnostics.push(error("INCOMPLETE_REACHABLE_DETAIL","reachable stock remains beyond the allowed ridge/numerical tolerance; cleanup did not converge"));
    }
    if samples.iter().any(|s| {
        !s.reachability_resolved
            || s.unavoidable_residual_lower_mm <= ctx.detail + ctx.tolerance
                && s.unavoidable_residual_upper_mm > ctx.detail + ctx.tolerance
            || s.missed_reachable_mm <= s.allowed_ridge_mm + ctx.tolerance
                && s.missed_reachable_upper_mm > s.allowed_ridge_mm + ctx.tolerance
    }) {
        uncertain = true;
        diagnostics.push(error(
            "REACHABILITY_UNCERTAINTY",
            "reachability bounds straddle a quality criterion or exceed their resource budget",
        ));
    }
    let status = if uncertain {
        PlanStatus::Inconclusive
    } else if !diagnostics.is_empty() {
        PlanStatus::Incomplete
    } else if moves.is_empty() && endmill.motions.is_empty() {
        PlanStatus::Empty
    } else {
        PlanStatus::Complete
    };
    Ok(CombinedAnalysis{status,medial_axis:axis,slices,accessible_floor:accessible,floor_check_depth_mm:floor_check,floor_numerical_depth_budget_mm:ctx.tolerance/2.,missing_floor_beyond_tolerance:missing,
        max_sampled_residual_mm:samples.iter().map(|s|s.residual_mm).fold(0.,f64::max),max_sampled_missed_reachable_mm:samples.iter().map(|s|s.missed_reachable_mm).fold(0.,f64::max),max_sampled_unavoidable_residual_mm:samples.iter().map(|s|s.unavoidable_residual_lower_mm).fold(0.,f64::max),
        samples:quality.samples,minimum_center_margin_lower_bound_mm:minimum,finish_paths_expected:0,finish_paths_executed:0,pruned_air_paths:0,cleanup_paths:0,diagnostics,
        meaning:"M4 candidate-family completion, continuous segment clearance, floor slice coverage, and sampled combined finish-quality evidence.".into(),
        limitations:vec!["Quality maxima are measured at the configured lattice and motion witnesses, not certified over the entire height field; adaptive full-volume verification remains M5.".into(),"Individual variable-radius capsule brackets include snapping. Repeated polygon Boolean errors and topology are not a formal interval proof.".into(),"Guarded V-bit depths/contours reserve geometric clearance; normalized source error is reported separately by job inspection.".into(),"Tool transitions here are planning markers only; machine output requires M6 export. Fixture/holder clearance and cutting loads remain outside this planning analysis.".into()]})
}

#[cfg(test)]
mod slice_order_tests {
    use super::*;

    #[test]
    fn cleanup_floor_reuse_matches_fresh_analysis_and_requires_exact_motion_identity() {
        let job = crate::job::Job::from_json(include_str!("../../../../fixtures/m4/island.json"))
            .unwrap();
        let plan = super::super::plan_combined(&job).unwrap();
        let ctx = Context::new(&job).unwrap();
        let seed = || {
            let (samples, floor) = cleanup(&ctx, &plan.endmill, &plan.vbit_motions).unwrap();
            SampleReuse::new(samples, &plan.vbit_motions, Some(floor.slice)).unwrap()
        };
        let checked = || {
            super::super::verify::executions(
                &ctx,
                &plan.endmill,
                &plan.transition,
                &plan.vbit_motions,
                &plan.executions,
            )
            .unwrap()
        };
        let axis = &plan.analysis.medial_axis;
        let fresh = analyze(&ctx, &plan.endmill, checked(), axis.clone(), None).unwrap();
        let retained = analyze(&ctx, &plan.endmill, checked(), axis.clone(), Some(seed())).unwrap();
        assert_eq!(
            serde_json::to_value(fresh).unwrap(),
            serde_json::to_value(retained).unwrap()
        );
        assert!(
            seed()
                .recover(&plan.vbit_motions)
                .unwrap()
                .unwrap()
                .1
                .is_some()
        );

        let mut changed = plan.vbit_motions.clone();
        let repeated = changed.iter().find(|m| m.kind.cutting()).unwrap().clone();
        changed.push(repeated);
        assert!(
            seed().recover(&changed).unwrap().unwrap().1.is_none(),
            "even identical appended sweeps require fresh contributing-motion IDs"
        );
        changed.pop();
        changed[2].end.z -= 0.01;
        assert!(seed().recover(&changed).unwrap().is_none());
    }

    #[test]
    fn cleanup_samples_are_reused_only_for_unchanged_prefix_and_duplicate_finish() {
        let job = crate::job::Job::from_json(include_str!("../../../../fixtures/m4/island.json"))
            .unwrap();
        let plan = super::super::plan_combined(&job).unwrap();
        let ctx = Context::new(&job).unwrap();
        let seed = || {
            SampleReuse::new(
                sample(&ctx, &plan.endmill, &plan.vbit_motions).unwrap(),
                &plan.vbit_motions,
                None,
            )
            .unwrap()
        };
        let mut moves = plan.vbit_motions.clone();
        let mut repeated = moves
            .iter()
            .find(|m| m.kind == crate::motion::MotionKind::Cut)
            .unwrap()
            .clone();
        repeated.id = plan.endmill.motions.len() + moves.len();
        moves.push(repeated);
        let (cached, floor) = seed()
            .recover(&moves)
            .unwrap()
            .expect("duplicate cuts must reuse samples");
        assert!(floor.is_none(), "appended cuts must refresh slice IDs");
        let fresh = sample(&ctx, &plan.endmill, &moves).unwrap();
        assert_eq!(cached.limited, fresh.limited);
        assert_eq!(
            serde_json::to_value(cached.samples).unwrap(),
            serde_json::to_value(fresh.samples).unwrap()
        );
        moves.last_mut().unwrap().end.z -= 0.01;
        assert!(
            seed().recover(&moves).unwrap().is_none(),
            "new removal must be sampled"
        );
        moves.pop();
        moves[0].id += 1;
        assert!(
            seed().recover(&moves).unwrap().is_none(),
            "a changed prefix invalidates its samples"
        );
        assert!(
            seed()
                .recover(&plan.vbit_motions[..plan.vbit_motions.len() - 1])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn cached_endmill_access_matches_fresh_area_and_preserves_contact_fallback() {
        for fixture in [
            include_str!("../../../../fixtures/m4/island.json"),
            include_str!("../../../../fixtures/m4/exact-fit.json"),
        ] {
            let job = crate::job::Job::from_json(fixture).unwrap();
            let endmill = crate::pocket::plan_endmill(&job).unwrap();
            let ctx = Context::new(&job).unwrap();
            let cap = ctx.target.depth_cap().mm();
            let access = ctx
                .target
                .endmill_centers(
                    &ctx.mill,
                    Depth::new(cap).unwrap(),
                    Length::new(job.operation.wall_allowance_mm.unwrap()).unwrap(),
                )
                .unwrap();
            let expected = accessible_area(&access, ctx.mill.radius().mm()).unwrap();
            let cached = endmill_accessible_floor(&ctx, &endmill, cap).unwrap();
            assert_eq!(
                serde_json::to_value(cached).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    #[test]
    fn parallel_quality_samples_match_serial_values_and_error_order() {
        for fixture in [
            include_str!("../../../../fixtures/m4/island.json"),
            include_str!("../../../../fixtures/m4/finite-tip.json"),
        ] {
            let job = crate::job::Job::from_json(fixture).unwrap();
            let plan = super::super::plan_combined(&job).unwrap();
            let ctx = Context::new(&job).unwrap();
            let mut points: Vec<_> = (0..8193)
                .map(|i| Point::new((i % 97) as f64 / 4., (i / 97) as f64 / 4.))
                .collect();
            let sequential =
                sample_points(&ctx, &plan.endmill, &plan.vbit_motions, &points, 1).unwrap();
            let parallel =
                sample_points(&ctx, &plan.endmill, &plan.vbit_motions, &points, 4).unwrap();
            assert_eq!(
                serde_json::to_value(parallel).unwrap(),
                serde_json::to_value(sequential).unwrap()
            );
            points[4100] = Point::new(f64::NAN, 0.);
            points[8100] = Point::new(f64::INFINITY, 0.);
            let sequential =
                sample_points(&ctx, &plan.endmill, &plan.vbit_motions, &points, 1).unwrap_err();
            let parallel =
                sample_points(&ctx, &plan.endmill, &plan.vbit_motions, &points, 4).unwrap_err();
            assert_eq!(
                serde_json::to_value(parallel).unwrap(),
                serde_json::to_value(sequential).unwrap()
            );
        }
    }

    #[test]
    fn parallel_slices_match_sequential_geometry_and_error_order() {
        let job = crate::job::Job::from_json(include_str!("../../../../fixtures/m4/island.json"))
            .unwrap();
        let plan = super::super::plan_combined(&job).unwrap();
        let ctx = Context::new(&job).unwrap();
        let depths = [0.25, 0.5, 1., 1.5, 1.875, 2.];
        let parallel = stock_slices(&ctx, &plan.endmill, &plan.vbit_motions, &depths).unwrap();
        let sequential: Vec<_> = depths
            .iter()
            .map(|&d| combined_slice(&ctx, &plan.endmill, &plan.vbit_motions, d).unwrap())
            .collect();
        assert_eq!(
            serde_json::to_value(parallel).unwrap(),
            serde_json::to_value(sequential).unwrap()
        );
        let error =
            stock_slices(&ctx, &plan.endmill, &plan.vbit_motions, &[0.5, -1., 3.]).unwrap_err();
        assert_eq!(error.code, "STOCK_DEPTH");
    }
}
