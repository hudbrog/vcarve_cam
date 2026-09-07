//! Reconstruct long, exactly connected constant-radius stretches as round
//! strokes. Every point is an actual clipped sweep endpoint; no shortcut is
//! introduced through a gap or varying cutter radius at the inspected slice.
use super::{CapsuleBounds, SliceSweep, variable_capsule_bounds};
use crate::geometry::{
    Diagnostic, Grid, Point, Region, Result,
    spatial::{Aabb, SpatialIndex},
    union::UnionAccumulator,
};

struct Footprint {
    indices: Vec<usize>,
    bounds: Aabb,
}

fn footprints(grid: Grid, sweeps: &[SliceSweep], order: &[usize]) -> Option<Vec<Footprint>> {
    // Keep the larger reserve for stroke intersections below the existing
    // geometry budget. Coarser grids retain individual capsule reconstruction.
    if order.len() < 256 || grid.snap_bound_mm() > grid.tolerance_mm() / 64. {
        return None;
    }
    let mut indices = order.to_vec();
    indices.sort_unstable();
    let eligible = |s: &SliceSweep| {
        s.ra == s.rb
            && s.ra >= 2. * grid.tolerance_mm()
            && s.a != s.b
            && (std::f64::consts::PI * (2. * s.ra / grid.tolerance_mm()).sqrt()).ceil() <= 1024.
    };
    let mut groups: Vec<Vec<usize>> = vec![];
    for i in indices {
        let s = &sweeps[i];
        if let Some(group) = groups.last_mut() {
            let p = &sweeps[*group.last().unwrap()];
            let sides = (std::f64::consts::PI * (2. * s.ra / grid.tolerance_mm()).sqrt())
                .ceil()
                .max(16.);
            if group.len() < 64
                && (group.len() + 1) as f64 * sides <= 4096.
                && eligible(p)
                && eligible(s)
                && p.rb == s.ra
                && p.b == s.a
            {
                group.push(i);
                continue;
            }
        }
        groups.push(vec![i]);
    }
    if groups
        .iter()
        .filter(|g| g.len() >= 16)
        .map(Vec::len)
        .sum::<usize>()
        < 128
    {
        return None;
    }
    let mut output = vec![];
    let mut push = |indices: Vec<usize>| {
        let bounds = indices
            .iter()
            .map(|&i| Aabb::new(sweeps[i].a, sweeps[i].b))
            .reduce(Aabb::union)
            .unwrap();
        output.push(Footprint { indices, bounds });
    };
    for group in groups {
        if group.len() >= 16 {
            push(group);
        } else {
            for i in group {
                push(vec![i]);
            }
        }
    }
    Some(output)
}

fn stroke(grid: Grid, sweeps: &[SliceSweep], indices: &[usize]) -> Result<CapsuleBounds> {
    let s = &sweeps[indices[0]];
    if indices.len() == 1 {
        return variable_capsule_bounds(grid, s.a, s.ra, s.b, s.rb);
    }
    let mut points: Vec<Point> = vec![s.a];
    points.extend(indices.iter().map(|&i| sweeps[i].b));
    let snap = grid.snap_bound_mm();
    let arc = grid.arc_tolerance_mm() / 2.;
    // Reserve one snap for the centerline, output rounding and intersection
    // rounding, plus the two checked normalization passes permitted by
    // Region::from_tree (a split and another union per pass). Eight snaps
    // exceed that seven-snap displacement. Round joins are inscribed chords
    // with sagitta at most `arc`; the upper stroke also reserves this sagitta.
    let reserve = 8. * snap;
    let lower = Region::stroke(grid, &points, s.ra - reserve, arc)?;
    let upper = Region::stroke(grid, &points, s.ra + reserve + 2. * arc, arc)?;
    Ok(CapsuleBounds {
        lower,
        upper,
        radial_error_mm: reserve + 7. * snap + 2. * arc,
    })
}

pub(super) fn bounds(
    grid: Grid,
    sweeps: &[SliceSweep],
    order: &[usize],
    workers: usize,
) -> Option<Result<CapsuleBounds>> {
    let parts = footprints(grid, sweeps, order)?;
    Some(reconstruct(grid, sweeps, parts, workers))
}

