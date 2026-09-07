//! Nominal target and tool access, independent of any path planning or stock simulation.
mod access;
mod reachability;

use crate::{
    geometry::{
        BoundaryQuery, Clearance, Diagnostic, Point, PointLocation, Region, Result, VoronoiDiagram,
    },
    model::{Depth, Endmill, IncludedAngle, Length, VBit},
};
pub use access::{CenterSet, CenterSetStatus, FitStatus, PoseFit};
pub use reachability::{Reachability, ReachabilityOptions, ReachabilityStatus};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Mutex, OnceLock},
};

pub struct Target {
    region: Region,
    boundary: BoundaryQuery,
    depth_cap: Depth,
    angle: IncludedAngle,
    diagram: OnceLock<Result<VoronoiDiagram>>,
    center_sets: Mutex<VecDeque<(f64, CenterSet, usize)>>,
    // First center-set query retains at most 131072 exact boundary samples.
    // Clearance is independent of cutter radius; subsequent tools and medial
    // paths can read this immutable cache without contending on a mutex.
    diagram_samples: OnceLock<HashMap<[u64; 2], Clearance>>,
    input_snap_bound_mm: f64,
}

impl Target {
    pub fn new(region: Region, depth_cap: Depth, angle: IncludedAngle) -> Result<Self> {
        let input_snap_bound_mm = region.grid().snap_bound_mm();
        if region.rings().is_empty() {
            return Err(error(
                "EMPTY_GEOMETRY",
                "target requires a nonempty removal region",
            ));
        }
        if depth_cap.mm() == 0.0 {
            return Err(error(
                "INVALID_DEPTH_CAP",
                "maximum carve depth must be positive",
            ));
        }
        if !(depth_cap.mm() * angle.slope()).is_finite()
            || depth_cap.mm() * angle.slope() <= 0.0
            || depth_cap.mm() * angle.slope() > region.grid().max_coordinate_mm()
        {
            return Err(error(
                "TARGET_RANGE",
                "target depth and angle exceed the shared coordinate range",
            ));
        }
        let boundary = BoundaryQuery::new(&region);
        Ok(Self {
            region,
            boundary,
            depth_cap,
            angle,
            diagram: OnceLock::new(),
            center_sets: Mutex::new(VecDeque::new()),
            diagram_samples: OnceLock::new(),
            input_snap_bound_mm,
        })
    }
    /// Planning refines only construction arithmetic; the normalized input
    /// boundary and its reported source snapping uncertainty stay unchanged.
    pub fn for_planning(region: Region, depth_cap: Depth, angle: IncludedAngle) -> Result<Self> {
        let input_snap_bound_mm = region.grid().snap_bound_mm();
        let region = if depth_cap.mm() * angle.slope() <= region.grid().max_coordinate_mm() / 16. {
            region.refine_construction_grid()
        } else {
            region
        };
        let mut target = Self::new(region, depth_cap, angle)?;
        target.input_snap_bound_mm = input_snap_bound_mm;
        Ok(target)
    }
    pub fn region(&self) -> &Region {
        &self.region
    }
    pub fn boundary(&self) -> &BoundaryQuery {
        &self.boundary
    }
    pub(crate) fn sample_key(p: Point) -> [u64; 2] {
        [p.x, p.y].map(|v| if v == 0. { 0 } else { v.to_bits() })
    }
    pub(crate) fn cached_sample(&self, p: Point) -> Result<Clearance> {
        if let Some(&sample) = self
            .diagram_samples
            .get()
            .and_then(|c| c.get(&Self::sample_key(p)))
        {
            return Ok(sample);
        }
        self.boundary.sample(p)
    }
    pub fn depth_cap(&self) -> Depth {
        self.depth_cap
    }
    pub fn angle(&self) -> IncludedAngle {
        self.angle
    }
    pub fn nominal_depth(&self, p: Point) -> Result<Depth> {
        let clearance = self.boundary.sample(p)?;
        Depth::new(if clearance.location == PointLocation::Inside {
            self.depth_cap
                .mm()
                .min(clearance.distance_mm / self.angle.slope())
        } else {
            0.0
        })
    }
    pub fn section(&self, depth: Depth) -> Result<CenterSet> {
        self.validate_depth(depth)?;
        self.center_set(Length::new(depth.mm() * self.angle.slope())?)
    }
    /// Area-only stock comparisons do not need Voronoi contact/witness analysis.
    pub(crate) fn section_area(&self, depth: Depth) -> Result<Region> {
        self.validate_depth(depth)?;
        self.region.erode(depth.mm() * self.angle.slope())
    }
    pub fn endmill_centers(
        &self,
        tool: &Endmill,
        depth: Depth,
        allowance: Length,
    ) -> Result<CenterSet> {
        self.validate_depth(depth)?;
        tool.validate_depth(depth)?;
        self.center_set(Length::new(
            depth.mm() * self.angle.slope() + tool.radius().mm() + allowance.mm(),
        )?)
    }
    pub fn vbit_centers(&self, tool: &VBit, depth: Depth) -> Result<CenterSet> {
        self.validate_depth(depth)?;
        self.validate_vbit(tool)?;
        self.center_set(Length::new(
            depth.mm() * self.angle.slope() + tool.tip_radius().mm(),
        )?)
    }
    /// Depth at this center is distinct from the achievable removal at this XY location.
    pub fn max_vbit_center_depth(&self, tool: &VBit, p: Point) -> Result<Depth> {
        self.validate_vbit(tool)?;
        let clearance = self.boundary.sample(p)?.signed_distance_mm;
        Depth::new(
            ((clearance - tool.tip_radius().mm()) / self.angle.slope())
                .max(0.0)
                .min(self.depth_cap.mm()),
        )
    }
    pub fn max_endmill_center_depth(
        &self,
        tool: &Endmill,
        p: Point,
        allowance: Length,
    ) -> Result<Depth> {
        let clearance = self.boundary.sample(p)?.signed_distance_mm;
        Depth::new(
            ((clearance - tool.radius().mm() - allowance.mm()) / self.angle.slope())
                .max(0.0)
                .min(self.depth_cap.mm())
                .min(tool.cutting_length().mm()),
        )
    }
    pub fn endmill_pose_fit(
        &self,
        tool: &Endmill,
        p: Point,
        depth: Depth,
        allowance: Length,
    ) -> Result<PoseFit> {
        self.validate_depth(depth)?;
        tool.validate_depth(depth)?;
        self.pose_fit(
            p,
            depth.mm() * self.angle.slope() + tool.radius().mm() + allowance.mm(),
        )
    }
    pub fn vbit_pose_fit(&self, tool: &VBit, p: Point, depth: Depth) -> Result<PoseFit> {
        self.validate_depth(depth)?;
        self.validate_vbit(tool)?;
        self.pose_fit(p, depth.mm() * self.angle.slope() + tool.tip_radius().mm())
    }
    fn validate_depth(&self, depth: Depth) -> Result<()> {
        if depth > self.depth_cap {
            return Err(error(
                "DEPTH_RANGE",
                "section/pose depth exceeds the target depth cap",
            ));
        }
        Ok(())
    }
    fn validate_vbit(&self, tool: &VBit) -> Result<()> {
        if tool.angle() != self.angle {
            return Err(error(
                "ANGLE_MISMATCH",
                "V-bit angle must match the nominal target angle",
            ));
        }
        tool.validate_depth(self.depth_cap)
    }
    pub(crate) fn diagram(&self) -> Result<&VoronoiDiagram> {
        self.diagram
            .get_or_init(|| VoronoiDiagram::build(&self.region))
            .as_ref()
            .map_err(Clone::clone)
    }
}

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("target")
}

