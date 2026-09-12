//! GUI2's bounded adapter over the released ui-9 service. Lives only in the
//! persistent compute process/Worker; neither a serialized plan nor a client
//! receipt can enter the retained service.
use crate::compute::{Package, SceneMeta, package};
use crate::sim::Motion;
use cam_core::project::v5::{self, CamJobV5, OperationSettingsV5};
use cam_service::collection::{CollectionCommand as C, CollectionScope};
use cam_service::retained::Retained;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;

pub const PROTOCOL: &str = "gui2-retained-5";
pub const FLOWER: &str = include_str!("../../../fixtures/gui2/flower.job.json");
pub const PROFILE: &str = include_str!("../../../fixtures/gui2/machine.json");
pub const MOTION_LIMIT: usize = 100_000;
/// Bound on the timeline's visible path groups (one per executed stage).
pub const MAX_DISPLAY_GROUPS: usize = 32;
thread_local! { static SERVICE: RefCell<Retained> = RefCell::new(Retained::new()); }
thread_local! { static DISPLAY: RefCell<Option<Display>> = const { RefCell::new(None) }; }
struct Display {
    fingerprint: String,
    playback: crate::sim::Playback,
    motions: Vec<Motion>,
    meta: crate::stock_preview::PreviewMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    Operation {
        job: String,
        action: crate::operation_authoring::Action,
    },
    ImportKnifeSvg {
        filename: String,
        svg: String,
    },
    ImportProfileSvg {
        filename: String,
        svg: String,
    },
    ImportSvg {
        filename: String,
        svg: String,
    },
    Artwork {
        job: String,
        /// The operation whose selection this command edits. Geometry belongs
        /// to one operation, so the command never guesses from document order.
        operation_id: String,
        action: ArtworkCommand,
    },
    Resource {
        job: String,
        action: Box<crate::resources::ResourceCommand>,
    },
    Preview {
        job: String,
    },
    ValidatePlan {
        job: String,
        handle: String,
        scope: GenerateScope,
    },
    Seek {
        handle: String,
        prefix: usize,
    },
    Open {
        json: String,
    },
    Migrate {
        json: String,
    },
    ApplyProfile {
        job: String,
        json: String,
    },
    Generate {
        job: String,
        scope: GenerateScope,
    },
    Prepare {
        job: String,
        handle: String,
    },
}

/// The executed prefix a generation request covers. `ThroughOperation` is the
/// plan scope GUI7 exports from: the retained plan binds it, so preparation can
/// never widen it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GenerateScope {
    AllEnabled,
    ThroughOperation { operation_id: String },
}

impl GenerateScope {
    pub fn wire(&self) -> CollectionScope {
        match self {
            Self::AllEnabled => CollectionScope::AllEnabled,
            Self::ThroughOperation { operation_id } => CollectionScope::ThroughOperation {
                operation_id: operation_id.clone(),
            },
        }
    }
    pub fn readiness(&self) -> v5::ReadinessScope {
        match self {
            Self::AllEnabled => v5::ReadinessScope::AllEnabled,
            Self::ThroughOperation { operation_id } => v5::ReadinessScope::ThroughOperation {
                operation_id: operation_id.clone(),
            },
        }
    }
}

