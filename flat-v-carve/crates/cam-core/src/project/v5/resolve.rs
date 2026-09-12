//! Cross-source machining resolution (plan section 22.4, H3): turn one
//! operation's qualified geometry references into the resolved geometry the
//! existing planners consume.
//!
//! * Flat V-carve resolves its selected filled components per owning item
//!   through the one SVG importer, then unions them onto one
//!   operation-compatible grid — the finest selected tolerance, never the
//!   coarsest. A single item resolves to exactly the region the schema-4
//!   adapter produced, so migrated documents machine identically.
//! * Profile and Knife resolve through an assembled owner-qualified
//!   [`ContourCatalogue`]: per-item entries renamed to their derived wire
//!   IDs, each anchor resolving through its owning item's placement.
//!
//! The planner-facing settings mappers here are internal projections onto the
//! frozen schema-4 planner shapes (wire-ID strings for qualified geometry
//! references); they are never serialized and never flatten a collection
//! document.
use super::{
    ArtworkItem, CamJobV5, CombinedCatalogue, DragKnifeSettingsV5, FaceSettingsV5,
    FlatVcarveSettingsV5, GeometryRef, GeometryRefKind, MillingAssignmentV5, OperationSettingsV5,
    OperationV5, ProfileSettingsV5, artwork::wire_id,
};
use crate::{
    contours::{Contour, ContourCatalogue, ContourRole},
    geometry::{Diagnostic, Grid, Region, Result},
    operations::{PlanContext, PlannerGeometry, PublishedFaceMap},
    project,
    sequence::PlannedOperation,
    svg::Bounds,
};
use std::collections::BTreeSet;

/// The resolved machining input of one Flat V-carve operation: the union of
/// its selected filled components in setup coordinates, on one grid, plus
/// the union's bounds (uniform-top admission and stock queries).
#[derive(Clone, Debug)]
pub struct ResolvedVcarveRegion {
    pub region: Region,
    pub bounds: Option<Bounds>,
    /// Tolerance of the reconciled grid (the finest selected import
    /// tolerance); the planners' own precision checks run against it.
    pub grid_tolerance_mm: f64,
}

fn located(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("resolve")
}

/// Resolve one geometry reference against the combined catalogue. The outcome
/// codes mirror [`super::references`] inspection exactly; this path returns
/// the geometry itself because planners consume it.
enum RefResolution {
    Resolved,
    Issue(Diagnostic),
}

fn resolve_reference(
    reference: &GeometryRef,
    combined: &CombinedCatalogue,
    field: &str,
) -> RefResolution {
    let kind_name = match reference.kind {
        GeometryRefKind::FilledComponent => "filled component",
        GeometryRefKind::ClosedContour => "closed contour",
        GeometryRefKind::Centerline => "centerline chain",
    };
    let Some(item) = combined.item(&reference.artwork_item_id) else {
        return RefResolution::Issue(located(
            "ARTWORK_REFERENCE",
            format!(
                "{field} references unknown artwork item '{}'",
                reference.artwork_item_id.0
            ),
        ));
    };
    if item.revision != reference.source_revision {
        return RefResolution::Issue(located(
            "ARTWORK_REVISION_MISMATCH",
            format!(
                "{field} references artwork item '{}' at an older source revision; reattach it after the content change",
                reference.artwork_item_id.0
            ),
        ));
    }
    let Some(catalogue) = &item.catalogue else {
        return RefResolution::Issue(located(
            "ARTWORK_IMPORT_FAILED",
            format!(
                "{field} references artwork item '{}' whose content failed to import",
                reference.artwork_item_id.0
            ),
        ));
    };
    let exists = match reference.kind {
        GeometryRefKind::FilledComponent => catalogue
            .contours
            .iter()
            .any(|c| c.role == ContourRole::Outer && c.component_id == reference.local_geometry_id),
        GeometryRefKind::ClosedContour => catalogue.contour(&reference.local_geometry_id).is_some(),
        GeometryRefKind::Centerline => catalogue.chain(&reference.local_geometry_id).is_some(),
    };
    if !exists {
        return RefResolution::Issue(located(
            "GEOMETRY_REFERENCE",
            format!(
                "{field} references unknown {kind_name} '{}' in artwork item '{}'",
                reference.local_geometry_id, reference.artwork_item_id.0
            ),
        ));
    }
    RefResolution::Resolved
}

