//! ui-8 sequence DTOs: open/migration, operation-list edits, prefix scoping,
//! export and capabilities, shared byte-for-byte by every adapter.
use cam_core::project::CamJob;
use cam_service::sequence::{
    OperationEdit, PlanScope, SEQUENCE_API_VERSION, SequenceCommand, execute, fingerprint,
    validate_identity,
};
use serde_json::{Value, json};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");
const LEGACY_PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");

fn sequence_profile() -> Value {
    json!({
        "schema_version": 2,
        "id": "printnc",
        "work_offset": "G54",
        "clearance_z_mm": 5,
        "decimal_places": 3,
        "program_start_position_mm": null,
        "length_compensation": "macro_managed",
        "path_control": {"kind": "exact_path"},
        "tools": [
            {"tool_id": "endmill", "tool_number": 1, "length_offset_number": null},
            {"tool_id": "vbit", "tool_number": 2, "length_offset_number": null}
        ],
        "spindle_spinup_seconds": 0.5,
        "coolant": "off",
        "m6": serde_json::from_str::<Value>(LEGACY_PROFILE).unwrap()["m6"].clone()
    })
}

fn opened() -> Value {
    execute(SequenceCommand::Open {
        json: M3_RECTANGLE.into(),
    })
    .unwrap()
}

fn job_of(document: &Value) -> Value {
    document["job"].clone()
}

