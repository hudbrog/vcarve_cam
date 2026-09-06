//! Stock derived only from recorded cutting motions. Slice polygons supplement exact point queries.
use crate::geometry::spatial::{Aabb, SpatialIndex};
use crate::{
    geometry::{Diagnostic, Grid, Point, Region, Result, union::UnionAccumulator},
    motion::Motion,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Exact duplicate footprints (often depth pass + final finish). This only
/// avoids repeated polygon construction/union; every motion ID is retained.
/// Bound memory independently of the caller's motion budget.
#[derive(Default)]
struct Footprints(HashSet<[u64; 6]>);
impl Footprints {
    fn repeated(&mut self, a: Point, ra: f64, b: Point, rb: f64) -> bool {
        let a = [a.x.to_bits(), a.y.to_bits(), ra.to_bits()];
        let b = [b.x.to_bits(), b.y.to_bits(), rb.to_bits()];
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let key = [a[0], a[1], a[2], b[0], b[1], b[2]];
        // Sweeps arrive in spatial order, so an old full cache is unlikely to
        // help in the next neighborhood. Clearing can only miss duplicates.
        if self.0.len() >= 65536 {
            self.0.clear();
        }
        !self.0.insert(key)
    }
}

struct SliceSweep {
    id: usize,
    a: Point,
    b: Point,
    ra: f64,
    rb: f64,
}
fn sweep_order(sweeps: &[SliceSweep]) -> Vec<usize> {
    SpatialIndex::new(sweeps.iter().map(|s| Aabb::new(s.a, s.b)).collect()).into_spatial_order()
}

fn moving_endpoint_radii(sweeps: &[SliceSweep]) -> HashMap<(u64, u64), f64> {
    let mut endpoints = HashMap::<_, f64>::new();
    for s in sweeps.iter().filter(|s| s.a != s.b) {
        for (p, r) in [(s.a, s.ra), (s.b, s.rb)] {
            let key = (p.x.to_bits(), p.y.to_bits());
            if let Some(prior) = endpoints.get_mut(&key) {
                *prior = prior.max(r);
            } else if endpoints.len() < 131072 {
                endpoints.insert(key, r);
            }
        }
    }
    endpoints
}

fn covered_stationary(s: &SliceSweep, endpoints: &HashMap<(u64, u64), f64>) -> bool {
    // A plunge's slice is a disk. A nonstationary sweep with an equally large
    // endpoint disk at exactly the same center already contains all of it.
    // Its conservative outer bound therefore also covers the omitted disk;
    // retaining fewer inner polygons remains a conservative lower bound.
    s.a == s.b
        && endpoints
            .get(&(s.a.x.to_bits(), s.a.y.to_bits()))
            .is_some_and(|&r| r >= s.ra.max(s.rb))
}

enum QueryTool<'a> {
    Endmill(f64),
    Vbit(&'a crate::model::VBit),
}
/// Cached broad phase for repeated stock samples. Analytic motion-removal
/// formulas are shared with the independent full-scan public queries below.
pub(crate) struct StockQuery<'a> {
    motions: Vec<&'a Motion>,
    index: SpatialIndex,
    tool: QueryTool<'a>,
}
impl<'a> StockQuery<'a> {
    pub fn endmill(motions: &'a [Motion], radius: f64) -> Result<Self> {
        removed_depth_at(motions, radius, Point::new(0., 0.))?;
        Ok(Self::build(motions, QueryTool::Endmill(radius)))
    }
    pub fn vbit(motions: &'a [Motion], tool: &'a crate::model::VBit) -> Result<Self> {
        vbit_removed_depth_at(motions, tool, Point::new(0., 0.))?;
        Ok(Self::build(motions, QueryTool::Vbit(tool)))
    }
    fn build(motions: &'a [Motion], tool: QueryTool<'a>) -> Self {
        // Repeated passes add no removal. Keep exact forward XYZ duplicates
        // out of the query index, without modifying the recorded motions.
        // Do not canonicalize reversal: preserve the original floating-point
        // evaluation of each direction in the independent analytic formulas.
        let mut seen = HashSet::new();
        let motions: Vec<_> = motions
            .iter()
            .filter(|m| m.kind.cutting())
            .filter(|m| {
                let key = [
                    m.start.x.to_bits(),
                    m.start.y.to_bits(),
                    m.start.z.to_bits(),
                    m.end.x.to_bits(),
                    m.end.y.to_bits(),
                    m.end.z.to_bits(),
                ];
                if seen.len() < 131072 {
                    seen.insert(key)
                } else {
                    !seen.contains(&key)
                }
            })
            .collect();
        let index = SpatialIndex::new(
            motions
                .iter()
                .map(|m| {
                    let radius = match tool {
                        QueryTool::Endmill(r) => r,
                        QueryTool::Vbit(t) => {
                            t.tip_radius().mm()
                                + m.start.depth().max(m.end.depth()) * t.angle().slope()
                        }
                    };
                    let b = Aabb::new(m.start.xy(), m.end.xy());
                    let magnitude = b
                        .min
                        .x
                        .abs()
                        .max(b.max.x.abs())
                        .max(b.min.y.abs())
                        .max(b.max.y.abs());
                    let r = radius + 1024. * f64::EPSILON * (magnitude + radius + 1.);
                    Aabb::new(
                        Point::new(b.min.x - r, b.min.y - r),
                        Point::new(b.max.x + r, b.max.y + r),
                    )
                })
                .collect(),
        );
        Self {
            motions,
            index,
            tool,
        }
    }
    pub fn removed_depth(&self, p: Point) -> Result<f64> {
        if !p.finite() {
            return Err(Diagnostic::new(
                "STOCK_QUERY",
                "finite query point required",
            ));
        }
        let mut candidates = vec![];
        self.index.visit(Aabb::new(p, p), |i| {
            candidates.push(i);
            Ok(())
        })?;
        let motions = candidates.into_iter().map(|i| self.motions[i]);
        match self.tool {
            QueryTool::Endmill(r) => endmill_depth(motions, r, p),
            QueryTool::Vbit(t) => vbit_depth(motions, t, p),
        }
    }
}

