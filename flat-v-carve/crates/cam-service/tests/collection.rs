//! ui-9 collection DTOs (H4): open/migration, resource commands, the applied
//! machine configuration and scope-aware planning through the shared
//! projection every adapter serves. Scenario 7 and 8 flows run end-to-end
//! through the transport, and readouts resolve to the same values the
//! resolver exports.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::v5::{self, ArtworkContent, ArtworkItemId, CamJobV5, GeometryRef, GeometryRefKind},
    svg::{ImportMode, Placement},
};
use cam_service::collection::{
    COLLECTION_API_VERSION, CollectionCommand, CollectionScope, execute, fingerprint,
    validate_identity,
};
use serde_json::{Value, json};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");
/// One filled plate plus one stroked centerline (the H3 fixture geometry).
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

fn profile_operation() -> v5::OperationV5 {
    let component = catalogue_reference(GeometryRefKind::ClosedContour);
    v5::OperationV5 {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: v5::OperationSettingsV5::Profile(v5::ProfileSettingsV5 {
            contours: vec![v5::ProfileContourV5 {
                geometry: component,
                side: cam_core::project::ContourSide::Outside,
                traversal: None,
            }],
            assignment: v5::MillingAssignmentV5 {
                tool_id: "t1".into(),
                spindle_rpm: Some(10_000.),
                spindle_direction: Some(cam_core::project::SpindleDirection::Clockwise),
                cutting_feed_mm_min: Some(300.),
                plunge_feed_mm_min: Some(100.),
                max_stepdown_mm: Some(1.),
                stepover_mm: Some(1.5),
                applied_profile: None,
            },
            top: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::OperationTop,
                offset_mm: -2.,
            },
            stepdown_mm: Some(2.),
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

fn job_value(operations: Vec<v5::OperationV5>) -> Value {
    serde_json::to_value(base_job(operations)).unwrap()
}

fn library() -> Value {
    json!({
        "schema_version": 1,
        "revision": 4,
        "tools": [{
            "id": "lib-e",
            "name": "Library endmill",
            "geometry": {"kind": "endmill", "dimensions": {"diameter_mm": 3.0, "cutting_length_mm": 8.0, "plunge_capable": true}},
            "ramp_capable": true,
            "plunge_capable": true,
            "cutting_presets": [{
                "id": "soft",
                "name": "Soft wood",
                "material": "pine",
                "machine": null,
                "spindle_rpm": 12000.0,
                "cutting_feed_mm_min": 400.0,
                "plunge_feed_mm_min": 120.0,
                "max_stepdown_mm": 0.8,
                "stepover_mm": 2.0
            }],
            "knife_cutting_presets": []
        }]
    })
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

#[test]
fn open_migrates_older_jobs_and_refuses_future_schemas() {
    let document = execute(CollectionCommand::Open {
        json: M3_RECTANGLE.into(),
    })
    .unwrap();
    assert_eq!(document["migrated"], json!(true));
    assert_eq!(document["job"]["schema_version"], json!(5));
    // The migration moved the single source into the artwork collection and
    // the inspection DTOs describe the migrated document.
    assert_eq!(
        document["artwork"].as_array().unwrap().len(),
        1,
        "{document}"
    );
    assert_eq!(
        document["inspection"]["machiningOrder"][0]["kind"],
        json!("flat_vcarve")
    );
    assert_eq!(
        document["inspection"]["assignments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Reopening the schema-5 result is not a migration.
    let reopened = execute(CollectionCommand::Open {
        json: document["job"].to_string(),
    })
    .unwrap();
    assert_eq!(reopened["migrated"], json!(false));
    assert_eq!(
        reopened["documentFingerprint"],
        document["documentFingerprint"]
    );
    // Future schemas are refused, never flattened.
    let mut future = document["job"].clone();
    future["schema_version"] = json!(6);
    let error = execute(CollectionCommand::Open {
        json: future.to_string(),
    })
    .unwrap_err();
    assert_eq!(error.code, "COLLECTION_SCHEMA_UNSUPPORTED");
}

/// Scenario 7 through the transport: applying, editing and resetting a
/// cutting profile needs the library only as an explicit Apply input.
#[test]
fn cutting_profile_commands_round_trip_through_the_transport() {
    let job = job_value(vec![profile_operation(), knife_operation()]);
    let applied = execute(CollectionCommand::ApplyCuttingProfile {
        job: job.clone(),
        library: library(),
        library_id: "lib-1".into(),
        operation_id: "profile-1".into(),
        role: v5::resources::AssignmentRole::Milling,
        library_tool_id: "lib-e".into(),
        preset_id: "soft".into(),
    })
    .unwrap();
    let applied_job = applied["job"].clone();
    assert_eq!(
        applied_job["operations"][0]["settings"]["settings"]["assignment"]["spindle_rpm"],
        json!(12000.0)
    );
    let status_of = |assignments: &Value| {
        assignments
            .as_array()
            .unwrap()
            .iter()
            .find(|status| status["operationId"] == json!("profile-1"))
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status_of(&applied["assignments"]), json!("applied"));
    // Edit one copied value: the assignment becomes modified.
    let mut edited = applied_job.clone();
    edited["operations"][0]["settings"]["settings"]["assignment"]["cutting_feed_mm_min"] =
        json!(999.0);
    let inspected = execute(CollectionCommand::Inspect {
        job: edited.clone(),
    })
    .unwrap();
    assert_eq!(
        status_of(&inspected["inspection"]["assignments"]),
        json!("modified")
    );
    // Reset without any library document restores the copied baseline.
    let reset = execute(CollectionCommand::ResetAssignment {
        job: edited.clone(),
        operation_id: "profile-1".into(),
        role: v5::resources::AssignmentRole::Milling,
    })
    .unwrap();
    assert_eq!(status_of(&reset["assignments"]), json!("applied"));
    assert_eq!(
        reset["job"]["operations"][0]["settings"]["settings"]["assignment"]["cutting_feed_mm_min"],
        json!(400.0)
    );
    // The updated document is a stable receipt of itself.
    let parsed: CamJobV5 = serde_json::from_value(reset["job"].clone()).unwrap();
    assert_eq!(fingerprint(&parsed).len(), 64);
}

/// Scenario 8 through the transport: apply the configuration, map exactly
/// the scoped tools, and every readout/export projection uses the same
/// values.
#[test]
fn machine_configuration_readouts_and_resolver_agree() {
    let job = job_value(vec![profile_operation(), knife_operation()]);
    let applied = execute(CollectionCommand::ApplyMachineConfiguration {
        job,
        profile: machine_profile(
            json!([{"tool_id": "t1", "tool_number": 3, "length_offset_number": null}]),
        ),
        configuration_name: "Workbench".into(),
    })
    .unwrap();
    let applied_job = applied["job"].clone();
    // The prefix scope (profile-1) resolves with only t1 mapped.
    let resolved = execute(CollectionCommand::ResolveProfile {
        job: applied_job.clone(),
        scope: CollectionScope::ThroughOperation {
            operation_id: "profile-1".into(),
        },
    })
    .unwrap();
    assert_eq!(
        resolved["profile"]["tools"],
        json!([{"tool_id": "t1", "tool_number": 3}])
    );
    // The machine readout in the Inspect projection carries the same values.
    let inspected = execute(CollectionCommand::Inspect {
        job: applied_job.clone(),
    })
    .unwrap();
    assert_eq!(
        inspected["inspection"]["machine"]["rows"],
        json!([{
            "jobToolId": "t1", "toolNumber": 3, "lengthOffsetNumber": null,
            "toolExists": true,
            "assignments": [{"operationId": "profile-1", "role": "milling"}],
        }])
    );
    // Map the knife tool for the full scope.
    let mapped = execute(CollectionCommand::SetToolMapping {
        job: applied_job,
        job_tool_id: "t3".into(),
        tool_number: Some(5),
        length_offset_number: None,
    })
    .unwrap();
    let full = execute(CollectionCommand::ResolveProfile {
        job: mapped["job"].clone(),
        scope: CollectionScope::AllEnabled,
    })
    .unwrap();
    assert_eq!(full["profile"]["tools"].as_array().unwrap().len(), 2);
    // An unmapped active tool is a diagnostic, not a silent default.
    let mut missing = mapped["job"].clone();
    missing["machine_configuration"]["tools"] = json!([{
        "job_tool_id": "t1", "tool_number": 3, "length_offset_number": null
    }]);
    let error = execute(CollectionCommand::ResolveProfile {
        job: missing,
        scope: CollectionScope::AllEnabled,
    })
    .unwrap_err();
    assert_eq!(error.code, "MACHINE_MAPPING_MISSING");
    assert!(error.message.contains("t3"), "{error:?}");
}

/// Planning through the transport returns the summary, the plan inspection
/// DTO and a bounded motion page; the knife-evidence command exports through
/// the applied machine configuration with no separate profile parameter.
#[test]
fn plan_and_knife_evidence_flow_through_the_transport() {
    let knife_only = {
        let mut job = base_job(vec![knife_operation()]);
        job.tools.retain(|tool| tool.id == "t3");
        serde_json::to_value(job).unwrap()
    };
    let planned = execute(CollectionCommand::Plan {
        job: knife_only.clone(),
        scope: CollectionScope::AllEnabled,
    })
    .unwrap();
    assert!(planned["summary"]["motionCount"].as_u64().unwrap() > 0);
    assert_eq!(planned["inspection"]["stock"]["thicknessMm"], json!(8.0));
    assert_eq!(
        planned["motions"]["total"],
        planned["summary"]["motionCount"]
    );
    // Evidence needs the applied machine configuration first.
    let error = execute(CollectionCommand::KnifeEvidence {
        job: knife_only.clone(),
        scope: CollectionScope::AllEnabled,
        sample_limit: None,
    })
    .unwrap_err();
    assert_eq!(error.code, "MACHINE_CONFIGURATION_ABSENT");
    let applied = execute(CollectionCommand::ApplyMachineConfiguration {
        job: knife_only,
        profile: machine_profile(
            json!([{"tool_id": "t3", "tool_number": 7, "length_offset_number": null}]),
        ),
        configuration_name: "Kiosk".into(),
    })
    .unwrap();
    let evidence = execute(CollectionCommand::KnifeEvidence {
        job: applied["job"].clone(),
        scope: CollectionScope::AllEnabled,
        sample_limit: Some(256),
    })
    .unwrap();
    assert_eq!(evidence["report"]["status"], json!("within"));
    assert_eq!(evidence["stock"]["thicknessMm"], json!(8.0));
    assert_eq!(evidence["stock"]["hasKnifeStages"], json!(true));
}

#[test]
fn capabilities_advertise_the_collection_surface() {
    let capabilities = execute(CollectionCommand::Capabilities).unwrap();
    assert_eq!(capabilities["apiVersion"], json!(COLLECTION_API_VERSION));
    assert_eq!(capabilities["features"]["cuttingProfiles"], json!(true));
    assert_eq!(
        capabilities["features"]["appliedMachineConfiguration"],
        json!(true)
    );
    assert_eq!(capabilities["limits"]["libraryBytes"], json!(8_000_000));
}

#[test]
fn admission_mirrors_the_sequence_route_rules() {
    assert!(validate_identity(COLLECTION_API_VERSION, "inst", "req-1", 1, "inst").is_ok());
    let wrong_version = validate_identity("ui-8", "inst", "req-1", 1, "inst").unwrap_err();
    assert_eq!(wrong_version.1, "TASK_INSTANCE");
    let wrong_instance =
        validate_identity(COLLECTION_API_VERSION, "other", "req-1", 1, "inst").unwrap_err();
    assert_eq!(wrong_instance.1, "TASK_INSTANCE");
    let weak_id = validate_identity(COLLECTION_API_VERSION, "inst", "", 1, "inst").unwrap_err();
    assert_eq!(weak_id.1, "REQUEST_IDENTITY");
    let huge_revision = validate_identity(
        COLLECTION_API_VERSION,
        "inst",
        "req-1",
        9_007_199_254_740_992,
        "inst",
    )
    .unwrap_err();
    assert_eq!(huge_revision.1, "REQUEST_IDENTITY");
    // The envelope wraps data and diagnostics identically for every adapter.
    let data = cam_service::collection::envelope("req-1", 2, Ok(json!({"x": 1})));
    assert_eq!(data["data"]["x"], json!(1));
    let diagnostic = cam_service::collection::envelope(
        "req-1",
        2,
        Err(cam_core::geometry::Diagnostic::new("X", "boom")),
    );
    assert_eq!(diagnostic["diagnostic"]["code"], json!("X"));
}
