//! Contour floor cleanup restricted to stock left by recorded endmill sweeps.
//! The final boundary/medial families still own wall and rising-detail finish.
use super::{Candidate, Context, PathFamily, error, lanes};
use crate::{
    geometry::{BooleanOp, Region, Result},
    motion::Position,
};

pub(super) fn floor_paths(
    ctx: &Context,
    centers: &Region,
    needed: &Region,
    spacing: f64,
) -> Result<Vec<Candidate>> {
    let workers = if centers
        .rings()
        .iter()
        .map(|r| r.points().len())
        .sum::<usize>()
        >= 4096
    {
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(4)
    } else {
        1
    };
    floor_paths_with_workers(ctx, centers, needed, spacing, workers)
}

fn floor_paths_with_workers(
    ctx: &Context,
    centers: &Region,
    needed: &Region,
    spacing: f64,
    workers: usize,
) -> Result<Vec<Candidate>> {
    // A cutter centered just inside the cleared region can still be needed to
    // finish its edge. Keep a full allowed-ridge footprint around residual
    // stock, plus the planning guard; never simply clip tip centers to stock.
    let support = needed
        .dilate(ctx.tool.tip_radius().mm() + ctx.ridge * ctx.tool.angle().slope() + ctx.guard)?;
    let mut paths = vec![];
    let mut add = |polylines: &[Vec<crate::geometry::Point>]| -> Result<()> {
        for clipped in
            support.clip_polylines(&polylines.iter().map(Vec::as_slice).collect::<Vec<_>>())?
        {
            if clipped.len() > ctx.settings.max_curve_segments + 1
                || paths.len() >= ctx.settings.max_paths
            {
                return Err(error(
                    "VBIT_PATH_LIMIT",
                    "rest contours exceed the path/segment budget",
                ));
            }
            paths.push(Candidate {
                family: PathFamily::Floor,
                points: clipped
                    .into_iter()
                    .map(|p| Position::new(p, -ctx.target.depth_cap().mm()))
                    .collect(),
                source_branch: None,
            });
        }
        Ok(())
    };
    let mut current = centers.clone();
    let mut offsets = std::collections::VecDeque::new();
    for level in 0..ctx.settings.max_paths {
        if current
            .boolean(BooleanOp::Intersection, &support)?
            .rings()
            .is_empty()
        {
            return Ok(paths);
        }
        // The zero-offset contour is already the mandatory boundary family.
        if level > 0 {
            let mut rings = current.rings_mm();
            for ring in &mut rings {
                ring.push(ring[0]);
            }
            add(&rings)?;
        }
        // Offset the original region, avoiding accumulated offset error.
        if offsets.is_empty() {
            let count = workers.min(ctx.settings.max_paths - level);
            offsets = if count == 1 {
                [centers.erode((level + 1) as f64 * spacing)].into()
            } else {
                std::thread::scope(|scope| {
                    let handles: Vec<_> = (1..=count)
                        .map(|i| scope.spawn(move || centers.erode((level + i) as f64 * spacing)))
                        .collect();
                    handles
                        .into_iter()
                        .map(|h| {
                            h.join().unwrap_or_else(|_| {
                                Err(error("OFFSET_WORKER_PANIC", "floor offset worker failed"))
                            })
                        })
                        .collect()
                })
            };
        }
        let next = offsets.pop_front().unwrap()?;
        if next.rings().is_empty() {
            // The final thin core may have no positive-area next offset. A
            // small local raster covers it; deep medial edges are deliberately
            // excluded from the separate wall-detail family.
            let lanes = lanes(&current, spacing / 2., ctx.settings.max_paths)?;
            add(&lanes
                .iter()
                .map(|lane| lane.points.iter().map(|p| p.xy()).collect())
                .collect::<Vec<_>>())?;
            return Ok(paths);
        }
        current = next;
    }
    Err(error(
        "VBIT_PATH_LIMIT",
        "rest-contour offset budget exhausted",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Grid, Point};

    #[test]
    fn batched_clipping_preserves_independent_paths_holes_and_crossings() {
        let rectangle = |a, b, c, d| {
            vec![
                Point::new(a, b),
                Point::new(c, b),
                Point::new(c, d),
                Point::new(a, d),
            ]
        };
        let support = Region::from_rings(
            Grid::new(0.001, 30.).unwrap(),
            &[rectangle(0., 0., 20., 20.), rectangle(5., 5., 10., 10.)],
        )
        .unwrap();
        let subjects = [
            vec![Point::new(-1., 2.), Point::new(25., 2.)],
            vec![Point::new(-1., 8.), Point::new(25., 8.)],
            vec![Point::new(12., -1.), Point::new(12., 25.)],
            vec![
                Point::new(1., 1.),
                Point::new(4., 1.),
                Point::new(4., 15.),
                Point::new(1., 15.),
                Point::new(1., 1.),
            ],
        ];
        let mut separate = subjects
            .iter()
            .flat_map(|p| support.clip_polyline(p).unwrap())
            .map(|p| serde_json::to_string(&p).unwrap())
            .collect::<Vec<_>>();
        let mut batched = support
            .clip_polylines(&subjects.iter().map(Vec::as_slice).collect::<Vec<_>>())
            .unwrap()
            .iter()
            .map(|p| serde_json::to_string(p).unwrap())
            .collect::<Vec<_>>();
        separate.sort();
        batched.sort();
        assert_eq!(
            batched, separate,
            "intersecting open subjects must not acquire bridges or lose hole splits"
        );
    }

    #[test]
    fn prefetched_offsets_preserve_contours_and_resource_errors() {
        let job =
            crate::job::Job::from_json(include_str!("../../../../fixtures/m4/finite-tip.json"))
                .unwrap();
        let mut ctx = Context::new(&job).unwrap();
        let p = Point::new;
        let centers = Region::from_rings(
            ctx.target.region().grid(),
            &[vec![p(5., 5.), p(8., 5.), p(8., 7.), p(5., 7.)]],
        )
        .unwrap();
        let serial = floor_paths_with_workers(&ctx, &centers, &centers, 0.1, 1).unwrap();
        let parallel = floor_paths_with_workers(&ctx, &centers, &centers, 0.1, 4).unwrap();
        assert_eq!(
            serde_json::to_value(parallel).unwrap(),
            serde_json::to_value(serial).unwrap()
        );
        ctx.settings.max_paths = 2;
        assert_eq!(
            floor_paths_with_workers(&ctx, &centers, &centers, 0.1, 1).unwrap_err(),
            floor_paths_with_workers(&ctx, &centers, &centers, 0.1, 4).unwrap_err()
        );
    }
}