/// Assemble the owner-qualified planning catalogue: every item's contours and
/// chains renamed to their derived wire IDs, each entry keeping its owning
/// item's placement. Coincident cuts across items stay separately
/// selectable; nothing is unioned here.
pub fn assembled_catalogue(combined: &CombinedCatalogue) -> Result<ContourCatalogue> {
    let mut contours = vec![];
    let mut open_chains = vec![];
    for item in &combined.items {
        let Some(catalogue) = &item.catalogue else {
            // Items that fail to import contribute no entries; the operations
            // referencing them are blocked by reference readiness instead.
            continue;
        };
        for contour in &catalogue.contours {
            contours.push(qualified(contour, &item.id, GeometryRefKind::ClosedContour));
        }
        for chain in &catalogue.open_chains {
            open_chains.push(qualified(chain, &item.id, GeometryRefKind::Centerline));
        }
    }
    let unique = contours
        .iter()
        .chain(open_chains.iter())
        .map(|c| c.id.as_str())
        .collect::<BTreeSet<_>>();
    if unique.len() != contours.len() + open_chains.len() {
        return Err(super::error(
            "CONTOUR_ID_COLLISION",
            "assembled catalogue entries collide on a wire ID",
        ));
    }
    Ok(ContourCatalogue {
        contours,
        open_chains,
    })
}

fn qualified(contour: &Contour, item: &super::ArtworkItemId, kind: GeometryRefKind) -> Contour {
    let mut remapped = contour.clone();
    remapped.id = wire_id(item, kind, &contour.id);
    remapped
}

/// Resolve a Flat V-carve component selection into its filled union in setup
/// coordinates (plan section 22.4). Each item imports through its own
/// interpretation and placement; the union is taken on the finest selected
/// import grid, and any coarser item must independently satisfy the
/// operation's tolerance budgets or be rejected by name — it cannot ride
/// along on a finer partner's grid.
pub fn resolve_vcarve_region(
    job: &CamJobV5,
    _operation_id: &str,
    settings: &FlatVcarveSettingsV5,
    combined: &CombinedCatalogue,
) -> Result<ResolvedVcarveRegion> {
    let mut groups: Vec<(&ArtworkItem, Vec<String>)> = vec![];
    for (index, reference) in settings.components.iter().enumerate() {
        match resolve_reference(reference, combined, &format!("components[{index}]")) {
            RefResolution::Resolved => {}
            RefResolution::Issue(issue) => return Err(issue),
        }
        match groups
            .iter_mut()
            .find(|(item, _)| item.id == reference.artwork_item_id)
        {
            Some((_, local_ids)) => local_ids.push(reference.local_geometry_id.clone()),
            None => {
                let item = job
                    .artwork
                    .iter()
                    .find(|item| item.id == reference.artwork_item_id)
                    .expect("resolution checked the item exists");
                groups.push((item, vec![reference.local_geometry_id.clone()]));
            }
        }
    }
    if groups.is_empty() {
        return Err(located(
            "EMPTY_SELECTION",
            "select at least one filled component before planning",
        ));
    }
    // Import each item's share through the single importer boundary with that
    // item's own options; identical bytes/settings produce the identical
    // per-item region the schema-4 path generated.
    let mut imported: Vec<(f64, Region)> = vec![];
    for (item, local_ids) in &groups {
        let snapshot = match &item.content {
            super::ArtworkContent::Svg(snapshot) => snapshot.clone(),
        };
        let geometry =
            crate::svg::import_svg(&snapshot.svg, &item.import_options(), Some(local_ids))?;
        imported.push((
            item.import_settings.geometry_tolerance_mm,
            geometry.selected,
        ));
    }
    // Reconcile onto one grid: the finest selected tolerance. Items sharing
    // a tolerance share the identical grid by construction (the grid scale
    // depends on the tolerance only), so equal-tolerance unions never regrid
    // and a single-item selection returns that item's region unchanged.
    let finest = imported
        .iter()
        .map(|(tolerance, _)| *tolerance)
        .fold(f64::INFINITY, f64::min);
    let distinct = imported
        .iter()
        .any(|(tolerance, _)| (*tolerance - finest).abs() > 1e-12);
    let regions: Vec<Region> = if !distinct {
        imported.into_iter().map(|(_, region)| region).collect()
    } else {
        // A coarser item must satisfy the operation tolerance budgets under
        // its own import error, or be rejected by name: regridding its
        // geometry onto the fine grid is not evidence it meets the budget
        // (plan section 22.4).
        for (item, _) in &groups {
            let tolerance = item.import_settings.geometry_tolerance_mm;
            if (tolerance - finest).abs() <= 1e-12 {
                continue;
            }
            let grid = Grid::new(tolerance, 0.)?;
            let motion = job.tolerances.motion_tolerance_mm.unwrap_or(f64::INFINITY);
            let verification = job
                .tolerances
                .verification_tolerance_mm
                .unwrap_or(f64::INFINITY);
            if motion < grid.arc_tolerance_mm() + grid.snap_bound_mm()
                || verification < 8. * tolerance
            {
                return Err(located(
                    "ARTWORK_PRECISION_INCOMPATIBLE",
                    format!(
                        "artwork item '{}' imports at {} mm tolerance, which the operation tolerances cannot absorb alongside finer sources; use a compatible import setting",
                        item.id.0, tolerance
                    ),
                ));
            }
        }
        let bounds = imported
            .iter()
            .filter_map(|(_, region)| Bounds::of(region))
            .collect::<Vec<_>>();
        let extent = bounds
            .iter()
            .flat_map(|b| [b.max.x.abs(), b.max.y.abs(), b.min.x.abs(), b.min.y.abs()])
            .fold(0., f64::max);
        let grid = Grid::new(finest, extent)?;
        imported
            .into_iter()
            .map(|(_, region)| Region::from_rings(grid, &region.rings_mm()))
            .collect::<Result<Vec<_>>>()?
    };
    let region = if regions.len() == 1 {
        regions.into_iter().next().expect("one region")
    } else {
        let reference_grid = regions[0].grid();
        Region::union_all(reference_grid, &regions.iter().collect::<Vec<_>>())?
    };
    let bounds = Bounds::of(&region);
    Ok(ResolvedVcarveRegion {
        region,
        bounds,
        grid_tolerance_mm: finest,
    })
}

