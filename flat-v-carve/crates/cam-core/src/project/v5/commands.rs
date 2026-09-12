//! Typed schema-5 document commands for the artwork collection, geometry
//! assignment and fit stock (plan sections 22.3, 22.4, 22.5 and the H2 slice
//! of 22.11).
//!
//! Every command is a pure function over the current document: it either
//! returns an updated document plus its post-command reference issues and
//! the stable affected identities, or fails without touching anything — a
//! failed command can never partially rewrite other artwork, selections or
//! settings. Selection inputs are [`artwork::GeometryPick`]s naming the
//! owner, kind and local ID: the command itself binds the item's *current*
//! source revision and contour fingerprint, so assignments cannot carry
//! stale revisions, and it validates every pick against the shared
//! [`super::artwork`] resolver.
//!
//! Fit stock follows the proposal/commit split: [`propose_fit_stock`]
//! computes placed bounds plus margins without mutating the document;
//! [`apply_stock_rectangle`] changes physical stock only when its result is
//! explicitly applied.
use super::{
    ArtworkContent, ArtworkItemId, CamJobV5, ContourAnchorV5, GeometryRef, GeometryRefKind,
    OperationSettingsV5, ProfileContourV5, ProfileSettingsV5, StartSelectionV5, SvgInterpretation,
    TabPlacementV5, artwork,
};
use crate::{
    geometry::{Diagnostic, Result},
    job::SourceSnapshot,
    operations::LocatedDiagnostic,
    project::{ContourSide, RectXY, TraversalDirection},
    svg::{MAX_SVG_BYTES, Placement},
};
use std::collections::BTreeSet;

/// A stable entity a command affected, for undo grouping and UI refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AffectedEntity {
    ArtworkItem(ArtworkItemId),
    Operation(String),
    Setup,
    /// A job tool snapshot was created or replaced (H4 resource commands).
    JobTool(String),
    /// The applied machine configuration changed.
    MachineConfiguration,
}

/// The uniform result of a document command: the updated document, the
/// located issues of the new state (the same list `inspect_references`
/// produces, so callers never re-derive it), and the stable affected IDs.
#[derive(Clone, Debug)]
pub struct CommandOutcome {
    pub job: CamJobV5,
    pub issues: Vec<LocatedDiagnostic>,
    pub affected: Vec<AffectedEntity>,
}

impl CommandOutcome {
    /// Validate the candidate structurally, enforce the aggregate serialized
    /// budget, then inspect references. Failing any step preserves the prior
    /// document because commands never mutate their input.
    pub(crate) fn commit(job: CamJobV5, affected: Vec<AffectedEntity>) -> Result<Self> {
        job.to_json()?;
        let issues = super::references::inspect_references(&job)?.issues;
        Ok(Self {
            job,
            issues,
            affected,
        })
    }
}

fn command_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("ARTWORK_COMMAND", message).at_stage("project")
}

fn kind_name(kind: GeometryRefKind) -> &'static str {
    match kind {
        GeometryRefKind::FilledComponent => "filled component",
        GeometryRefKind::ClosedContour => "closed contour",
        GeometryRefKind::Centerline => "centerline chain",
    }
}

/// One file offered to [`add_artwork`].
#[derive(Clone, Debug)]
pub struct ArtworkInput {
    pub filename: String,
    pub svg: String,
    pub interpretation: SvgInterpretation,
    pub placement: Placement,
    /// Display name; defaults to the filename.
    pub name: Option<String>,
}

/// One file rejected by [`add_artwork`], with its own error.
#[derive(Clone, Debug)]
pub struct FileRejection {
    pub filename: String,
    pub error: Diagnostic,
}

#[derive(Clone, Debug)]
pub struct AddArtworkOutcome {
    pub outcome: CommandOutcome,
    pub rejected: Vec<FileRejection>,
}

