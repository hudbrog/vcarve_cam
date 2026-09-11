//! H5 retained execution through the ui-9 transport (plan section 22.8,
//! checklist scenario 9): cancellable tasks, leased handles, paging
//! without replanning, scope-bound preparation, exact-byte retry, and the
//! proof that late replies, stale documents and evicted handles can never
//! authorize output.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    post::sequence::OutputLayout,
    project::v5::{self, ArtworkContent, ArtworkItemId, CamJobV5, GeometryRef, GeometryRefKind},
    svg::{ImportMode, Placement},
};
use cam_service::{
    collection::{CollectionCommand, CollectionScope, execute},
    retained::{RETAINED_BUNDLES, RETAINED_PLANS, Retained},
};
use serde_json::{Value, json};

/// One filled plate plus one stroked centerline (the H3/H4 fixture).
const PLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="16" height="10" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M25 25 L35 25"/></svg>"##;

fn plate_item() -> v5::ArtworkItem {
    v5::ArtworkItem {
        id: ArtworkItemId("plate".into()),
        name: "Plate".into(),
        content: ArtworkContent::Svg(SourceSnapshot {
            filename: "plate.svg".into(),
            svg: PLATE.into(),
        }),
        import_settings: v5::SvgInterpretation {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            mode: ImportMode::Centerline,
        },
        placement: Placement {
            origin_mm: Point::new(0., 0.),
            scale: 1.,
            rotation_deg: 0.,
        },
    }
}

fn tools() -> Vec<v5::JobToolV5> {
    vec![
        v5::JobToolV5 {
            id: "t1".into(),
            name: "3mm endmill".into(),
            geometry: Some(cam_core::project::ToolGeometry::Endmill(
                cam_core::project::EndmillGeometry {
                    diameter_mm: 3.,
                    cutting_length_mm: 8.,
                },
            )),
            capabilities: Default::default(),
            library_origin: None,
        },
        v5::JobToolV5 {
            id: "t3".into(),
            name: "drag knife".into(),
            geometry: Some(cam_core::project::ToolGeometry::DragKnife(
                cam_core::project::DragKnifeSpec {
                    blade_offset_mm: 1.,
                    max_cut_depth_mm: 2.,
                },
            )),
            capabilities: Default::default(),
            library_origin: None,
        },
    ]
}

fn catalogue_reference(kind: GeometryRefKind) -> GeometryRef {
    let job = base_job(vec![]);
    let combined = v5::artwork::inspect_artwork(&job).unwrap();
    combined
        .item(&ArtworkItemId("plate".into()))
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.kind == kind)
        .unwrap()
        .reference
        .clone()
}

/// The profile depth is the per-job varying effective value: distinct
/// depths are distinct machining identities and distinct plans.
fn profile_operation(depth: f64) -> v5::OperationV5 {
    v5::OperationV5 {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: v5::OperationSettingsV5::Profile(v5::ProfileSettingsV5 {
            contours: vec![v5::ProfileContourV5 {
                geometry: catalogue_reference(GeometryRefKind::ClosedContour),
                side: cam_core::project::ContourSide::Outside,
                traversal: None,
            }],
            assignment: v5::MillingAssignmentV5 {
                tool_id: "t1".into(),
                spindle_rpm: Some(10_000.),
                spindle_direction: Some(cam_core::project::SpindleDirection::Clockwise),
                cutting_feed_mm_min: Some(300.),
                plunge_feed_mm_min: Some(100.),
                max_stepdown_mm: Some(depth),
                stepover_mm: Some(1.5),
                applied_profile: None,
            },
            top: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::OperationTop,
                offset_mm: -depth,
            },
            stepdown_mm: Some(depth),
            through_cut_allowance_mm: None,
            direction: Some(cam_core::project::CutDirection::Climb),
            order: Default::default(),
            start: Default::default(),
            finish: Default::default(),
            entry: cam_core::project::ProfileEntry::Plunge,
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    }
}

fn knife_operation() -> v5::OperationV5 {
    v5::OperationV5 {
        id: "knife-1".into(),
        name: "Knife".into(),
        enabled: true,
        settings: v5::OperationSettingsV5::DragKnife(v5::DragKnifeSettingsV5 {
            chains: vec![catalogue_reference(GeometryRefKind::Centerline)],
            assignment: v5::KnifeAssignmentV5 {
                tool_id: "t3".into(),
                cutting_feed_mm_min: Some(150.),
                plunge_feed_mm_min: Some(50.),
                swivel_feed_mm_min: Some(75.),
                max_stepdown_mm: Some(1.),
                applied_profile: None,
            },
            top: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockTop,
                offset_mm: -1.,
            },
            stepdown_mm: Some(1.),
            swivel_depth_mm: Some(0.5),
            corner_threshold_deg: Some(90.),
            through_cut_allowance_mm: None,
            start: Default::default(),
            closure_overlap_mm: None,
            alignment: cam_core::project::KnifeAlignment {
                initial_heading_deg: Some(180.),
            },
        }),
    }
}