// ---------------------------------------------------------------------------
// Planner-facing settings projections (schema-5 shapes onto the frozen
// schema-4 planner settings). Wire IDs stand in for qualified references;
// these projections exist in memory only for the planner call.
// ---------------------------------------------------------------------------

pub(crate) fn to_milling_assignment(
    assignment: &MillingAssignmentV5,
) -> project::MillingAssignment {
    project::MillingAssignment {
        tool_id: assignment.tool_id.clone(),
        spindle_rpm: assignment.spindle_rpm,
        spindle_direction: assignment.spindle_direction,
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
        stepover_mm: assignment.stepover_mm,
    }
}

fn to_knife_assignment(assignment: &super::KnifeAssignmentV5) -> project::KnifeAssignment {
    project::KnifeAssignment {
        tool_id: assignment.tool_id.clone(),
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        swivel_feed_mm_min: assignment.swivel_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
    }
}

fn to_anchor(anchor: &super::ContourAnchorV5) -> project::ContourAnchor {
    project::ContourAnchor {
        contour_id: wire_id(
            &anchor.geometry.artwork_item_id,
            anchor.geometry.kind,
            &anchor.geometry.local_geometry_id,
        ),
        source_geometry_fingerprint: anchor.source_geometry_fingerprint.clone(),
        fraction_along_source_contour: anchor.fraction_along_source_contour,
    }
}

fn to_start(start: &super::StartSelectionV5) -> project::StartSelection {
    match start {
        super::StartSelectionV5::Automatic => project::StartSelection::Automatic,
        super::StartSelectionV5::Anchor(anchor) => {
            project::StartSelection::Anchor(Box::new(to_anchor(anchor)))
        }
    }
}

pub(crate) fn to_flat_vcarve_settings(
    settings: &FlatVcarveSettingsV5,
) -> project::FlatVcarveSettings {
    project::FlatVcarveSettings {
        component_ids: settings
            .components
            .iter()
            .map(|r| r.local_geometry_id.clone())
            .collect(),
        mode: settings.mode,
        endmill: to_milling_assignment(&settings.endmill),
        vbit: to_milling_assignment(&settings.vbit),
        top: settings.top.clone(),
        max_depth_mm: settings.max_depth_mm,
        wall_allowance_mm: settings.wall_allowance_mm,
        max_floor_ridge_mm: settings.max_floor_ridge_mm,
        max_detail_residual_mm: settings.max_detail_residual_mm,
        rough: settings.rough.clone(),
        finish: if settings.mode == project::FlatVcarveMode::Combined {
            settings.finish.clone()
        } else {
            None
        },
    }
}

pub(crate) fn to_face_settings(settings: &FaceSettingsV5) -> project::FaceSettings {
    project::FaceSettings {
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
        assignment: to_milling_assignment(&settings.assignment),
    }
}

