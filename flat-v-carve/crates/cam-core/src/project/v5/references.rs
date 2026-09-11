//! Reference inspection and planning readiness for schema-5 documents
//! (plan sections 22.3 and 22.5).
//!
//! Inspection resolves every qualified geometry reference against its owning
//! item's current content and reports located issues without rejecting the
//! document: missing artwork/tools/geometry, source-revision mismatches,
//! forward or deleted face references and anchors needing reattachment are
//! all saveable draft states. Readiness applies the same resolution to one
//! requested enabled scope and turns unresolved references into blockers for
//! exactly the operations that own them.
//!
//! Numeric machining requirements (feeds, stepdowns, headings, ...) are not
//! re-derived here: the planners remain their single authority until H3
//! feeds them resolved collection geometry, and this layer only guarantees
//! that invalid references cannot reach planners.
use super::{
    AppliedMachineConfiguration, CamJobV5, ContourAnchorV5, GeometryRef, GeometryRefKind,
    OperationSettingsV5, OperationV5, error,
};
use crate::{
    contours::ContourCatalogue, geometry::Result, operations::LocatedDiagnostic,
    project::HeightReference,
};
use std::collections::BTreeMap;

/// Scope of a readiness query, mirroring the planning scopes: every enabled
/// operation, or the enabled prefix ending at one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadinessScope {
    AllEnabled,
    ThroughOperation { operation_id: String },
}

/// One item's resolved state: its current revision and, when its content
/// imports cleanly, the catalogue local geometry references resolve against.
/// Derived from the shared [`super::artwork`] resolver so inspection and the
/// H2 commands always see the same import boundary.
struct ItemResolution {
    revision: super::SourceRevision,
    catalogue: Option<ContourCatalogue>,
    import_error: Option<String>,
}

fn resolve_items(job: &CamJobV5) -> Result<BTreeMap<String, ItemResolution>> {
    let combined = super::artwork::inspect_artwork(job)?;
    let mut items = BTreeMap::new();
    for item in combined.items {
        items.insert(
            item.id.0.clone(),
            ItemResolution {
                revision: item.revision,
                catalogue: item.catalogue,
                import_error: item.import_error,
            },
        );
    }
    Ok(items)
}

fn located(
    code: &str,
    operation_id: Option<&str>,
    field: &str,
    message: impl Into<String>,
) -> LocatedDiagnostic {
    LocatedDiagnostic {
        code: code.into(),
        message: message.into(),
        operation_id: operation_id.map(str::to_string),
        tool_id: None,
        field_path: Some(field.into()),
    }
}

enum RefOutcome<'a> {
    Resolved {
        contour: Option<&'a crate::contours::Contour>,
    },
    UnknownItem,
    RevisionMismatch,
    UnknownGeometry,
    Unimported,
}

fn resolve_geometry_ref<'a>(
    reference: &GeometryRef,
    items: &'a BTreeMap<String, ItemResolution>,
) -> RefOutcome<'a> {
    let Some(item) = items.get(&reference.artwork_item_id.0) else {
        return RefOutcome::UnknownItem;
    };
    if item.revision != reference.source_revision {
        return RefOutcome::RevisionMismatch;
    }
    let Some(catalogue) = &item.catalogue else {
        return RefOutcome::Unimported;
    };
    let contour = match reference.kind {
        GeometryRefKind::FilledComponent => {
            // Filled components are selected by the importer's component ID;
            // the catalogue exposes them as the component lineage of contours.
            catalogue
                .contours
                .iter()
                .find(|c| c.component_id == reference.local_geometry_id)
        }
        GeometryRefKind::ClosedContour => catalogue.contour(&reference.local_geometry_id),
        GeometryRefKind::Centerline => catalogue.chain(&reference.local_geometry_id),
    };
    if contour.is_none() {
        return RefOutcome::UnknownGeometry;
    }
    RefOutcome::Resolved { contour }
}

