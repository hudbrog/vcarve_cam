use super::{Target, error};
use crate::{
    geometry::{
        BoundaryQuery, Clearance, Diagnostic, Point, PointLocation, Region, Result, Segment,
        SiteKind,
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
        Ok(self.fit_from_clearance(sample, required))
    }

    fn fit_from_clearance(&self, sample: Clearance, required: f64) -> PoseFit {
        let margin = sample.signed_distance_mm - required;
        let reserve = sample.numerical_reserve_mm + 32.0 * f64::EPSILON * required;
        let status = if margin > reserve {
            FitStatus::Clearance
        } else if margin < -reserve {
            FitStatus::Infeasible
        } else {
            FitStatus::Contact
        };
        PoseFit {
            status,
            clearance_mm: sample.signed_distance_mm,
            required_clearance_mm: required,
            margin_mm: margin,
            numerical_reserve_mm: reserve,
            input_snap_bound_mm: self.input_snap_bound_mm,
        }
    }

    /// The authoritative predicate is signed boundary distance >= required radius.
    /// Polygon area is accompanied by exact-fit contacts and representation diagnostics.
    pub fn center_set(&self, radius: Length) -> Result<CenterSet> {
        let mut timing = crate::timing::Timer::new("target center set");
        let required = radius.mm();
        if let Ok(cache) = self.center_sets.lock()
            && let Some((_, result, _)) = cache.iter().find(|(r, _, _)| *r == required)
        {
            return Ok(result.clone());
        }
        let area = self.region.erode(required)?;
        timing.lap("erode");
        let mut result = CenterSet {
            required_clearance_mm: required,
            diagnostics: area.diagnostics().to_vec(),
            area,
            contact_segments: vec![],
            contact_points: vec![],
            status: CenterSetStatus::Empty,
            input_snap_bound_mm: self.input_snap_bound_mm,
        };
        if required == 0.0 {
            result.status = CenterSetStatus::Area;
            return Ok(result);
        }
        let diagram = self.diagram()?;
        timing.lap("diagram");
        // A Voronoi vertex occurs on several edges. Reuse its exact query,
        // retaining every witness occurrence and the original error order.
        let key = |p: Point| [p.x, p.y].map(|v| if v == 0. { 0 } else { v.to_bits() });
        let mut clearances = std::collections::HashMap::new();
        let mut sample = |p: Point| -> Result<Clearance> {
            if let Some(&q) = clearances.get(&key(p)) {
                return Ok(q);
            }
            let q = self.boundary.sample(p)?;
            if clearances.len() < 131072 {
                clearances.insert(key(p), q);
            }
            Ok(q)
        };
        let mut witnesses = vec![];
        let mut candidates = vec![];
        let mut interior_vertices = 0;
        for edge in &diagram.edges {
            for p in [edge.start, edge.end].into_iter().flatten() {
                let q = sample(p)?;
                if q.location != PointLocation::Inside {
                    continue;
                }
                interior_vertices += 1;
                let fit = self.fit_from_clearance(q, required);
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
            let middle = sample(mid)?;
            if middle.location == PointLocation::Inside
                && self.fit_from_clearance(middle, required).status == FitStatus::Clearance
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
            let fit = self.fit_from_clearance(middle, required);
            if fit.status != FitStatus::Contact
                || self.fit_from_clearance(sample(start)?, required).status != FitStatus::Contact
                || self.fit_from_clearance(sample(end)?, required).status != FitStatus::Contact
            {
                continue;
            }
            let segment = Segment { start, end };
            if middle.location == PointLocation::Inside
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
            let reserve = self
                .fit_from_clearance(sample(p)?, required)
                .numerical_reserve_mm;
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
        let mut area_locations = std::collections::HashMap::new();
        for p in witnesses {
            let outside = if !has_area {
                true
            } else if let Some(&outside) = area_locations.get(&key(p)) {
                outside
            } else {
                let outside = area_query.sample(p)?.location == PointLocation::Outside;
                if area_locations.len() < 131072 {
                    area_locations.insert(key(p), outside);
                }
                outside
            };
            if outside {
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
        // Target geometry is immutable. Bound both cache entry count and total
        // retained geometry; unusually large results are simply not cached.
        let weight = result
            .area
            .rings()
            .iter()
            .map(|r| r.points().len())
            .sum::<usize>()
            + result.contact_points.len()
            + 2 * result.contact_segments.len();
        if weight <= 131072
            && let Ok(mut cache) = self.center_sets.lock()
        {
            while cache.len() >= 8
                || cache.iter().map(|(_, _, n)| n).sum::<usize>() + weight > 131072
            {
                cache.pop_front();
            }
            cache.push_back((required, result.clone(), weight));
        }
        Ok(result)
    }
}
