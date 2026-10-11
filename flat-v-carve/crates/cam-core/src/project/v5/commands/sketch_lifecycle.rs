//! Apply geometry and its reference updates as one document transaction.
use super::*;
use crate::sketch::materialize::MaterializedArtwork;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputChange {
    Added,
    Removed,
    TopologyChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutputImpact {
    pub kind: GeometryRefKind,
    pub local_id: String,
    pub change: OutputChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingChange {
    Preserved,
    Removed,
    TopologyChanged,
    BoundaryChanged,
    AlreadyUnresolved,
    GeometryUnavailable,
}
impl BindingChange {
    pub fn needs_repair(self) -> bool {
        self != Self::Preserved
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Preserved => "selection preserved",
            Self::Removed => "output removed; select its replacement",
            Self::TopologyChanged => "topology changed; confirm a new selection",
            Self::BoundaryChanged => "boundary changed; place the anchor again",
            Self::AlreadyUnresolved => "existing unresolved reference still needs repair",
            Self::GeometryUnavailable => "geometry unavailable; repair the sketch first",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BindingImpact {
    pub operation_id: String,
    pub field_path: String,
    pub kind: GeometryRefKind,
    pub local_id: String,
    pub change: BindingChange,
}

/// Derived, transient explanation of a transaction, never saved as binding authority.
#[derive(Clone, Debug, Serialize)]
pub struct SketchApplyReport {
    pub artwork_item_id: ArtworkItemId,
    pub geometry_changed: bool,
    pub geometry_available: bool,
    pub outputs: Vec<OutputImpact>,
    pub bindings: Vec<BindingImpact>,
}
impl SketchApplyReport {
    pub fn repair_count(&self) -> usize {
        self.bindings
            .iter()
            .filter(|b| b.change.needs_repair())
            .count()
    }
    pub fn preserved_count(&self) -> usize {
        self.bindings.len() - self.repair_count()
    }
}

#[derive(Clone, Debug)]
pub struct SketchApplyOutcome {
    pub outcome: CommandOutcome,
    pub report: SketchApplyReport,
}

struct Continuity<'a> {
    owner: &'a ArtworkItemId,
    old_revision: &'a super::super::SourceRevision,
    new_revision: &'a super::super::SourceRevision,
    before: Option<&'a MaterializedArtwork>,
    after: Option<&'a MaterializedArtwork>,
}
fn boundary<'a>(m: &'a MaterializedArtwork, key: &str) -> Option<&'a crate::contours::Contour> {
    m.catalogue.contour(key).or_else(|| m.catalogue.chain(key))
}
impl Continuity<'_> {
    fn classification(&self, r: &GeometryRef, fingerprint: Option<&str>) -> BindingChange {
        if &r.source_revision != self.old_revision {
            return BindingChange::AlreadyUnresolved;
        }
        let (Some(before), Some(after)) = (self.before, self.after) else {
            return BindingChange::GeometryUnavailable;
        };
        let Some(old) = before.provenance(r.kind, &r.local_geometry_id) else {
            return BindingChange::AlreadyUnresolved;
        };
        // An old anchor must already be valid; applying an unrelated sketch
        // edit must never quietly repair a stale fingerprint.
        let old_boundary = fingerprint.and_then(|_| boundary(before, &r.local_geometry_id));
        if let Some(fingerprint) = fingerprint
            && old_boundary.is_none_or(|c| c.source_fingerprint != fingerprint)
        {
            return BindingChange::AlreadyUnresolved;
        }
        match after.provenance(r.kind, &r.local_geometry_id) {
            None => return BindingChange::Removed,
            Some(new) if old != new => return BindingChange::TopologyChanged,
            _ => {}
        }
        if let Some(old) = old_boundary
            && boundary(after, &r.local_geometry_id).is_none_or(|new| {
                old.source_fingerprint != new.source_fingerprint
                    || old.page_vertices != new.page_vertices
            })
        {
            return BindingChange::BoundaryChanged;
        }
        BindingChange::Preserved
    }
    fn bind(
        &self,
        r: &mut GeometryRef,
        fingerprint: Option<&str>,
        operation: &str,
        field: String,
        impacts: &mut Vec<BindingImpact>,
    ) {
        if &r.artwork_item_id != self.owner {
            return;
        }
        let change = self.classification(r, fingerprint);
        if change == BindingChange::Preserved {
            r.source_revision = self.new_revision.clone();
        }
        impacts.push(BindingImpact {
            operation_id: operation.into(),
            field_path: format!("operations[{operation}].{field}"),
            kind: r.kind,
            local_id: r.local_geometry_id.clone(),
            change,
        });
    }
    fn anchor(
        &self,
        a: &mut ContourAnchorV5,
        op: &str,
        field: String,
        impacts: &mut Vec<BindingImpact>,
    ) {
        self.bind(
            &mut a.geometry,
            Some(&a.source_geometry_fingerprint),
            op,
            field,
            impacts,
        );
    }
    fn start(&self, start: &mut StartSelectionV5, op: &str, impacts: &mut Vec<BindingImpact>) {
        if let StartSelectionV5::Anchor(a) = start {
            self.anchor(a, op, "start".into(), impacts);
        }
    }
}

