//! Nominal target and tool access, independent of any path planning or stock simulation.
mod access;
mod reachability;

use crate::{
    geometry::{BoundaryQuery, Diagnostic, Point, PointLocation, Region, Result, VoronoiDiagram},
    model::{Depth, Endmill, IncludedAngle, Length, VBit},
};
pub use access::{CenterSet, CenterSetStatus, FitStatus, PoseFit};
pub use reachability::{Reachability, ReachabilityOptions, ReachabilityStatus};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

pub struct Target {
    region: Region,
    boundary: BoundaryQuery,
    depth_cap: Depth,
    angle: IncludedAngle,
    diagram: OnceLock<Result<VoronoiDiagram>>,
    center_sets: Mutex<VecDeque<(f64, CenterSet, usize)>>,
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
