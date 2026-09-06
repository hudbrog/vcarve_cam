//! Bounded work across slices, preserving the original balanced Boolean tree.
use super::{SliceRemoval, SliceSweep, prepare_vbit_slice, variable_capsule_bounds};
use crate::{
    geometry::{Diagnostic, Grid, Result, union::UnionAccumulator},
    model::VBit,
    motion::Motion,
};
use std::sync::atomic::{AtomicUsize, Ordering};

// A full subtree contains 64 batches of eight footprints. Each convex capsule
// has <= 2048 vertices, so the accumulator's vertex limit cannot split a batch.
const BLOCK_SWEEPS: usize = 512;
const MAX_WORKERS: usize = 8;

struct Bounds {
    lower: UnionAccumulator,
    upper: UnionAccumulator,
    error: f64,
    hull_time: std::time::Duration,
    union_time: std::time::Duration,
}
impl Bounds {
    fn new(grid: Grid) -> Self {
        Self {
            lower: UnionAccumulator::new(grid),
            upper: UnionAccumulator::new(grid),
            error: 0.,
            hull_time: std::time::Duration::ZERO,
            union_time: std::time::Duration::ZERO,
        }
    }
    fn append(&mut self, other: Self, timing: &crate::timing::Timer) -> Result<()> {
        let start = timing.start_sample();
        self.lower.append_aligned(other.lower)?;
        self.upper.append_aligned(other.upper)?;
        self.error = self.error.max(other.error);
        self.hull_time += other.hull_time;
        self.union_time +=
            other.union_time + start.map_or(std::time::Duration::ZERO, |s| s.elapsed());
        Ok(())
    }
}

fn block(
    grid: Grid,
    sweeps: &[SliceSweep],
    order: &[usize],
    timing: &crate::timing::Timer,
) -> Result<Bounds> {
    let mut result = Bounds::new(grid);
    for &i in order {
        let s = &sweeps[i];
        let start = timing.start_sample();
        let c = variable_capsule_bounds(grid, s.a, s.ra, s.b, s.rb)?;
        result.hull_time += start.map_or(std::time::Duration::ZERO, |s| s.elapsed());
        let start = timing.start_sample();
        result.lower.push(c.lower)?;
        result.upper.push(c.upper)?;
        result.union_time += start.map_or(std::time::Duration::ZERO, |s| s.elapsed());
        result.error = result.error.max(c.radial_error_mm);
    }
    Ok(result)
}

fn append_blocks(
    accumulated: &mut Result<Bounds>,
    blocks: Vec<Result<Bounds>>,
    timing: &crate::timing::Timer,
) {
    for result in blocks {
        if let Ok(prior) = accumulated
            && let Err(error) = result.and_then(|b| prior.append(b, timing))
        {
            *accumulated = Err(error);
        }
    }
}

fn finish_slice(
    prepared: Result<(Vec<SliceSweep>, Vec<usize>)>,
    bounds: Result<Bounds>,
    depth: f64,
) -> Result<SliceRemoval> {
    let (sweeps, _) = prepared?;
    let bounds = bounds?;
    Ok(SliceRemoval {
        depth_mm: depth,
        lower: bounds.lower.finish()?,
        upper: bounds.upper.finish()?,
        contributing_motion_ids: sweeps.iter().map(|s| s.id).collect(),
        capsule_radial_error_mm: bounds.error,
    })
}

pub(crate) fn vbit_removal_at_slices(
    grid: Grid,
    motions: &[Motion],
    tool: &VBit,
    depths: &[f64],
) -> Result<Vec<SliceRemoval>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(MAX_WORKERS);
    removal_with_workers(grid, motions, tool, depths, workers)
}

fn removal_with_workers(
    grid: Grid,
    motions: &[Motion],
    tool: &VBit,
    depths: &[f64],
    workers: usize,
) -> Result<Vec<SliceRemoval>> {
    let mut output = Vec::with_capacity(depths.len());
    // Retain no more prepared motion copies than the former eight slice workers.
    for group in depths.chunks(MAX_WORKERS) {
        output.extend(group_with_workers(grid, motions, tool, group, workers)?);
    }
    Ok(output)
}

