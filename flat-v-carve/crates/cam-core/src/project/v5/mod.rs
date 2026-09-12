//! Canonical schema-5 CAM job: an artwork collection, qualified geometry
//! references, copied cutting-profile provenance and one applied machine
//! configuration (plan sections 22.2, 22.3, 22.5, 22.6 and 22.7).
//!
//! Validation is layered deliberately, unlike the schema-4 document gate:
//!
//! * **Structural integrity** ([`CamJobV5::validate_structure`]) is the
//!   save/open gate: supported shapes, finite representable values, unique
//!   valid IDs and bounded content. Malformed documents stay rejected.
//! * **Reference inspection** ([`references::inspect_references`]) reports
//!   missing artwork/tools/geometry, source-revision mismatches, forward or
//!   deleted face references and anchors needing reattachment as located
//!   issues while preserving the typed document. Well-formed dangling
//!   references are saveable.
//! * **Planning readiness** ([`references::planning_readiness`]) blocks only
//!   the requested enabled scope; an unresolved reference in an unrelated
//!   disabled operation or outside a planned prefix never blocks that prefix.
//!
//! The frozen schema-4 DTO ([`crate::project::CamJob`]) is the migration
//! input; [`migrate`] upgrades validated schema-4 documents (and, through the
//! existing compatibility step, legacy schema-1/2/3 jobs) into this model.
use crate::{
    geometry::{Diagnostic, Result},
    job::{MachineProfile, PlanningTolerances, SourceSnapshot},
    motion::Position,
    post::{Coolant, LengthCompensation, M6Contract, PathControl},
    preview,
    svg::{ImportMode, ImportOptions, Placement},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod artwork;
pub mod commands;
pub mod inspection;
pub mod machine;
pub mod migrate;
pub mod references;
pub mod resolve;
pub mod resources;

pub use artwork::{CombinedCatalogue, GeometryPick, SetupBounds, inspect_artwork, item_catalogue};
pub use references::ReadinessScope;

pub const CAM_JOB_V5_SCHEMA_VERSION: u32 = 5;

/// Deterministic item ID assigned to the single source of a migrated
/// schema-4 job (plan section 22.3, migration rule 2).
pub const MIGRATED_ARTWORK_ITEM_ID: &str = "artwork-1";

/// Version of the source-revision digest algorithm (content + interpretation).
/// Bumping it invalidates every stored geometry reference conservatively when
/// the interpretation or fingerprint algorithms change (plan section 22.3).
pub const SOURCE_REVISION_ALGORITHM_VERSION: u32 = 1;

/// Total serialized document budget, matching the schema-4 gate. Per-item
/// content is additionally bounded by [`crate::svg::MAX_SVG_BYTES`], and the
/// aggregate limit is what actually protects the job boundary.
const MAX_JOB_BYTES: usize = 64_000_000;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("project")
}

fn number(value: Option<f64>, name: &str, positive: bool) -> Result<()> {
    if value.is_some_and(|v| !v.is_finite() || if positive { v <= 0. } else { v < 0. }) {
        return Err(error(
            "PROJECT_PARAMETER",
            format!(
                "{name} must be finite and {}",
                if positive { "positive" } else { "nonnegative" }
            ),
        ));
    }
    Ok(())
}

fn short_label(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 1000 {
        return Err(error(
            "PROJECT_PARAMETER",
            format!("{name} must be nonempty and at most 1000 characters"),
        ));
    }
    Ok(())
}

/// Stable identifier of one artwork item in the collection.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtworkItemId(pub String);

impl ArtworkItemId {
    fn validate(&self) -> Result<()> {
        if !preview::valid_id(&self.0) {
            return Err(error(
                "PROJECT_PARAMETER",
                format!("artwork item ID '{}' must be a valid identifier", self.0),
            ));
        }
        Ok(())
    }
}