impl Command {
    /// Generate every enabled operation. Scoped generation is an explicit
    /// [`GenerateScope`] so a prefix is never inferred from document order.
    pub fn generate(job: impl Into<String>) -> Self {
        Command::Generate {
            job: job.into(),
            scope: GenerateScope::AllEnabled,
        }
    }
    /// An artwork command for the whole job (the operation-scoped selection
    /// commands name their target operation explicitly).
    pub fn artwork(job: impl Into<String>, action: ArtworkCommand) -> Self {
        Command::Artwork {
            job: job.into(),
            operation_id: String::new(),
            action,
        }
    }
    /// Revalidate a retained plan against the whole enabled list.
    pub fn validate_plan(job: impl Into<String>, handle: impl Into<String>) -> Self {
        Command::ValidatePlan {
            job: job.into(),
            handle: handle.into(),
            scope: GenerateScope::AllEnabled,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ArtworkCommand {
    /// Replace the Flat V-carve operation's own filled-component selection.
    CarveSelection {
        references: Vec<v5::GeometryRef>,
    },
    KnifeOutlines {
        item: v5::ArtworkItemId,
    },
    KnifeStart {
        reference: Option<v5::GeometryRef>,
        fraction: f64,
    },
    KnifeSelection {
        references: Vec<v5::GeometryRef>,
    },
    /// Replace a Profile operation's closed-contour selection with explicit
    /// per-row sides.
    ProfileSelection {
        rows: Vec<crate::profile::SelectionRow>,
    },
    AddMany {
        files: Vec<crate::platform::SvgFile>,
    },
    Duplicate {
        item: v5::ArtworkItemId,
    },
    Reorder {
        items: Vec<v5::ArtworkItemId>,
    },
    Add {
        filename: String,
        svg: String,
    },
    Replace {
        item: v5::ArtworkItemId,
        filename: String,
        svg: String,
    },
    Delete {
        item: v5::ArtworkItemId,
    },
    Repair {
        expected: v5::GeometryRef,
        replacement: v5::GeometryRef,
    },
}

fn artwork_command(
    job: &CamJobV5,
    operation_id: &str,
    action: ArtworkCommand,
) -> Result<(CamJobV5, Value), String> {
    use v5::commands;
    let knife_target = if operation_id.is_empty() {
        crate::knife::settings(job).is_some()
    } else {
        crate::knife::settings_in(job, operation_id).is_some()
    };
    let interpretation = if knife_target {
        crate::knife::interpretation()
    } else {
        Default::default()
    };
    let target = if operation_id.is_empty() {
        job.operations
            .first()
            .map(|operation| operation.id.clone())
            .unwrap_or_default()
    } else {
        operation_id.to_owned()
    };
    let mut rejected = Vec::new();
    let (outcome, selected) = match action {
        ArtworkCommand::CarveSelection { references } => {
            if crate::session::settings_in(job, &target).is_none() {
                return Err("Select a Flat V-carve operation before selecting its geometry".into());
            }
            // Only the displayed catalogue's exact references are accepted:
            // a reference from a replaced source is reattached deliberately,
            // never rebound by re-sending it.
            let catalogue = v5::artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
            let available = crate::authoring::catalogue_components(&catalogue);
            if references
                .iter()
                .any(|reference| !available.iter().any(|c| &c.reference == reference))
            {
                return Err("Selected geometry changed; select the filled components again".into());
            }
            let outcome = commands::set_component_selection(
                job,
                &target,
                &crate::authoring::picks(&references),
            )
            .map_err(|e| e.to_string())?;
            return Ok((
                open(&outcome.job.to_json().map_err(|e| e.to_string())?)?,
                json!({"kind":"carve_selection","issues":outcome.issues}),
            ));
        }
        ArtworkCommand::KnifeOutlines { item } => {
            let result = crate::knife_outlines::create(job, &item)?;
            let active = result.artwork.last().map(|i| i.id.clone());
            return Ok((
                result,
                json!({"kind":"knife_outlines","activeArtwork":active}),
            ));
        }
        ArtworkCommand::KnifeStart {
            reference,
            fraction,
        } => {
            return Ok((
                crate::knife::set_start_in(job, &target, reference.as_ref(), fraction)?,
                json!({"kind":"knife_start"}),
            ));
        }
        ArtworkCommand::KnifeSelection { references } => {
            return Ok((
                crate::knife::select_in(job, &target, &references)?,
                json!({"kind":"knife_selection"}),
            ));
        }
        ArtworkCommand::ProfileSelection { rows } => {
            return Ok((
                crate::profile::select_in(job, &target, &rows)?,
                json!({"kind":"profile_selection"}),
            ));
        }
        ArtworkCommand::AddMany { files } => {
            let inputs = files
                .into_iter()
                .filter_map(|file| match file.content {
                    Ok(svg) => Some(commands::ArtworkInput {
                        filename: file.filename,
                        svg,
                        interpretation: interpretation.clone(),
                        placement: Default::default(),
                        name: None,
                    }),
                    Err(error) => {
                        rejected.push(format!("{}: {}", file.filename, error));
                        None
                    }
                })
                .collect();
            let result = commands::add_artwork(job, inputs).map_err(|e| e.to_string())?;
            rejected.extend(
                result
                    .rejected
                    .iter()
                    .map(|r| format!("{}: {}", r.filename, r.error)),
            );
            if result.outcome.affected.is_empty() {
                return Err(rejected.join("\n"));
            }
            let selected = result.outcome.job.artwork.last().map(|i| i.id.clone());
            (result.outcome, selected)
        }
        ArtworkCommand::Duplicate { item } => {
            let result =
                commands::duplicate_artwork(job, &item, None).map_err(|e| e.to_string())?;
            let selected = result
                .job
                .artwork
                .iter()
                .find(|i| !job.artwork.iter().any(|old| old.id == i.id))
                .map(|i| i.id.clone());
            (result, selected)
        }
        ArtworkCommand::Reorder { items } => (
            commands::reorder_artwork(job, &items).map_err(|e| e.to_string())?,
            None,
        ),
        ArtworkCommand::Add { filename, svg } => {
            let result = commands::add_artwork(
                job,
                vec![commands::ArtworkInput {
                    filename,
                    svg,
                    interpretation,
                    placement: Default::default(),
                    name: None,
                }],
            )
            .map_err(|e| e.to_string())?;
            if let Some(rejection) = result.rejected.first() {
                return Err(rejection.error.to_string());
            }
            let selected = result.outcome.job.artwork.last().map(|i| i.id.clone());
            (result.outcome, selected)
        }
        ArtworkCommand::Replace {
            item,
            filename,
            svg,
        } => (
            commands::replace_artwork(
                job,
                &item,
                cam_core::job::SourceSnapshot { filename, svg },
                None,
            )
            .map_err(|e| e.to_string())?,
            Some(item),
        ),
        ArtworkCommand::Delete { item } => (
            commands::remove_artwork(job, &item).map_err(|e| e.to_string())?,
            None,
        ),
        ArtworkCommand::Repair {
            expected,
            replacement,
        } => {
            if crate::session::operation(job, &target).is_none() {
                return Err("Select an operation before repairing its selection".into());
            }
            // Reject a target picked from an obsolete displayed catalogue.
            let catalogue = v5::artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
            if !crate::authoring::catalogue_components(&catalogue)
                .iter()
                .any(|c| c.reference == replacement)
            {
                return Err("Replacement source changed; pick the component again".into());
            }
            (
                commands::replace_component_reference(
                    job,
                    &target,
                    &expected,
                    &v5::artwork::GeometryPick {
                        artwork_item_id: replacement.artwork_item_id.clone(),
                        kind: replacement.kind,
                        local_geometry_id: replacement.local_geometry_id,
                    },
                )
                .map_err(|e| e.to_string())?,
                None,
            )
        }
    };
    let report = json!({"kind":"artwork", "activeArtwork":selected, "issues":outcome.issues,"rejectedFiles":rejected});
    let job = open(&outcome.job.to_json().map_err(|e| e.to_string())?)?;
    Ok((job, report))
}

pub fn open(text: &str) -> Result<CamJobV5, String> {
    if text.len() > 8_000_000 {
        return Err("GUI2 supports jobs up to 8 MB".into());
    }
    let job = CamJobV5::from_json(text).map_err(|e| e.to_string())?;
    if job.operations.len() > crate::operation_authoring::MAX_OPERATIONS
        || job.operations.iter().any(|op| {
            !matches!(
                op.settings,
                OperationSettingsV5::FlatVcarve(_)
                    | OperationSettingsV5::Face(_)
                    | OperationSettingsV5::Profile(_)
                    | OperationSettingsV5::DragKnife(_)
            )
        })
        || job
            .artwork
            .iter()
            .any(|item| !matches!(item.content, v5::ArtworkContent::Svg(_)))
    {
        return Err(format!(
            "This workspace supports SVG artwork and up to {} ordered Flat V-carve, Face, Profile or Drag knife operations; the current document was retained",
            crate::operation_authoring::MAX_OPERATIONS
        ));
    }
    Ok(job)
}

/// The operation with this stable ID, if the document still carries it.
pub fn operation<'a>(job: &'a CamJobV5, id: &str) -> Option<&'a v5::OperationV5> {
    job.operations.iter().find(|op| op.id == id)
}

pub fn operation_mut<'a>(job: &'a mut CamJobV5, id: &str) -> Option<&'a mut v5::OperationV5> {
    job.operations.iter_mut().find(|op| op.id == id)
}

