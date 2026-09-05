use super::{Target, error};
use crate::{
    geometry::{
        BoundaryQuery, Diagnostic, Point, PointLocation, Region, Result, Segment, SiteKind,
    },
    model::Length,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FitStatus {
    Clearance,
    Contact,
    Infeasible,
}

#[derive(Clone, Debug, Serialize)]
pub struct PoseFit {
    pub status: FitStatus,
    pub clearance_mm: f64,
    pub required_clearance_mm: f64,
    pub margin_mm: f64,
    pub numerical_reserve_mm: f64,
    pub input_snap_bound_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterSetStatus {
    Area,
    AreaAndContacts,
    ContactOnly,
    Empty,
    Unresolved,
}

#[derive(Clone, Debug, Serialize)]
pub struct CenterSet {
    pub required_clearance_mm: f64,
    pub area: Region,
    /// Entire segments at constant clearance, not just sampled admissible endpoints.
    pub contact_segments: Vec<Segment>,
    /// Valid zero-margin vertices. Some can be redundant with area boundary junctions.
    pub contact_points: Vec<Point>,
    pub status: CenterSetStatus,
    pub input_snap_bound_mm: f64,
    pub diagnostics: Vec<Diagnostic>,
}

impl Target {
    pub(super) fn pose_fit(&self, p: Point, required: f64) -> Result<PoseFit> {
        if !required.is_finite()
            || required < 0.0
            || required > self.region.grid().max_coordinate_mm()
        {
            return Err(error(
                "CENTER_RANGE",
                "required clearance exceeds the precision range",
            ));
        }
        let sample = self.boundary.sample(p)?;
        let margin = sample.signed_distance_mm - required;
        let reserve = sample.numerical_reserve_mm + 32.0 * f64::EPSILON * required;
        let status = if margin > reserve {
            FitStatus::Clearance
        } else if margin < -reserve {
            FitStatus::Infeasible
        } else {
            FitStatus::Contact
        };
        Ok(PoseFit {
            status,
            clearance_mm: sample.signed_distance_mm,
            required_clearance_mm: required,
            margin_mm: margin,
            numerical_reserve_mm: reserve,
            input_snap_bound_mm: self.region.grid().snap_bound_mm(),
        })
    }

    /// The authoritative predicate is signed boundary distance >= required radius.
    /// Polygon area is accompanied by exact-fit contacts and representation diagnostics.
    pub fn center_set(&self, radius: Length) -> Result<CenterSet> {
        let required = radius.mm();
        let area = self.region.erode(required)?;
        let mut result = CenterSet {
            required_clearance_mm: required,
            diagnostics: area.diagnostics().to_vec(),
            area,
            contact_segments: vec![],
            contact_points: vec![],
            status: CenterSetStatus::Empty,
            input_snap_bound_mm: self.region.grid().snap_bound_mm(),
        };
        if required == 0.0 {
            result.status = CenterSetStatus::Area;
            return Ok(result);
        }
        let diagram = self.diagram()?;
        let mut witnesses = vec![];
        let mut candidates = vec![];
        let mut interior_vertices = 0;
        for edge in &diagram.edges {
            for p in [edge.start, edge.end].into_iter().flatten() {
                if self.boundary.sample(p)?.location != PointLocation::Inside {
                    continue;
                }
                interior_vertices += 1;
                let fit = self.pose_fit(p, required)?;
                if fit.status == FitStatus::Contact {
                    candidates.push(p);
                }
                if fit.status == FitStatus::Clearance {
                    witnesses.push(p);
                }
            }
            if !edge.primary {
                continue;
            }
            let (Some(start), Some(end)) = (edge.start, edge.end) else {
                continue;
            };
            let Some(curve) = &edge.curve else { continue };
            let mid = curve.evaluate(0.5)?;
            if self.boundary.sample(mid)?.location == PointLocation::Inside
                && self.pose_fit(mid, required)?.status == FitStatus::Clearance
            {
                witnesses.push(mid);
            }
            if edge.curved || !edge.sites.iter().all(|s| s.kind == SiteKind::Segment) {
                continue;
            }
            let a = diagram.source_segments[edge.sites[0].segment];
            let b = diagram.source_segments[edge.sites[1].segment];
            let grid = self.region.grid();
            let (a0, a1, b0, b1) = (
                grid.quantize(a.start)?,
                grid.quantize(a.end)?,
                grid.quantize(b.start)?,
                grid.quantize(b.end)?,
            );
            let parallel = (a1.x - a0.x) as i128 * (b1.y - b0.y) as i128
                == (a1.y - a0.y) as i128 * (b1.x - b0.x) as i128;
            if !parallel || start == end {
                continue;
            }
            let fit = self.pose_fit(mid, required)?;
            if fit.status != FitStatus::Contact
                || self.pose_fit(start, required)?.status != FitStatus::Contact
                || self.pose_fit(end, required)?.status != FitStatus::Contact
            {
                continue;
            }
            let segment = Segment { start, end };
            if self.boundary.sample(mid)?.location == PointLocation::Inside
                && self.boundary.segment_distance_mm(segment)? + fit.numerical_reserve_mm
                    >= required
            {
                result.contact_segments.push(segment);
            }
        }
        if interior_vertices == 0 {
            return Err(error(
                "CENTER_SET_UNRESOLVED",
                "nonempty input yielded no interior Voronoi vertices; empty access cannot be established",
            ));
        }
        for p in candidates {
            let reserve = self.pose_fit(p, required)?.numerical_reserve_mm;
            if result
                .contact_segments
                .iter()
                .any(|s| s.distance(p) <= reserve)
                || result
                    .contact_points
                    .iter()
                    .any(|q| q.distance(p) <= reserve)
            {
                continue;
            }
            result.contact_points.push(p);
        }
        let area_query = BoundaryQuery::new(&result.area);
        let has_area = !result.area.rings().is_empty();
        // A positive-clearance witness missing from offset area must not disappear silently.
        let mut missing = vec![];
        for p in witnesses {
            if !has_area || area_query.sample(p)?.location == PointLocation::Outside {
                missing.push(p);
            }
        }
        let contacts = !result.contact_points.is_empty() || !result.contact_segments.is_empty();
        result.status = if !missing.is_empty() {
            CenterSetStatus::Unresolved
        } else if has_area && contacts {
            CenterSetStatus::AreaAndContacts
        } else if has_area {
            CenterSetStatus::Area
        } else if contacts {
            CenterSetStatus::ContactOnly
        } else {
            CenterSetStatus::Empty
        };
        if !missing.is_empty() {
            result.diagnostics.push(error("CENTER_SET_UNRESOLVED",format!("{} positive-clearance witnesses are absent from polygon area; use a finer grid before relying on this representation (first: {}, {})",missing.len(),missing[0].x,missing[0].y)));
        }
        if contacts {
            result.diagnostics.push(error("EXACT_FIT_CONTACT","zero-margin tool-center contacts are retained; input snapping, entry capability and motion verification still need accounting").warning());
        }
        Ok(result)
    }
}