/// How one artwork item interprets its source. This is `ImportOptions`
/// split at the collection boundary: interpretation here, placement on the
/// item. Reassembling both through [`ArtworkItem::import_options`] is the
/// only way the current SVG importer is called, so a placement can never be
/// applied twice or stay editable in two places.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SvgInterpretation {
    pub geometry_tolerance_mm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks_per_mm: Option<f64>,
    /// Centerline mode is omitted while it is the default, mirroring the
    /// frozen schema-4 spelling of the same setting.
    #[serde(default, skip_serializing_if = "is_default_mode")]
    pub mode: ImportMode,
}
fn is_default_mode(mode: &ImportMode) -> bool {
    *mode == ImportMode::Fill
}
impl Default for SvgInterpretation {
    fn default() -> Self {
        Self {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            mode: ImportMode::Fill,
        }
    }
}
impl SvgInterpretation {
    fn validate(&self) -> Result<()> {
        if !self.geometry_tolerance_mm.is_finite() || self.geometry_tolerance_mm <= 0. {
            return Err(error(
                "PROJECT_PARAMETER",
                "geometry_tolerance_mm must be finite and positive",
            ));
        }
        number(self.ticks_per_mm, "import_settings.ticks_per_mm", true)
    }
}

/// Embedded artwork content. A tagged enum so a future Sketch item can
/// produce the same catalogue; only `Svg` exists today and unsupported kinds
/// are rejected by strict parsing rather than treated as empty items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtworkContent {
    Svg(SourceSnapshot),
}

impl ArtworkContent {
    fn svg(&self) -> &SourceSnapshot {
        match self {
            Self::Svg(snapshot) => snapshot,
        }
    }
}

/// One independently placed artwork item. Items stay logically independent
/// even when their embedded bytes are identical; display name and placement
/// never participate in the source revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtworkItem {
    pub id: ArtworkItemId,
    pub name: String,
    pub content: ArtworkContent,
    pub import_settings: SvgInterpretation,
    pub placement: Placement,
}

impl ArtworkItem {
    /// The temporary [`ImportOptions`] handed to the current SVG importer:
    /// this item's interpretation joined with this item's placement.
    pub fn import_options(&self) -> ImportOptions {
        ImportOptions {
            geometry_tolerance_mm: self.import_settings.geometry_tolerance_mm,
            ticks_per_mm: self.import_settings.ticks_per_mm,
            placement: self.placement.clone(),
            mode: self.import_settings.mode,
        }
    }

    /// Placement-independent identity of the exact embedded content plus its
    /// interpretation (plan section 22.3). It excludes the display name and
    /// placement: renaming or moving an item never invalidates references,
    /// while any content or interpretation change does, including edits
    /// smaller than the contour fingerprint grid.
    pub fn source_revision(&self) -> Result<SourceRevision> {
        let digest = crate::plan_hash::hash(&(&self.content.svg().svg, &self.import_settings))
            .map_err(|e| error("PROJECT_JSON", e.to_string()))?;
        Ok(SourceRevision {
            content_digest: digest,
            algorithm_version: SOURCE_REVISION_ALGORITHM_VERSION,
        })
    }

    fn validate(&self) -> Result<()> {
        self.id.validate()?;
        short_label(&self.name, "artwork item name")?;
        let snapshot = self.content.svg();
        short_label(&snapshot.filename, "artwork source filename")?;
        if snapshot.svg.len() > crate::svg::MAX_SVG_BYTES {
            return Err(error(
                "PROJECT_RESOURCE_LIMIT",
                format!(
                    "artwork item '{}' exceeds the {} byte per-item source limit",
                    self.id.0,
                    crate::svg::MAX_SVG_BYTES
                ),
            ));
        }
        self.import_settings.validate()?;
        let placement = &self.placement;
        if !placement.origin_mm.finite()
            || !placement.scale.is_finite()
            || placement.scale <= 0.
            || !placement.rotation_deg.is_finite()
        {
            return Err(error(
                "PROJECT_PARAMETER",
                format!(
                    "artwork item '{}' requires a finite origin/rotation and positive scale",
                    self.id.0
                ),
            ));
        }
        Ok(())
    }
}

/// Identity of an item's exact embedded content and interpretation at the
/// time geometry was assigned. Stored inside every [`GeometryRef`] so a
/// replaced source keeps its references dangling until explicitly reattached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRevision {
    pub content_digest: String,
    pub algorithm_version: u32,
}

/// Which local geometry space a reference addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryRefKind {
    /// A filled component of the item's fill import (Flat V-carve selection).
    FilledComponent,
    /// A closed boundary contour from the item's contour catalogue.
    ClosedContour,
    /// A centerline chain from the item's centerline import (knife).
    Centerline,
}

