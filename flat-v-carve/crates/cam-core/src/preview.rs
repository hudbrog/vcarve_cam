//! Serializable M1 target and cutter previews built by the same in-memory core as the CLI.
use crate::{
    geometry::{Diagnostic, Grid, Point, Region, Result},
    model::{Depth, Endmill, EndmillSpec, Length, VBit, VBitSpec},
    target::{
        CenterSet, CenterSetStatus, Reachability, ReachabilityOptions, ReachabilityStatus, Target,
    },
};
use serde::{Deserialize, Serialize};

pub const MODEL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInput {
    pub schema_version: u32,
    pub id: String,
    pub description: String,
    pub geometry_tolerance_mm: f64,
    pub ticks_per_mm: Option<f64>,
    pub rings: Vec<Vec<Point>>,
    pub max_depth_mm: f64,
    pub endmill: EndmillSpec,
    pub vbit: VBitSpec,
    pub wall_allowance_mm: f64,
    pub section_depths_mm: Vec<f64>,
    pub cross_sections: Vec<CrossSectionInput>,
    pub preview_depth_tolerance_mm: f64,
    pub max_reachability_cells: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossSectionInput {
    pub id: String,
    pub start: Point,
    pub end: Point,
    pub samples: usize,
}

pub struct ValidatedModel {
    pub target: Target,
    pub endmill: Endmill,
    pub vbit: VBit,
    pub wall_allowance: Length,
}

#[derive(Clone, Debug, Serialize)]
pub struct DepthSection {
    pub depth_mm: f64,
    pub machine_z_mm: f64,
    pub nominal: CenterSet,
    pub endmill: CenterSet,
    pub vbit: CenterSet,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileSample {
    pub xy: Point,
    pub distance_mm: f64,
    pub nominal_depth_mm: f64,
    pub nominal_z_mm: f64,
    pub max_vbit_center_depth_mm: f64,
    pub max_endmill_center_depth_mm: f64,
    pub vbit_removal: Reachability,
}

#[derive(Clone, Debug, Serialize)]
pub struct CrossSection {
    pub input: CrossSectionInput,
    pub samples: Vec<ProfileSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    Complete,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize)]
pub struct TargetPreview {
    pub schema_version: u32,
    pub input: ModelInput,
    pub status: PreviewStatus,
    pub normalized_region: Region,
    pub endmill: Endmill,
    pub vbit: VBit,
    pub sections: Vec<DepthSection>,
    pub cross_sections: Vec<CrossSection>,
    pub diagnostics: Vec<Diagnostic>,
}

fn invalid(code: &str, message: &str) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("model")
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl ModelInput {
    /// Strict geometry settings for M1, not the editable/incomplete job schema planned for M2.
    pub fn validate(&self) -> Result<ValidatedModel> {
        if self.schema_version != MODEL_SCHEMA_VERSION {
            return Err(invalid(
                "MODEL_SCHEMA_VERSION",
                "unsupported M1 model schema version",
            ));
        }
        if !valid_id(&self.id) {
            return Err(invalid(
                "INVALID_MODEL_ID",
                "model id must contain 1..=100 ASCII letters, digits, underscores or hyphens",
            ));
        }
        if self.section_depths_mm.is_empty()
            || self.section_depths_mm.len() > 64
            || self.cross_sections.is_empty()
            || self.cross_sections.len() > 16
        {
            return Err(invalid(
                "PREVIEW_LIMIT",
                "preview requires 1..=64 depth sections and 1..=16 cross-sections",
            ));
        }
        if !self.preview_depth_tolerance_mm.is_finite()
            || self.preview_depth_tolerance_mm <= 0.0
            || self.max_reachability_cells == 0
            || self.max_reachability_cells > 1_000_000
        {
            return Err(invalid(
                "INVALID_REACHABILITY_OPTIONS",
                "positive finite preview depth tolerance and 1..=1000000 cells required",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        let mut total_samples = 0;
        for section in &self.cross_sections {
            if !valid_id(&section.id) || !ids.insert(&section.id) {
                return Err(invalid(
                    "INVALID_PROFILE_ID",
                    "profile IDs must be valid and unique",
                ));
            }
            if !(2..=1025).contains(&section.samples) {
                return Err(invalid(
                    "PREVIEW_LIMIT",
                    "each cross-section needs 2..=1025 samples",
                ));
            }
            total_samples += section.samples;
            if !section.start.finite() || !section.end.finite() || section.start == section.end {
                return Err(invalid(
                    "INVALID_CROSS_SECTION",
                    "cross-section endpoints must be finite and distinct",
                ));
            }
        }
        if total_samples > 4096 {
            return Err(invalid(
                "PREVIEW_LIMIT",
                "at most 4096 cross-section samples per preview",
            ));
        }
        let depth = Depth::new(self.max_depth_mm)?;
        let vbit = VBit::try_from(self.vbit.clone())?;
        let endmill = Endmill::try_from(self.endmill.clone())?;
        vbit.validate_depth(depth)?;
        endmill.validate_depth(depth)?;
        let wall_allowance = Length::new(self.wall_allowance_mm)?;
        for &d in &self.section_depths_mm {
            if Depth::new(d)? > depth {
                return Err(invalid(
                    "DEPTH_RANGE",
                    "requested section is deeper than the target",
                ));
            }
        }
        let extent = self
            .rings
            .iter()
            .flatten()
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(0.0, f64::max);
        let grid = match self.ticks_per_mm {
            Some(scale) => Grid::with_scale(self.geometry_tolerance_mm, extent, scale)?,
            None => Grid::new(self.geometry_tolerance_mm, extent)?,
        };
        let target = Target::new(Region::from_rings(grid, &self.rings)?, depth, vbit.angle())?;
        if depth.mm() * vbit.angle().slope() + endmill.radius().mm() + wall_allowance.mm()
            > grid.max_coordinate_mm()
            || depth.mm() * vbit.angle().slope() + vbit.tip_radius().mm() > grid.max_coordinate_mm()
        {
            return Err(invalid(
                "CENTER_RANGE",
                "tool-center clearance exceeds the shared grid range",
            ));
        }
        for section in &self.cross_sections {
            target.boundary().sample(section.start)?;
            target.boundary().sample(section.end)?;
        }
        Ok(ValidatedModel {
            target,
            endmill,
            vbit,
            wall_allowance,
        })
    }
}

pub fn build_preview(input: ModelInput) -> Result<TargetPreview> {
    let model = input.validate()?;
    let mut result = TargetPreview {
        schema_version: MODEL_SCHEMA_VERSION,
        input: input.clone(),
        status: PreviewStatus::Complete,
        normalized_region: model.target.region().clone(),
        endmill: model.endmill.clone(),
        vbit: model.vbit.clone(),
        sections: vec![],
        cross_sections: vec![],
        diagnostics: model.target.region().diagnostics().to_vec(),
    };
    for &value in &input.section_depths_mm {
        let depth = Depth::new(value)?;
        let nominal = model.target.section(depth)?;
        let endmill = model
            .target
            .endmill_centers(&model.endmill, depth, model.wall_allowance)?;
        let vbit = model.target.vbit_centers(&model.vbit, depth)?;
        for section in [&nominal, &endmill, &vbit] {
            if section.status == CenterSetStatus::Unresolved {
                result.status = PreviewStatus::Inconclusive;
            }
            for diagnostic in &section.diagnostics {
                if !result.diagnostics.contains(diagnostic) {
                    result.diagnostics.push(diagnostic.clone());
                }
            }
        }
        result.sections.push(DepthSection {
            depth_mm: depth.mm(),
            machine_z_mm: depth.machine_z_mm(),
            nominal,
            endmill,
            vbit,
        });
    }
    let options = ReachabilityOptions {
        depth_tolerance_mm: input.preview_depth_tolerance_mm,
        max_cells: input.max_reachability_cells,
    };
    let mut unresolved = 0;
    for section in &input.cross_sections {
        let mut samples = Vec::with_capacity(section.samples);
        let length = section.start.distance(section.end);
        for i in 0..section.samples {
            let t = i as f64 / (section.samples - 1) as f64;
            let xy = section.start.lerp(section.end, t);
            let depth = model.target.nominal_depth(xy)?;
            let vbit_removal = model.target.vbit_reachability(&model.vbit, xy, options)?;
            if vbit_removal.status != ReachabilityStatus::Resolved {
                unresolved += 1;
            }
            samples.push(ProfileSample {
                xy,
                distance_mm: length * t,
                nominal_depth_mm: depth.mm(),
                nominal_z_mm: depth.machine_z_mm(),
                max_vbit_center_depth_mm: model.target.max_vbit_center_depth(&model.vbit, xy)?.mm(),
                max_endmill_center_depth_mm: model
                    .target
                    .max_endmill_center_depth(&model.endmill, xy, model.wall_allowance)?
                    .mm(),
                vbit_removal,
            });
        }
        result.cross_sections.push(CrossSection {
            input: section.clone(),
            samples,
        });
    }
    if unresolved > 0 {
        result.status = PreviewStatus::Inconclusive;
        result.diagnostics.push(Diagnostic::new("REACHABILITY_INCONCLUSIVE",format!("{unresolved} cross-section samples could not meet the requested depth uncertainty; bounds are retained")).at_stage("preview"));
    }
    Ok(result)
}