fn group_with_workers(
    grid: Grid,
    motions: &[Motion],
    tool: &VBit,
    depths: &[f64],
    workers: usize,
) -> Result<Vec<SliceRemoval>> {
    let mut timing = crate::timing::Timer::new("vbit stock slices");
    let prepared: Vec<_> = depths
        .iter()
        .map(|&d| prepare_vbit_slice(motions, tool, d))
        .collect();
    let longest = prepared
        .iter()
        .filter_map(|p| p.as_ref().ok())
        .map(|(_, order)| order.len())
        .max()
        .unwrap_or(0);
    // Interleave depths so completed subtree merges also have independent work.
    // Order within each depth remains exactly the sequential spatial order.
    let mut jobs = vec![];
    for start in (0..longest).step_by(BLOCK_SWEEPS) {
        for (slice, p) in prepared.iter().enumerate() {
            if p.as_ref().is_ok_and(|(_, order)| start < order.len()) {
                jobs.push((slice, start));
            }
        }
    }
    let workers = workers.clamp(1, MAX_WORKERS).min(jobs.len().max(1));
    let mut accumulated: Vec<Result<Bounds>> =
        depths.iter().map(|_| Ok(Bounds::new(grid))).collect();
    timing.lap("prepare sweeps");
    // Short waves bound completed-but-unmerged geometry, even if one block is
    // much slower than the others. A single worker budget covers every slice.
    for wave in jobs.chunks(workers * 2) {
        let evaluate = |i: usize| {
            let (slice, start) = wave[i];
            let (sweeps, order) = prepared[slice].as_ref().unwrap();
            (
                i,
                block(
                    grid,
                    sweeps,
                    &order[start..(start + BLOCK_SWEEPS).min(order.len())],
                    &timing,
                ),
            )
        };
        let mut results = if workers == 1 {
            (0..wave.len()).map(evaluate).collect::<Vec<_>>()
        } else {
            let next = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..workers.min(wave.len()))
                    .map(|_| {
                        let next = &next;
                        let evaluate = &evaluate;
                        scope.spawn(move || {
                            let mut results = vec![];
                            loop {
                                let i = next.fetch_add(1, Ordering::Relaxed);
                                if i >= wave.len() {
                                    break;
                                }
                                results.push(evaluate(i));
                            }
                            results
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().map_err(|_| {
                            Diagnostic::new("STOCK_WORKER_PANIC", "V-bit stock worker failed")
                                .at_stage("stock")
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })?
            .into_iter()
            .flatten()
            .collect()
        };
        results.sort_by_key(|(i, _)| *i);
        let mut per_slice: Vec<_> = depths.iter().map(|_| vec![]).collect();
        for (i, result) in results {
            per_slice[wave[i].0].push(result);
        }
        let mut updates = accumulated
            .iter_mut()
            .zip(per_slice)
            .filter(|(_, b)| !b.is_empty());
        loop {
            let updates: Vec<_> = updates.by_ref().take(workers).collect();
            if updates.is_empty() {
                break;
            }
            if updates.len() == 1 {
                for (a, b) in updates {
                    append_blocks(a, b, &timing);
                }
            } else {
                std::thread::scope(|scope| {
                    let handles: Vec<_> = updates
                        .into_iter()
                        .map(|(a, b)| {
                            let timing = &timing;
                            scope.spawn(move || append_blocks(a, b, timing))
                        })
                        .collect();
                    for handle in handles {
                        handle.join().map_err(|_| {
                            Diagnostic::new("STOCK_WORKER_PANIC", "V-bit stock merge worker failed")
                                .at_stage("stock")
                        })?;
                    }
                    Ok::<_, Diagnostic>(())
                })?;
            }
        }
    }
    timing.lap("union blocks");
    timing.accumulated(
        "capsule construction",
        accumulated
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|b| b.hull_time)
            .sum(),
    );
    timing.accumulated(
        "polygon unions",
        accumulated
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|b| b.union_time)
            .sum(),
    );
    let finish = |((p, bounds), &depth)| finish_slice(p, bounds, depth);
    let mut slices = prepared.into_iter().zip(accumulated).zip(depths);
    let mut output = vec![];
    loop {
        let wave: Vec<_> = slices.by_ref().take(workers).collect();
        if wave.is_empty() {
            break;
        }
        if wave.len() == 1 {
            output.extend(wave.into_iter().map(finish));
        } else {
            output.extend(std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .into_iter()
                    .map(|s| scope.spawn(move || finish(s)))
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().map_err(|_| {
                            Diagnostic::new(
                                "STOCK_WORKER_PANIC",
                                "V-bit stock finalization worker failed",
                            )
                            .at_stage("stock")
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })?);
        }
    }
    output.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Point,
        model::VBitSpec,
        motion::{MotionKind, Position},
    };

    fn tool(tip: f64) -> VBit {
        VBit::try_from(VBitSpec {
            included_angle_deg: 90.,
            tip_diameter_mm: tip,
            max_cutting_diameter_mm: 12.,
            cutting_height_mm: 5.,
        })
        .unwrap()
    }
    fn motions() -> Vec<Motion> {
        let mut result = vec![];
        // Intersecting annular tracks leave holes and isolated components;
        // varying depths exercise clipped endpoints, apexes and finite tips.
        for cx in [10., 16., 40.] {
            let position = |i: usize| {
                let t = i as f64 * std::f64::consts::TAU / 400.;
                Position::new(
                    Point::new(cx + 7. * t.cos(), 10. + 7. * t.sin()),
                    -1.2 - 0.2 * (3. * t).cos(),
                )
            };
            for i in 0..400 {
                result.push(Motion {
                    id: result.len(),
                    tool_id: "vbit".into(),
                    operation_id: "stock-test".into(),
                    layer: 0,
                    kind: MotionKind::Cut,
                    start: position(i),
                    end: position(i + 1),
                    feed_mm_min: Some(100.),
                });
            }
        }
        let mut duplicate = result[1].clone();
        duplicate.id = result.len();
        result.push(duplicate);
        let mut plunge = result[0].clone();
        plunge.id = result.len();
        plunge.kind = MotionKind::Plunge;
        plunge.end = plunge.start;
        plunge.start.z = 0.;
        result.push(plunge);
        result
    }

    // The former sequential algorithm: construct every retained footprint and
    // push into one accumulator, with no subtree append or worker scheduler.
    fn reference(grid: Grid, motions: &[Motion], tool: &VBit, depth: f64) -> Result<SliceRemoval> {
        let (sweeps, order) = prepare_vbit_slice(motions, tool, depth)?;
        let mut lower = UnionAccumulator::new(grid);
        let mut upper = UnionAccumulator::new(grid);
        let mut error: f64 = 0.;
        for i in order {
            let s = &sweeps[i];
            let c = variable_capsule_bounds(grid, s.a, s.ra, s.b, s.rb)?;
            lower.push(c.lower)?;
            upper.push(c.upper)?;
            error = error.max(c.radial_error_mm);
        }
        Ok(SliceRemoval {
            depth_mm: depth,
            lower: lower.finish()?,
            upper: upper.finish()?,
            contributing_motion_ids: sweeps.iter().map(|s| s.id).collect(),
            capsule_radial_error_mm: error,
        })
    }

    #[test]
    fn parallel_subtrees_match_sequential_polygons_ids_and_bounds_exactly() {
        let grid = Grid::new(0.001, 100.).unwrap();
        let motions = motions();
        for tip in [0., 0.4] {
            let tool = tool(tip);
            let depths = [0.1, 1.1, 1.4, 2.];
            let expected: Vec<_> = depths
                .iter()
                .map(|&d| reference(grid, &motions, &tool, d).unwrap())
                .collect();
            assert!(expected[0].lower.hole_count() > 0);
            assert!(expected[0].lower.component_count() > 1);
            for workers in [1, 2, 8] {
                let actual = removal_with_workers(grid, &motions, &tool, &depths, workers).unwrap();
                assert_eq!(
                    serde_json::to_value(&actual).unwrap(),
                    serde_json::to_value(&expected).unwrap()
                );
            }
        }
    }

    #[test]
    fn partial_blocks_and_more_than_eight_slices_keep_original_order() {
        let grid = Grid::new(0.005, 100.).unwrap();
        let motions = motions();
        let tool = tool(0.2);
        let depths = [1.35, 1.32, 1.38, 1.30, 1.34, 1.36, 1.39, 1.37, 1.4];
        let expected: Vec<_> = depths
            .iter()
            .map(|&d| reference(grid, &motions, &tool, d).unwrap())
            .collect();
        let actual = removal_with_workers(grid, &motions, &tool, &depths, 8).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert!(
            removal_with_workers(grid, &motions, &tool, &[], 8)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn parallel_errors_follow_slice_order_even_when_preparation_fails_later() {
        let grid = Grid::new(1e-6, 100.).unwrap();
        let motions = motions();
        let tool = tool(0.4);
        // The earlier slice fails during capsule construction. A later negative
        // depth fails immediately during preparation but must not hide it.
        for (depths, code) in [
            ([0.1, -1.], "STOCK_RESOURCE_LIMIT"),
            ([-1., 0.1], "STOCK_DEPTH"),
        ] {
            for workers in [1, 8] {
                let actual =
                    removal_with_workers(grid, &motions, &tool, &depths, workers).unwrap_err();
                let expected = depths
                    .iter()
                    .map(|&d| reference(grid, &motions, &tool, d))
                    .collect::<Result<Vec<_>>>()
                    .unwrap_err();
                assert_eq!(actual.code, code);
                assert_eq!(actual.code, expected.code);
                assert_eq!(actual.message, expected.message);
            }
        }
    }
}
