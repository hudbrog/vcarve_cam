//! Display-only projection of existing core analysis, never a new stock calculation.
use crate::document::UiDiagnostic;
use cam_core::{
    geometry::{Point, Region},
    pocket::EndmillPlan,
    stock::SliceRemoval,
    vcarve::CombinedPlan,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SLICE_VERTICES: usize = 60_000;
pub const TOTAL_VERTICES: usize = 200_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayBounds {
    pub min: Point,
    pub max: Point,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionInfo {
    pub key: String,
    pub area_mm2: f64,
    pub vertex_count: usize,
    pub bounds: Option<DisplayBounds>,
    pub geometry_tolerance_mm: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ring {
    pub hole: bool,
    pub points: Vec<Point>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionRings {
    pub key: String,
    pub rings: Vec<Ring>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SliceInfo {
    pub id: String,
    pub stage: String,
    pub depth_mm: f64,
    pub status: Option<String>,
    pub contributing_motion_count: usize,
    pub capsule_radial_error_mm: f64,
    pub regions: Vec<RegionInfo>,
    pub diagnostics: Vec<Value>,
    pub omitted_diagnostics: usize,
    pub unavailable_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Slice {
    pub info: SliceInfo,
    pub geometry: Option<Vec<RegionRings>>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Inspection {
    pub slices: Vec<Slice>,
}

fn region_info(key: &str, region: &Region) -> RegionInfo {
    let mut bounds: Option<DisplayBounds> = None;
    let grid = region.grid();
    for point in region.rings().iter().flat_map(|ring| ring.points()) {
        let p = grid.point(*point);
        bounds = Some(match bounds {
            None => DisplayBounds { min: p, max: p },
            Some(mut b) => {
                b.min.x = b.min.x.min(p.x);
                b.min.y = b.min.y.min(p.y);
                b.max.x = b.max.x.max(p.x);
                b.max.y = b.max.y.max(p.y);
                b
            }
        });
    }
    RegionInfo {
        key: key.into(),
        area_mm2: region.area_mm2(),
        vertex_count: region.rings().iter().map(|r| r.points().len()).sum(),
        bounds,
        geometry_tolerance_mm: grid.tolerance_mm(),
    }
}
fn project(
    info: &mut SliceInfo,
    regions: &[(&str, &Region)],
    budget: &mut usize,
) -> Option<Vec<RegionRings>> {
    info.regions = regions
        .iter()
        .map(|(key, region)| region_info(key, region))
        .collect();
    let count: usize = info.regions.iter().map(|r| r.vertex_count).sum();
    if count > SLICE_VERTICES || count > *budget {
        info.unavailable_reason = Some(format!(
            "Slice contains {count} vertices; display permits {SLICE_VERTICES} per slice and {TOTAL_VERTICES} across this result. Metrics are retained; no partial polygons are shown."
        ));
        return None;
    }
    *budget -= count;
    Some(
        regions
            .iter()
            .map(|(key, region)| RegionRings {
                key: (*key).into(),
                rings: region
                    .rings()
                    .iter()
                    .zip(region.rings_mm())
                    .map(|(ring, points)| Ring {
                        hole: ring.is_hole(),
                        points,
                    })
                    .collect(),
            })
            .collect(),
    )
}
fn slice_info(stage: &str, index: usize, removal: &SliceRemoval) -> SliceInfo {
    SliceInfo {
        id: format!("{stage}-{index}"),
        stage: stage.into(),
        depth_mm: removal.depth_mm,
        status: None,
        contributing_motion_count: removal.contributing_motion_ids.len(),
        capsule_radial_error_mm: removal.capsule_radial_error_mm,
        regions: vec![],
        diagnostics: vec![],
        omitted_diagnostics: 0,
        unavailable_reason: None,
    }
}
impl Inspection {
    fn append_endmill(&mut self, plan: &EndmillPlan, budget: &mut usize) {
        for (i, layer) in plan.analysis.layers.iter().enumerate() {
            let mut info = slice_info("endmill", i, &layer.removal);
            info.status = Some(
                serde_json::to_value(layer.status)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
            );
            info.diagnostics = layer
                .diagnostics
                .iter()
                .take(100)
                .cloned()
                .map(|d| json!(UiDiagnostic::from(d)))
                .collect();
            info.omitted_diagnostics = layer.diagnostics.len().saturating_sub(100);
            let geometry = project(
                &mut info,
                &[
                    ("nominalTarget", &layer.nominal_section),
                    ("removedLower", &layer.removal.lower),
                    ("removedUpper", &layer.removal.upper),
                    ("remainingTarget", &layer.remaining_target),
                    ("possibleOvercut", &layer.possible_overcut),
                    ("accessibleFloor", &layer.accessible_floor),
                    ("missingFloor", &layer.missing_floor_beyond_tolerance),
                    ("requestedCenters", &layer.requested_centers),
                ],
                budget,
            );
            self.slices.push(Slice { info, geometry });
        }
    }
    pub fn endmill(plan: &EndmillPlan) -> Self {
        let mut result = Self::default();
        let mut budget = TOTAL_VERTICES;
        result.append_endmill(plan, &mut budget);
        result
    }
    pub fn combined(plan: &CombinedPlan) -> Self {
        let mut result = Self::default();
        let mut budget = TOTAL_VERTICES;
        result.append_endmill(&plan.endmill, &mut budget);
        for (i, layer) in plan.analysis.slices.iter().enumerate() {
            let mut info = slice_info("combined", i, &layer.removal);
            let geometry = project(
                &mut info,
                &[
                    ("nominalTarget", &layer.nominal_section),
                    ("removedLower", &layer.removal.lower),
                    ("removedUpper", &layer.removal.upper),
                    ("remainingTarget", &layer.remaining_target),
                    ("possibleOvercut", &layer.possible_overcut),
                ],
                &mut budget,
            );
            result.slices.push(Slice { info, geometry });
        }
        result
    }
    pub fn omit_geometry(&mut self) {
        for slice in &mut self.slices {
            slice.geometry = None;
            slice.info.unavailable_reason = Some("Stock polygons exceed the worker transfer limit. Metrics and the plan artifact are retained.".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cam_core::{job::Job, pocket::plan_endmill};
    #[test]
    fn projection_keeps_holes_coordinates_areas_and_fails_closed_at_display_budget() {
        let job = Job::from_json(include_str!("../../../fixtures/m3/island.json")).unwrap();
        let plan = plan_endmill(&job).unwrap();
        let data = Inspection::endmill(&plan);
        let layer = &plan.analysis.layers[0];
        let first = &data.slices[0];
        assert_eq!(first.info.depth_mm, layer.depth_mm);
        let nominal = &first.geometry.as_ref().unwrap()[0];
        assert_eq!(
            nominal.rings.iter().map(|r| &r.points).collect::<Vec<_>>(),
            layer.nominal_section.rings_mm().iter().collect::<Vec<_>>()
        );
        assert!(nominal.rings.iter().any(|ring| ring.hole));
        assert_eq!(
            first.info.regions[0].area_mm2,
            layer.nominal_section.area_mm2()
        );
        let mut limited = first.info.clone();
        assert!(
            project(
                &mut limited,
                &[("nominalTarget", &layer.nominal_section)],
                &mut 0
            )
            .is_none()
        );
        assert!(limited.unavailable_reason.is_some());
        assert_eq!(limited.regions[0].area_mm2, first.info.regions[0].area_mm2);
    }
}
