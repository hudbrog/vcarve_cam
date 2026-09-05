use super::{Context, MedialAxis, error};
use crate::{
    geometry::{BooleanOp, Diagnostic, Point, Region, Result},
    model::{Depth, Length},
    motion::Motion,
    pocket::{EndmillPlan, PlanStatus},
    stock::{
        SliceRemoval, capsule_bounds, removal_at_slice, removed_depth_at, vbit_removal_at_slice,
        vbit_removed_depth_at,
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
pub(super) fn sample(ctx: &Context, endmill: &EndmillPlan, moves: &[Motion]) -> Result<Samples> {
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
    let mut samples = vec![];
    for p in points {
        let nominal = ctx.target.nominal_depth(p)?.mm();
        if nominal == 0. {
            continue;
        }
        let ea = removed_depth_at(&endmill.motions, ctx.mill.radius().mm(), p)?;
        let va = vbit_removed_depth_at(moves, &ctx.tool, p)?;
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
    Ok(Samples { samples, limited })
}
fn combined_slice(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
    depth: f64,
) -> Result<CombinedSlice> {
    let grid = ctx.target.region().grid();
    let e = removal_at_slice(grid, &endmill.motions, ctx.mill.radius().mm(), depth)?;
    let v = vbit_removal_at_slice(grid, moves, &ctx.tool, depth)?;
    let lower = e.lower.boolean(BooleanOp::Union, &v.lower)?;
    let upper = e.upper.boolean(BooleanOp::Union, &v.upper)?;
    let nominal = ctx.target.section(Depth::new(depth)?)?.area;
    let remaining = nominal.boolean(BooleanOp::Difference, &lower)?;
    let overcut = upper.boolean(BooleanOp::Difference, &nominal)?;
    let mut ids = e.contributing_motion_ids;
    ids.extend(v.contributing_motion_ids);
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

pub(super) fn floor_cleanup_centers(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
) -> Result<Vec<Point>> {
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
    Ok(points)
}
pub(super) fn analyze(
    ctx: &Context,
    endmill: &EndmillPlan,
    moves: &[Motion],
    axis: MedialAxis,
) -> Result<CombinedAnalysis> {
    let minimum = super::verify::verify_vbit_motions(&endmill.job, &endmill.motions, moves)?;
    let quality = sample(ctx, endmill, moves)?;
    let cap = ctx.target.depth_cap().mm();
    let floor_check = (cap - ctx.ridge - ctx.tolerance / 2.).max(0.);
    let mut depths = (1..=ctx.settings.stock_slices)
        .map(|i| i as f64 * cap / ctx.settings.stock_slices as f64)
        .collect::<Vec<_>>();
    depths.push(floor_check);
    depths.sort_by(f64::total_cmp);
    depths.dedup();
    let slices = depths
        .into_iter()
        .map(|d| combined_slice(ctx, endmill, moves, d))
        .collect::<Result<Vec<_>>>()?;
    let access = ctx.target.vbit_centers(&ctx.tool, Depth::new(cap)?)?;
    let ea = ctx.target.endmill_centers(
        &ctx.mill,
        Depth::new(cap)?,
        Length::new(endmill.job.operation.wall_allowance_mm.unwrap())?,
    )?;
    let accessible = accessible_area(&access, ctx.tool.tip_radius().mm())?.boolean(
        BooleanOp::Union,
        &accessible_area(&ea, ctx.mill.radius().mm())?,
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
        limitations:vec!["Quality maxima are measured at the configured lattice and motion witnesses, not certified over the entire height field; adaptive full-volume verification remains M5.".into(),"Individual variable-radius capsule brackets include snapping. Repeated polygon Boolean errors and topology are not a formal interval proof.".into(),"Guarded V-bit depths/contours reserve geometric clearance; normalized source error is reported separately by job inspection.".into(),"Tool transitions are planning markers only. M6 machine output, fixture/holder clearance, and cutting loads are not implemented.".into()]})
}