/// The canonical document representation of one selected geometry (plan
/// section 22.3): an owning artwork item, the local geometry space and ID,
/// and the source revision the selection was made against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryRef {
    pub artwork_item_id: ArtworkItemId,
    pub kind: GeometryRefKind,
    pub local_geometry_id: String,
    pub source_revision: SourceRevision,
}

impl GeometryRef {
    /// Well-formedness only: a bounded local key and a present revision.
    /// Local ID spaces (including the importer's `shape::0` component
    /// spelling) are owned by the item's catalogue derivation, not the
    /// document alphabet; where the referenced geometry actually exists is
    /// reference inspection, so a typed dangling reference stays saveable.
    fn validate(&self) -> Result<()> {
        self.artwork_item_id.validate()?;
        if self.local_geometry_id.is_empty() || self.local_geometry_id.len() > 256 {
            return Err(error(
                "PROJECT_PARAMETER",
                "local geometry IDs must be nonempty and at most 256 characters",
            ));
        }
        if self.source_revision.content_digest.trim().is_empty() {
            return Err(error(
                "PROJECT_PARAMETER",
                "geometry references require a source revision digest",
            ));
        }
        Ok(())
    }
}

/// Start or tab anchor in the collection model: parameterized by qualified
/// source geometry, never by offset vertices. The stored fingerprint is
/// contour-level evidence carried from assignment time; the binding source
/// revision lives on the inner [`GeometryRef`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContourAnchorV5 {
    pub geometry: GeometryRef,
    pub source_geometry_fingerprint: String,
    /// Fraction in [0,1) along the source contour.
    pub fraction_along_source_contour: f64,
}

impl ContourAnchorV5 {
    fn validate(&self) -> Result<()> {
        self.geometry.validate()?;
        if self.source_geometry_fingerprint.trim().is_empty() {
            return Err(error(
                "PROJECT_PARAMETER",
                "anchors require a source geometry fingerprint",
            ));
        }
        let f = self.fraction_along_source_contour;
        if !f.is_finite() || !(0. ..1.).contains(&f) {
            return Err(error(
                "PROJECT_PARAMETER",
                "anchor fraction must lie in [0,1)",
            ));
        }
        Ok(())
    }
}

/// Where a job tool's geometry was copied from. Provenance is display and
/// reapplication metadata only: missing records never invalidate otherwise
/// valid copied machining values (plan section 22.6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryOrigin {
    pub library_id: String,
    pub tool_id: String,
    pub copied_revision: u64,
    pub name_at_copy: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobToolV5 {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub geometry: Option<crate::project::ToolGeometry>,
    #[serde(default)]
    pub capabilities: crate::project::ToolCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_origin: Option<LibraryOrigin>,
}

impl JobToolV5 {
    fn validate(&self) -> Result<()> {
        if !preview::valid_id(&self.id) {
            return Err(error(
                "PROJECT_TOOL_ID",
                "job tool IDs must be valid identifiers",
            ));
        }
        short_label(&self.name, "job tool name")?;
        if let Some(geometry) = &self.geometry {
            geometry.validate()?;
        }
        if let Some(origin) = &self.library_origin {
            short_label(&origin.library_id, "library_origin.library_id")?;
            short_label(&origin.tool_id, "library_origin.tool_id")?;
            short_label(&origin.name_at_copy, "library_origin.name_at_copy")?;
        }
        Ok(())
    }
}

/// Typed complete applicable cutting fields copied when a profile was
/// applied, including unset values: a missing field in the baseline is a
/// meaningful "was not supplied", never a zero. There is deliberately no
/// default baseline: application always copies real values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CuttingBaseline {
    Milling {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spindle_rpm: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cutting_feed_mm_min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plunge_feed_mm_min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_stepdown_mm: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stepover_mm: Option<f64>,
    },
    Knife {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cutting_feed_mm_min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plunge_feed_mm_min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        swivel_feed_mm_min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_stepdown_mm: Option<f64>,
    },
}