pub(crate) fn to_profile_settings(settings: &ProfileSettingsV5) -> project::ProfileSettings {
    project::ProfileSettings {
        contours: settings
            .contours
            .iter()
            .map(|contour| project::ProfileContour {
                contour_id: wire_id(
                    &contour.geometry.artwork_item_id,
                    GeometryRefKind::ClosedContour,
                    &contour.geometry.local_geometry_id,
                ),
                side: contour.side,
                traversal: contour.traversal,
            })
            .collect(),
        assignment: to_milling_assignment(&settings.assignment),
        top: settings.top.clone(),
        bottom: settings.bottom.clone(),
        stepdown_mm: settings.stepdown_mm,
        through_cut_allowance_mm: settings.through_cut_allowance_mm,
        direction: settings.direction,
        order: settings.order,
        start: to_start(&settings.start),
        finish: settings.finish.clone(),
        entry: settings.entry.clone(),
        lead_in: settings.lead_in.clone(),
        lead_out: settings.lead_out.clone(),
        tabs: settings.tabs.as_ref().map(|tabs| project::TabSettings {
            height_mm: tabs.height_mm,
            width_mm: tabs.width_mm,
            shape: tabs.shape,
            placement: match &tabs.placement {
                super::TabPlacementV5::Automatic { count, spacing_mm } => {
                    project::TabPlacement::Automatic {
                        count: *count,
                        spacing_mm: *spacing_mm,
                    }
                }
                super::TabPlacementV5::Manual { anchors } => project::TabPlacement::Manual {
                    anchors: anchors.iter().map(to_anchor).collect(),
                },
            },
        }),
    }
}

pub(crate) fn to_knife_settings(settings: &DragKnifeSettingsV5) -> project::DragKnifeSettings {
    project::DragKnifeSettings {
        chains: settings
            .chains
            .iter()
            .map(|r| {
                wire_id(
                    &r.artwork_item_id,
                    GeometryRefKind::Centerline,
                    &r.local_geometry_id,
                )
            })
            .collect(),
        assignment: to_knife_assignment(&settings.assignment),
        top: settings.top.clone(),
        bottom: settings.bottom.clone(),
        stepdown_mm: settings.stepdown_mm,
        swivel_depth_mm: settings.swivel_depth_mm,
        corner_threshold_deg: settings.corner_threshold_deg,
        through_cut_allowance_mm: settings.through_cut_allowance_mm,
        start: to_start(&settings.start),
        closure_overlap_mm: settings.closure_overlap_mm,
        alignment: settings.alignment.clone(),
    }
}

/// Dispatch one enabled collection operation through the same planner bodies
/// the schema-4 path uses (plan section 22.4: both paths call the same
/// machining algorithms). Cross-source resolution failures are generation
/// outcomes: the operation turns incomplete with the located issue while the
/// rest of the job stays inspectable.
pub(crate) fn plan_operation_v5(
    job: &CamJobV5,
    ctx: &PlanContext,
    operation: &OperationV5,
    combined: &CombinedCatalogue,
    catalogue: &ContourCatalogue,
    published_faces: &PublishedFaceMap,
    prior_motions: &[crate::toolpath::PlannedMotion],
) -> Result<PlannedOperation> {
    match &operation.settings {
        OperationSettingsV5::FlatVcarve(settings) => {
            match resolve_vcarve_region(job, &operation.id, settings, combined) {
                Ok(resolved) => crate::operations::flat_vcarve::plan_v5(
                    ctx,
                    job,
                    &operation.id,
                    settings,
                    &resolved,
                    published_faces,
                    prior_motions,
                ),
                Err(diagnostic) => Ok(incomplete_with(&operation.id, diagnostic)),
            }
        }
        OperationSettingsV5::Face(settings) => {
            crate::operations::face::plan(ctx, &operation.id, &to_face_settings(settings))
        }
        OperationSettingsV5::Profile(settings) => crate::operations::profile::plan(
            ctx,
            &operation.id,
            &to_profile_settings(settings),
            published_faces,
            &PlannerGeometry::Catalogue(catalogue),
        ),
        OperationSettingsV5::DragKnife(settings) => crate::operations::drag_knife::plan(
            ctx,
            &operation.id,
            &to_knife_settings(settings),
            published_faces,
            prior_motions,
            &PlannerGeometry::Catalogue(catalogue),
        ),
    }
}

pub(crate) fn incomplete_with(operation_id: &str, diagnostic: Diagnostic) -> PlannedOperation {
    PlannedOperation {
        status: crate::sequence::GenerationStatus::Incomplete,
        stages: vec![],
        motions: vec![],
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues: vec![crate::sequence::PlanIssue {
            code: diagnostic.code,
            message: diagnostic.message,
            operation_id: Some(operation_id.into()),
            stage_id: None,
        }],
        preparation: vec![],
        named_outputs: vec![],
    }
}
