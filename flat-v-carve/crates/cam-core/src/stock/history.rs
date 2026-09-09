//! Ordered removal history for prefix stock queries (plan section 9.1).
//!
//! The history composes the existing analytical cutter models over an
//! ordered prefix of recorded milling sweeps: for a point inside the stock
//! rectangle, the material top is the lowest surface reached by the
//! preceding sweeps, clamped at the stock bottom. Knife traces never enter
//! this model. No voxel grid or duplicated mesh is built; queries walk the
//! recorded batches, which is enough for planner-scale point queries and
//! keeps prefix identities cheap to fingerprint.
use crate::{
    geometry::{Diagnostic, Point, Result},
    model::{Length, VBit},
    motion::Position,
    project::RectXY,
};
use serde::{Deserialize, Serialize};

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("stock-history")
}

/// The cutter whose sweeps a batch records.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SweepCutter {
    /// Flat-bottom endmill: every point within `radius_mm` of the axis
    /// path is cut down to the commanded floor.
    FlatEndmill { radius_mm: f64 },
    /// Conical V-bit under the legacy removal model.
    VBit { spec: crate::model::VBitSpec },
}

/// One cutting move; non-cutting motions never enter a batch.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepMotion {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepBatch {
    pub stage_id: String,
    pub operation_id: String,
    pub cutter: SweepCutter,
    pub motions: Vec<SweepMotion>,
}

/// A rectangular prism plus the ordered batches that milled it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StockHistory {
    pub thickness_mm: f64,
    /// None keeps the legacy unbounded-XY behavior explicit.
    pub xy: Option<RectXY>,
    pub batches: Vec<SweepBatch>,
}

impl StockHistory {
    pub fn new(thickness_mm: f64, xy: Option<RectXY>) -> Result<Self> {
        if !thickness_mm.is_finite() || thickness_mm <= 0. {
            return Err(error(
                "STOCK_HISTORY_PARAMETER",
                "stock thickness must be finite and positive",
            ));
        }
        Ok(Self {
            thickness_mm,
            xy,
            batches: vec![],
        })
    }

    pub fn push(&mut self, batch: SweepBatch) {
        self.batches.push(batch);
    }

    /// True when the point is inside the physical stock rectangle; outside
    /// there is no material. A null rectangle keeps the legacy unbounded
    /// interpretation (the caller decides whether that is admissible).
    pub fn inside_stock(&self, p: Point) -> bool {
        match &self.xy {
            Some(rect) => {
                p.x >= rect.min_x_mm
                    && p.x <= rect.min_x_mm + rect.width_mm
                    && p.y >= rect.min_y_mm
                    && p.y <= rect.min_y_mm + rect.length_mm
            }
            None => true,
        }
    }

    fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length_squared = dx * dx + dy * dy;
        if length_squared == 0. {
            return p.distance(a);
        }
        let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared).clamp(0., 1.);
        p.distance(Point::new(a.x + t * dx, a.y + t * dy))
    }

    fn batch_floor_at(&self, batch: &SweepBatch, p: Point) -> Result<Option<f64>> {
        let mut floor: Option<f64> = None;
        for motion in &batch.motions {
            let a = motion.start.xy();
            let b = motion.end.xy();
            let cut_z = motion.start.z.min(motion.end.z);
            if cut_z >= 0. {
                // A sweep entirely at or above the original top removes nothing.
                continue;
            }
            let distance = Self::segment_distance(p, a, b);
            let candidate = match &batch.cutter {
                SweepCutter::FlatEndmill { radius_mm } => {
                    if distance <= *radius_mm {
                        Some(cut_z)
                    } else {
                        None
                    }
                }
                SweepCutter::VBit { spec } => {
                    let tool = VBit::try_from(spec.clone())?;
                    let depth = (-cut_z).min(tool.cutting_height().mm());
                    let removed = tool
                        .removal_depth(crate::model::Depth::new(depth)?, Length::new(distance)?)?;
                    if removed.mm() > 0. {
                        Some(-removed.mm())
                    } else {
                        None
                    }
                }
            };
            if let Some(value) = candidate {
                floor = Some(floor.map_or(value, |current: f64| current.min(value)));
            }
        }
        Ok(floor)
    }

    /// Remaining material top at a setup-space XY point: the lowest floor
    /// reached by the recorded sweeps, clamped at stock bottom. Outside the
    /// physical rectangle (when declared) the query is an error — there is
    /// no material to ask about, and callers must handle that case
    /// explicitly instead of receiving a silent zero.
    pub fn material_top_at(&self, p: Point) -> Result<f64> {
        if !self.inside_stock(p) {
            return Err(error(
                "STOCK_POINT_OUTSIDE",
                "the queried point lies outside the physical stock rectangle",
            ));
        }
        let mut top = 0_f64;
        for batch in &self.batches {
            if let Some(floor) = self.batch_floor_at(batch, p)? {
                top = top.min(floor);
            }
        }
        Ok(top.max(-self.thickness_mm))
    }

    /// Whether a straight XY transit of the given tool never dips below the
    /// remaining material. The corridor is sampled at the tool radius plus a
    /// numerical reserve; B2 planners use it for conservative link checks.
    pub fn can_traverse_without_cutting(
        &self,
        from: Point,
        to: Point,
        tool_radius_mm: f64,
        transit_z: f64,
    ) -> Result<bool> {
        let distance = from.distance(to);
        let samples = ((distance / tool_radius_mm.max(0.001)).ceil() as usize).clamp(1, 4096);
        for index in 0..=samples {
            let t = index as f64 / samples as f64;
            let center = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            // Points outside a declared stock rectangle are air; only the
            // material the corridor touches matters.
            if !self.inside_stock(center) {
                continue;
            }
            let top = self.material_top_at(center)?;
            if transit_z < top - tool_radius_mm * 1e-9 {
                // The center itself would collide; the swept corridor is
                // checked through the same query at B2's conservative
                // sampling, which sub-samples the radius bound.
                return Ok(false);
            }
            // Corridor edges: probe the widest remaining material around the
            // center within one tool radius.
            for (dx, dy) in [
                (tool_radius_mm, 0.),
                (-tool_radius_mm, 0.),
                (0., tool_radius_mm),
                (0., -tool_radius_mm),
            ] {
                let edge = Point::new(center.x + dx, center.y + dy);
                if self.inside_stock(edge) && transit_z < self.material_top_at(edge)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