#[cfg(test)]
mod sample_cache_tests {
    use super::*;

    #[test]
    fn cached_samples_match_fresh_boundary_queries_and_preserve_range_errors() {
        let job = crate::job::Job::from_json(include_str!("../../../../fixtures/m4/island.json"))
            .unwrap();
        let make_target = || {
            Target::for_planning(
                job.inspect().unwrap().geometry.selected,
                Depth::new(2.).unwrap(),
                IncludedAngle::new(90.).unwrap(),
            )
            .unwrap()
        };
        let target = make_target();
        let fresh = make_target();
        // Independent radii can initialize the same cache concurrently.
        std::thread::scope(|scope| {
            let a = scope.spawn(|| target.center_set(Length::new(1.).unwrap()).unwrap());
            let b = target.center_set(Length::new(2.).unwrap()).unwrap();
            for (radius, actual) in [(1., a.join().unwrap()), (2., b)] {
                let expected = fresh.center_set(Length::new(radius).unwrap()).unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
        });
        let cached = target.diagram_samples.get().unwrap();
        assert!(!cached.is_empty());
        assert!(cached.len() <= 131072);
        let mut points: Vec<_> = cached
            .keys()
            .map(|k| Point::new(f64::from_bits(k[0]), f64::from_bits(k[1])))
            .collect();
        points.extend([
            Point::new(0., -0.),
            Point::new(-0., 0.),
            Point::new(-1.234, 5.678),
        ]);
        for p in points {
            assert_eq!(
                serde_json::to_value(target.cached_sample(p).unwrap()).unwrap(),
                serde_json::to_value(fresh.boundary().sample(p).unwrap()).unwrap()
            );
        }
        for p in [
            Point::new(f64::NAN, 0.),
            Point::new(f64::INFINITY, 0.),
            Point::new(1e100, 0.),
        ] {
            assert_eq!(
                target.cached_sample(p).unwrap_err().code,
                fresh.boundary().sample(p).unwrap_err().code
            );
        }
    }
}