fn geometry_issue(
    reference: &GeometryRef,
    outcome: &RefOutcome,
    operation_id: &str,
    field: &str,
) -> Option<LocatedDiagnostic> {
    match outcome {
        RefOutcome::Resolved { .. } => None,
        RefOutcome::UnknownItem => Some(located(
            "ARTWORK_REFERENCE",
            Some(operation_id),
            field,
            format!(
                "{} references unknown artwork item '{}'",
                field, reference.artwork_item_id.0
            ),
        )),
        RefOutcome::RevisionMismatch => Some(located(
            "ARTWORK_REVISION_MISMATCH",
            Some(operation_id),
            field,
            format!(
                "{} references artwork item '{}' at an older source revision; reattach it after the content change",
                field, reference.artwork_item_id.0
            ),
        )),
        RefOutcome::Unimported => Some(located(
            "ARTWORK_IMPORT_FAILED",
            Some(operation_id),
            field,
            format!(
                "{} references artwork item '{}' whose content failed to import",
                field, reference.artwork_item_id.0
            ),
        )),
        RefOutcome::UnknownGeometry => Some(located(
            "GEOMETRY_REFERENCE",
            Some(operation_id),
            field,
            format!(
                "{} references unknown {} '{}' in artwork item '{}'",
                field,
                match reference.kind {
                    GeometryRefKind::FilledComponent => "filled component",
                    GeometryRefKind::ClosedContour => "closed contour",
                    GeometryRefKind::Centerline => "centerline chain",
                },
                reference.local_geometry_id,
                reference.artwork_item_id.0
            ),
        )),
    }
}

fn check_anchor(
    anchor: &ContourAnchorV5,
    items: &BTreeMap<String, ItemResolution>,
    operation_id: &str,
    field: &str,
    issues: &mut Vec<LocatedDiagnostic>,
) {
    let outcome = resolve_geometry_ref(&anchor.geometry, items);
    if let Some(issue) = geometry_issue(&anchor.geometry, &outcome, operation_id, field) {
        issues.push(issue);
        return;
    }
    let RefOutcome::Resolved { contour } = outcome else {
        return;
    };
    // The stored fingerprint is contour-level evidence: the binding identity
    // is the source revision on the reference, and a fingerprint mismatch
    // means the anchor needs explicit reattachment, never silent repair.
    if let Some(contour) = contour
        && contour.source_fingerprint != anchor.source_geometry_fingerprint
    {
        issues.push(located(
            "CONTOUR_ANCHOR_UNRESOLVED",
            Some(operation_id),
            field,
            format!(
                "{} addresses geometry '{}' whose source fingerprint no longer matches; reattach the anchor",
                field, anchor.geometry.local_geometry_id
            ),
        ));
    }
}

fn check_heights(
    job: &CamJobV5,
    operation: &OperationV5,
    top: &crate::project::HeightRef,
    bottom: Option<&crate::project::HeightRef>,
    require_enabled_preceding_face: bool,
    issues: &mut Vec<LocatedDiagnostic>,
) {
    let id = operation.id.as_str();
    for (field, height) in std::iter::once(("top", top)).chain(bottom.map(|b| ("bottom", b))) {
        let HeightReference::FaceResult {
            operation_id: target,
        } = &height.reference
        else {
            continue;
        };
        let Some((index, target_op)) = job
            .operations
            .iter()
            .enumerate()
            .find(|(_, op)| op.id == *target)
        else {
            issues.push(located(
                "PROJECT_HEIGHT_REFERENCE",
                Some(id),
                field,
                format!(
                    "operation '{id}' {field} references deleted or unknown operation '{target}'"
                ),
            ));
            continue;
        };
        if !matches!(target_op.settings, OperationSettingsV5::Face(_)) {
            issues.push(located(
                "PROJECT_HEIGHT_REFERENCE",
                Some(id),
                field,
                format!("operation '{id}' {field} references non-face operation '{target}'"),
            ));
            continue;
        }
        let own_index = job
            .operations
            .iter()
            .position(|op| op.id == operation.id)
            .unwrap_or(0);
        if index >= own_index {
            issues.push(located(
                "HEIGHT_REFERENCE_FORWARD",
                Some(id),
                field,
                format!("operation '{id}' {field} references later operation '{target}'"),
            ));
        } else if require_enabled_preceding_face && !target_op.enabled {
            // Planning never sees a disabled face publish its plane, so a
            // reference to it cannot resolve in this scope.
            issues.push(located(
                "PROJECT_HEIGHT_REFERENCE",
                Some(id),
                field,
                format!("operation '{id}' {field} references disabled face operation '{target}'"),
            ));
        }
    }
}

