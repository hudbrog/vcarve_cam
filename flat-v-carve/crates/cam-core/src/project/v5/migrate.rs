//! Migration of validated schema-4 documents (and, through the existing
//! compatibility step, legacy schema-1/2/3 jobs) into the canonical schema-5
//! collection model (plan section 22.3, migration rules 1-7).
//!
//! The frozen schema-4 DTO/parser is the migration input and is never
//! replaced here. Missing machining values stay missing; migration invents no
//! provenance, no applied machine configuration and no geometry. Selections
//! and anchors are rewritten through an explicit local-ID -> qualified
//! reference map built from the migrated snapshot; IDs the snapshot cannot
//! resolve are preserved as typed dangling references that inspection
//! reports, never silently dropped or repaired.
use super::{
    ArtworkContent, ArtworkItem, ArtworkItemId, CAM_JOB_V5_SCHEMA_VERSION, CamJobV5,
    ContourAnchorV5, DragKnifeSettingsV5, FaceSettingsV5, FlatVcarveSettingsV5, GeometryRef,
    GeometryRefKind, JobToolV5, KnifeAssignmentV5, MIGRATED_ARTWORK_ITEM_ID, MillingAssignmentV5,
    OperationSettingsV5, OperationV5, ProfileContourV5, ProfileSettingsV5, SourceRevision,
    StartSelectionV5, TabPlacementV5, TabSettingsV5, error,
};
use crate::{
    geometry::Result,
    project::{
        CamJob, ContourAnchor, DragKnifeSettings, FaceSettings, FlatVcarveSettings,
        KnifeAssignment, MillingAssignment, ProfileContour, ProfileSettings, TabPlacement,
        TabSettings,
    },
};
use std::collections::BTreeMap;

fn migrate_assignment(assignment: &MillingAssignment) -> MillingAssignmentV5 {
    MillingAssignmentV5 {
        tool_id: assignment.tool_id.clone(),
        spindle_rpm: assignment.spindle_rpm,
        spindle_direction: assignment.spindle_direction,
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
        stepover_mm: assignment.stepover_mm,
        // Migration cannot invent provenance: no profile was ever applied
        // through the schema-4 command surface.
        applied_profile: None,
    }
}

fn migrate_knife_assignment(assignment: &KnifeAssignment) -> KnifeAssignmentV5 {
    KnifeAssignmentV5 {
        tool_id: assignment.tool_id.clone(),
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        swivel_feed_mm_min: assignment.swivel_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
        applied_profile: None,
    }
}

/// The explicit old-ID -> qualified-reference map over the migrated snapshot
/// (plan section 22.3, rule 3). When the source cannot build a catalogue the
/// maps stay empty and every selection migrates as a typed dangling
/// reference; inspection reports the item's import failure.
struct ReferenceMaps {
    revision: SourceRevision,
    components: BTreeMap<String, GeometryRef>,
    contours: BTreeMap<String, GeometryRef>,
    chains: BTreeMap<String, GeometryRef>,
}

impl ReferenceMaps {
    fn build(item: &ArtworkItem) -> Result<Self> {
        let revision = item.source_revision()?;
        let mut maps = Self {
            revision: revision.clone(),
            components: BTreeMap::new(),
            contours: BTreeMap::new(),
            chains: BTreeMap::new(),
        };
        let Ok(catalogue) = super::artwork::item_catalogue(item) else {
            return Ok(maps);
        };
        let reference = |kind, local: &str| GeometryRef {
            artwork_item_id: item.id.clone(),
            kind,
            local_geometry_id: local.to_string(),
            source_revision: revision.clone(),
        };
        for contour in &catalogue.contours {
            maps.contours.insert(
                contour.id.clone(),
                reference(GeometryRefKind::ClosedContour, &contour.id),
            );
            // Filled components are selected by the importer's component ID;
            // every component owns exactly one outer contour.
            maps.components.insert(
                contour.component_id.clone(),
                reference(GeometryRefKind::FilledComponent, &contour.component_id),
            );
        }
        for chain in &catalogue.open_chains {
            maps.chains.insert(
                chain.id.clone(),
                reference(GeometryRefKind::Centerline, &chain.id),
            );
        }
        Ok(maps)
    }

    /// Look a local ID up, preserving it as a typed dangling reference of
    /// the requested kind when the snapshot cannot resolve it.
    fn rewrite(&self, local_id: &str, kind: GeometryRefKind) -> GeometryRef {
        let found = match kind {
            GeometryRefKind::FilledComponent => self.components.get(local_id),
            GeometryRefKind::ClosedContour => self.contours.get(local_id),
            GeometryRefKind::Centerline => self.chains.get(local_id),
        };
        found.cloned().unwrap_or_else(|| GeometryRef {
            artwork_item_id: ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
            kind,
            local_geometry_id: local_id.to_string(),
            source_revision: self.revision.clone(),
        })
    }