/// Apply a sketch and report persistent output changes and every affected
/// binding. Only exact provenance establishes continuity; no proximity or
/// perimeter-fraction matching is used. Unresolved references stay saveable.
pub fn apply_sketch_with_report(
    job: &CamJobV5,
    id: &ArtworkItemId,
    sketch: crate::sketch::SketchDocument,
) -> Result<SketchApplyOutcome> {
    sketch.validate_structure()?;
    let index = item_index(job, id)?;
    if !matches!(job.artwork[index].content, ArtworkContent::Sketch(_)) {
        return Err(command_error(
            "apply_sketch requires an existing sketch item",
        ));
    }
    let old_revision = job.artwork[index].source_revision()?;
    let before = artwork::materialize_item(&job.artwork[index], None).ok();
    let mut candidate = job.clone();
    candidate.artwork[index].content = ArtworkContent::Sketch(sketch);
    let new_revision = candidate.artwork[index].source_revision()?;
    let after = artwork::materialize_item(&candidate.artwork[index], None).ok();
    let mut report = SketchApplyReport {
        artwork_item_id: id.clone(),
        geometry_changed: old_revision != new_revision,
        geometry_available: after.is_some(),
        outputs: vec![],
        bindings: vec![],
    };
    // An unavailable materialization is not evidence that every output was
    // deleted; keep that state distinct from a valid empty sketch.
    if let (Some(before), Some(after)) = (&before, &after) {
        for old in &before.provenance {
            let change = match after.provenance(old.kind, &old.local_id) {
                None => OutputChange::Removed,
                Some(new) if new != old => OutputChange::TopologyChanged,
                _ => continue,
            };
            report.outputs.push(OutputImpact {
                kind: old.kind,
                local_id: old.local_id.clone(),
                change,
            });
        }
        for new in &after.provenance {
            if before.provenance(new.kind, &new.local_id).is_none() {
                report.outputs.push(OutputImpact {
                    kind: new.kind,
                    local_id: new.local_id.clone(),
                    change: OutputChange::Added,
                });
            }
        }
    }
    let continuity = Continuity {
        owner: id,
        old_revision: &old_revision,
        new_revision: &new_revision,
        before: before.as_ref(),
        after: after.as_ref(),
    };
    let mut affected = vec![AffectedEntity::ArtworkItem(id.clone())];
    for op in &mut candidate.operations {
        let count = report.bindings.len();
        let impacts = &mut report.bindings;
        match &mut op.settings {
            OperationSettingsV5::FlatVcarve(s) => {
                for (i, r) in s.components.iter_mut().enumerate() {
                    continuity.bind(r, None, &op.id, format!("components[{i}]"), impacts);
                }
            }
            OperationSettingsV5::Pocket(s) => {
                for (i, r) in s.components.iter_mut().enumerate() {
                    continuity.bind(r, None, &op.id, format!("components[{i}]"), impacts);
                }
            }
            OperationSettingsV5::Profile(s) => {
                for (i, c) in s.contours.iter_mut().enumerate() {
                    continuity.bind(
                        &mut c.geometry,
                        None,
                        &op.id,
                        format!("contours[{i}].geometry"),
                        impacts,
                    );
                }
                continuity.start(&mut s.start, &op.id, impacts);
                if let Some(tabs) = &mut s.tabs
                    && let TabPlacementV5::Manual { anchors } = &mut tabs.placement
                {
                    for (i, a) in anchors.iter_mut().enumerate() {
                        continuity.anchor(
                            a,
                            &op.id,
                            format!("tabs.placement.anchors[{i}]"),
                            impacts,
                        );
                    }
                }
            }
            OperationSettingsV5::DragKnife(s) => {
                for (i, r) in s.chains.iter_mut().enumerate() {
                    continuity.bind(r, None, &op.id, format!("chains[{i}]"), impacts);
                }
                continuity.start(&mut s.start, &op.id, impacts);
            }
            OperationSettingsV5::Drill(s) => {
                for (i, r) in s.points.iter_mut().enumerate() {
                    continuity.bind(r, None, &op.id, format!("points[{i}]"), impacts);
                }
            }
            OperationSettingsV5::Face(_) => {}
        }
        if report.bindings.len() > count {
            affected.push(AffectedEntity::Operation(op.id.clone()));
        }
    }
    Ok(SketchApplyOutcome {
        outcome: CommandOutcome::commit(candidate, affected)?,
        report,
    })
}