/// Normalize a filename into the document identifier alphabet: identifiers,
/// not raw filenames, are the owner key of qualified geometry, so no
/// unescaped path or name ever enters an identity.
fn id_base_from_filename(filename: &str) -> String {
    let stem = match filename.rsplit_once('.') {
        Some((stem, _)) => stem,
        None => filename,
    };
    let mut normalized = String::new();
    let mut dash = false;
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            normalized.push(c);
            dash = false;
        } else if !dash {
            normalized.push('-');
            dash = true;
        }
    }
    let trimmed = normalized.trim_matches('-');
    let base = if trimmed.is_empty() {
        "artwork".to_string()
    } else {
        trimmed.to_string()
    };
    // Leave room for the uniqueness suffix inside the 100-byte ID limit.
    base.chars().take(90).collect()
}

fn fresh_id(used: &BTreeSet<String>, base: &str) -> ArtworkItemId {
    let base = if crate::preview::valid_id(base) {
        base.to_string()
    } else {
        "artwork".to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    ArtworkItemId(candidate)
}

fn used_item_ids(job: &CamJobV5) -> BTreeSet<String> {
    job.artwork.iter().map(|item| item.id.0.clone()).collect()
}

// Deleted owners remain reserved while referenced. Otherwise importing an
// identical same-named file could silently repair a dangling assignment.
fn reserved_item_ids(job: &CamJobV5) -> BTreeSet<String> {
    let mut used = used_item_ids(job);
    let mut reserve = |r: &GeometryRef| {
        used.insert(r.artwork_item_id.0.clone());
    };
    for operation in &job.operations {
        match &operation.settings {
            OperationSettingsV5::FlatVcarve(s) => {
                for r in &s.components {
                    reserve(r);
                }
            }
            OperationSettingsV5::Profile(s) => {
                for c in &s.contours {
                    reserve(&c.geometry);
                }
                if let StartSelectionV5::Anchor(a) = &s.start {
                    reserve(&a.geometry);
                }
                if let Some(tabs) = &s.tabs
                    && let TabPlacementV5::Manual { anchors } = &tabs.placement
                {
                    for a in anchors {
                        reserve(&a.geometry);
                    }
                }
            }
            OperationSettingsV5::DragKnife(s) => {
                for r in &s.chains {
                    reserve(r);
                }
                if let StartSelectionV5::Anchor(a) = &s.start {
                    reserve(&a.geometry);
                }
            }
            OperationSettingsV5::Face(_) => {}
        }
    }
    used
}

fn item_index(job: &CamJobV5, id: &ArtworkItemId) -> Result<usize> {
    job.artwork
        .iter()
        .position(|item| item.id == *id)
        .ok_or_else(|| command_error(format!("unknown artwork item '{}'", id.0)))
}

/// Batch-import artwork: per-file rejections (oversized or unimportable
/// content) are reported individually, the accepted additions commit
/// together, and the aggregate document budget rejects the whole command
/// with the prior document preserved. Existing selections never expand onto
/// the new items (plan section 22.3).
pub fn add_artwork(job: &CamJobV5, inputs: Vec<ArtworkInput>) -> Result<AddArtworkOutcome> {
    let mut rejected = vec![];
    let mut candidate = job.clone();
    let mut used = reserved_item_ids(job);
    let mut affected = vec![];
    for input in inputs {
        if input.svg.len() > MAX_SVG_BYTES {
            rejected.push(FileRejection {
                filename: input.filename.clone(),
                error: Diagnostic::new(
                    "PROJECT_RESOURCE_LIMIT",
                    format!(
                        "file '{}' exceeds the {} byte per-item source limit",
                        input.filename, MAX_SVG_BYTES
                    ),
                )
                .at_stage("project"),
            });
            continue;
        }
        let item = super::ArtworkItem {
            id: fresh_id(&used, &id_base_from_filename(&input.filename)),
            name: input.name.clone().unwrap_or_else(|| input.filename.clone()),
            content: ArtworkContent::Svg(SourceSnapshot {
                filename: input.filename.clone(),
                svg: input.svg.clone(),
            }),
            import_settings: input.interpretation.clone(),
            placement: input.placement.clone(),
        };
        if let Err(error) = artwork::item_catalogue(&item) {
            rejected.push(FileRejection {
                filename: input.filename.clone(),
                error,
            });
            continue;
        }
        used.insert(item.id.0.clone());
        affected.push(AffectedEntity::ArtworkItem(item.id.clone()));
        candidate.artwork.push(item);
    }
    let outcome = CommandOutcome::commit(candidate, affected)?;
    Ok(AddArtworkOutcome { outcome, rejected })
}

/// Rename one item. Display names never participate in the source revision,
/// so references stay valid.
pub fn rename_artwork(job: &CamJobV5, id: &ArtworkItemId, name: &str) -> Result<CommandOutcome> {
    let index = item_index(job, id)?;
    let mut candidate = job.clone();
    candidate.artwork[index].name = name.to_string();
    CommandOutcome::commit(candidate, vec![AffectedEntity::ArtworkItem(id.clone())])
}

/// Duplicate one item: a new item ID and qualified geometry identities, with
/// operation assignments left unchanged — nothing selects the duplicate yet.
pub fn duplicate_artwork(
    job: &CamJobV5,
    id: &ArtworkItemId,
    new_name: Option<&str>,
) -> Result<CommandOutcome> {
    let index = item_index(job, id)?;
    let source = job.artwork[index].clone();
    let mut used = reserved_item_ids(job);
    let base = format!("{}-copy", source.id.0);
    let new_id = fresh_id(&used, &base);
    used.insert(new_id.0.clone());
    let duplicate = super::ArtworkItem {
        id: new_id.clone(),
        name: new_name
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} (copy)", source.name)),
        ..source
    };
    let mut candidate = job.clone();
    // Insert directly after the source: display convenience only.
    candidate.artwork.insert(index + 1, duplicate);
    CommandOutcome::commit(candidate, vec![AffectedEntity::ArtworkItem(new_id)])
}