/// The selected operation's kind, or None when the job has no operations.
pub fn kind(job: &CamJobV5, id: &str) -> Option<OperationKind> {
    Some(match &operation(job, id)?.settings {
        OperationSettingsV5::FlatVcarve(_) => OperationKind::FlatVcarve,
        OperationSettingsV5::Face(_) => OperationKind::Face,
        OperationSettingsV5::Profile(_) => OperationKind::Profile,
        OperationSettingsV5::DragKnife(_) => OperationKind::DragKnife,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    FlatVcarve,
    Face,
    Profile,
    DragKnife,
}

/// Display name of one operation's kind.
pub fn kind_label(job: &CamJobV5, id: &str) -> &'static str {
    match kind(job, id) {
        Some(OperationKind::Face) => "Face",
        Some(OperationKind::FlatVcarve) => "Flat V-carve",
        Some(OperationKind::DragKnife) => "Drag knife",
        Some(OperationKind::Profile) => "Profile",
        None => "Operation",
    }
}

pub fn face<'a>(job: &'a CamJobV5, id: &str) -> Option<&'a v5::FaceSettingsV5> {
    match &operation(job, id)?.settings {
        OperationSettingsV5::Face(settings) => Some(settings),
        _ => None,
    }
}

pub fn face_mut<'a>(job: &'a mut CamJobV5, id: &str) -> Option<&'a mut v5::FaceSettingsV5> {
    match &mut operation_mut(job, id)?.settings {
        OperationSettingsV5::Face(settings) => Some(settings),
        _ => None,
    }
}