fn reconstruct(
    grid: Grid,
    sweeps: &[SliceSweep],
    parts: Vec<Footprint>,
    workers: usize,
) -> Result<CapsuleBounds> {
    let order = SpatialIndex::new(parts.iter().map(|p| p.bounds).collect()).into_spatial_order();
    let block = |indices: &[usize]| -> Result<CapsuleBounds> {
        let mut lower = UnionAccumulator::new(grid);
        let mut upper = UnionAccumulator::new(grid);
        let mut error: f64 = 0.;
        for &i in indices {
            let c = stroke(grid, sweeps, &parts[i].indices)?;
            lower.push(c.lower)?;
            upper.push(c.upper)?;
            error = error.max(c.radial_error_mm);
        }
        Ok(CapsuleBounds {
            lower: lower.finish()?,
            upper: upper.finish()?,
            radial_error_mm: error,
        })
    };
    // Deterministic partitioning, with bounded worker and geometry counts.
    let blocks: Vec<_> = order.chunks(8).collect();
    let mut lower = UnionAccumulator::new(grid);
    let mut upper = UnionAccumulator::new(grid);
    let mut error: f64 = 0.;
    let workers = workers.clamp(1, 8);
    for wave in blocks.chunks(workers * 16) {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut results = if workers == 1 || wave.len() == 1 {
            wave.iter()
                .enumerate()
                .map(|(i, indices)| (i, block(indices)))
                .collect::<Vec<_>>()
        } else {
            std::thread::scope(|scope| {
                let next = &next;
                let handles: Vec<_> = (0..workers.min(wave.len()))
                    .map(|_| {
                        let block = &block;
                        scope.spawn(move || {
                            let mut results = vec![];
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some(indices) = wave.get(i) else {
                                    break;
                                };
                                results.push((i, block(indices)));
                            }
                            results
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().map_err(|_| {
                            Diagnostic::new("STOCK_WORKER_PANIC", "stroke worker failed")
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })?
            .into_iter()
            .flatten()
            .collect()
        };
        results.sort_by_key(|(i, _)| *i);
        for (_, c) in results {
            let c = c?;
            lower.push(c.lower)?;
            upper.push(c.upper)?;
            error = error.max(c.radial_error_mm);
        }
    }
    let (lower, upper) = if workers > 1 {
        std::thread::scope(|scope| {
            let lower = scope.spawn(|| lower.finish());
            let upper = upper.finish();
            Ok((
                lower.join().map_err(|_| {
                    Diagnostic::new("STOCK_WORKER_PANIC", "stroke union worker failed")
                })??,
                upper?,
            ))
        })?
    } else {
        (lower.finish()?, upper.finish()?)
    };
    Ok(CapsuleBounds {
        lower,
        upper,
        radial_error_mm: error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{BoundaryQuery, PointLocation, Segment};

    fn sweeps(points: &[Point], radius: f64) -> Vec<SliceSweep> {
        points
            .windows(2)
            .enumerate()
            .map(|(id, w)| SliceSweep {
                id,
                a: w[0],
                b: w[1],
                ra: radius,
                rb: radius,
            })
            .collect()
    }

    #[test]
    fn strokes_enclose_analytic_capsules_at_turns_crossings_holes_and_retraces() {
        let p = Point::new;
        let paths = [
            vec![p(0., 0.), p(10., 0.), p(0., 0.), p(0., 10.)],
            vec![p(0., 0.), p(10., 0.), p(10., 10.), p(0., 10.), p(0., 0.)],
            vec![p(0., 0.), p(10., 10.), p(0., 10.), p(10., 0.)],
            vec![p(0., 0.), p(10., 0.), p(0.01, 0.001), p(10., 0.002)],
            vec![p(0., 0.), p(1e-7, 1e-7), p(2e-7, 0.)],
        ];
        for grid in [
            Grid::new(0.005, 100.).unwrap(),
            Grid::with_scale(0.005, 100., 160000.).unwrap(),
        ] {
            for radius in [0.01, 0.05, 1.5] {
                for path in &paths {
                    let sweeps = sweeps(path, radius);
                    let indices: Vec<_> = (0..sweeps.len()).collect();
                    let bounds = stroke(grid, &sweeps, &indices).unwrap();
                    let lower = BoundaryQuery::new(&bounds.lower);
                    let upper = BoundaryQuery::new(&bounds.upper);
                    let distance = |p| {
                        sweeps
                            .iter()
                            .map(|s| {
                                Segment {
                                    start: s.a,
                                    end: s.b,
                                }
                                .distance(p)
                            })
                            .fold(f64::INFINITY, f64::min)
                    };
                    // Every reported lower edge is within the true swept disks;
                    // every reported upper boundary stays outside that envelope.
                    for (region, inner) in [(&bounds.lower, true), (&bounds.upper, false)] {
                        for edge in region.segments() {
                            for t in [0., 0.25, 0.5, 0.75, 1.] {
                                let p = edge.start.lerp(edge.end, t);
                                let d = distance(p);
                                if inner {
                                    assert!(d <= radius + 1e-10, "inner {p:?}: {d} > {radius}");
                                } else {
                                    assert!(d >= radius - 1e-10, "outer {p:?}: {d} < {radius}");
                                }
                                assert!(
                                    (d - radius).abs() <= bounds.radial_error_mm + 1e-10,
                                    "unreported radial error: {d}, {radius}"
                                );
                            }
                        }
                    }
                    for sweep in &sweeps {
                        for t in [0., 0.25, 0.5, 0.75, 1.] {
                            let center = sweep.a.lerp(sweep.b, t);
                            for angle in 0..128 {
                                let a = angle as f64 * std::f64::consts::TAU / 128.;
                                let at =
                                    p(center.x + radius * a.cos(), center.y + radius * a.sin());
                                assert_ne!(
                                    upper.location(at).unwrap(),
                                    PointLocation::Outside,
                                    "uncovered {at:?}"
                                );
                                let inside = radius - bounds.radial_error_mm;
                                let at =
                                    p(center.x + inside * a.cos(), center.y + inside * a.sin());
                                assert_ne!(
                                    lower.location(at).unwrap(),
                                    PointLocation::Outside,
                                    "missing inner reserve {at:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn chain_partition_preserves_gaps_radii_and_every_retained_sweep() {
        let grid = Grid::new(0.005, 100.).unwrap();
        let points: Vec<_> = (0..=300).map(|i| Point::new(i as f64 / 30., 0.)).collect();
        let mut sweeps = sweeps(&points, 0.5);
        sweeps[170].a.y = 0.1;
        sweeps[180].rb = 0.4;
        let order = super::super::sweep_order(&sweeps);
        let parts = footprints(grid, &sweeps, &order).unwrap();
        let mut retained: Vec<usize> = vec![];
        for p in parts {
            retained.extend(&p.indices);
            for pair in p.indices.windows(2) {
                let a = &sweeps[pair[0]];
                let b = &sweeps[pair[1]];
                assert_eq!(a.b, b.a);
                assert_eq!(a.ra, a.rb);
                assert_eq!(a.rb, b.ra);
                assert_eq!(b.ra, b.rb);
            }
        }
        retained.sort_unstable();
        assert_eq!(retained, (0..sweeps.len()).collect::<Vec<_>>());
        let serial = bounds(grid, &sweeps, &order, 1).unwrap().unwrap();
        for workers in [2, 8] {
            let parallel = bounds(grid, &sweeps, &order, workers).unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(&serial.lower).unwrap(),
                serde_json::to_value(parallel.lower).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&serial.upper).unwrap(),
                serde_json::to_value(parallel.upper).unwrap()
            );
        }
        assert!(footprints(Grid::new(0.001, 100.).unwrap(), &sweeps, &order).is_none());
        for s in &mut sweeps {
            s.ra = 0.;
            s.rb = 0.001;
        }
        assert!(footprints(grid, &sweeps, &order).is_none());
    }
}