/// Reference issues owned by one operation (enabled or disabled).
fn operation_reference_issues(
    job: &CamJobV5,
    operation: &OperationV5,
    items: &BTreeMap<String, ItemResolution>,
    require_enabled_preceding_face: bool,
) -> Vec<LocatedDiagnostic> {
    let mut issues = vec![];
    let id = operation.id.as_str();
    let require_tool = |issues: &mut Vec<LocatedDiagnostic>,
                        tool_id: &str,
                        role: &str,
                        field: &str| {
        let Some(tool) = job.tools.iter().find(|t| t.id == tool_id) else {
            issues.push(located(
                "PROJECT_TOOL_REFERENCE",
                Some(id),
                field,
                format!("operation '{id}' references deleted or unknown {role} tool '{tool_id}'"),
            ));
            return;
        };
        let Some(geometry) = &tool.geometry else {
            return;
        };
        let kind = geometry_kind_name(geometry);
        let expected = match role {
            "knife" => "drag_knife",
            "endmill" => "endmill",
            "V-bit" => "vbit",
            _ => return,
        };
        if kind != expected {
            issues.push(located(
                "PROJECT_TOOL_KIND",
                Some(id),
                field,
                format!("operation '{id}' {role} tool '{tool_id}' has {kind} geometry"),
            ));
        }
    };
    match &operation.settings {
        OperationSettingsV5::FlatVcarve(settings) => {
            require_tool(
                &mut issues,
                &settings.endmill.tool_id,
                "endmill",
                "endmill.tool_id",
            );
            require_tool(&mut issues, &settings.vbit.tool_id, "V-bit", "vbit.tool_id");
            if settings.endmill.tool_id == settings.vbit.tool_id {
                issues.push(located(
                    "PROJECT_OPERATION",
                    Some(id),
                    "endmill.tool_id",
                    format!("operation '{id}' must reference distinct endmill and V-bit tools"),
                ));
            }
            for (index, component) in settings.components.iter().enumerate() {
                let field = format!("components[{index}]");
                let outcome = resolve_geometry_ref(component, items);
                if let Some(issue) = geometry_issue(component, &outcome, id, &field) {
                    issues.push(issue);
                }
            }
            check_heights(
                job,
                operation,
                &settings.top,
                None,
                require_enabled_preceding_face,
                &mut issues,
            );
        }
        OperationSettingsV5::Face(settings) => {
            require_tool(
                &mut issues,
                &settings.assignment.tool_id,
                "milling",
                "assignment.tool_id",
            );
            check_heights(
                job,
                operation,
                &settings.top,
                Some(&settings.bottom),
                require_enabled_preceding_face,
                &mut issues,
            );
        }
        OperationSettingsV5::Profile(settings) => {
            require_tool(
                &mut issues,
                &settings.assignment.tool_id,
                "milling",
                "assignment.tool_id",
            );
            for (index, contour) in settings.contours.iter().enumerate() {
                let field = format!("contours[{index}].geometry");
                let outcome = resolve_geometry_ref(&contour.geometry, items);
                if let Some(issue) = geometry_issue(&contour.geometry, &outcome, id, &field) {
                    issues.push(issue);
                }
            }
            if let super::StartSelectionV5::Anchor(anchor) = &settings.start {
                check_anchor(anchor, items, id, "start", &mut issues);
            }
            if let Some(tabs) = &settings.tabs
                && let super::TabPlacementV5::Manual { anchors } = &tabs.placement
            {
                for (index, anchor) in anchors.iter().enumerate() {
                    check_anchor(
                        anchor,
                        items,
                        id,
                        &format!("tabs.placement.anchors[{index}]"),
                        &mut issues,
                    );
                }
            }
            check_heights(
                job,
                operation,
                &settings.top,
                Some(&settings.bottom),
                require_enabled_preceding_face,
                &mut issues,
            );
        }
        OperationSettingsV5::DragKnife(settings) => {
            require_tool(
                &mut issues,
                &settings.assignment.tool_id,
                "knife",
                "assignment.tool_id",
            );
            for (index, chain) in settings.chains.iter().enumerate() {
                let field = format!("chains[{index}]");
                let outcome = resolve_geometry_ref(chain, items);
                if let Some(issue) = geometry_issue(chain, &outcome, id, &field) {
                    issues.push(issue);
                }
            }
            if let super::StartSelectionV5::Anchor(anchor) = &settings.start {
                check_anchor(anchor, items, id, "start", &mut issues);
            }
            check_heights(
                job,
                operation,
                &settings.top,
                Some(&settings.bottom),
                require_enabled_preceding_face,
                &mut issues,
            );
        }
    }
    issues
}