impl CuttingBaseline {
    fn validate(&self) -> Result<()> {
        let values: Vec<Option<f64>> = match self {
            Self::Milling {
                spindle_rpm,
                cutting_feed_mm_min,
                plunge_feed_mm_min,
                max_stepdown_mm,
                stepover_mm,
            } => vec![
                *spindle_rpm,
                *cutting_feed_mm_min,
                *plunge_feed_mm_min,
                *max_stepdown_mm,
                *stepover_mm,
            ],
            Self::Knife {
                cutting_feed_mm_min,
                plunge_feed_mm_min,
                swivel_feed_mm_min,
                max_stepdown_mm,
            } => vec![
                *cutting_feed_mm_min,
                *plunge_feed_mm_min,
                *swivel_feed_mm_min,
                *max_stepdown_mm,
            ],
        };
        for value in values {
            // Baselines copy library preset values, which may legitimately
            // be zero-or-unset markers; they must stay finite and nonnegative.
            if value.is_some_and(|v| !v.is_finite() || v < 0.) {
                return Err(error(
                    "PROJECT_PARAMETER",
                    "applied-profile baselines must be finite and nonnegative",
                ));
            }
        }
        Ok(())
    }
}

/// Copied application metadata for one assignment (plan section 22.6). The
/// baseline is authoritative for Reset/Modified; reapplying loads a current
/// revision explicitly and stores it as a new baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfile {
    pub library_id: String,
    pub library_tool_id: String,
    pub preset_id: String,
    pub revision_at_application: u64,
    pub name_at_application: String,
    pub baseline: CuttingBaseline,
}