fn validate_motions(motions: &[Motion]) -> Result<()> {
    if motions.iter().any(|m| !m.start.finite() || !m.end.finite()) {
        return Err(
            Diagnostic::new("STOCK_MOTION", "stock input contains a nonfinite motion")
                .at_stage("stock"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod indexed_query_tests {
    use super::*;
    use crate::{
        model::{VBit, VBitSpec},
        motion::{MotionKind, Position},
    };
    #[test]
    fn analytic_index_deduplicates_only_identical_forward_sweeps() {
        let motion = Motion {
            id: 0,
            tool_id: "test".into(),
            operation_id: "test".into(),
            layer: 0,
            kind: MotionKind::Cut,
            start: Position::new(Point::new(10.12345, 5.99876), -0.5),
            end: Position::new(Point::new(12.5689, 8.1101), -1.5),
            feed_mm_min: Some(100.),
        };
        let mut motions = vec![motion.clone(); 600];
        let mut reverse = motion.clone();
        std::mem::swap(&mut reverse.start, &mut reverse.end);
        motions.push(reverse);
        let mut deeper = motion;
        deeper.end.z = -2.;
        motions.push(deeper);
        for (id, motion) in motions.iter_mut().enumerate() {
            motion.id = id;
        }
        for tip in [0., 0.5] {
            let tool = VBit::try_from(VBitSpec {
                included_angle_deg: 90.,
                tip_diameter_mm: tip,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 5.,
            })
            .unwrap();
            let vbit = StockQuery::vbit(&motions, &tool).unwrap();
            let endmill = StockQuery::endmill(&motions, 1.5).unwrap();
            assert_eq!(vbit.motions.len(), 3);
            assert_eq!(endmill.motions.len(), 3);
            for i in 0..400 {
                let p = Point::new(9. + (i % 20) as f64 / 4., 5. + (i / 20) as f64 / 4.);
                assert_eq!(
                    vbit.removed_depth(p).unwrap(),
                    vbit_removed_depth_at(&motions, &tool, p).unwrap()
                );
                assert_eq!(
                    endmill.removed_depth(p).unwrap(),
                    removed_depth_at(&motions, 1.5, p).unwrap()
                );
            }
        }
        motions[0].start.x = f64::NAN;
        assert!(StockQuery::endmill(&motions, 1.5).is_err());
    }
    #[test]
    fn covered_plunge_disks_are_redundant_but_deeper_plunges_are_retained() {
        let grid = Grid::new(0.001, 100.).unwrap();
        let tool = VBit::try_from(VBitSpec {
            included_angle_deg: 90.,
            tip_diameter_mm: 0.5,
            max_cutting_diameter_mm: 12.,
            cutting_height_mm: 5.,
        })
        .unwrap();
        let cut = Motion {
            id: 20,
            tool_id: "vbit".into(),
            operation_id: "test".into(),
            layer: 0,
            kind: MotionKind::Cut,
            start: Position::new(Point::new(10., 10.), -1.),
            end: Position::new(Point::new(14., 10.), -1.5),
            feed_mm_min: Some(100.),
        };
        let mut plunge = cut.clone();
        plunge.id = 10;
        plunge.kind = MotionKind::Plunge;
        plunge.start.z = 0.;
        plunge.end = cut.start;
        let expected = vbit_removal_at_slice(grid, std::slice::from_ref(&cut), &tool, 0.5).unwrap();
        let actual =
            vbit_removal_at_slice(grid, &[plunge.clone(), cut.clone()], &tool, 0.5).unwrap();
        assert_eq!(actual.contributing_motion_ids, [10, 20]);
        assert_eq!(
            serde_json::to_value(actual.lower).unwrap(),
            serde_json::to_value(&expected.lower).unwrap()
        );
        assert_eq!(
            serde_json::to_value(actual.upper).unwrap(),
            serde_json::to_value(&expected.upper).unwrap()
        );
        plunge.end.z = -2.5;
        let deeper = vbit_removal_at_slice(grid, &[plunge, cut], &tool, 0.5).unwrap();
        assert!(deeper.lower.area_mm2() > expected.upper.area_mm2());
        assert_eq!(deeper.contributing_motion_ids, [10, 20]);
    }
    #[test]
    fn subgrid_apex_inner_sweeps_may_be_empty_but_outer_bounds_are_retained() {
        let grid = Grid::new(0.001, 100.).unwrap();
        let radius = 2.1 * grid.snap_bound_mm();
        let a = Point::new(10., 10.);
        for b in [a, Point::new(14., 10.)] {
            let bounds = variable_capsule_bounds(grid, a, radius, b, radius).unwrap();
            assert!(bounds.lower.rings().is_empty());
            let analytic_area =
                std::f64::consts::PI * radius * radius + 2. * radius * a.distance(b);
            assert!(bounds.upper.area_mm2() >= analytic_area);
            assert!(bounds.radial_error_mm >= radius);
            let query = crate::geometry::BoundaryQuery::new(&bounds.upper);
            for p in [a, a.lerp(b, 0.5), b] {
                assert_ne!(
                    query.sample(p).unwrap().location,
                    crate::geometry::PointLocation::Outside
                );
            }
        }
    }
    #[test]
    fn dense_short_sweeps_bracket_a_single_analytic_capsule() {
        let grid = Grid::new(0.001, 100.).unwrap();
        let motions: Vec<_> = (0..1000)
            .map(|i| Motion {
                id: i,
                tool_id: "test".into(),
                operation_id: "test".into(),
                layer: 0,
                kind: MotionKind::Cut,
                start: Position::new(Point::new(10. + i as f64 / 1000., 10.), -2.),
                end: Position::new(Point::new(10. + (i + 1) as f64 / 1000., 10.), -2.),
                feed_mm_min: Some(100.),
            })
            .collect();
        let removal = removal_at_slice(grid, &motions, 1.5, 2.).unwrap();
        let analytic_area = std::f64::consts::PI * 1.5_f64.powi(2) + 3.;
        assert!(removal.lower.area_mm2() <= analytic_area);
        assert!(removal.upper.area_mm2() >= analytic_area);
        assert!(removal.upper.area_mm2() - removal.lower.area_mm2() < 0.01);
        assert_eq!(removal.lower.component_count(), 1);
        assert_eq!(removal.lower.hole_count(), 0);
        assert_eq!(removal.contributing_motion_ids.len(), 1000);
    }
    #[test]
    fn duplicate_and_reversed_sweeps_keep_all_ids_and_the_same_bounds() {
        let grid = Grid::new(0.001, 100.).unwrap();
        let tool = VBit::try_from(VBitSpec {
            included_angle_deg: 90.,
            tip_diameter_mm: 0.5,
            max_cutting_diameter_mm: 12.,
            cutting_height_mm: 5.,
        })
        .unwrap();
        let motion = Motion {
            id: 0,
            tool_id: "test".into(),
            operation_id: "test".into(),
            layer: 0,
            kind: MotionKind::Cut,
            start: Position::new(Point::new(10., 10.), -1.),
            end: Position::new(Point::new(13., 11.), -2.),
            feed_mm_min: Some(100.),
        };
        let mut repeated = vec![motion.clone(); 600];
        for (i, m) in repeated.iter_mut().enumerate() {
            m.id = i;
            if i % 2 == 1 {
                std::mem::swap(&mut m.start, &mut m.end);
            }
        }
        for depth in [0.5, 1., 1.5, 2.] {
            for vbit in [false, true] {
                let compute = |motions: &[Motion]| {
                    if vbit {
                        vbit_removal_at_slice(grid, motions, &tool, depth)
                    } else {
                        removal_at_slice(grid, motions, 1.5, depth)
                    }
                    .unwrap()
                };
                let expected = compute(std::slice::from_ref(&motion));
                let actual = compute(&repeated);
                assert_eq!(actual.contributing_motion_ids, (0..600).collect::<Vec<_>>());
                assert_eq!(
                    serde_json::to_value(actual.lower).unwrap(),
                    serde_json::to_value(expected.lower).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(actual.upper).unwrap(),
                    serde_json::to_value(expected.upper).unwrap()
                );
                assert_eq!(
                    actual.capsule_radial_error_mm,
                    expected.capsule_radial_error_mm
                );
            }
        }
    }
    #[test]
    fn cached_queries_match_full_scans_at_flanks_endpoints_and_flat_tip_edges() {
        let motions: Vec<_> = (0..200)
            .map(|i| Motion {
                id: i,
                tool_id: "test".into(),
                operation_id: "test".into(),
                layer: 0,
                kind: if i % 2 == 0 {
                    MotionKind::Cut
                } else {
                    MotionKind::Plunge
                },
                start: Position::new(Point::new(i as f64 * 10., 10000.), -0.5),
                end: Position::new(
                    Point::new(i as f64 * 10. + if i % 2 == 0 { 3. } else { 0. }, 10000.),
                    -2.,
                ),
                feed_mm_min: Some(100.),
            })
            .collect();
        let endmill = StockQuery::endmill(&motions, 2.).unwrap();
        for tip in [0., 1.] {
            let tool = VBit::try_from(VBitSpec {
                included_angle_deg: 90.,
                tip_diameter_mm: tip,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 5.,
            })
            .unwrap();
            let vbit = StockQuery::vbit(&motions, &tool).unwrap();
            for i in 0..200 {
                for dx in [-2.500000001, -2.5, 0., 1.5, 3., 5., 5.500000001] {
                    for dy in [-2.5, -2., -0.5, 0., 0.5, 2., 2.5] {
                        let p = Point::new(i as f64 * 10. + dx, 10000. + dy);
                        assert!(
                            (endmill.removed_depth(p).unwrap()
                                - removed_depth_at(&motions, 2., p).unwrap())
                            .abs()
                                < 1e-12
                        );
                        assert!(
                            (vbit.removed_depth(p).unwrap()
                                - vbit_removed_depth_at(&motions, &tool, p).unwrap())
                            .abs()
                                < 1e-12
                        );
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SliceRemoval {
    pub depth_mm: f64,
    pub lower: Region,
    pub upper: Region,
    pub contributing_motion_ids: Vec<usize>,
    pub capsule_radial_error_mm: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CapsuleBounds {
    pub lower: Region,
    pub upper: Region,
    pub radial_error_mm: f64,
}

fn circle_directions(sides: usize) -> &'static [Point] {
    // Circle resolution is bounded to 1024 sides by both callers. Reuse the
    // exact same trigonometric values across footprints, tools, and slices.
    static CIRCLES: [std::sync::OnceLock<Vec<Point>>; 1025] =
        [const { std::sync::OnceLock::new() }; 1025];
    CIRCLES[sides].get_or_init(|| {
        (0..sides)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / sides as f64;
                Point::new(t.cos(), t.sin())
            })
            .collect()
    })
}

pub fn capsule_bounds(grid: Grid, a: Point, b: Point, radius: f64) -> Result<CapsuleBounds> {
    let snap = grid.snap_bound_mm();
    let tolerance = grid.tolerance_mm();
    if !radius.is_finite() || radius <= 4. * snap {
        return Err(Diagnostic::new(
            "STOCK_PRECISION",
            "tool radius is too small for bounded sweep polygons",
        )
        .at_stage("stock"));
    }
    let n = (std::f64::consts::PI * (2. * radius / tolerance).sqrt())
        .ceil()
        .max(16.);
    if n > 1024. {
        return Err(Diagnostic::new(
            "STOCK_RESOURCE_LIMIT",
            "capsule needs more than 1024 circle sides",
        )
        .at_stage("stock"));
    }
    let n = n as usize;
    let cosine = (std::f64::consts::PI / n as f64).cos();
    let inner = radius - 2. * snap;
    let outer = (radius + 2. * snap) / cosine;
    let hull = |r: f64| {
        let mut points = Vec::with_capacity(2 * n);
        for p in [a, b] {
            for direction in circle_directions(n) {
                points.push(Point::new(p.x + r * direction.x, p.y + r * direction.y));
            }
        }
        Region::convex_hull(grid, &points)
    };
    Ok(CapsuleBounds {
        lower: hull(inner)?,
        upper: hull(outer)?,
        radial_error_mm: (radius - inner * cosine + snap).max(outer + snap - radius),
    })
}

pub fn removal_at_slice(
    grid: Grid,
    motions: &[Motion],
    radius: f64,
    depth: f64,
) -> Result<SliceRemoval> {
    let _timing = crate::timing::Timer::new("endmill stock slice");
    validate_motions(motions)?;
    if !radius.is_finite() || radius <= 0. {
        return Err(
            Diagnostic::new("STOCK_PRECISION", "positive finite tool radius required")
                .at_stage("stock"),
        );
    }
    if !depth.is_finite() || depth < 0. {
        return Err(
            Diagnostic::new("STOCK_DEPTH", "slice depth must be finite and nonnegative")
                .at_stage("stock"),
        );
    }
    let mut lower = UnionAccumulator::new(grid);
    let mut upper = UnionAccumulator::new(grid);
    let mut error: f64 = 0.;
    let mut footprints = Footprints::default();
    let sweeps: Vec<_> = motions
        .iter()
        .filter_map(|m| {
            m.at_depth(depth).map(|(a, b)| SliceSweep {
                id: m.id,
                a,
                b,
                ra: radius,
                rb: radius,
            })
        })
        .collect();
    let endpoints = moving_endpoint_radii(&sweeps);
    // Merge neighbors in space, not widely separated depth/final-pass records.
    // This avoids repeatedly intersecting large overlapping stock prefixes.
    for i in sweep_order(&sweeps) {
        let s = &sweeps[i];
        if covered_stationary(s, &endpoints) || footprints.repeated(s.a, radius, s.b, radius) {
            continue;
        }
        let capsule = capsule_bounds(grid, s.a, s.b, radius)?;
        error = error.max(capsule.radial_error_mm);
        lower.push(capsule.lower)?;
        upper.push(capsule.upper)?;
    }
    Ok(SliceRemoval {
        depth_mm: depth,
        lower: lower.finish()?,
        upper: upper.finish()?,
        contributing_motion_ids: sweeps.iter().map(|s| s.id).collect(),
        capsule_radial_error_mm: error,
    })
}

/// Exact linear-motion/cylindrical-tool point query (up to floating-point arithmetic).
/// Solve the interval where XY lies within the disk, then maximize the linear depth.
pub fn removed_depth_at(motions: &[Motion], radius: f64, p: Point) -> Result<f64> {
    validate_motions(motions)?;
    endmill_depth(motions.iter().filter(|m| m.kind.cutting()), radius, p)
}
fn endmill_depth<'a>(
    motions: impl Iterator<Item = &'a Motion>,
    radius: f64,
    p: Point,
) -> Result<f64> {
    if !p.finite() || !radius.is_finite() || radius <= 0. {
        return Err(
            Diagnostic::new("STOCK_QUERY", "positive radius and finite point required")
                .at_stage("stock"),
        );
    }
    let mut removed: f64 = 0.;
    for m in motions {
        let a = m.start.xy();
        let b = m.end.xy();
        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let dx = a.x - p.x;
        let dy = a.y - p.y;
        let aa = vx * vx + vy * vy;
        if !aa.is_finite() || !(dx * dx + dy * dy + radius * radius).is_finite() {
            return Err(Diagnostic::new(
                "STOCK_QUERY_RANGE",
                "query arithmetic exceeds the supported range",
            )
            .at_stage("stock"));
        }
        if aa == 0. {
            if dx.hypot(dy) <= radius {
                removed = removed.max(m.start.depth()).max(m.end.depth());
            }
            continue;
        }
        let center = -(dx * vx + dy * vy) / aa;
        let closest = Point::new(dx + center * vx, dy + center * vy);
        let discriminant = radius * radius - closest.x * closest.x - closest.y * closest.y;
        if discriminant < 0. {
            continue;
        }
        let half = (discriminant / aa).sqrt();
        let lo = (center - half).max(0.);
        let hi = (center + half).min(1.);
        if lo <= hi {
            removed = removed
                .max(m.start.lerp(m.end, lo).depth())
                .max(m.start.lerp(m.end, hi).depth());
        }
    }
    Ok(removed)
}

/// Convex hull of two disks with different radii. For a tapered apex, move the
/// inner endpoint a bounded distance into the sweep before snapping its disk.
pub fn variable_capsule_bounds(
    grid: Grid,
    a: Point,
    ra: f64,
    b: Point,
    rb: f64,
) -> Result<CapsuleBounds> {
    if !ra.is_finite() || !rb.is_finite() || ra < 0. || rb < 0. {
        return Err(
            Diagnostic::new("STOCK_RADIUS", "finite nonnegative radii required").at_stage("stock"),
        );
    }
    let r = ra.max(rb);
    let snap = grid.snap_bound_mm();
    let n = (std::f64::consts::PI * (2. * r / grid.tolerance_mm()).sqrt())
        .ceil()
        .max(16.);
    if n > 1024. {
        return Err(Diagnostic::new(
            "STOCK_RESOURCE_LIMIT",
            "variable capsule requires more than 1024 circle sides",
        )
        .at_stage("stock"));
    }
    let n = n as usize;
    let cosine = (std::f64::consts::PI / n as f64).cos();
    let trim = |p: Point, radius: f64, q: Point, other: f64| {
        if radius <= 2. * snap && other > 4. * snap {
            let t = (4. * snap - radius) / (other - radius);
            (
                p.lerp(q, t),
                4. * snap,
                t * (p.distance(q) + (other - radius).abs()),
            )
        } else {
            (p, radius, 0.)
        }
    };
    let ta = trim(a, ra, b, rb);
    let tb = trim(b, rb, a, ra);
    let hull = |upper: bool| -> Result<Region> {
        let mut points = vec![];
        for (p, radius) in if upper {
            [(a, ra), (b, rb)]
        } else {
            [(ta.0, ta.1), (tb.0, tb.1)]
        } {
            if !upper && radius <= 2. * snap {
                continue;
            }
            let radius = if upper {
                (radius + 2. * snap) / cosine
            } else {
                radius - 2. * snap
            };
            for direction in circle_directions(n) {
                points.push(Point::new(
                    p.x + radius * direction.x,
                    p.y + radius * direction.y,
                ));
            }
        }
        if points.is_empty() {
            Region::from_rings(grid, &[])
        } else {
            match Region::convex_hull(grid, &points) {
                // A near-apex inner disk can disappear on the grid. Empty is
                // still a conservative lower bound; the outer sweep remains
                // mandatory and must construct successfully.
                Err(e) if !upper && e.code == "CONVEX_HULL_COLLAPSE" => {
                    Region::from_rings(grid, &[])
                }
                result => result,
            }
        }
    };
    let lower = hull(false)?;
    let upper = hull(true)?;
    let collapsed_reserve = if lower.rings().is_empty() { r } else { 0. };
    Ok(CapsuleBounds {
        lower,
        upper,
        radial_error_mm: (r - (r - 2. * snap).max(0.) * cosine + snap)
            .max((r + 2. * snap) / cosine + snap - r)
            .max(collapsed_reserve)
            + ta.2.max(tb.2),
    })
}

pub fn vbit_removal_at_slice(
    grid: Grid,
    motions: &[Motion],
    tool: &crate::model::VBit,
    depth: f64,
) -> Result<SliceRemoval> {
    let mut timing = crate::timing::Timer::new("vbit stock slice");
    validate_motions(motions)?;
    if motions.iter().filter(|m| m.kind.cutting()).any(|m| {
        m.start.depth() > tool.cutting_height().mm() || m.end.depth() > tool.cutting_height().mm()
    }) {
        return Err(Diagnostic::new(
            "STOCK_CUTTING_HEIGHT",
            "recorded depth exceeds the modeled V-bit cutting height",
        )
        .at_stage("stock"));
    }
    if !depth.is_finite() || depth < 0. {
        return Err(
            Diagnostic::new("STOCK_DEPTH", "finite nonnegative slice depth required")
                .at_stage("stock"),
        );
    }
    let mut lower = UnionAccumulator::new(grid);
    let mut upper = UnionAccumulator::new(grid);
    let mut error: f64 = 0.;
    let mut footprints = Footprints::default();
    let mut sweeps = vec![];
    for m in motions.iter().filter(|m| m.kind.cutting()) {
        let da = m.start.depth();
        let db = m.end.depth();
        if da < depth && db < depth {
            continue;
        }
        let (lo, hi) = if da >= depth && db >= depth {
            (0., 1.)
        } else if db > da {
            ((depth - da) / (db - da), 1.)
        } else {
            (0., (depth - da) / (db - da))
        };
        let a = m.start.lerp(m.end, lo);
        let b = m.start.lerp(m.end, hi);
        let radius = |d: f64| tool.tip_radius().mm() + (d - depth).max(0.) * tool.angle().slope();
        sweeps.push(SliceSweep {
            id: m.id,
            a: a.xy(),
            b: b.xy(),
            ra: radius(a.depth()),
            rb: radius(b.depth()),
        });
    }
    let order = sweep_order(&sweeps);
    let endpoints = moving_endpoint_radii(&sweeps);
    timing.lap("prepare sweeps");
    let mut hull_time = std::time::Duration::ZERO;
    let mut union_time = std::time::Duration::ZERO;
    for i in order {
        let s = &sweeps[i];
        if covered_stationary(s, &endpoints) || footprints.repeated(s.a, s.ra, s.b, s.rb) {
            continue;
        }
        let start = timing.start_sample();
        let c = variable_capsule_bounds(grid, s.a, s.ra, s.b, s.rb)?;
        hull_time += start.map_or(std::time::Duration::ZERO, |s| s.elapsed());
        let start = timing.start_sample();
        lower.push(c.lower)?;
        upper.push(c.upper)?;
        union_time += start.map_or(std::time::Duration::ZERO, |s| s.elapsed());
        error = error.max(c.radial_error_mm);
    }
    timing.accumulated("capsule construction", hull_time);
    timing.accumulated("polygon unions", union_time);
    timing.lap("accumulate footprints");
    Ok(SliceRemoval {
        depth_mm: depth,
        lower: lower.finish()?,
        upper: upper.finish()?,
        contributing_motion_ids: sweeps.iter().map(|s| s.id).collect(),
        capsule_radial_error_mm: error,
    })
}

/// Maximize the concave linear-depth-minus-cone-height function analytically.
/// Endpoint, flat-tip interval, closest-point, and flank stationary candidates
/// suffice; there is no sampling of the motion parameter.
pub fn vbit_removed_depth_at(
    motions: &[Motion],
    tool: &crate::model::VBit,
    p: Point,
) -> Result<f64> {
    validate_motions(motions)?;
    if motions.iter().filter(|m| m.kind.cutting()).any(|m| {
        m.start.depth() > tool.cutting_height().mm() || m.end.depth() > tool.cutting_height().mm()
    }) {
        return Err(Diagnostic::new(
            "STOCK_CUTTING_HEIGHT",
            "recorded depth exceeds the modeled V-bit cutting height",
        )
        .at_stage("stock"));
    }
    vbit_depth(motions.iter().filter(|m| m.kind.cutting()), tool, p)
}
fn vbit_depth<'a>(
    motions: impl Iterator<Item = &'a Motion>,
    tool: &crate::model::VBit,
    p: Point,
) -> Result<f64> {
    if !p.finite() {
        return Err(
            Diagnostic::new("STOCK_QUERY", "finite query point required").at_stage("stock"),
        );
    }
    let mut removed: f64 = 0.;
    let slope = tool.angle().slope();
    let tip = tool.tip_radius().mm();
    for m in motions {
        let a = m.start.xy();
        let b = m.end.xy();
        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let length = vx.hypot(vy);
        let evaluate = |t: f64| {
            let q = m.start.lerp(m.end, t);
            (q.depth() - ((q.xy().distance(p) - tip).max(0.) / slope)).max(0.)
        };
        removed = removed.max(evaluate(0.)).max(evaluate(1.));
        if length == 0. {
            continue;
        }
        let dx = a.x - p.x;
        let dy = a.y - p.y;
        let along = (dx * vx + dy * vy) / length;
        let perpendicular = (dx * vy - dy * vx).abs() / length;
        if !along.is_finite() || !perpendicular.is_finite() {
            return Err(Diagnostic::new(
                "STOCK_QUERY_RANGE",
                "point query exceeds supported arithmetic range",
            )
            .at_stage("stock"));
        }
        let mut candidates = [-along / length, f64::NAN, f64::NAN, f64::NAN];
        if tip >= perpendicular {
            let half = ((tip - perpendicular) * (tip + perpendicular)).sqrt();
            candidates[1] = (-along - half) / length;
            candidates[2] = (-along + half) / length;
        }
        let k = slope * (m.end.depth() - m.start.depth()) / length;
        if k.abs() < 1. {
            let u = k * perpendicular / (1. - k * k).sqrt();
            candidates[3] = (u - along) / length;
        }
        for t in candidates {
            if t > 0. && t < 1. {
                removed = removed.max(evaluate(t));
            }
        }
    }
    Ok(removed)
}

/// Conservative air proof against one recorded endmill sweep. Centers may be
/// in cleared space; this tests containment of the entire V-bit footprint.
pub fn vbit_air_in_endmill(
    motion: &Motion,
    vbit: &crate::model::VBit,
    endmill_moves: &[Motion],
    radius: f64,
    reserve: f64,
) -> bool {
    if !radius.is_finite()
        || radius <= 0.
        || !reserve.is_finite()
        || reserve < 0.
        || !motion.start.finite()
        || !motion.end.finite()
        || motion.start.depth().max(motion.end.depth()) > vbit.cutting_height().mm()
    {
        return false;
    }
    let r = vbit.tip_radius().mm()
        + motion.start.depth().max(motion.end.depth()) * vbit.angle().slope()
        + reserve;
    if r >= radius {
        return false;
    }
    endmill_moves
        .iter()
        .filter(|m| m.kind.cutting())
        .any(|prior| {
            let depth = if prior.start.xy() == prior.end.xy() {
                prior.start.depth().max(prior.end.depth())
            } else {
                prior.start.depth().min(prior.end.depth())
            };
            let segment = crate::geometry::Segment {
                start: prior.start.xy(),
                end: prior.end.xy(),
            };
            depth >= motion.start.depth().max(motion.end.depth())
                && [motion.start.xy(), motion.end.xy()]
                    .iter()
                    .all(|&p| segment.distance(p) + r <= radius)
        })
}
