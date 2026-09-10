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
    // The drag-knife planner is not implemented: knife data/import exist but
    // the operation kind stays unadvertised (F1 acceptance).
    assert_eq!(
        capabilities["operationKinds"],
        json!(["flat_vcarve", "face", "profile"])
    );
    assert!(
        !capabilities["operationKinds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|kind| kind == "drag_knife")
    );
    assert_eq!(capabilities["features"]["openContours"], json!(true));
    assert_eq!(capabilities["features"]["knifeToolLibrary"], json!(true));
    assert_eq!(capabilities["features"]["knifeReplay"], json!(false));
    assert_eq!(capabilities["features"]["legacyJobMigration"], json!(true));
    assert_eq!(capabilities["features"]["profileEntries"], json!(true));
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

/// The D3 mixed fixture through the ui-8 projection: face -> carve (rough +
/// finish) -> profile with the recurring endmill, plus the contour catalogue
/// and motion paging the stock/timeline display consumes.
#[test]
fn profile_workflow_commands_plan_page_and_catalogue_the_mixed_fixture() {
    let job = mixed_fixture_job();
    let scope = PlanScope::AllEnabled;
    let plan = execute(SequenceCommand::Plan {
        job: job.clone(),
        scope: scope.clone(),
    })
    .unwrap();
    let summary = &plan["summary"];
    let operations: Vec<&str> = summary["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["operationId"].as_str().unwrap())
        .collect();
    assert_eq!(operations, vec!["face-1", "carve-1", "profile-1"]);
    for op in summary["operations"].as_array().unwrap() {
        assert_eq!(op["generationStatus"], json!("complete"), "{op}");
    }
    let stages = summary["stages"].as_array().unwrap();
    let stage_tools: Vec<&str> = stages
        .iter()
        .map(|s| s["toolId"].as_str().unwrap())
        .collect();
    assert_eq!(stage_tools, vec!["endmill", "endmill", "vbit", "endmill"]);
    let roles: Vec<&str> = stages.iter().map(|s| s["role"].as_str().unwrap()).collect();
    assert_eq!(
        roles,
        vec!["face", "vcarve_rough", "vcarve_finish", "profile_rough"]
    );
    assert_eq!(summary["basicChecks"]["status"], json!("passed"));

    // Motion paging: pages tile the complete ordered stream, offsets beyond
    // the end are located errors, and page contents are motion-identical.
    let total = summary["motionCount"].as_u64().unwrap() as usize;
    let first = execute(SequenceCommand::Motions {
        job: job.clone(),
        scope: scope.clone(),
        offset: 0,
    })
    .unwrap();
    let page_size = first["motions"]["motions"].as_array().unwrap().len();
    assert!(page_size > 0 && page_size <= total);
    let mut collected: Vec<Value> = first["motions"]["motions"].as_array().unwrap().clone();
    let mut offset = page_size;
    while offset < total {
        let page = execute(SequenceCommand::Motions {
            job: job.clone(),
            scope: scope.clone(),
            offset,
        })
        .unwrap();
        assert_eq!(page["motions"]["offset"], json!(offset));
        collected.extend(
            page["motions"]["motions"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        offset += page["motions"]["motions"].as_array().unwrap().len();
    }
    assert_eq!(collected.len(), total, "paging covers every motion");
    assert_eq!(
        collected[0]["id"],
        json!(0),
        "global motion ids stay dense across pages"
    );
    let error = execute(SequenceCommand::Motions {
        job: job.clone(),
        scope: scope.clone(),
        offset: total + 1,
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_MOTION_OFFSET");

    // The contour catalogue projects stable IDs with roles and suggestions.
    let catalogue = execute(SequenceCommand::Contours { job: job.clone() }).unwrap();
    let contours = catalogue["contours"].as_array().unwrap();
    assert_eq!(contours.len(), 1);
    assert_eq!(contours[0]["id"], json!("pocket-0-outer"));
    assert_eq!(contours[0]["role"], json!("outer"));
    assert_eq!(contours[0]["suggestedSide"], json!("outside"));
    assert_eq!(contours[0]["parentContourId"], Value::Null);

    // The ordered export of the mixed fixture passes through the service.
    let export = execute(SequenceCommand::Export {
        job: job.clone(),
        profile: sequence_profile(),
    })
    .unwrap();
    assert_eq!(export["report"]["basicChecks"]["status"], json!("passed"));
    let gcode = export["program"]["gcode"].as_str().unwrap();
    let mut tool_groups = vec![];
    for line in gcode.lines() {
        if let Some(rest) = line.strip_prefix('T')
            && let Some(number) = rest.strip_suffix(" M6")
        {
            tool_groups.push(number.to_string());
        }
    }
    assert_eq!(tool_groups, vec!["1", "1", "2", "1"]);
}

/// The E3 entries slice through the whole service path: the mixed fixture's
/// profile grows a ramp entry, tangent leads, tabs and finishing through the
/// strict UpdateSettings command, plans completely with lead/ramp motions
/// and drawable tab footprints, and exports through the readback.
#[test]
fn profile_entries_plan_and_export_through_the_service() {
    let job = mixed_fixture_job();
    // The catalogue supplies the anchor fingerprint the start binds to.
    let catalogue = execute(SequenceCommand::Contours { job: job.clone() }).unwrap();
    let fingerprint = catalogue["contours"][0]["sourceFingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    let settings = json!({
        "kind": "profile",
        "settings": {
            "contours": [
                {"contour_id": "pocket-0-outer", "side": "outside", "traversal": null}
            ],
            "assignment": {
                "tool_id": "endmill", "spindle_rpm": 10000,
                "spindle_direction": "clockwise",
                "cutting_feed_mm_min": 350, "plunge_feed_mm_min": 100,
                "max_stepdown_mm": 8
            },
            "top": {"reference": {"kind": "face_result", "operation_id": "face-1"}, "offset_mm": 0},
            "bottom": {"reference": {"kind": "stock_bottom"}, "offset_mm": -0.2},
            "stepdown_mm": 3, "through_cut_allowance_mm": 0.2,
            "direction": "climb",
            "start": {
                "kind": "anchor", "contour_id": "pocket-0-outer",
                "source_geometry_fingerprint": fingerprint,
                "fraction_along_source_contour": 0.15
            },
            "entry": {"kind": "ramp", "max_angle_deg": 30, "feed_mm_min": 140},
            "lead_in": {"kind": "tangent_line", "length_mm": 3, "feed_mm_min": 180},
            "lead_out": {"kind": "tangent_line", "length_mm": 2, "feed_mm_min": 200},
            "tabs": {
                "height_mm": 2, "width_mm": 5, "shape": "rectangular",
                "placement": {"kind": "automatic", "count": 2, "spacing_mm": null}
            },
            "finish": {"enabled": true, "radial_allowance_mm": 0.5, "feed_mm_min": 300}
        }
    });
    let updated = execute(SequenceCommand::UpdateSettings {
        job: job.clone(),
        operation_id: "profile-1".into(),
        settings,
    })
    .unwrap();
    let job = job_of(&updated);

    let plan = execute(SequenceCommand::Plan {
        job: job.clone(),
        scope: PlanScope::AllEnabled,
    })
    .unwrap();
    let summary = &plan["summary"];
    for op in summary["operations"].as_array().unwrap() {
        assert_eq!(op["generationStatus"], json!("complete"), "{op}");
    }
    assert_eq!(summary["basicChecks"]["status"], json!("passed"));
    let roles: Vec<&str> = summary["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles.last(), Some(&"profile_finish"));

    // Paging surfaces the E3 motion purposes: sloped ramp entries, lead-in
    // and lead-out moves, and tab transitions protecting the bridges.
    let total = summary["motionCount"].as_u64().unwrap() as usize;
    let mut purposes = std::collections::BTreeSet::new();
    let mut offset = 0;
    while offset < total {
        let page = execute(SequenceCommand::Motions {
            job: job.clone(),
            scope: PlanScope::AllEnabled,
            offset,
        })
        .unwrap();
        let motions = page["motions"]["motions"].as_array().unwrap();
        for motion in motions {
            purposes.insert(motion["purpose"].as_str().unwrap().to_string());
        }
        offset += motions.len();
    }
    for expected in [
        "entry",
        "lead_in",
        "lead_out",
        "tab_transition",
        "rough",
        "finish",
    ] {
        assert!(
            purposes.contains(expected),
            "missing {expected}: {purposes:?}"
        );
    }

    // Resolved tab bridges carry exact drawable footprints for overlays.
    let profile_output = summary["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operationId"] == json!("profile-1"))
        .unwrap();
    let placements = profile_output["namedOutputs"][0]["tabPlacements"]
        .as_array()
        .unwrap();
    assert_eq!(placements.len(), 2);
    for placement in placements {
        let footprint = placement["footprintMm"].as_array().unwrap();
        assert_eq!(footprint.len(), 4, "a drawable bridge quad");
        assert_eq!(placement["topZMm"], json!(-6.0));
    }

    let export = execute(SequenceCommand::Export {
        job,
        profile: sequence_profile(),
    })
    .unwrap();
    assert_eq!(export["report"]["basicChecks"]["status"], json!("passed"));
    let gcode = export["program"]["gcode"].as_str().unwrap();
    for feed in ["F140", "F180", "F200", "F300"] {
        assert!(
            gcode.contains(feed),
            "feed {feed} survives into the program"
        );
    }
}

/// Added face/profile operations bind an explicit tool and report every
/// unset machining value through missing-field resolution.
#[test]
fn add_operation_edits_create_incomplete_face_and_profile_operations() {
    let document = opened();
    let job = job_of(&document);
    let added = execute(SequenceCommand::Edit {
        job: job.clone(),
        edits: vec![OperationEdit::Add {
            id: "profile-9".into(),
            name: "Cutout".into(),
            kind: cam_service::sequence::AddOperationKind::Profile,
            tool_id: "endmill".into(),
        }],
    })
    .unwrap();
    assert_eq!(added["operations"].as_array().unwrap().len(), 2);
    assert_eq!(added["operations"][1]["kind"], json!("profile"));
    assert_eq!(added["operations"][1]["toolIds"], json!(["endmill"]));
    let missing = added["missingByOperation"]["profile-9"].as_array().unwrap();
    assert!(missing.len() >= 5, "unset machining values are reported");
    assert!(
        missing
            .iter()
            .any(|entry| entry["fieldPath"].as_str().unwrap().contains("stepdown_mm"))
    );
    // Colliding IDs and unknown tools are located errors.
    let error = execute(SequenceCommand::Edit {
        job: job.clone(),
        edits: vec![OperationEdit::Add {
            id: "flat-v-carve".into(),
            name: "Collision".into(),
            kind: cam_service::sequence::AddOperationKind::Face,
            tool_id: "endmill".into(),
        }],
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_OPERATION_ID");
    let error = execute(SequenceCommand::Edit {
        job,
        edits: vec![OperationEdit::Add {
            id: "face-9".into(),
            name: "Face".into(),
            kind: cam_service::sequence::AddOperationKind::Face,
            tool_id: "ghost".into(),
        }],
    })
    .unwrap_err();
    assert_eq!(error.code, "PROJECT_TOOL_REFERENCE");
}

/// Face(T1) -> combined carve (T1 rough, T2 finish) -> profile (T1) on
/// 40x30x8 stock with one rectangular pocket component.
fn mixed_fixture_job() -> Value {
    use cam_core::job::{PlanningTolerances, SourceSnapshot};
    use cam_core::project::*;
    let milling = |tool: &str, feed: f64, stepover: f64| MillingAssignment {
        tool_id: tool.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(feed),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(8.),
        stepover_mm: Some(stepover),
    };
    let stock_top = || HeightRef {
        reference: HeightReference::StockTop,
        offset_mm: 0.,
    };
    let face = Operation {
        id: "face-1".into(),
        name: "Face".into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area: FaceArea::Rectangle {
                rect: RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                },
            },
            margins: FaceMargins::default(),
            entry_overrun_mm: Some(1.),
            exit_overrun_mm: Some(1.),
            top: stock_top(),
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -0.5,
            },
            stepdown_mm: Some(0.5),
            stepover_mm: Some(3.),
            pass_angle_deg: Some(0.),
            pattern: FacePattern::ZigZag,
            assignment: milling("endmill", 300., 3.),
        }),
    };
    let carve = Operation {
        id: "carve-1".into(),
        name: "Carve".into(),
        enabled: true,
        settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
            component_ids: vec!["pocket::0".into()],
            mode: FlatVcarveMode::Combined,
            top: HeightRef {
                reference: HeightReference::FaceResult {
                    operation_id: "face-1".into(),
                },
                offset_mm: 0.,
            },
            endmill: milling("endmill", 300., 1.5),
            vbit: milling("vbit", 250., 0.5),
            max_depth_mm: Some(2.),
            wall_allowance_mm: Some(0.5),
            max_floor_ridge_mm: Some(0.),
            max_detail_residual_mm: Some(0.),
            rough: Some(FlatVcarveRoughSettings {
                strategy: cam_core::pocket::ClearingStrategy::DepthDependent,
                entry: cam_core::pocket::EntryStrategy::Plunge,
                max_layers: 16,
                max_loops_per_layer: 64,
                max_motions: 10_000,
            }),
            finish: Some(cam_core::vcarve::VBitPlanningSettings {
                max_paths: 4096,
                max_motions: 100_000,
                max_curve_segments: 20_000,
                max_depth_passes: 8,
                max_cleanup_iterations: 2,
                quality_sample_spacing_mm: 0.5,
                max_quality_samples: 20_000,
                reachability_max_cells: 4096,
                stock_slices: 4,
            }),
        }),
    };
    let profile = Operation {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours: vec![ProfileContour {
                contour_id: "pocket-0-outer".into(),
                side: ContourSide::Outside,
                traversal: None,
            }],
            assignment: milling("endmill", 350., 1.5),
            top: HeightRef {
                reference: HeightReference::FaceResult {
                    operation_id: "face-1".into(),
                },
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockBottom,
                offset_mm: -0.2,
            },
            stepdown_mm: Some(3.),
            through_cut_allowance_mm: Some(0.2),
            direction: Some(CutDirection::Climb),
            order: Default::default(),
            start: Default::default(),
            finish: Default::default(),
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    };
    let job = CamJob {
        schema_version: 4,
        name: "Mixed".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#.into(),
        }),
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(cam_core::geometry::Point::new(0., 0.)),
        },
        tools: vec![
            JobTool {
                id: "endmill".into(),
                name: "4mm endmill".into(),
                geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                    diameter_mm: 4.,
                    cutting_length_mm: 12.,
                })),
                capabilities: ToolCapabilities {
                    plunge_capable: Some(true),
                    ramp_capable: Some(true),
                },
            },
            JobTool {
                id: "vbit".into(),
                name: "90 degree V-bit".into(),
                geometry: Some(ToolGeometry::Vbit(cam_core::model::VBitSpec {
                    included_angle_deg: 90.,
                    tip_diameter_mm: 1.,
                    max_cutting_diameter_mm: 12.,
                    cutting_height_mm: 5.,
                })),
                capabilities: ToolCapabilities {
                    plunge_capable: Some(true),
                    ramp_capable: None,
                },
            },
        ],
        operations: vec![face, carve, profile],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    };
    serde_json::to_value(job).unwrap()
}

