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
    // A cutter centered just inside the cleared region can still be needed to
    // finish its edge. Keep a full allowed-ridge footprint around residual
    // stock, plus the planning guard; never simply clip tip centers to stock.
    let support = needed
        .dilate(ctx.tool.tip_radius().mm() + ctx.ridge * ctx.tool.angle().slope() + ctx.guard)?;
    let mut paths = vec![];
    let mut add = |points: &[crate::geometry::Point]| -> Result<()> {
        for clipped in support.clip_polyline(points)? {
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
            for mut ring in current.rings_mm() {
                ring.push(ring[0]);
                add(&ring)?;
            }
        }
        // Offset the original region, avoiding accumulated offset error.
        let next = centers.erode((level + 1) as f64 * spacing)?;
        if next.rings().is_empty() {
            // The final thin core may have no positive-area next offset. A
            // small local raster covers it; deep medial edges are deliberately
            // excluded from the separate wall-detail family.
            for lane in lanes(&current, spacing / 2., ctx.settings.max_paths)? {
                add(&lane.points.iter().map(|p| p.xy()).collect::<Vec<_>>())?;
            }
            return Ok(paths);
        }
        current = next;
    }
    Err(error(
        "VBIT_PATH_LIMIT",
        "rest-contour offset budget exhausted",
    ))
}