    /// Anchors may address closed contours or centerline chains; unresolved
    /// IDs keep the context-appropriate kind.
    fn rewrite_anchor(&self, anchor: &ContourAnchor, fallback: GeometryRefKind) -> ContourAnchorV5 {
        let geometry = self
            .contours
            .get(&anchor.contour_id)
            .or_else(|| self.chains.get(&anchor.contour_id))
            .cloned()
            .unwrap_or_else(|| self.rewrite(&anchor.contour_id, fallback));
        // The stored fingerprint is validated against the migrated snapshot
        // by inspection; a mismatch stays unresolved instead of being
        // silently repaired (rule 4).
        ContourAnchorV5 {
            geometry,
            source_geometry_fingerprint: anchor.source_geometry_fingerprint.clone(),
            fraction_along_source_contour: anchor.fraction_along_source_contour,
        }
    }
}

fn migrate_start(
    start: &crate::project::StartSelection,
    maps: &ReferenceMaps,
    fallback: GeometryRefKind,
) -> StartSelectionV5 {
    match start {
        crate::project::StartSelection::Automatic => StartSelectionV5::Automatic,
        crate::project::StartSelection::Anchor(anchor) => {
            StartSelectionV5::Anchor(Box::new(maps.rewrite_anchor(anchor, fallback)))
        }
    }
}

fn migrate_tabs(tabs: &TabSettings, maps: &ReferenceMaps) -> TabSettingsV5 {
    TabSettingsV5 {
        height_mm: tabs.height_mm,
        width_mm: tabs.width_mm,
        shape: tabs.shape,
        placement: match &tabs.placement {
            TabPlacement::Automatic {
                count,
                spacing_mm: spacing,
            } => TabPlacementV5::Automatic {
                count: *count,
                spacing_mm: *spacing,
            },
            TabPlacement::Manual { anchors } => TabPlacementV5::Manual {
                anchors: anchors
                    .iter()
                    .map(|a| maps.rewrite_anchor(a, GeometryRefKind::ClosedContour))
                    .collect(),
            },
        },
    }
}

fn migrate_flat_vcarve(
    settings: &FlatVcarveSettings,
    maps: &ReferenceMaps,
) -> FlatVcarveSettingsV5 {
    FlatVcarveSettingsV5 {
        components: settings
            .component_ids
            .iter()
            .map(|id| maps.rewrite(id, GeometryRefKind::FilledComponent))
            .collect(),
        mode: settings.mode,
        endmill: migrate_assignment(&settings.endmill),
        vbit: migrate_assignment(&settings.vbit),
        top: settings.top.clone(),
        max_depth_mm: settings.max_depth_mm,
        wall_allowance_mm: settings.wall_allowance_mm,
        max_floor_ridge_mm: settings.max_floor_ridge_mm,
        max_detail_residual_mm: settings.max_detail_residual_mm,
        rough: settings.rough.clone(),
        finish: settings.finish.clone(),
    }
}

fn migrate_face(settings: &FaceSettings) -> FaceSettingsV5 {
    FaceSettingsV5 {
        area: settings.area.clone(),
        margins: settings.margins,
        entry_overrun_mm: settings.entry_overrun_mm,
        exit_overrun_mm: settings.exit_overrun_mm,
        top: settings.top.clone(),
        bottom: settings.bottom.clone(),
        stepdown_mm: settings.stepdown_mm,
        stepover_mm: settings.stepover_mm,
        pass_angle_deg: settings.pass_angle_deg,
        pattern: settings.pattern,
        assignment: migrate_assignment(&settings.assignment),
    }
}

fn migrate_profile(settings: &ProfileSettings, maps: &ReferenceMaps) -> ProfileSettingsV5 {
    ProfileSettingsV5 {
        contours: settings
            .contours
            .iter()
            .map(
                |ProfileContour {
                     contour_id,
                     side,
                     traversal,
                 }| ProfileContourV5 {
                    geometry: maps.rewrite(contour_id, GeometryRefKind::ClosedContour),
                    side: *side,
                    traversal: *traversal,
                },
            )
            .collect(),
        assignment: migrate_assignment(&settings.assignment),
        top: settings.top.clone(),
        bottom: settings.bottom.clone(),
        stepdown_mm: settings.stepdown_mm,
        through_cut_allowance_mm: settings.through_cut_allowance_mm,
        direction: settings.direction,
        order: settings.order,
        start: migrate_start(&settings.start, maps, GeometryRefKind::ClosedContour),
        finish: settings.finish.clone(),
        entry: settings.entry.clone(),
        lead_in: settings.lead_in.clone(),
        lead_out: settings.lead_out.clone(),
        tabs: settings.tabs.as_ref().map(|t| migrate_tabs(t, maps)),
    }
}