/// The Flat V-carve settings of the first Flat V-carve operation. Kept for the
/// GUI2–GUI6 call sites that still address the established carving workflow;
/// new code addresses an explicit operation ID.
pub fn settings(job: &CamJobV5) -> &v5::FlatVcarveSettingsV5 {
    carving(job).expect("operation is a Flat V-carve")
}

pub fn settings_in<'a>(job: &'a CamJobV5, id: &str) -> Option<&'a v5::FlatVcarveSettingsV5> {
    match &operation(job, id)?.settings {
        OperationSettingsV5::FlatVcarve(settings) => Some(settings),
        _ => None,
    }
}

pub fn carving(job: &CamJobV5) -> Option<&v5::FlatVcarveSettingsV5> {
    job.operations.iter().find_map(|op| match &op.settings {
        OperationSettingsV5::FlatVcarve(s) => Some(s),
        _ => None,
    })
}

pub fn run(command: Command) -> Result<(SceneMeta, Vec<u8>), String> {
    SERVICE.with(|service| execute(&mut service.borrow_mut(), command))
}

fn terminal(reply: &Value) -> Result<(), String> {
    if reply["task"]["state"] == "succeeded" {
        return Ok(());
    }
    Err(format!("{}", reply["task"]["diagnostic"]))
}

/// Enabled operations inside a scope, in document order. A prefix scope that
/// names an unknown operation is an error, never a silent whole-job plan.
fn scoped_operations<'a>(
    job: &'a CamJobV5,
    scope: &v5::ReadinessScope,
) -> Result<Vec<&'a v5::OperationV5>, String> {
    match scope {
        v5::ReadinessScope::AllEnabled => {
            Ok(job.operations.iter().filter(|op| op.enabled).collect())
        }
        v5::ReadinessScope::ThroughOperation { operation_id } => {
            let Some(end) = job.operations.iter().position(|op| op.id == *operation_id) else {
                return Err(format!(
                    "The selected operation '{operation_id}' is no longer in this job"
                ));
            };
            Ok(job.operations[..=end]
                .iter()
                .filter(|op| op.enabled)
                .collect())
        }
    }
}