fn base_job(operations: Vec<v5::OperationV5>) -> CamJobV5 {
    CamJobV5 {
        schema_version: v5::CAM_JOB_V5_SCHEMA_VERSION,
        name: "collection".into(),
        setup: cam_core::project::SetupSettings {
            stock: cam_core::project::StockSetup {
                thickness_mm: Some(8.),
                xy: Some(cam_core::project::RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        artwork: vec![plate_item()],
        tools: tools(),
        operations,
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    }
}

fn job_value(depth: f64) -> Value {
    serde_json::to_value(base_job(vec![profile_operation(depth), knife_operation()])).unwrap()
}

/// The document with the machine configuration applied (both tools mapped).
fn configured_job(depth: f64) -> Value {
    let applied = execute(CollectionCommand::ApplyMachineConfiguration {
        job: job_value(depth),
        profile: machine_profile(json!([
            {"tool_id": "t1", "tool_number": 3, "length_offset_number": null},
            {"tool_id": "t3", "tool_number": 7, "length_offset_number": null},
        ])),
        configuration_name: "Workbench".into(),
    })
    .unwrap();
    applied["job"].clone()
}

fn machine_profile(tool_rows: Value) -> Value {
    json!({
        "schema_version": 2,
        "id": "workbench",
        "work_offset": "G54",
        "clearance_z_mm": 5.0,
        "decimal_places": 3,
        "program_start_position_mm": null,
        "length_compensation": "macro_managed",
        "path_control": {"kind": "exact_path"},
        "tools": tool_rows,
        "spindle_spinup_seconds": 0.5,
        "coolant": "off",
        "m6": {
            "reference": "test-contract",
            "reviewed": true,
            "return_position": {"kind": "caller_position"},
            "preserves_work_datum": true,
            "local_offsets_unused": true,
            "tool_offsets_z_only": true
        }
    })
}

fn prefix_scope() -> CollectionScope {
    CollectionScope::ThroughOperation {
        operation_id: "profile-1".into(),
    }
}

/// Scenario 9: plan a prefix, page its motions without re-running the
/// planners, prepare and export that prefix, fail and retry a save of
/// identical bytes — and the prefix never exports the full job.
#[test]
fn prefix_pages_prepares_and_retries_identical_bytes() {
    let mut runtime = Retained::new();
    let job = configured_job(2.0);
    // Register, then drive: the two-step contract the async adapters use.
    let registered = runtime
        .execute(CollectionCommand::Generate {
            job: job.clone(),
            scope: prefix_scope(),
        })
        .unwrap();
    let task_id = registered["task"]["taskId"].as_str().unwrap().to_string();
    assert_eq!(registered["task"]["state"], json!("pending"));
    runtime.run_task(&task_id);
    let done = runtime
        .execute(CollectionCommand::TaskStatus {
            task_id: task_id.clone(),
        })
        .unwrap();
    assert_eq!(done["task"]["state"], json!("succeeded"));
    let plan_handle = done["task"]["planHandle"].as_str().unwrap().to_string();
    assert_eq!(done["retained"]["plansRun"], json!(1));

    // Page the complete ordered stream from the retained plan. Every page
    // (and re-reading earlier pages) must leave the planner untouched: the
    // retained generation is the only one that ever ran.
    let mut offset = 0;
    let total = loop {
        let page = runtime
            .execute(CollectionCommand::ReadMotions {
                plan_handle: plan_handle.clone(),
                offset,
            })
            .unwrap();
        let motions = &page["motions"];
        let page_count = motions["count"].as_u64().unwrap() as usize;
        assert!(page_count > 0, "empty page at {offset}");
        assert_eq!(motions["offset"], json!(offset));
        offset += page_count;
        if offset == motions["total"].as_u64().unwrap() as usize {
            break offset;
        }
    };
    let repaged = runtime
        .execute(CollectionCommand::ReadMotions {
            plan_handle: plan_handle.clone(),
            offset: 0,
        })
        .unwrap();
    assert_eq!(repaged["retained"]["plansRun"], json!(1));
    // The retained stream agrees one-for-one with a fresh one-shot plan of
    // the same scope, and the full document has strictly more work: a
    // prefix plan can never serve the full job.
    let fresh = execute(CollectionCommand::Plan {
        job: job.clone(),
        scope: prefix_scope(),
    })
    .unwrap();
    assert_eq!(
        total as u64,
        fresh["summary"]["motionCount"].as_u64().unwrap(),
        "retained prefix stream equals a fresh plan of the same scope"
    );
    let full = execute(CollectionCommand::Plan {
        job: job.clone(),
        scope: CollectionScope::AllEnabled,
    })
    .unwrap();
    assert!(
        full["summary"]["motionCount"].as_u64().unwrap() > total as u64,
        "the full job must have more motions than the prefix"
    );

    // Prepare the prefix: only the prefix scope is bound, and only the
    // prefix's stage appears in the bundle.
    let prepared_task = runtime
        .execute(CollectionCommand::PrepareOutput {
            plan_handle: plan_handle.clone(),
            job: job.clone(),
            layout: OutputLayout::SequentialFiles,
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    runtime.run_task(&prepared_task);
    let bundle = runtime
        .execute(CollectionCommand::PreparedOutput {
            task_id: prepared_task,
        })
        .unwrap();
    assert_eq!(bundle["retained"]["preparesRun"], json!(1));
    let bundle_handle = bundle["bundle"]["bundleHandle"]
        .as_str()
        .unwrap()
        .to_string();
    let files = bundle["bundle"]["files"].as_array().unwrap().clone();
    assert_eq!(files.len(), 1, "the prefix exports exactly its own stage");
    assert_eq!(files[0]["stageIds"], json!(["profile-1-profile-rough"]));
    assert_eq!(files[0]["toolNumber"], json!(3));
    assert_eq!(
        bundle["bundle"]["manifest"]["executionFingerprint"],
        fresh["summary"]["executionFingerprint"]
    );

    // Exact-byte read, a failed save, and the retry: identical bytes.
    let filename = files[0]["filename"].as_str().unwrap().to_string();
    let read = |runtime: &mut Retained| {
        runtime
            .execute(CollectionCommand::ReadPreparedBytes {
                bundle_handle: bundle_handle.clone(),
                filename: filename.clone(),
            })
            .unwrap()["file"]
            .clone()
    };
    let first = read(&mut runtime);
    assert!(first["gcode"].as_str().unwrap().contains("T3 M6"));
    let retry = read(&mut runtime);
    assert_eq!(first["gcode"], retry["gcode"]);
    assert_eq!(first["sha256"], retry["sha256"]);
    assert_eq!(first["sha256"], files[0]["sha256"]);
    assert_eq!(
        first["byteLength"].as_u64().unwrap() as usize,
        first["gcode"].as_str().unwrap().len()
    );
    // An unknown file name in a live bundle is a located refusal.
    let missing = runtime
        .execute(CollectionCommand::ReadPreparedBytes {
            bundle_handle,
            filename: "99-absent.ngc".into(),
        })
        .unwrap_err();
    assert_eq!(missing.code, "RETAINED_FILE_NOT_FOUND");
}

/// A relevant edit makes preparation stale; output-only edits do not. A
/// rename keeps the bytes identical, a work-zero move rebinds the retained
/// execution without replanning (same execution fingerprint, different
/// output bytes), and a used-source edit refuses with a located error.
#[test]
fn stale_machining_rejects_preparation_output_only_edits_rebind() {
    let mut runtime = Retained::new();
    let job = configured_job(2.0);
    let task = runtime
        .execute_driven(CollectionCommand::Generate {
            job: job.clone(),
            scope: prefix_scope(),
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    let plan_handle = runtime
        .execute(CollectionCommand::TaskStatus { task_id: task })
        .unwrap()["task"]["planHandle"]
        .as_str()
        .unwrap()
        .to_string();
    let prepare = |runtime: &mut Retained, job: &Value| -> Value {
        let task = runtime
            .execute_driven(CollectionCommand::PrepareOutput {
                plan_handle: plan_handle.clone(),
                job: job.clone(),
                layout: OutputLayout::OneProgram,
            })
            .unwrap()["task"]["taskId"]
            .as_str()
            .unwrap()
            .to_string();
        runtime
            .execute(CollectionCommand::PreparedOutput { task_id: task })
            .unwrap()
    };
    // Baseline bytes.
    let baseline = prepare(&mut runtime, &job);
    let baseline_sha = baseline["bundle"]["files"][0]["sha256"].clone();
    let baseline_fingerprint = baseline["bundle"]["manifest"]["executionFingerprint"].clone();
    // Renaming an operation changes neither machining nor output bytes.
    let mut renamed = job.clone();
    renamed["operations"][0]["name"] = json!("Renamed profile");
    let renamed_bundle = prepare(&mut runtime, &renamed);
    assert_eq!(
        renamed_bundle["bundle"]["files"][0]["sha256"], baseline_sha,
        "display names never enter machining or output"
    );
    // A work-zero move is output-only: preparation rebinds the retained
    // setup-coordinate motions to the current selection without replanning
    // — the execution fingerprint is unchanged, the output bytes are not.
    let mut moved = job.clone();
    moved["setup"]["work_zero"]["z"] = json!("stock_bottom");
    let moved_bundle = prepare(&mut runtime, &moved);
    assert_eq!(
        moved_bundle["retained"]["plansRun"],
        json!(1),
        "rebinding never replans"
    );
    assert_eq!(
        moved_bundle["bundle"]["manifest"]["executionFingerprint"], baseline_fingerprint,
        "the retained execution is the same one"
    );
    assert_ne!(
        moved_bundle["bundle"]["files"][0]["sha256"], baseline_sha,
        "the work-zero transform must change the output bytes"
    );
    // A used-source edit (placement) stales the retained plan: the stale
    // completion cannot authorize output.
    let mut edited = job.clone();
    edited["artwork"][0]["placement"]["origin_mm"] = json!({"x": 2.0, "y": 0.0});
    let stale_task = runtime
        .execute_driven(CollectionCommand::PrepareOutput {
            plan_handle: plan_handle.clone(),
            job: edited,
            layout: OutputLayout::OneProgram,
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    let status = runtime
        .execute(CollectionCommand::TaskStatus {
            task_id: stale_task.clone(),
        })
        .unwrap();
    assert_eq!(status["task"]["state"], json!("failed"), "{status}");
    let error = runtime
        .execute(CollectionCommand::PreparedOutput {
            task_id: stale_task,
        })
        .unwrap_err();
    assert_eq!(error.code, "RETAINED_PLAN_STALE", "{error:?}");
    // The failed preparation issued no bundle; the three output-only
    // preparations (baseline, rename, work zero) each issued one.
    assert_eq!(status["retained"]["retainedBundles"], json!(3));
}

/// Cancellation is the single authoritative winner: a completion arriving
/// after cancellation is discarded and issues no handle; cancelling a
/// finished task changes nothing.
#[test]
fn cancelled_completions_cannot_issue_handles() {
    let mut runtime = Retained::new();
    let job = configured_job(2.0);
    // Cancel before the work starts: nothing runs, nothing is issued.
    let task = runtime
        .execute(CollectionCommand::Generate {
            job: job.clone(),
            scope: prefix_scope(),
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    let cancelled = runtime
        .execute(CollectionCommand::CancelTask {
            task_id: task.clone(),
        })
        .unwrap();
    assert_eq!(cancelled["task"]["state"], json!("cancelled"));
    runtime.run_task(&task);
    let after = runtime
        .execute(CollectionCommand::TaskStatus {
            task_id: task.clone(),
        })
        .unwrap();
    assert_eq!(after["task"]["state"], json!("cancelled"));
    assert_eq!(after["task"]["planHandle"], Value::Null);
    assert_eq!(after["retained"]["plansRun"], json!(0));
    assert_eq!(after["retained"]["retainedPlans"], json!(0));
    // The race an async adapter really hits: claim, cancel, deliver — the
    // completed plan is discarded, exactly one authoritative winner.
    let task = runtime
        .execute(CollectionCommand::Generate {
            job: job.clone(),
            scope: prefix_scope(),
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    let claimed = runtime.claim(&task).unwrap().unwrap();
    runtime
        .execute(CollectionCommand::CancelTask {
            task_id: task.clone(),
        })
        .unwrap();
    let outcome = cam_service::retained::compute(claimed).unwrap();
    runtime.complete(&task, Ok(outcome));
    let late = runtime
        .execute(CollectionCommand::TaskStatus {
            task_id: task.clone(),
        })
        .unwrap();
    assert_eq!(late["task"]["state"], json!("cancelled"));
    assert_eq!(late["task"]["planHandle"], Value::Null);
    assert_eq!(late["retained"]["retainedPlans"], json!(0));
    assert_eq!(
        late["retained"]["plansRun"],
        json!(1),
        "the work ran; the result still cannot authorize anything"
    );
    // A cancelled preparation is refused with its own located code.
    let plan_task = runtime
        .execute_driven(CollectionCommand::Generate {
            job: job.clone(),
            scope: prefix_scope(),
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    let plan_handle = runtime
        .execute(CollectionCommand::TaskStatus { task_id: plan_task })
        .unwrap()["task"]["planHandle"]
        .as_str()
        .unwrap()
        .to_string();
    let prep_task = runtime
        .execute(CollectionCommand::PrepareOutput {
            plan_handle,
            job: job.clone(),
            layout: OutputLayout::OneProgram,
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    runtime
        .execute(CollectionCommand::CancelTask {
            task_id: prep_task.clone(),
        })
        .unwrap();
    let cancelled = runtime
        .execute(CollectionCommand::PreparedOutput { task_id: prep_task })
        .unwrap_err();
    assert_eq!(cancelled.code, "RETAINED_TASK_CANCELLED");
    // Cancel after success changes nothing (terminal state is final).
    let task = runtime
        .execute_driven(CollectionCommand::Generate {
            job,
            scope: prefix_scope(),
        })
        .unwrap()["task"]["taskId"]
        .as_str()
        .unwrap()
        .to_string();
    let done = runtime
        .execute(CollectionCommand::CancelTask {
            task_id: task.clone(),
        })
        .unwrap();
    assert_eq!(done["task"]["state"], json!("succeeded"));
    // A generation task is not an output preparation.
    let kind = runtime
        .execute(CollectionCommand::PreparedOutput { task_id: task })
        .unwrap_err();
    assert_eq!(kind.code, "RETAINED_TASK_KIND");
}

/// Retention is leased: pushing past the bounded sets expires the oldest
/// handles, and an expired handle can no longer authorize motions or bytes.
#[test]
fn evicted_handles_cannot_authorize_output() {
    let mut runtime = Retained::new();
    let mut first_plan = String::new();
    let mut first_bundle = String::new();
    let mut last_plan = String::new();
    let mut last_job = Value::Null;
    // RETAINED_PLANS + 1 distinct jobs: the first plan handle expires while
    // the newest stays live.
    for index in 0..=RETAINED_PLANS {
        let depth = 2.0 + index as f64 * 0.1;
        let job = configured_job(depth);
        let task = runtime
            .execute_driven(CollectionCommand::Generate {
                job: job.clone(),
                scope: prefix_scope(),
            })
            .unwrap()["task"]["taskId"]
            .as_str()
            .unwrap()
            .to_string();
        let handle = runtime
            .execute(CollectionCommand::TaskStatus { task_id: task })
            .unwrap()["task"]["planHandle"]
            .as_str()
            .unwrap()
            .to_string();
        if index == 0 {
            first_plan = handle.clone();
            // The bundle that will be evicted: prepared while its plan is
            // still the only one live.
            let prepare_task = runtime
                .execute_driven(CollectionCommand::PrepareOutput {
                    plan_handle: handle.clone(),
                    job: job.clone(),
                    layout: OutputLayout::OneProgram,
                })
                .unwrap()["task"]["taskId"]
                .as_str()
                .unwrap()
                .to_string();
            first_bundle = runtime
                .execute(CollectionCommand::PreparedOutput {
                    task_id: prepare_task,
                })
                .unwrap()["bundle"]["bundleHandle"]
                .as_str()
                .unwrap()
                .to_string();
        }
        last_plan = handle;
        last_job = job;
    }
    // The oldest plan expired; the newest is still live.
    let expired = runtime
        .execute(CollectionCommand::ReadMotions {
            plan_handle: first_plan,
            offset: 0,
        })
        .unwrap_err();
    assert_eq!(expired.code, "RETAINED_HANDLE_EXPIRED", "{expired:?}");
    runtime
        .execute(CollectionCommand::ReadMotions {
            plan_handle: last_plan.clone(),
            offset: 0,
        })
        .unwrap();
    // RETAINED_BUNDLES more preparations from the still-live plan evict the
    // first bundle: its exact bytes can no longer be read at all.
    for _ in 0..RETAINED_BUNDLES {
        runtime
            .execute_driven(CollectionCommand::PrepareOutput {
                plan_handle: last_plan.clone(),
                job: last_job.clone(),
                layout: OutputLayout::OneProgram,
            })
            .unwrap();
    }
    let expired_bundle = runtime
        .execute(CollectionCommand::ReadPreparedBytes {
            bundle_handle: first_bundle,
            filename: "sequence.ngc".into(),
        })
        .unwrap_err();
    assert_eq!(
        expired_bundle.code, "RETAINED_HANDLE_EXPIRED",
        "{expired_bundle:?}"
    );
    // Preparing from the expired plan handle is refused too.
    let stale = runtime
        .execute(CollectionCommand::PrepareOutput {
            plan_handle: "plan-1".into(),
            job: last_job,
            layout: OutputLayout::OneProgram,
        })
        .unwrap_err();
    assert_eq!(stale.code, "RETAINED_HANDLE_EXPIRED", "{stale:?}");
}

/// Capabilities advertise the retained surface; the stateless entry keeps
/// refusing retained commands; unknown handles and tasks are located.
#[test]
fn capabilities_and_stateless_refusal_stay_explicit() {
    let capabilities = execute(CollectionCommand::Capabilities).unwrap();
    assert_eq!(
        capabilities["outputLayouts"],
        json!(["one_program", "sequential_files"])
    );
    assert_eq!(capabilities["features"]["retainedExecution"], json!(true));
    assert_eq!(capabilities["features"]["retainedPaging"], json!(true));
    assert_eq!(capabilities["features"]["preparedBundles"], json!(true));
    assert_eq!(capabilities["features"]["sequentialFiles"], json!(true));
    assert_eq!(capabilities["features"]["exactByteRetry"], json!(true));
    assert_eq!(capabilities["limits"]["programBytes"], json!(8_000_000));
    assert_eq!(capabilities["limits"]["retainedPlans"], json!(8));
    assert_eq!(capabilities["limits"]["retainedBundles"], json!(8));
    assert_eq!(capabilities["limits"]["retainedTasks"], json!(64));
    let refused = execute(CollectionCommand::TaskStatus {
        task_id: "task-1".into(),
    })
    .unwrap_err();
    assert_eq!(refused.code, "RETAINED_STATE_REQUIRED");
    let mut runtime = Retained::new();
    let unknown = runtime
        .execute(CollectionCommand::ReadMotions {
            plan_handle: "plan-99".into(),
            offset: 0,
        })
        .unwrap_err();
    assert_eq!(unknown.code, "RETAINED_HANDLE_UNKNOWN");
    let missing_task = runtime
        .execute(CollectionCommand::TaskStatus {
            task_id: "task-9".into(),
        })
        .unwrap_err();
    assert_eq!(missing_task.code, "RETAINED_TASK_NOT_FOUND");
    let absent = runtime
        .execute(CollectionCommand::PreparedOutput {
            task_id: "task-1".into(),
        })
        .unwrap_err();
    assert_eq!(absent.code, "RETAINED_TASK_NOT_FOUND");
}