#[test]
fn open_chains_project_and_the_knife_planner_stays_unadvertised() {
    use cam_core::job::{PlanningTolerances, SourceSnapshot};
    use cam_core::project::*;
    use cam_core::svg::{ImportMode, ImportOptions};

    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 5 L25 5"/></svg>"##;
    let job = CamJob {
        schema_version: 4,
        name: "knife-ui8".into(),
        source: Some(SourceSnapshot {
            filename: "cut.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            mode: ImportMode::Centerline,
            ..Default::default()
        },
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(3.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![JobTool {
            id: "blade".into(),
            name: "drag knife".into(),
            geometry: Some(ToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.,
                max_cut_depth_mm: 2.,
            })),
            capabilities: Default::default(),
        }],
        operations: vec![Operation {
            id: "knife-1".into(),
            name: "knife-1".into(),
            enabled: true,
            settings: OperationSettings::DragKnife(DragKnifeSettings {
                chains: vec!["cut-chain-0".into()],
                assignment: KnifeAssignment {
                    tool_id: "blade".into(),
                    cutting_feed_mm_min: Some(150.),
                    plunge_feed_mm_min: Some(60.),
                    swivel_feed_mm_min: Some(50.),
                    max_stepdown_mm: Some(1.),
                },
                top: HeightRef {
                    reference: Default::default(),
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::OperationTop,
                    offset_mm: -1.,
                },
                stepdown_mm: Some(1.),
                swivel_depth_mm: Some(0.2),
                corner_threshold_deg: Some(30.),
                through_cut_allowance_mm: None,
                start: Default::default(),
                closure_overlap_mm: None,
                alignment: KnifeAlignment::default(),
            }),
        }],
        tolerances: PlanningTolerances::default(),
        legacy_machine_profile: None,
    };
    let document = execute(SequenceCommand::Open {
        json: serde_json::to_string(&job).unwrap(),
    })
    .unwrap();
    assert_eq!(document["operations"][0]["kind"], json!("drag_knife"));

    // The catalogue command projects the open chains beside the contours.
    let catalogue = execute(SequenceCommand::Contours {
        job: job_of(&document),
    })
    .unwrap();
    assert_eq!(catalogue["contours"], json!([]));
    let chains = catalogue["openChains"].as_array().unwrap();
    assert_eq!(chains.len(), 1, "one stroke is one centerline, not two");
    assert_eq!(chains[0]["id"], json!("cut-chain-0"));
    assert_eq!(chains[0]["role"], json!("open"));
    assert_eq!(chains[0]["closed"], json!(false));
    assert_eq!(chains[0]["suggestedSide"], json!("on"));
    assert_eq!(chains[0]["bounds"]["maxXmm"], json!(25.));

    // No unsupported knife planner is advertised: planning the enabled knife
    // operation is a located rejection, never a silent partial plan.
    let error = execute(SequenceCommand::Plan {
        job: job_of(&document),
        scope: PlanScope::AllEnabled,
    })
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_PLANNER_UNAVAILABLE");
}