pub fn execute(service: &mut Retained, command: Command) -> Result<(SceneMeta, Vec<u8>), String> {
    let (job, report) = match command {
        Command::Seek { handle, prefix } => {
            let plan = service.generated_plan(&handle).map_err(|e| e.to_string())?;
            return DISPLAY.with(|display| {
                let mut display = display.borrow_mut();
                let display = display.as_mut().filter(|d| d.fingerprint == plan.trusted.plan().execution_fingerprint).ok_or("Display execution expired; generate again")?;
                if prefix > display.motions.len() { return Err("Stock motion outside retained execution".into()); }
                display.playback.seek(&display.motions, prefix)?;
                let field = &display.playback.field;
                let cells = field.packed_tile_bytes();
                let mut meta = display.meta.clone();
                meta.frames = vec![crate::stock_preview::FrameMeta { prefix, stats:field.stats.clone(),checksum:field.checksum(),versions:field.versions.clone(),allocated:field.versions.iter().enumerate().filter(|(i,_)|field.tile_allocated(*i)).map(|(i,_)|i as u32).collect() }];
                meta.retained_bytes = cells.len();
                package(Package {name:String::new(),job:String::new(),report:json!({"protocol":PROTOCOL,"gui2":{"kind":"seek","prefix":prefix,"handle":handle}}),programs:vec![],bounds:[0.,0.,1.,1.],contour_vertices:0,rough_vertices:0,vertices:vec![],preview:Some(crate::stock_preview::Preview {meta,cells:vec![cells]}),sim:None})
            });
        }
        Command::ImportKnifeSvg { filename, svg } => (
            crate::knife::import_svg(filename, svg)?,
            json!({"kind":"imported"}),
        ),
        Command::ImportProfileSvg { filename, svg } => (
            crate::profile::import_svg(filename, svg)?,
            json!({"kind":"imported"}),
        ),
        Command::Operation { job, action } => {
            // Adding an operation selects it so its unset inputs are what the
            // editor addresses next; every other edit keeps the current
            // selection unless it no longer exists.
            let active = match &action {
                crate::operation_authoring::Action::Add { operation_id, .. } => {
                    Some(operation_id.clone())
                }
                _ => None,
            };
            (
                crate::operation_authoring::apply(&open(&job)?, action)?,
                json!({"kind":"operation","activeOperation":active}),
            )
        }
        Command::ImportSvg { filename, svg } => (
            crate::authoring::import_svg(filename, svg)?,
            json!({"kind":"imported"}),
        ),
        Command::Artwork {
            job,
            operation_id,
            action,
        } => artwork_command(&open(&job)?, &operation_id, action)?,
        Command::Resource { job, action } => {
            let job = open(&job)?;
            let clear = action.clear_fields(&job);
            (
                (*action).execute(&job)?,
                json!({"kind":"resource","clearFields":clear}),
            )
        }
        Command::Preview { job } => (open(&job)?, json!({"kind":"preview"})),
        Command::ValidatePlan { job, handle, scope } => {
            let job = open(&job)?;
            let retained = service.generated_plan(&handle).ok();
            let identity =
                cam_core::sequence::OperationPlanV5::machining_identity(&job, &scope.readiness())
                    .ok();
            if let Some(retained) =
                retained.filter(|p| Some(&p.machining_identity) == identity.as_ref())
            {
                let plan = retained.trusted.plan();
                return scene(
                    &job,
                    plan,
                    json!({"kind":"revalidated","handle":handle,"scope":scope,"executionFingerprint":plan.execution_fingerprint,"checks":retained.checks,"generationIssues":plan.generation_diagnostics}),
                );
            }
            return outline(&job, json!({"kind":"stale"}));
        }
        Command::Open { json } => (open(&json)?, json!({"kind":"opened"})),
        Command::Migrate { json } => {
            if json.len() > 8_000_000 {
                return Err("Input exceeds 8 MB limit".into());
            }
            let job = v5::migrate::migrate_json(&json).map_err(|e| e.to_string())?;
            (
                open(&job.to_json().map_err(|e| e.to_string())?)?,
                json!({"kind":"opened","migrated":true}),
            )
        }
        Command::ApplyProfile { job, json } => {
            let mut job = open(&job)?;
            let machine = cam_core::post::sequence::SequenceProfile::from_json(&json)
                .map_err(|e| e.to_string())?;
            let profile = v5::machine::apply_machine_configuration(&job, &machine, &machine.id)
                .map_err(|e| e.to_string())?;
            job = profile.job;
            job.setup.clearance_above_stock_mm = Some(machine.clearance_z_mm);
            (job, json!({"kind":"profile"}))
        }
        Command::Generate { job, scope } => {
            let job = open(&job)?;
            if job.operations.is_empty() {
                return Err("Add an operation before generating".into());
            }
            let readiness = scope.readiness();
            let mut issues = Vec::new();
            for operation in scoped_operations(&job, &readiness)? {
                issues.extend(
                    v5::inspection::inspect_operation_fields(&job, &operation.id)
                        .map_err(|e| e.to_string())?,
                );
            }
            issues.extend(
                v5::references::planning_readiness(&job, &readiness)
                    .map_err(|e| e.to_string())?
                    .blockers(),
            );
            if !issues.is_empty() {
                return outline(&job, json!({"kind":"issues", "issues":issues}));
            }
            let reply = service
                .execute_driven(C::Generate {
                    job: json!(job),
                    scope: scope.wire(),
                })
                .map_err(|e| e.to_string())?;
            terminal(&reply)?;
            let handle = reply["task"]["planHandle"]
                .as_str()
                .ok_or("Missing retained plan handle")?;
            let retained = service.generated_plan(handle).map_err(|e| e.to_string())?;
            let plan = retained.trusted.plan();
            if plan.motions.len() > MOTION_LIMIT {
                return Err("GUI2 exceeds 100,000 motions; no truncated scene admitted".into());
            }
            let result = scene(
                &job,
                plan,
                json!({"kind":"generated", "handle":handle, "scope":scope,
                "checks":retained.checks, "generationIssues":plan.generation_diagnostics, "executionFingerprint":plan.execution_fingerprint, "retained":reply["retained"]}),
            )?;
            let scene = crate::compute::Scene {
                meta: result.0.clone(),
                payload: std::sync::Arc::new(result.1.clone()),
            };
            // An incomplete plan has no executed stage: the scene carries the
            // inspection and reasons, and there is nothing to seed playback
            // with. Everything else keeps its stock preview.
            let (Some(input), Some(meta)) = (scene.sim_input()?, scene.meta.stock.as_ref()) else {
                return Ok(result);
            };
            let mut seed = Vec::new();
            for (index, frame) in meta.frames.iter().enumerate() {
                let field = crate::sim::Field::from_packed(
                    meta.stock,
                    &input.tools,
                    meta.cell_mm,
                    scene.stock_cells(index).ok_or("Missing checkpoint")?,
                    &frame.versions,
                    &frame.allocated,
                    frame.stats.clone(),
                )?;
                seed.push((frame.prefix, field));
            }
            let playback = crate::sim::Playback::seed(
                seed.last().ok_or("No checkpoints")?.1.clone(),
                seed[0].1.clone(),
                seed,
                crate::stock_preview::MAX_PREVIEW_BYTES,
            );
            DISPLAY.with(|d| {
                *d.borrow_mut() = Some(Display {
                    fingerprint: plan.execution_fingerprint.clone(),
                    playback,
                    motions: input.motions,
                    meta: meta.clone(),
                })
            });
            return Ok(result);
        }
        Command::Prepare { job, handle } => {
            let job = open(&job)?;
            let reply = service
                .execute_driven(C::PrepareOutput {
                    job: json!(job),
                    plan_handle: handle,
                    layout: cam_core::post::sequence::OutputLayout::OneProgram,
                })
                .map_err(|e| e.to_string())?;
            terminal(&reply)?;
            let task = reply["task"]["taskId"]
                .as_str()
                .ok_or("Missing preparation task")?;
            let bundle = service
                .execute(C::PreparedOutput {
                    task_id: task.into(),
                })
                .map_err(|e| e.to_string())?;
            let handle = bundle["bundle"]["bundleHandle"]
                .as_str()
                .ok_or("Missing bundle")?;
            let file = bundle["bundle"]["files"][0]["filename"]
                .as_str()
                .ok_or("Missing checked file")?;
            let bytes = service
                .execute(C::ReadPreparedBytes {
                    bundle_handle: handle.into(),
                    filename: file.into(),
                })
                .map_err(|e| e.to_string())?;
            (
                job,
                json!({"kind":"prepared", "file":bytes["file"], "bundle":bundle["bundle"], "retained":bytes["retained"]}),
            )
        }
    };
    outline(&job, report)
}

fn outline(job: &CamJobV5, report: Value) -> Result<(SceneMeta, Vec<u8>), String> {
    crate::scene::build(job, None, report)
}

pub fn scene(
    job: &CamJobV5,
    plan: &cam_core::sequence::OperationPlanV5,
    report: Value,
) -> Result<(SceneMeta, Vec<u8>), String> {
    crate::scene::build(job, Some(plan), report)
}