#[test]
fn open_migrates_legacy_jobs_and_reports_missing_fields_per_operation() {
    let document = opened();
    assert_eq!(document["migrated"], json!(true));
    assert_eq!(document["job"]["schema_version"], json!(4));
    let operations = document["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["kind"], json!("flat_vcarve"));
    assert_eq!(operations[0]["toolIds"], json!(["endmill", "vbit"]));
    // The complete m3 fixture has no missing machining values; clearing one
    // must surface it, located by operation and field path.
    assert_eq!(
        document["missingByOperation"]["flat-v-carve"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let mut cleared = job_of(&document);
    cleared["operations"][0]["settings"]["settings"]["max_depth_mm"] = Value::Null;
    let reopened = execute(SequenceCommand::Open {
        json: serde_json::to_string(&cleared).unwrap(),
    })
    .unwrap();
    let missing = reopened["missingByOperation"]["flat-v-carve"]
        .as_array()
        .unwrap();
    assert!(
        missing
            .iter()
            .any(|entry| entry["fieldPath"] == json!("operations[flat-v-carve].max_depth_mm"))
    );
    // Re-opening the schema-4 document is not a migration.
    let canonical = execute(SequenceCommand::Open {
        json: serde_json::to_string(&job_of(&document)).unwrap(),
    })
    .unwrap();
    assert_eq!(canonical["migrated"], json!(false));
    assert_eq!(
        canonical["documentFingerprint"],
        document["documentFingerprint"]
    );
    // The canonical document is a valid CamJob.
    CamJob::from_json(&serde_json::to_string(&job_of(&canonical)).unwrap()).unwrap();
}

#[test]
fn operation_list_edits_keep_documents_valid_and_ids_stable() {
    let document = opened();
    let job = job_of(&document);
    let edited = execute(SequenceCommand::Edit {
        job: job.clone(),
        edits: vec![
            OperationEdit::Duplicate {
                id: "flat-v-carve".into(),
                new_id: "carve-2".into(),
            },
            OperationEdit::Rename {
                id: "carve-2".into(),
                name: "Second carve".into(),
            },
            OperationEdit::Move {
                id: "carve-2".into(),
                to_index: 0,
            },
            OperationEdit::SetEnabled {
                id: "flat-v-carve".into(),
                enabled: false,
            },
        ],
    })
    .unwrap();
    let operations = edited["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0]["id"], json!("carve-2"));
    assert_eq!(operations[0]["enabled"], json!(true));
    assert_eq!(operations[1]["id"], json!("flat-v-carve"));
    assert_eq!(operations[1]["enabled"], json!(false));
    // Duplicating onto an existing ID is rejected, not silently merged.
    let error = execute(SequenceCommand::Edit {
        job: edited["job"].clone(),
        edits: vec![OperationEdit::Duplicate {
            id: "carve-2".into(),
            new_id: "flat-v-carve".into(),
        }],
    })
    .unwrap_err();
    assert_eq!(error.code, "PROJECT_OPERATION_ID");
    let error = execute(SequenceCommand::Edit {
        job: edited["job"].clone(),
        edits: vec![OperationEdit::Delete { id: "ghost".into() }],
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_OPERATION_ID");
}

#[test]
fn plan_reports_every_operation_and_rejects_prefix_scope_for_unknown_ids() {
    let document = opened();
    let result = execute(SequenceCommand::Plan {
        job: job_of(&document),
        scope: PlanScope::AllEnabled,
    })
    .unwrap();
    let summary = &result["summary"];
    assert!(summary["motionCount"].as_u64().unwrap() > 0);
    assert_eq!(summary["basicChecks"]["status"], json!("passed"));
    let operations = summary["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["operationId"], json!("flat-v-carve"));
    assert_eq!(operations[0]["generationStatus"], json!("complete"));
    let motions = &result["motions"];
    assert_eq!(motions["offset"], json!(0));
    assert_eq!(motions["total"], summary["motionCount"]);

    let error = execute(SequenceCommand::Plan {
        job: job_of(&document),
        scope: PlanScope::ThroughOperation {
            operation_id: "ghost".into(),
        },
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_OPERATION_ID");
}

#[test]
fn prefix_scope_plans_only_the_enabled_prefix() {
    let document = opened();
    let two = execute(SequenceCommand::Edit {
        job: job_of(&document),
        edits: vec![OperationEdit::Duplicate {
            id: "flat-v-carve".into(),
            new_id: "carve-2".into(),
        }],
    })
    .unwrap();
    let full = execute(SequenceCommand::Plan {
        job: two["job"].clone(),
        scope: PlanScope::AllEnabled,
    })
    .unwrap();
    let prefix = execute(SequenceCommand::Plan {
        job: two["job"].clone(),
        scope: PlanScope::ThroughOperation {
            operation_id: "flat-v-carve".into(),
        },
    })
    .unwrap();
    let full_ops = full["summary"]["operations"].as_array().unwrap().len();
    let prefix_ops = prefix["summary"]["operations"].as_array().unwrap().len();
    assert_eq!(full_ops, 2);
    assert_eq!(prefix_ops, 1);
    assert!(
        prefix["summary"]["motionCount"].as_u64().unwrap()
            < full["summary"]["motionCount"].as_u64().unwrap()
    );
}

#[test]
fn export_requires_resolved_process_state_until_a_profile_is_applied() {
    let document = opened();
    let error = execute(SequenceCommand::Export {
        job: job_of(&document),
        profile: sequence_profile(),
    })
    .unwrap_err();
    assert_eq!(error.code, "PROCESS_SPINDLE_STATE");

    // Apply the legacy machine profile, then export end to end.
    let applied = execute(SequenceCommand::ApplyProfile {
        job: job_of(&document),
        profile: serde_json::from_str(LEGACY_PROFILE).unwrap(),
    })
    .unwrap();
    let export = execute(SequenceCommand::Export {
        job: applied["job"].clone(),
        profile: sequence_profile(),
    })
    .unwrap();
    assert_eq!(export["program"]["filename"], json!("sequence.ngc"));
    assert!(
        export["program"]["gcode"]
            .as_str()
            .unwrap()
            .contains("M3 S10000")
    );
    let gcode = export["program"]["gcode"].as_str().unwrap();
    let emitted_blocks = gcode.matches("G0 ").count() + gcode.matches("G1 ").count();
    assert_eq!(
        emitted_blocks,
        export["report"]["motionCount"].as_u64().unwrap() as usize
    );
    assert_eq!(export["report"]["basicChecks"]["status"], json!("passed"));
}

#[test]
fn update_settings_replaces_operation_settings_through_strict_parsing() {
    let document = opened();
    let job = job_of(&document);
    let face_settings = |margin: f64| {
        json!({
            "kind": "face",
            "settings": {
                "area": {"kind": "rectangle", "rect": {
                    "min_x_mm": 0, "min_y_mm": 0, "width_mm": 40, "length_mm": 30}},
                "margins": {"min_x_mm": margin},
                "entry_overrun_mm": 2, "exit_overrun_mm": 1,
                "top": {"reference": {"kind": "stock_top"}, "offset_mm": 0},
                "bottom": {"reference": {"kind": "stock_top"}, "offset_mm": -0.5},
                "stepdown_mm": 0.5, "stepover_mm": 3, "pass_angle_deg": 0,
                "pattern": "zig_zag",
                "assignment": {
                    "tool_id": "endmill", "spindle_rpm": 10000,
                    "spindle_direction": "clockwise",
                    "cutting_feed_mm_min": 300, "plunge_feed_mm_min": 100,
                    "max_stepdown_mm": 1
                }
            }
        })
    };
    // The whole settings object is replaced (never merged); the document
    // projection reports the face operation with its updated values.
    let updated = execute(SequenceCommand::UpdateSettings {
        job: job.clone(),
        operation_id: "flat-v-carve".into(),
        settings: face_settings(1.),
    })
    .unwrap();
    assert_eq!(updated["operations"][0]["kind"], json!("face"));
    assert_eq!(updated["operations"][0]["toolIds"], json!(["endmill"]));
    let updated_job: CamJob = serde_json::from_value(job_of(&updated)).unwrap();
    assert!(matches!(
        updated_job.operations[0].settings,
        cam_core::project::OperationSettings::Face(_)
    ));

    // Invalid supplied values are rejected immediately; unknown fields never
    // parse. Both leave the document unchanged (the caller still holds `job`).
    let error = execute(SequenceCommand::UpdateSettings {
        job: job.clone(),
        operation_id: "flat-v-carve".into(),
        settings: face_settings(-1.),
    })
    .unwrap_err();
    assert_eq!(error.code, "PROJECT_PARAMETER");
    let mut unknown_field = face_settings(1.);
    unknown_field["settings"]["surprise"] = json!(true);
    let error = execute(SequenceCommand::UpdateSettings {
        job: job.clone(),
        operation_id: "flat-v-carve".into(),
        settings: unknown_field,
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_SETTINGS_JSON");
    let error = execute(SequenceCommand::UpdateSettings {
        job,
        operation_id: "ghost".into(),
        settings: face_settings(1.),
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_OPERATION_ID");
}

#[test]
fn capabilities_advertise_only_implemented_features() {
    let capabilities = execute(SequenceCommand::Capabilities).unwrap();
    assert_eq!(capabilities["apiVersion"], json!(SEQUENCE_API_VERSION));
    assert_eq!(
        capabilities["operationKinds"],
        json!(["flat_vcarve", "face", "profile"])
    );
    assert_eq!(capabilities["features"]["openContours"], json!(false));
    assert_eq!(capabilities["features"]["knifeReplay"], json!(false));
    assert_eq!(capabilities["features"]["legacyJobMigration"], json!(true));
}

#[test]
fn admission_rejects_foreign_api_versions_and_weak_request_identity() {
    assert!(validate_identity("ui-7", "i", "req-1", 1, "i").is_err());
    assert!(validate_identity(SEQUENCE_API_VERSION, "i", "", 1, "i").is_err());
    assert!(validate_identity(SEQUENCE_API_VERSION, "a", "req-1", 1, "b").is_err());
    assert!(validate_identity(SEQUENCE_API_VERSION, "i", "req-1", 0, "i").is_ok());
}

#[test]
fn fingerprints_track_document_content_not_identity() {
    let document = opened();
    let job: CamJob = serde_json::from_value(job_of(&document)).unwrap();
    let renamed = execute(SequenceCommand::Edit {
        job: job_of(&document),
        edits: vec![OperationEdit::Rename {
            id: "flat-v-carve".into(),
            name: "Renamed".into(),
        }],
    })
    .unwrap();
    let renamed_job: CamJob = serde_json::from_value(renamed["job"].clone()).unwrap();
    assert_ne!(fingerprint(&job), fingerprint(&renamed_job));
    assert_eq!(fingerprint(&job), fingerprint(&job.clone()));
}