fn migrate_knife(settings: &DragKnifeSettings, maps: &ReferenceMaps) -> DragKnifeSettingsV5 {
    DragKnifeSettingsV5 {
        chains: settings
            .chains
            .iter()
            .map(|id| maps.rewrite(id, GeometryRefKind::Centerline))
            .collect(),
        assignment: migrate_knife_assignment(&settings.assignment),
        top: settings.top.clone(),
        bottom: settings.bottom.clone(),
        stepdown_mm: settings.stepdown_mm,
        swivel_depth_mm: settings.swivel_depth_mm,
        corner_threshold_deg: settings.corner_threshold_deg,
        through_cut_allowance_mm: settings.through_cut_allowance_mm,
        start: migrate_start(&settings.start, maps, GeometryRefKind::Centerline),
        closure_overlap_mm: settings.closure_overlap_mm,
        alignment: settings.alignment.clone(),
    }
}

/// Migrate a validated schema-4 job to the schema-5 collection model.
///
/// A source-free job becomes an empty collection. A job with a source gets
/// one deterministic migration item with the exact embedded bytes, the
/// original interpretation and placement. Operation/tool IDs, order, enable
/// state and all cutting values are preserved; no applied machine
/// configuration is fabricated from `legacy_machine_profile` (rule 5).
///
/// Schema-4 documents whose operations carry selections without any source
/// (structurally valid but unplannable in schema 4) keep those selections as
/// references to the deterministic missing migration item, which inspection
/// reports exactly like any other deleted artwork.
pub fn migrate_v4(job: &CamJob) -> Result<CamJobV5> {
    job.validate()?;
    let artwork = job.source.as_ref().map(|source| {
        let name = if source.filename.trim().is_empty() {
            "Artwork 1".to_string()
        } else {
            source.filename.clone()
        };
        ArtworkItem {
            id: ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
            name,
            content: ArtworkContent::Svg(source.clone()),
            import_settings: super::SvgInterpretation {
                geometry_tolerance_mm: job.import.geometry_tolerance_mm,
                ticks_per_mm: job.import.ticks_per_mm,
                mode: job.import.mode,
            },
            placement: job.import.placement.clone(),
        }
    });
    let maps = match &artwork {
        Some(item) => ReferenceMaps::build(item)?,
        None => ReferenceMaps {
            revision: SourceRevision {
                content_digest: String::new(),
                algorithm_version: super::SOURCE_REVISION_ALGORITHM_VERSION,
            },
            components: BTreeMap::new(),
            contours: BTreeMap::new(),
            chains: BTreeMap::new(),
        },
    };
    let migrated = CamJobV5 {
        schema_version: CAM_JOB_V5_SCHEMA_VERSION,
        name: job.name.clone(),
        setup: job.setup.clone(),
        artwork: artwork.into_iter().collect(),
        tools: job
            .tools
            .iter()
            .map(|tool| JobToolV5 {
                id: tool.id.clone(),
                name: tool.name.clone(),
                geometry: tool.geometry.clone(),
                capabilities: tool.capabilities.clone(),
                library_origin: None,
            })
            .collect(),
        operations: job
            .operations
            .iter()
            .map(|op| OperationV5 {
                id: op.id.clone(),
                name: op.name.clone(),
                enabled: op.enabled,
                settings: match &op.settings {
                    crate::project::OperationSettings::FlatVcarve(settings) => {
                        OperationSettingsV5::FlatVcarve(migrate_flat_vcarve(settings, &maps))
                    }
                    crate::project::OperationSettings::Face(settings) => {
                        OperationSettingsV5::Face(migrate_face(settings))
                    }
                    crate::project::OperationSettings::Profile(settings) => {
                        OperationSettingsV5::Profile(migrate_profile(settings, &maps))
                    }
                    crate::project::OperationSettings::DragKnife(settings) => {
                        OperationSettingsV5::DragKnife(migrate_knife(settings, &maps))
                    }
                },
            })
            .collect(),
        tolerances: job.tolerances.clone(),
        machine_configuration: None,
        legacy_machine_profile: job.legacy_machine_profile.clone(),
    };
    migrated.validate_structure()?;
    Ok(migrated)
}

/// Load any supported older job JSON (schema 1-4) and migrate it to schema 5.
/// Schema-5 documents parse directly; unknown schema versions are rejected.
/// There is deliberately no reverse path: a client that cannot preserve a
/// collection document must refuse it, never flatten it to schema 4.
pub fn migrate_json(json: &str) -> Result<CamJobV5> {
    let version: serde_json::Value =
        serde_json::from_str(json).map_err(|e| error("PROJECT_JSON", e.to_string()))?;
    match version.get("schema_version").and_then(|v| v.as_u64()) {
        Some(v) if v == u64::from(CAM_JOB_V5_SCHEMA_VERSION) => super::CamJobV5::from_json(json),
        Some(4) => migrate_v4(&CamJob::from_json(json)?),
        Some(1..=3) => migrate_v4(&crate::project::migrate::migrate_legacy_json(json)?),
        _ => Err(error(
            "CAM_JOB_SCHEMA_VERSION",
            "unsupported or missing CamJob schema version",
        )),
    }
}
