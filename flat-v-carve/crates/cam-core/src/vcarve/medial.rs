use super::{
    Candidate, PathFamily,
    settings::{Context, error},
};
use crate::{
    geometry::{Curve, Point, PointLocation, Result, Segment, Site, SiteKind, VoronoiDiagram},
    motion::Position,
};
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct MedialBranch {
    pub id: usize,
    pub sites: [Site; 2],
    pub curve: Curve,
    pub split_parameters: Vec<f64>,
    pub points: Vec<Position>,
    pub clearance_radii_mm: Vec<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct MedialAxis {
    pub branches: Vec<MedialBranch>,
    pub excluded_edges: usize,
    pub curved_branches: usize,
}

fn roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    let epsilon = 64. * f64::EPSILON * (a.abs() + b.abs() + c.abs()).max(1.);
    if a.abs() <= epsilon {
        return if b.abs() > epsilon {
            vec![-c / b]
        } else {
            vec![]
        };
    }
    let d = b * b - 4. * a * c;
    if d < 0. {
        return vec![];
    }
    let q = -0.5 * (b + d.sqrt().copysign(b));
    if q == 0. {
        vec![-b / (2. * a)]
    } else {
        vec![q / a, c / q]
    }
}
fn threshold_roots(
    curve: &Curve,
    sites: [Site; 2],
    sources: &[Segment],
    radius: f64,
) -> Result<Vec<f64>> {
    let a = curve.evaluate(0.)?;
    let b = curve.evaluate(0.5)?;
    let c = curve.evaluate(1.)?;
    if let Some(site) = sites.iter().find(|s| s.kind == SiteKind::Segment) {
        let edge = sources[site.segment];
        let dx = edge.end.x - edge.start.x;
        let dy = edge.end.y - edge.start.y;
        let l = dx.hypot(dy);
        let f = |p: Point| ((p.x - edge.start.x) * dy - (p.y - edge.start.y) * dx) / l;
        let (f0, fm, f1) = (f(a), f(b), f(c));
        let aa = 2. * (f0 + f1 - 2. * fm);
        let bb = 4. * fm - 3. * f0 - f1;
        let mut all = roots(aa, bb, f0 - radius);
        all.extend(roots(aa, bb, f0 + radius));
        Ok(all)
    } else {
        let site = sites[0];
        let edge = sources[site.segment];
        let p = if site.kind == SiteKind::SegmentStart {
            edge.start
        } else {
            edge.end
        };
        let dx = c.x - a.x;
        let dy = c.y - a.y;
        let qx = a.x - p.x;
        let qy = a.y - p.y;
        Ok(roots(
            dx * dx + dy * dy,
            2. * (qx * dx + qy * dy),
            qx * qx + qy * qy - radius * radius,
        ))
    }
}
fn closest(site: Site, p: Point, sources: &[Segment]) -> Point {
    let e = sources[site.segment];
    match site.kind {
        SiteKind::SegmentStart => e.start,
        SiteKind::SegmentEnd => e.end,
        SiteKind::Segment => {
            let dx = e.end.x - e.start.x;
            let dy = e.end.y - e.start.y;
            let t = ((p.x - e.start.x) * dx + (p.y - e.start.y) * dy) / (dx * dx + dy * dy);
            e.start.lerp(e.end, t.clamp(0., 1.))
        }
    }
}
fn append_interval(
    ctx: &Context,
    curve: &Curve,
    a: f64,
    b: f64,
    points: &mut Vec<Position>,
    budget: &mut usize,
    level: usize,
) -> Result<()> {
    let qa = curve.evaluate(a)?;
    let qb = curve.evaluate(b)?;
    let pa = Position::new(qa, -ctx.safe_depth(qa)?);
    let pb = Position::new(qb, -ctx.safe_depth(qb)?);
    let r = |p: Position| ctx.tool.tip_radius().mm() + p.depth() * ctx.tool.angle().slope();
    let margin = ctx.target.boundary().variable_radius_margin_mm(
        Segment { start: qa, end: qb },
        r(pa) + ctx.guard / 2.,
        r(pb) + ctx.guard / 2.,
    )?;
    let slope = ctx.tool.angle().slope();
    let accuracy_radius =
        |p: Position| (r(p) + ctx.guard - ctx.motion_tolerance * slope / 4.).max(0.);
    let accuracy_margin = ctx.target.boundary().variable_radius_margin_mm(
        Segment { start: qa, end: qb },
        accuracy_radius(pa),
        accuracy_radius(pb),
    )?;
    // Quadratic chord error on this interval, using the exact curve midpoint.
    let mid = curve.evaluate((a + b) / 2.)?;
    let chord = mid.distance(qa.lerp(qb, 0.5));
    let xy_budget = ctx.motion_tolerance * slope.min(1.) / 8.;
    if curve.reconstruction_bound_mm() >= xy_budget {
        return Err(error(
            "MEDIAL_PRECISION",
            "curve reconstruction cannot meet the requested XYZ motion tolerance",
        ));
    }
    // Endpoint depths reserve two geometry tolerances. Keep half that
    // clearance along the entire chord as well: otherwise a safe but nearly
    // tangent chord can leave its conservative polygon bracket outside the
    // nominal stock section. Subdivision retains the requested path accuracy
    // without relaxing either the motion or stock verification.
    if margin < 0. || accuracy_margin < 0. || chord + curve.reconstruction_bound_mm() > xy_budget {
        if level >= 32 || *budget < 2 {
            return Err(error(
                "MEDIAL_RESOURCE_LIMIT",
                "bounded medial subdivision exhausted its budget",
            ));
        }
        append_interval(ctx, curve, a, (a + b) / 2., points, budget, level + 1)?;
        append_interval(ctx, curve, (a + b) / 2., b, points, budget, level + 1)?;
    } else {
        if *budget == 0 {
            return Err(error(
                "MEDIAL_RESOURCE_LIMIT",
                "medial segment budget exhausted",
            ));
        }
        *budget -= 1;
        if points.is_empty() {
            points.push(pa);
        }
        points.push(pb);
    }
    Ok(())
}
pub(super) fn build(ctx: &Context) -> Result<(MedialAxis, Vec<Candidate>)> {
    let diagram = VoronoiDiagram::build(ctx.target.region())?;
    let mut branches = vec![];
    let mut paths = vec![];
    let mut excluded = 0;
    let mut budget = ctx.settings.max_curve_segments;
    for (id, edge) in diagram.edges.iter().enumerate() {
        let Some(curve) = &edge.curve else {
            excluded += 1;
            continue;
        };
        let mid = curve.evaluate(0.5)?;
        let q = ctx.target.boundary().sample(mid)?;
        if !edge.primary
            || q.location != PointLocation::Inside
            || closest(edge.sites[0], mid, &diagram.source_segments).distance(closest(
                edge.sites[1],
                mid,
                &diagram.source_segments,
            )) <= ctx.target.region().grid().snap_bound_mm()
        {
            excluded += 1;
            continue;
        }
        if edge.sites.iter().any(|s| {
            (s.distance_mm(mid, &diagram.source_segments) - q.distance_mm).abs() > ctx.guard
        }) {
            return Err(error(
                "MEDIAL_ASSOCIATION",
                "interior branch disagrees with independent boundary clearance",
            ));
        }
        let low = ctx.tool.tip_radius().mm() + ctx.guard;
        let high = low + ctx.target.depth_cap().mm() * ctx.tool.angle().slope();
        let mut splits = vec![0., 1.];
        for r in [low, high] {
            splits.extend(
                threshold_roots(curve, edge.sites, &diagram.source_segments, r)?
                    .into_iter()
                    .filter(|t| *t > 0. && *t < 1.),
            );
        }
        splits.sort_by(f64::total_cmp);
        splits.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let mut all = vec![];
        for pair in splits.windows(2) {
            let t = (pair[0] + pair[1]) / 2.;
            let depth = ctx.safe_depth(curve.evaluate(t)?)?;
            if depth <= 0. {
                continue;
            }
            // Deep branches are redundant with the full-depth wall and floor families.
            let middle = ctx.target.boundary().sample(curve.evaluate(t)?)?;
            if middle.distance_mm > high + middle.numerical_reserve_mm {
                continue;
            }
            let mut points = vec![];
            append_interval(ctx, curve, pair[0], pair[1], &mut points, &mut budget, 0)?;
            all.extend(points.iter().copied());
            paths.push(Candidate {
                family: PathFamily::Medial,
                points,
                source_branch: Some(id),
            });
        }
        if !all.is_empty() {
            let radii = all
                .iter()
                .map(|p| ctx.target.boundary().sample(p.xy()).map(|q| q.distance_mm))
                .collect::<Result<Vec<_>>>()?;
            branches.push(MedialBranch {
                id,
                sites: edge.sites,
                curve: curve.clone(),
                split_parameters: splits,
                points: all,
                clearance_radii_mm: radii,
            });
        }
    }
    let curved_branches = branches.iter().filter(|b| b.curve.is_curved()).count();
    Ok((
        MedialAxis {
            branches,
            excluded_edges: excluded,
            curved_branches,
        },
        paths,
    ))
}