impl AppliedProfile {
    fn validate(&self) -> Result<()> {
        short_label(&self.library_id, "applied_profile.library_id")?;
        short_label(&self.library_tool_id, "applied_profile.library_tool_id")?;
        short_label(&self.preset_id, "applied_profile.preset_id")?;
        short_label(
            &self.name_at_application,
            "applied_profile.name_at_application",
        )?;
        self.baseline.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MillingAssignmentV5 {
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spindle_rpm: Option<f64>,
    /// Unset for migrated legacy jobs until explicitly supplied; required
    /// before new Face/Profile planning and before process export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spindle_direction: Option<crate::project::SpindleDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutting_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plunge_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_profile: Option<AppliedProfile>,
}

impl MillingAssignmentV5 {
    fn validate(&self, prefix: &str) -> Result<()> {
        if !preview::valid_id(&self.tool_id) {
            return Err(error(
                "PROJECT_PARAMETER",
                format!("{prefix}.tool_id must be a valid identifier"),
            ));
        }
        for (value, name) in [
            (self.spindle_rpm, "spindle_rpm"),
            (self.cutting_feed_mm_min, "cutting_feed_mm_min"),
            (self.plunge_feed_mm_min, "plunge_feed_mm_min"),
            (self.max_stepdown_mm, "max_stepdown_mm"),
            (self.stepover_mm, "stepover_mm"),
        ] {
            number(value, &format!("{prefix}.{name}"), true)?;
        }
        if let Some(profile) = &self.applied_profile {
            profile.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnifeAssignmentV5 {
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutting_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plunge_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swivel_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_profile: Option<AppliedProfile>,
}

impl KnifeAssignmentV5 {
    fn validate(&self, prefix: &str) -> Result<()> {
        if !preview::valid_id(&self.tool_id) {
            return Err(error(
                "PROJECT_PARAMETER",
                format!("{prefix}.tool_id must be a valid identifier"),
            ));
        }
        for (value, name) in [
            (self.cutting_feed_mm_min, "cutting_feed_mm_min"),
            (self.plunge_feed_mm_min, "plunge_feed_mm_min"),
            (self.swivel_feed_mm_min, "swivel_feed_mm_min"),
            (self.max_stepdown_mm, "max_stepdown_mm"),
        ] {
            number(value, &format!("{prefix}.{name}"), true)?;
        }
        if let Some(profile) = &self.applied_profile {
            profile.validate()?;
        }
        Ok(())
    }
}

/// One job tool's controller mapping inside the applied machine
/// configuration. Mapping rows may exist for tools outside the current
/// executable scope; dangling rows are repairable draft issues, not
/// document errors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedToolMapping {
    pub job_tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_offset_number: Option<u32>,
}

/// Copied origin of the applied configuration (display only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationOrigin {
    pub configuration_id: String,
    pub name: String,
}

/// The one portable applied machine configuration (plan section 22.7). It is
/// the only object addressed by Setup → Machine, mapping readouts and
/// export. Preparation fields may stay unset: the H4 resolver turns a
/// complete snapshot into the validated schema-2 profile required for
/// export, and an incomplete snapshot never blocks saving the job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedMachineConfiguration {
    pub origin: ConfigurationOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance_z_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimal_places: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_start_position_mm: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_compensation: Option<LengthCompensation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_control: Option<PathControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spindle_spinup_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coolant: Option<Coolant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m6: Option<M6Contract>,
    #[serde(default)]
    pub tools: Vec<AppliedToolMapping>,
}

impl AppliedMachineConfiguration {
    fn validate(&self) -> Result<()> {
        short_label(
            &self.origin.configuration_id,
            "machine_configuration.origin.id",
        )?;
        short_label(&self.origin.name, "machine_configuration.origin.name")?;
        if let Some(offset) = &self.work_offset
            && !matches!(
                offset.as_str(),
                "G54" | "G55" | "G56" | "G57" | "G58" | "G59" | "G59.1" | "G59.2" | "G59.3"
            )
        {
            return Err(error(
                "PROJECT_MACHINE",
                "unsupported work offset in the applied machine configuration",
            ));
        }
        number(
            self.clearance_z_mm,
            "machine_configuration.clearance_z_mm",
            true,
        )?;
        number(
            self.spindle_spinup_seconds,
            "machine_configuration.spindle_spinup_seconds",
            false,
        )?;
        if let Some(PathControl::Blend {
            tolerance_mm,
            naive_cam_tolerance_mm,
        }) = &self.path_control
        {
            number(
                Some(*tolerance_mm),
                "machine_configuration.path_control.tolerance_mm",
                true,
            )?;
            if let Some(q) = naive_cam_tolerance_mm
                && q > tolerance_mm
            {
                return Err(error(
                    "PROJECT_MACHINE",
                    "naive CAM tolerance may not exceed the blend tolerance",
                ));
            }
        }
        let mut mapped = BTreeSet::new();
        for mapping in &self.tools {
            if !preview::valid_id(&mapping.job_tool_id)
                || !mapped.insert(mapping.job_tool_id.as_str())
            {
                return Err(error(
                    "PROJECT_MACHINE",
                    format!(
                        "machine mappings must reference valid job tool IDs exactly once; '{}' does not",
                        mapping.job_tool_id
                    ),
                ));
            }
            if mapping.tool_number == Some(0) {
                return Err(error(
                    "PROJECT_MACHINE",
                    format!("tool number for '{}' must be positive", mapping.job_tool_id),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatVcarveSettingsV5 {
    /// Selected filled components as qualified references; may be empty
    /// while the job is incomplete.
    pub components: Vec<GeometryRef>,
    pub mode: crate::project::FlatVcarveMode,
    pub endmill: MillingAssignmentV5,
    /// Kept even in endmill-only mode: its geometry defines the nominal
    /// V-shaped target the rough stage clears toward.
    pub vbit: MillingAssignmentV5,
    /// Where carving starts: the original stock top by default, or a plane
    /// published by a preceding face operation. Depth is measured below it.
    #[serde(default)]
    pub top: crate::project::HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_allowance_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_floor_ridge_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_detail_residual_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rough: Option<crate::project::FlatVcarveRoughSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<crate::vcarve::VBitPlanningSettings>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceSettingsV5 {
    pub area: crate::project::FaceArea,
    #[serde(default)]
    pub margins: crate::project::FaceMargins,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_overrun_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_overrun_mm: Option<f64>,
    pub top: crate::project::HeightRef,
    pub bottom: crate::project::HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_angle_deg: Option<f64>,
    #[serde(default)]
    pub pattern: crate::project::FacePattern,
    pub assignment: MillingAssignmentV5,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileContourV5 {
    pub geometry: GeometryRef,
    pub side: crate::project::ContourSide,
    /// Required for on-contour selections, which have no retained side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traversal: Option<crate::project::TraversalDirection>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StartSelectionV5 {
    #[default]
    Automatic,
    Anchor(Box<ContourAnchorV5>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TabPlacementV5 {
    Automatic {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spacing_mm: Option<f64>,
    },
    Manual {
        anchors: Vec<ContourAnchorV5>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabSettingsV5 {
    pub height_mm: Option<f64>,
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub shape: crate::project::TabShape,
    pub placement: TabPlacementV5,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettingsV5 {
    pub contours: Vec<ProfileContourV5>,
    pub assignment: MillingAssignmentV5,
    pub top: crate::project::HeightRef,
    pub bottom: crate::project::HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_cut_allowance_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<crate::project::CutDirection>,
    #[serde(default)]
    pub order: crate::project::ContourOrder,
    #[serde(default)]
    pub start: StartSelectionV5,
    #[serde(default)]
    pub finish: crate::project::ProfileFinishSettings,
    #[serde(default)]
    pub entry: crate::project::ProfileEntry,
    #[serde(default)]
    pub lead_in: crate::project::LeadSpec,
    #[serde(default)]
    pub lead_out: crate::project::LeadSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<TabSettingsV5>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DragKnifeSettingsV5 {
    /// Selected centerline chains as qualified references; may be empty
    /// while incomplete.
    pub chains: Vec<GeometryRef>,
    pub assignment: KnifeAssignmentV5,
    pub top: crate::project::HeightRef,
    pub bottom: crate::project::HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swivel_depth_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_threshold_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_cut_allowance_mm: Option<f64>,
    #[serde(default)]
    pub start: StartSelectionV5,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_overlap_mm: Option<f64>,
    #[serde(default)]
    pub alignment: crate::project::KnifeAlignment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "settings",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OperationSettingsV5 {
    FlatVcarve(FlatVcarveSettingsV5),
    Face(FaceSettingsV5),
    Profile(ProfileSettingsV5),
    DragKnife(DragKnifeSettingsV5),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationV5 {
    pub id: String,
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub settings: OperationSettingsV5,
}
fn default_enabled() -> bool {
    true
}

/// The canonical schema-5 CAM job. The `operations` vector alone orders
/// machining; the artwork collection has no machining order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CamJobV5 {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub setup: crate::project::SetupSettings,
    /// The artwork collection; empty for source-free (e.g. face-only) jobs.
    /// No top-level `source`/`import` fallback authorities exist.
    #[serde(default)]
    pub artwork: Vec<ArtworkItem>,
    pub tools: Vec<JobToolV5>,
    pub operations: Vec<OperationV5>,
    #[serde(default)]
    pub tolerances: PlanningTolerances,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_configuration: Option<AppliedMachineConfiguration>,
    /// Preserved legacy descriptive metadata; never fabricated into an
    /// applied machine configuration and never a reviewed export contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_machine_profile: Option<MachineProfile>,
}

fn validate_flat_vcarve(settings: &FlatVcarveSettingsV5, id: &str) -> Result<()> {
    settings.endmill.validate("flat_vcarve.endmill")?;
    settings.vbit.validate("flat_vcarve.vbit")?;
    number(settings.max_depth_mm, "flat_vcarve.max_depth_mm", true)?;
    number(
        settings.wall_allowance_mm,
        "flat_vcarve.wall_allowance_mm",
        false,
    )?;
    number(
        settings.max_floor_ridge_mm,
        "flat_vcarve.max_floor_ridge_mm",
        false,
    )?;
    number(
        settings.max_detail_residual_mm,
        "flat_vcarve.max_detail_residual_mm",
        false,
    )?;
    if let Some(rough) = &settings.rough {
        rough.validate()?;
    }
    match (settings.mode, &settings.finish) {
        (crate::project::FlatVcarveMode::Combined, Some(finish)) => finish.validate()?,
        (crate::project::FlatVcarveMode::Combined, None) => {
            return Err(error(
                "PROJECT_OPERATION",
                format!("operation '{id}' is combined mode and requires V-bit finish settings"),
            ));
        }
        (crate::project::FlatVcarveMode::EndmillOnly, None) => {}
        // Retain inactive finishing policy in portable jobs. Mode remains the
        // execution authority; the legacy execution adapter omits this policy.
        (crate::project::FlatVcarveMode::EndmillOnly, Some(finish)) => finish.validate()?,
    }
    for component in &settings.components {
        component.validate()?;
    }
    Ok(())
}

fn validate_face(settings: &FaceSettingsV5) -> Result<()> {
    if let crate::project::FaceArea::Rectangle { rect } = &settings.area {
        rect.validate()?;
    }
    number(settings.entry_overrun_mm, "face.entry_overrun_mm", false)?;
    number(settings.exit_overrun_mm, "face.exit_overrun_mm", false)?;
    for (value, name) in [
        (settings.margins.min_x_mm, "face.margins.min_x_mm"),
        (settings.margins.max_x_mm, "face.margins.max_x_mm"),
        (settings.margins.min_y_mm, "face.margins.min_y_mm"),
        (settings.margins.max_y_mm, "face.margins.max_y_mm"),
    ] {
        number(value, name, false)?;
    }
    number(settings.stepdown_mm, "face.stepdown_mm", true)?;
    number(settings.stepover_mm, "face.stepover_mm", true)?;
    if settings.pass_angle_deg.is_some_and(|v| !v.is_finite()) {
        return Err(error(
            "PROJECT_PARAMETER",
            "face.pass_angle_deg must be finite",
        ));
    }
    settings.assignment.validate("face.assignment")
}

fn validate_tabs(tabs: &TabSettingsV5) -> Result<()> {
    number(tabs.height_mm, "tabs.height_mm", true)?;
    number(tabs.width_mm, "tabs.width_mm", true)?;
    if let TabPlacementV5::Automatic {
        count,
        spacing_mm: spacing,
    } = &tabs.placement
    {
        if count.is_none() && spacing.is_none() {
            return Err(error(
                "PROJECT_PARAMETER",
                "automatic tab placement requires a count or a spacing",
            ));
        }
        number(*spacing, "tabs.placement.spacing_mm", true)?;
    }
    if let TabPlacementV5::Manual { anchors } = &tabs.placement {
        for anchor in anchors {
            anchor.validate()?;
        }
    }
    Ok(())
}

fn validate_profile(settings: &ProfileSettingsV5, id: &str) -> Result<()> {
    for contour in &settings.contours {
        contour.geometry.validate()?;
        if contour.side == crate::project::ContourSide::On && contour.traversal.is_none() {
            return Err(error(
                "PROJECT_PARAMETER",
                format!(
                    "operation '{id}' has an on-contour selection that requires an explicit traversal direction"
                ),
            ));
        }
    }
    number(settings.stepdown_mm, "profile.stepdown_mm", true)?;
    number(
        settings.through_cut_allowance_mm,
        "profile.through_cut_allowance_mm",
        false,
    )?;
    settings.entry.validate()?;
    settings.lead_in.validate("profile.lead_in")?;
    settings.lead_out.validate("profile.lead_out")?;
    if let Some(tabs) = &settings.tabs {
        validate_tabs(tabs)?;
    }
    if settings.finish.enabled {
        number(
            settings.finish.radial_allowance_mm,
            "finish.radial_allowance_mm",
            false,
        )?;
        number(settings.finish.feed_mm_min, "finish.feed_mm_min", true)?;
    }
    settings.assignment.validate("profile.assignment")?;
    if let StartSelectionV5::Anchor(anchor) = &settings.start {
        anchor.validate()?;
    }
    Ok(())
}

fn validate_drag_knife(settings: &DragKnifeSettingsV5) -> Result<()> {
    for chain in &settings.chains {
        chain.validate()?;
    }
    number(settings.stepdown_mm, "knife.stepdown_mm", true)?;
    number(settings.swivel_depth_mm, "knife.swivel_depth_mm", true)?;
    number(
        settings.corner_threshold_deg,
        "knife.corner_threshold_deg",
        true,
    )?;
    number(
        settings.through_cut_allowance_mm,
        "knife.through_cut_allowance_mm",
        false,
    )?;
    number(
        settings.closure_overlap_mm,
        "knife.closure_overlap_mm",
        false,
    )?;
    if settings
        .alignment
        .initial_heading_deg
        .is_some_and(|v| !v.is_finite())
    {
        return Err(error(
            "PROJECT_PARAMETER",
            "knife.alignment.initial_heading_deg must be finite",
        ));
    }
    settings.assignment.validate("knife.assignment")?;
    if let StartSelectionV5::Anchor(anchor) = &settings.start {
        anchor.validate()?;
    }
    Ok(())
}

impl CamJobV5 {
    /// Structural document integrity (plan section 22.5): the save/open
    /// gate. Everything about *where* referenced entities live is reference
    /// inspection instead, so well-formed dangling references and forward
    /// face references stay saveable.
    pub fn validate_structure(&self) -> Result<()> {
        if self.schema_version != CAM_JOB_V5_SCHEMA_VERSION {
            return Err(error(
                "CAM_JOB_SCHEMA_VERSION",
                "unsupported or missing CamJob schema version",
            ));
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err(error(
                "PROJECT_NAME",
                "job names must be short and nonempty",
            ));
        }
        let mut item_ids = BTreeSet::new();
        for item in &self.artwork {
            item.validate()?;
            if !item_ids.insert(item.id.0.as_str()) {
                return Err(error(
                    "PROJECT_ARTWORK_ID",
                    "artwork item IDs must be unique",
                ));
            }
        }
        let mut tool_ids = BTreeSet::new();
        for tool in &self.tools {
            tool.validate()?;
            if !tool_ids.insert(tool.id.as_str()) {
                return Err(error(
                    "PROJECT_TOOL_ID",
                    "job tool IDs must be valid and unique",
                ));
            }
        }
        let mut operation_ids = BTreeSet::new();
        for operation in &self.operations {
            if !preview::valid_id(&operation.id) || !operation_ids.insert(operation.id.as_str()) {
                return Err(error(
                    "PROJECT_OPERATION_ID",
                    "operation IDs must be valid and unique",
                ));
            }
            short_label(&operation.name, "operation name")?;
            let id = operation.id.as_str();
            let (top, bottom) = match &operation.settings {
                OperationSettingsV5::FlatVcarve(settings) => {
                    validate_flat_vcarve(settings, id)?;
                    (Some(&settings.top), None)
                }
                OperationSettingsV5::Face(settings) => {
                    validate_face(settings)?;
                    (Some(&settings.top), Some(&settings.bottom))
                }
                OperationSettingsV5::Profile(settings) => {
                    validate_profile(settings, id)?;
                    (Some(&settings.top), Some(&settings.bottom))
                }
                OperationSettingsV5::DragKnife(settings) => {
                    validate_drag_knife(settings)?;
                    (Some(&settings.top), Some(&settings.bottom))
                }
            };
            if let Some(top) = top {
                top.validate("top", false)?;
            }
            if let Some(bottom) = bottom {
                bottom.validate("bottom", true)?;
            }
        }
        self.setup.validate()?;
        number(
            self.tolerances.motion_tolerance_mm,
            "tolerances.motion_tolerance_mm",
            true,
        )?;
        number(
            self.tolerances.verification_tolerance_mm,
            "tolerances.verification_tolerance_mm",
            true,
        )?;
        if let Some(profile) = &self.legacy_machine_profile {
            // Reuse the frozen schema-4 legacy metadata validation unchanged.
            super::validate_machine_profile(profile)?;
        }
        if let Some(configuration) = &self.machine_configuration {
            configuration.validate()?;
        }
        Ok(())
    }

    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > MAX_JOB_BYTES {
            return Err(error(
                "PROJECT_RESOURCE_LIMIT",
                "job exceeds the 64 MB input limit",
            ));
        }
        let job: Self =
            serde_json::from_str(json).map_err(|e| error("PROJECT_JSON", e.to_string()))?;
        job.validate_structure()?;
        Ok(job)
    }

    pub fn to_json(&self) -> Result<String> {
        self.validate_structure()?;
        let json = serde_json::to_string_pretty(self)
            .map(|s| s + "\n")
            .map_err(|e| error("PROJECT_JSON", e.to_string()))?;
        // The aggregate boundary is what actually protects the job: per-item
        // content limits alone do not bound the collection (plan section
        // 22.4). Measuring actual serialized bytes keeps JSON overhead and
        // every item in the accounting.
        if json.len() > MAX_JOB_BYTES {
            return Err(error(
                "PROJECT_RESOURCE_LIMIT",
                format!(
                    "the serialized job exceeds the {} byte document limit",
                    MAX_JOB_BYTES
                ),
            ));
        }
        Ok(json)
    }
}