fn geometry_kind_name(geometry: &crate::project::ToolGeometry) -> &'static str {
    match geometry {
        crate::project::ToolGeometry::Endmill(_) => "endmill",
        crate::project::ToolGeometry::Vbit(_) => "vbit",
        crate::project::ToolGeometry::DragKnife(_) => "drag_knife",
    }
}

fn machine_configuration_issues(
    configuration: &AppliedMachineConfiguration,
    tools: &[super::JobToolV5],
) -> Vec<LocatedDiagnostic> {
    let mut issues = vec![];
    for mapping in &configuration.tools {
        if !tools.iter().any(|t| t.id == mapping.job_tool_id) {
            issues.push(located(
                "MACHINE_MAPPING_REFERENCE",
                None,
                &format!("machine_configuration.tools['{}']", mapping.job_tool_id),
                format!(
                    "the applied machine configuration maps deleted or unknown job tool '{}'",
                    mapping.job_tool_id
                ),
            ));
        }
    }
    issues
}

/// The document-wide reference inspection: located, saveable issues covering
/// every operation (enabled or disabled), every artwork item's importability
/// and the applied machine configuration's mappings. The typed document is
/// preserved; nothing here blocks saving.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReferenceInspection {
    pub issues: Vec<LocatedDiagnostic>,
}

pub fn inspect_references(job: &CamJobV5) -> Result<ReferenceInspection> {
    let items = resolve_items(job)?;
    let mut issues = vec![];
    for item in &job.artwork {
        if let Some(state) = items.get(&item.id.0)
            && let Some(detail) = &state.import_error
        {
            // The item's own import failure is located at the item; the
            // operations referencing it add their own blockers below.
            issues.push(located(
                "ARTWORK_IMPORT_FAILED",
                None,
                &format!("artwork[{}].content", item.id.0),
                format!("artwork item '{}' failed to import: {detail}", item.id.0),
            ));
        }
    }
    for operation in &job.operations {
        issues.extend(operation_reference_issues(job, operation, &items, false));
    }
    if let Some(configuration) = &job.machine_configuration {
        issues.extend(machine_configuration_issues(configuration, &job.tools));
    }
    Ok(ReferenceInspection { issues })
}

/// Per-operation reference readiness inside one requested scope.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationReadiness {
    pub operation_id: String,
    pub ready: bool,
    pub blockers: Vec<LocatedDiagnostic>,
}

/// The scope-gated planning gate: unresolved references in operations inside
/// the requested enabled scope block exactly those operations. Disabled
/// operations and operations outside a prefix never contribute blockers.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanningReadiness {
    pub operations: Vec<OperationReadiness>,
}

impl PlanningReadiness {
    pub fn ready(&self) -> bool {
        self.operations.iter().all(|o| o.ready)
    }

    pub fn blockers(&self) -> Vec<LocatedDiagnostic> {
        self.operations
            .iter()
            .flat_map(|o| o.blockers.iter().cloned())
            .collect()
    }
}

pub fn planning_readiness(job: &CamJobV5, scope: &ReadinessScope) -> Result<PlanningReadiness> {
    job.validate_structure()?;
    let scoped: Vec<&OperationV5> = match scope {
        ReadinessScope::AllEnabled => job.operations.iter().filter(|op| op.enabled).collect(),
        ReadinessScope::ThroughOperation { operation_id } => {
            let Some(end) = job.operations.iter().position(|op| op.id == *operation_id) else {
                return Err(error(
                    "READINESS_SCOPE",
                    format!("prefix scope references unknown operation '{operation_id}'"),
                ));
            };
            job.operations[..=end]
                .iter()
                .filter(|op| op.enabled)
                .collect()
        }
    };
    let items = resolve_items(job)?;
    let operations = scoped
        .into_iter()
        .map(|operation| {
            // Inside a planning scope, height references additionally need
            // their preceding face to actually run: a disabled face never
            // publishes a plane.
            let blockers = operation_reference_issues(job, operation, &items, true);
            OperationReadiness {
                operation_id: operation.id.clone(),
                ready: blockers.is_empty(),
                blockers,
            }
        })
        .collect();
    Ok(PlanningReadiness { operations })
}