/// Reorder the artwork collection. Row order is display organization only:
/// it never defines machining order (the operations vector alone does) and
/// references resolve the same way afterwards.
pub fn reorder_artwork(job: &CamJobV5, ordered: &[ArtworkItemId]) -> Result<CommandOutcome> {
    let mut remaining: BTreeSet<String> = used_item_ids(job);
    let mut requested = BTreeSet::new();
    for id in ordered {
        if !remaining.remove(&id.0) || !requested.insert(id.0.clone()) {
            return Err(command_error(format!(
                "artwork reorder must list every existing item exactly once; '{}' is unknown or repeated",
                id.0
            )));
        }
    }
    if !remaining.is_empty() {
        return Err(command_error(format!(
            "artwork reorder is missing item(s) [{}]",
            remaining.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    let mut candidate = job.clone();
    candidate.artwork = ordered
        .iter()
        .map(|id| {
            job.artwork
                .iter()
                .find(|item| item.id == *id)
                .cloned()
                .expect("validated above")
        })
        .collect();
    CommandOutcome::commit(
        candidate,
        ordered
            .iter()
            .map(|id| AffectedEntity::ArtworkItem(id.clone()))
            .collect(),
    )
}

/// Replace one item's content (and optionally its interpretation), keeping
/// the item ID. Unimportable content is rejected with the old source
/// unchanged; valid changed content commits a new source revision and the
/// resulting unresolved references are reported as issues rather than
/// repaired. Identical bytes and interpretation retain every reference.
pub fn replace_artwork(
    job: &CamJobV5,
    id: &ArtworkItemId,
    snapshot: SourceSnapshot,
    interpretation: Option<SvgInterpretation>,
) -> Result<CommandOutcome> {
    let index = item_index(job, id)?;
    if snapshot.svg.len() > MAX_SVG_BYTES {
        return Err(command_error(format!(
            "replacement content exceeds the {} byte per-item source limit",
            MAX_SVG_BYTES
        )));
    }
    let replaced = super::ArtworkItem {
        id: id.clone(),
        name: job.artwork[index].name.clone(),
        content: ArtworkContent::Svg(snapshot),
        import_settings: interpretation
            .unwrap_or_else(|| job.artwork[index].import_settings.clone()),
        placement: job.artwork[index].placement.clone(),
    };
    if let Err(error) = artwork::item_catalogue(&replaced) {
        return Err(command_error(format!(
            "artwork item '{}' could not be replaced: {}",
            id.0, error.message
        )));
    }
    let mut candidate = job.clone();
    candidate.artwork[index] = replaced;
    CommandOutcome::commit(candidate, vec![AffectedEntity::ArtworkItem(id.clone())])
}

/// Remove one item. Its references stay in the document as typed dangling
/// references with their owner IDs; inspection reports them until they are
/// reassigned or reattached.
pub fn remove_artwork(job: &CamJobV5, id: &ArtworkItemId) -> Result<CommandOutcome> {
    let index = item_index(job, id)?;
    let mut candidate = job.clone();
    candidate.artwork.remove(index);
    CommandOutcome::commit(candidate, vec![AffectedEntity::ArtworkItem(id.clone())])
}

/// Move (or scale/rotate) one item. Placement edits preserve every
/// reference and anchor fraction; anchors re-resolve through the new
/// placement into their new setup positions, and no other item moves.
pub fn place_artwork(
    job: &CamJobV5,
    id: &ArtworkItemId,
    placement: Placement,
) -> Result<CommandOutcome> {
    let index = item_index(job, id)?;
    let mut candidate = job.clone();
    candidate.artwork[index].placement = placement;
    CommandOutcome::commit(candidate, vec![AffectedEntity::ArtworkItem(id.clone())])
}

/// Resolve one pick against the combined catalogue and bind the item's
/// current revision, returning the canonical reference plus the geometry's
/// current fingerprint for anchor construction.
fn bind_pick(
    catalogue: &artwork::CombinedCatalogue,
    pick: &artwork::GeometryPick,
    context: &str,
) -> Result<(GeometryRef, String)> {
    let item = catalogue.item(&pick.artwork_item_id).ok_or_else(|| {
        command_error(format!(
            "{context} addresses unknown artwork item '{}'",
            pick.artwork_item_id.0
        ))
    })?;
    if let Some(detail) = &item.import_error {
        return Err(command_error(format!(
            "{context} addresses artwork item '{}' whose content failed to import: {detail}",
            pick.artwork_item_id.0
        )));
    }
    let entry = item
        .entries
        .iter()
        .find(|entry| {
            entry.kind == pick.kind && entry.reference.local_geometry_id == pick.local_geometry_id
        })
        .ok_or_else(|| {
            command_error(format!(
                "{context} addresses unknown {} '{}' in artwork item '{}'",
                kind_name(pick.kind),
                pick.local_geometry_id,
                pick.artwork_item_id.0
            ))
        })?;
    Ok((entry.reference.clone(), entry.source_fingerprint.clone()))
}

fn operation_index(job: &CamJobV5, operation_id: &str) -> Result<usize> {
    job.operations
        .iter()
        .position(|op| op.id == operation_id)
        .ok_or_else(|| command_error(format!("unknown operation '{operation_id}'")))
}

/// Replace a Flat V-carve operation's filled-component selection. An empty
/// list clears the selection (an incomplete operation); every pick must be
/// a filled component that exists in its item's current catalogue.
pub fn set_component_selection(
    job: &CamJobV5,
    operation_id: &str,
    components: &[artwork::GeometryPick],
) -> Result<CommandOutcome> {
    let catalogue = artwork::inspect_artwork(job)?;
    let mut bound = Vec::with_capacity(components.len());
    for (index, pick) in components.iter().enumerate() {
        if pick.kind != GeometryRefKind::FilledComponent {
            return Err(command_error(format!(
                "operation '{operation_id}' selects filled components; '{}' in item '{}' is a {}",
                pick.local_geometry_id,
                pick.artwork_item_id.0,
                kind_name(pick.kind)
            )));
        }
        bound.push(
            bind_pick(
                &catalogue,
                pick,
                &format!("operation '{operation_id}' components[{index}]"),
            )?
            .0,
        );
    }
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    match &mut candidate.operations[index].settings {
        OperationSettingsV5::FlatVcarve(settings) => settings.components = bound,
        _ => {
            return Err(command_error(format!(
                "operation '{operation_id}' is not a Flat V-carve operation"
            )));
        }
    }
    CommandOutcome::commit(
        candidate,
        vec![AffectedEntity::Operation(operation_id.into())],
    )
}

/// Replace one exact Flat V-carve reference with an explicitly chosen current
/// component. Other unresolved references remain untouched for separate repair.
pub fn replace_component_reference(
    job: &CamJobV5,
    operation_id: &str,
    expected: &GeometryRef,
    replacement: &artwork::GeometryPick,
) -> Result<CommandOutcome> {
    if replacement.kind != GeometryRefKind::FilledComponent {
        return Err(command_error("Replacement must be a filled component"));
    }
    let catalogue = artwork::inspect_artwork(job)?;
    let bound = bind_pick(&catalogue, replacement, "replacement")?.0;
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    let OperationSettingsV5::FlatVcarve(settings) = &mut candidate.operations[index].settings
    else {
        return Err(command_error("Expected a Flat V-carve operation"));
    };
    if !settings.components.contains(expected) {
        return Err(command_error(
            "Reference changed before repair; select it again",
        ));
    }
    for reference in &mut settings.components {
        if reference == expected {
            *reference = bound.clone();
        }
    }
    let mut unique = Vec::new();
    settings.components.retain(|r| {
        if unique.contains(r) {
            false
        } else {
            unique.push(r.clone());
            true
        }
    });
    CommandOutcome::commit(
        candidate,
        vec![AffectedEntity::Operation(operation_id.into())],
    )
}

/// One row of a Profile selection with its explicit compensation side.
#[derive(Clone, Debug)]
pub struct ProfileContourPick {
    pub geometry: artwork::GeometryPick,
    pub side: ContourSide,
    pub traversal: Option<TraversalDirection>,
}

/// Replace a Profile operation's contour selection. Sides are stored
/// explicitly per contour (the entry's suggested side is only advisory), and
/// on-contour rows require an explicit traversal direction.
pub fn set_contour_selection(
    job: &CamJobV5,
    operation_id: &str,
    contours: &[ProfileContourPick],
) -> Result<CommandOutcome> {
    let catalogue = artwork::inspect_artwork(job)?;
    let mut bound = Vec::with_capacity(contours.len());
    for (index, pick) in contours.iter().enumerate() {
        if pick.geometry.kind != GeometryRefKind::ClosedContour {
            return Err(command_error(format!(
                "operation '{operation_id}' contours[{index}] must be a closed contour; '{}' in item '{}' is a {}",
                pick.geometry.local_geometry_id,
                pick.geometry.artwork_item_id.0,
                kind_name(pick.geometry.kind)
            )));
        }
        if pick.side == ContourSide::On && pick.traversal.is_none() {
            return Err(command_error(format!(
                "operation '{operation_id}' contours[{index}] is on-contour and requires an explicit traversal direction"
            )));
        }
        bound.push(ProfileContourV5 {
            geometry: bind_pick(
                &catalogue,
                &pick.geometry,
                &format!("operation '{operation_id}' contours[{index}]"),
            )?
            .0,
            side: pick.side,
            traversal: pick.traversal,
        });
    }
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    match &mut candidate.operations[index].settings {
        OperationSettingsV5::Profile(settings) => settings.contours = bound,
        _ => {
            return Err(command_error(format!(
                "operation '{operation_id}' is not a profile operation"
            )));
        }
    }
    CommandOutcome::commit(
        candidate,
        vec![AffectedEntity::Operation(operation_id.into())],
    )
}

/// Replace a Drag Knife operation's centerline chain selection.
pub fn set_chain_selection(
    job: &CamJobV5,
    operation_id: &str,
    chains: &[artwork::GeometryPick],
) -> Result<CommandOutcome> {
    let catalogue = artwork::inspect_artwork(job)?;
    let mut bound = Vec::with_capacity(chains.len());
    for (index, pick) in chains.iter().enumerate() {
        if pick.kind != GeometryRefKind::Centerline {
            return Err(command_error(format!(
                "operation '{operation_id}' chains[{index}] must be a centerline; '{}' in item '{}' is a {}",
                pick.local_geometry_id,
                pick.artwork_item_id.0,
                kind_name(pick.kind)
            )));
        }
        bound.push(
            bind_pick(
                &catalogue,
                pick,
                &format!("operation '{operation_id}' chains[{index}]"),
            )?
            .0,
        );
    }
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    match &mut candidate.operations[index].settings {
        OperationSettingsV5::DragKnife(settings) => settings.chains = bound,
        _ => {
            return Err(command_error(format!(
                "operation '{operation_id}' is not a drag-knife operation"
            )));
        }
    }
    CommandOutcome::commit(
        candidate,
        vec![AffectedEntity::Operation(operation_id.into())],
    )
}

/// Which anchor of an operation a [`reattach_anchor`] call addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorTarget {
    Start,
    /// The n-th manual tab anchor of a profile operation.
    Tab(usize),
}

/// Reattach one existing anchor onto explicitly picked geometry: the command
/// binds the item's current source revision and the geometry's current
/// fingerprint, keeping the stored fraction unless a replacement in `[0,1)`
/// is supplied. The target must be addressed explicitly — there is no
/// nearest-contour repair — and introducing a brand-new anchor is an
/// operation-settings edit, not a reattachment.
pub fn reattach_anchor(
    job: &CamJobV5,
    operation_id: &str,
    target: AnchorTarget,
    pick: &artwork::GeometryPick,
    fraction: Option<f64>,
) -> Result<CommandOutcome> {
    if let Some(fraction) = fraction
        && !(0. ..1.).contains(&fraction)
    {
        return Err(command_error(
            "the replacement anchor fraction must lie in [0,1)",
        ));
    }
    if pick.kind == GeometryRefKind::FilledComponent {
        return Err(command_error(format!(
            "operation '{operation_id}' anchors address closed contours or chains; '{}' is a filled component",
            pick.local_geometry_id
        )));
    }
    let catalogue = artwork::inspect_artwork(job)?;
    let (geometry, fingerprint) = bind_pick(
        &catalogue,
        pick,
        &format!("operation '{operation_id}' anchor"),
    )?;
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    let rebuild = |anchor: &mut ContourAnchorV5| {
        anchor.geometry = geometry.clone();
        anchor.source_geometry_fingerprint = fingerprint.clone();
        if let Some(fraction) = fraction {
            anchor.fraction_along_source_contour = fraction;
        }
    };
    match &mut candidate.operations[index].settings {
        OperationSettingsV5::Profile(settings) => {
            let anchor = match target {
                AnchorTarget::Start => match &mut settings.start {
                    StartSelectionV5::Anchor(anchor) => anchor.as_mut(),
                    StartSelectionV5::Automatic => {
                        return Err(command_error(format!(
                            "operation '{operation_id}' has no anchor start to reattach"
                        )));
                    }
                },
                AnchorTarget::Tab(position) => {
                    manual_tab_anchor(settings, position).ok_or_else(|| {
                        command_error(format!(
                            "operation '{operation_id}' has no manual tab anchor {position}"
                        ))
                    })?
                }
            };
            rebuild(anchor);
        }
        OperationSettingsV5::DragKnife(settings) => {
            if target != AnchorTarget::Start {
                return Err(command_error(format!(
                    "operation '{operation_id}' has no tab anchors"
                )));
            }
            match &mut settings.start {
                StartSelectionV5::Anchor(anchor) => rebuild(anchor.as_mut()),
                StartSelectionV5::Automatic => {
                    return Err(command_error(format!(
                        "operation '{operation_id}' has no anchor start to reattach"
                    )));
                }
            }
        }
        _ => {
            return Err(command_error(format!(
                "operation '{operation_id}' has no reattachable anchors"
            )));
        }
    }
    CommandOutcome::commit(
        candidate,
        vec![AffectedEntity::Operation(operation_id.into())],
    )
}

fn manual_tab_anchor(
    settings: &mut ProfileSettingsV5,
    position: usize,
) -> Option<&mut ContourAnchorV5> {
    let tabs = settings.tabs.as_mut()?;
    let TabPlacementV5::Manual { anchors } = &mut tabs.placement else {
        return None;
    };
    anchors.get_mut(position)
}

/// Per-side fit margins in setup millimeters; all sides nonnegative.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FitStockMargins {
    pub min_x_mm: f64,
    pub max_x_mm: f64,
    pub min_y_mm: f64,
    pub max_y_mm: f64,
}

#[derive(Clone, Debug)]
pub struct FitStockRequest {
    /// Explicit item IDs to fit; hidden or locked items participate when
    /// explicitly included (visibility is workspace state).
    pub item_ids: Vec<ArtworkItemId>,
    pub margins: FitStockMargins,
}

/// A proposed stock rectangle: the placed geometry bounds of the requested
/// items expanded by the margins. Proposing never changes the document.
#[derive(Clone, Debug, PartialEq)]
pub struct FitStockProposal {
    pub rect: RectXY,
}

/// Compute the fit-stock proposal (plan section 22.4). Every listed item
/// must exist and import cleanly; a job whose listed items carry no placed
/// geometry has nothing to fit.
pub fn propose_fit_stock(job: &CamJobV5, request: &FitStockRequest) -> Result<FitStockProposal> {
    let margins = request.margins;
    for (value, name) in [
        (margins.min_x_mm, "min_x_mm"),
        (margins.max_x_mm, "max_x_mm"),
        (margins.min_y_mm, "min_y_mm"),
        (margins.max_y_mm, "max_y_mm"),
    ] {
        if !value.is_finite() || value < 0. {
            return Err(command_error(format!(
                "fit-stock margin {name} must be finite and nonnegative"
            )));
        }
    }
    if request.item_ids.is_empty() {
        return Err(command_error(
            "fit stock requires at least one artwork item",
        ));
    }
    let catalogue = artwork::inspect_artwork(job)?;
    for id in &request.item_ids {
        let item = catalogue.item(id).ok_or_else(|| {
            command_error(format!(
                "fit stock addresses unknown artwork item '{}'",
                id.0
            ))
        })?;
        if let Some(detail) = &item.import_error {
            return Err(command_error(format!(
                "artwork item '{}' failed to import, so its bounds are unknown: {detail}",
                id.0
            )));
        }
    }
    let bounds = catalogue.placed_bounds(&request.item_ids)?;
    Ok(FitStockProposal {
        rect: RectXY {
            min_x_mm: bounds.min_x_mm - margins.min_x_mm,
            min_y_mm: bounds.min_y_mm - margins.min_y_mm,
            width_mm: (bounds.max_x_mm - bounds.min_x_mm) + margins.min_x_mm + margins.max_x_mm,
            length_mm: (bounds.max_y_mm - bounds.min_y_mm) + margins.min_y_mm + margins.max_y_mm,
        },
    })
}

/// Apply a stock rectangle to the setup. This is the commit half of fit
/// stock (or any explicit stock edit): physical stock changes only here.
pub fn apply_stock_rectangle(job: &CamJobV5, rect: RectXY) -> Result<CommandOutcome> {
    let mut candidate = job.clone();
    candidate.setup.stock.xy = Some(rect);
    CommandOutcome::commit(candidate, vec![AffectedEntity::Setup])
}
