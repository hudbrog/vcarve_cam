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
fn schema5_collection_documents_are_refused_not_flattened() {
    // A minimal structurally valid schema-5 document: the ui-8 client must
    // refuse it explicitly instead of flattening it to schema 4 or routing
    // it through the legacy migration path.
    let collection = json!({
        "schema_version": 5,
        "name": "collection",
        "artwork": [],
        "tools": [],
        "operations": []
    });
    let error = execute(SequenceCommand::Open {
        json: collection.to_string(),
    })
    .unwrap_err();
    assert_eq!(error.code, "SEQUENCE_SCHEMA_UNSUPPORTED");
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
    // The knife geometry planner ships with F2: the operation kind and its
    // independent replay gate are advertised alongside the milling kinds.
    assert_eq!(
        capabilities["operationKinds"],
        json!(["flat_vcarve", "face", "profile", "drag_knife"])
    );
    assert_eq!(capabilities["features"]["openContours"], json!(true));
    assert_eq!(capabilities["features"]["knifeToolLibrary"], json!(true));
    assert_eq!(capabilities["features"]["knifeReplay"], json!(true));
    // Unimplemented features stay unadvertised.
    assert_eq!(capabilities["features"]["rampedTabs"], json!(false));
    assert_eq!(capabilities["features"]["rotatedFacing"], json!(false));
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
fn open_chains_project_and_knife_planning_round_trips() {
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

    // The knife planner is advertised with its replay gate (F2), and the
    // unconfigured initial heading is a located missing field — never a
    // silently defaulted one.
    let capabilities = execute(SequenceCommand::Capabilities).unwrap();
    assert!(
        capabilities["operationKinds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|kind| kind == "drag_knife")
    );
    assert_eq!(capabilities["features"]["knifeReplay"], json!(true));
    assert!(
        document["missingByOperation"]["knife-1"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["fieldPath"]
                .as_str()
                .unwrap()
                .contains("alignment.initial_heading_deg"))
    );
    let summary = execute(SequenceCommand::Plan {
        job: job_of(&document),
        scope: PlanScope::AllEnabled,
    })
    .unwrap();
    assert_eq!(
        summary["summary"]["operations"][0]["generationStatus"],
        json!("incomplete")
    );

    // Supplying the heading through the strict UpdateSettings command (and
    // the motion tolerance on the job) plans the knife operation completely;
    // motion pages carry the pivot/tip display contract (bladeHeadingDeg).
    let mut configured = document["job"].clone();
    configured["tolerances"]["motion_tolerance_mm"] = json!(0.01);
    let mut settings = document["job"]["operations"][0]["settings"].clone();
    settings["settings"]["alignment"]["initial_heading_deg"] = json!(90.);
    let updated = execute(SequenceCommand::UpdateSettings {
        job: configured,
        operation_id: "knife-1".into(),
        settings,
    })
    .unwrap();
    let summary = execute(SequenceCommand::Plan {
        job: job_of(&updated),
        scope: PlanScope::AllEnabled,
    })
    .unwrap();
    assert_eq!(
        summary["summary"]["operations"][0]["generationStatus"],
        json!("complete"),
        "{summary}"
    );
    assert_eq!(summary["summary"]["stages"][0]["role"], json!("knife"));
    assert!(summary["summary"]["motionCount"].as_u64().unwrap() > 0);
    let motions = &summary["motions"]["motions"];
    assert!(
        motions
            .as_array()
            .unwrap()
            .iter()
            .any(|motion| motion["purpose"] == json!("knife_cut")
                && motion["bladeHeadingDeg"].is_array())
    );
}

// --- F3d: typed knife commands through the shared service ---

/// A configured knife-only job (the open-chains fixture of the F2 test,
/// with the initial heading supplied) plus a second knife operation sharing
/// the tool, so assignment targeting is observable.
fn knife_service_job() -> Value {
    use cam_core::job::{PlanningTolerances, SourceSnapshot};
    use cam_core::project::*;
    use cam_core::svg::{ImportMode, ImportOptions};

    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 5 L25 5"/></svg>"##;
    let settings = DragKnifeSettings {
        chains: vec!["cut-chain-0".into()],
        assignment: KnifeAssignment {
            tool_id: "blade".into(),
            cutting_feed_mm_min: Some(150.),
            plunge_feed_mm_min: Some(60.),
            swivel_feed_mm_min: Some(50.),
            max_stepdown_mm: Some(1.),
        },
        top: HeightRef {
            reference: HeightReference::StockTop,
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
        alignment: KnifeAlignment {
            initial_heading_deg: Some(90.),
        },
    };
    let job = CamJob {
        schema_version: 4,
        name: "knife-service".into(),
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
        operations: vec![
            Operation {
                id: "knife-1".into(),
                name: "knife-1".into(),
                enabled: true,
                settings: OperationSettings::DragKnife(settings.clone()),
            },
            Operation {
                id: "knife-2".into(),
                name: "knife-2".into(),
                enabled: true,
                settings: OperationSettings::DragKnife({
                    // A sibling sharing the same tool with different values:
                    // applying a preset to knife-1 must never touch these.
                    let mut sibling = settings.clone();
                    sibling.assignment.cutting_feed_mm_min = Some(300.);
                    sibling.assignment.max_stepdown_mm = Some(2.);
                    sibling
                }),
            },
        ],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: None,
        },
        legacy_machine_profile: None,
    };
    job.validate().unwrap();
    serde_json::to_value(&job).unwrap()
}

fn knife_library() -> Value {
    use cam_core::project::DragKnifeSpec;
    use cam_core::tool_library::{KnifeCuttingPreset, LibraryGeometry, LibraryTool, ToolLibrary};
    let library = ToolLibrary {
        tools: vec![LibraryTool {
            spindle_direction: None,
            id: "blade".into(),
            name: "Drag knife".into(),
            geometry: LibraryGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.5,
                max_cut_depth_mm: 3.,
            }),
            ramp_capable: None,
            plunge_capable: None,
            cutting_presets: vec![],
            knife_cutting_presets: vec![KnifeCuttingPreset {
                id: "cardboard".into(),
                name: "Cardboard".into(),
                material: Some("cardboard".into()),
                machine: None,
                cutting_feed_mm_min: Some(150.),
                plunge_feed_mm_min: Some(60.),
                swivel_feed_mm_min: Some(50.),
                max_stepdown_mm: Some(1.),
            }],
        }],
        ..Default::default()
    };
    serde_json::to_value(&library).unwrap()
}

/// The knife-mapped variant of the shared sequence profile.
fn knife_profile() -> Value {
    let mut profile = sequence_profile();
    profile["tools"] =
        json!([{"tool_id": "blade", "tool_number": 3, "length_offset_number": null}]);
    profile
}

#[test]
fn add_operation_edits_create_incomplete_drag_knife_operations() {
    let document = execute(SequenceCommand::Open {
        json: knife_service_job().to_string(),
    })
    .unwrap();
    let added = execute(SequenceCommand::Edit {
        job: job_of(&document),
        edits: vec![OperationEdit::Add {
            id: "knife-3".into(),
            name: "Third cut".into(),
            kind: cam_service::sequence::AddOperationKind::DragKnife,
            tool_id: "blade".into(),
        }],
    })
    .unwrap();
    assert_eq!(added["operations"].as_array().unwrap().len(), 3);
    assert_eq!(added["operations"][2]["kind"], json!("drag_knife"));
    assert_eq!(added["operations"][2]["toolIds"], json!(["blade"]));
    // Nothing is invented: the never-defaulted initial heading and every
    // feed value are reported as missing machining settings.
    let missing = added["missingByOperation"]["knife-3"].as_array().unwrap();
    assert!(missing.iter().any(|entry| {
        entry["fieldPath"]
            .as_str()
            .unwrap()
            .contains("alignment.initial_heading_deg")
    }));
    assert!(missing.iter().any(|entry| {
        entry["fieldPath"]
            .as_str()
            .unwrap()
            .contains("swivel_feed_mm_min")
    }));
}

#[test]
fn apply_knife_tool_targets_one_assignment_and_preserves_siblings() {
    let job = knife_service_job();
    let applied = execute(SequenceCommand::ApplyKnifeTool {
        job: job.clone(),
        library: knife_library(),
        operation_id: "knife-1".into(),
        tool_id: "blade".into(),
        preset_id: Some("cardboard".into()),
    })
    .unwrap();
    let applied_job = &applied["job"];
    let assignment = |document: &Value, id: usize| {
        document["operations"][id]["settings"]["settings"]["assignment"].clone()
    };
    // The target assignment received the typed preset values and the new
    // geometry snapshot (offset 1.5, not the job's original 1.0). Its own
    // values were 150/1 before, so equality alone proves nothing; the
    // sibling's distinct values are what must survive untouched.
    assert_eq!(
        assignment(applied_job, 0)["cutting_feed_mm_min"],
        json!(150.)
    );
    assert_eq!(assignment(applied_job, 0)["swivel_feed_mm_min"], json!(50.));
    assert_eq!(
        applied_job["tools"][0]["geometry"]["dimensions"]["blade_offset_mm"],
        json!(1.5)
    );
    // The sibling knife operation shares the replaced tool snapshot but
    // keeps its own assignment values: applying to one operation never
    // updates another.
    assert_eq!(
        assignment(applied_job, 1)["cutting_feed_mm_min"],
        json!(300.)
    );
    assert_eq!(assignment(applied_job, 1)["max_stepdown_mm"], json!(2.));

    // Unknown operations and unknown library tools are located errors, and
    // the submitted document is never partially rewritten on failure.
    let error = execute(SequenceCommand::ApplyKnifeTool {
        job: job.clone(),
        library: knife_library(),
        operation_id: "ghost".into(),
        tool_id: "blade".into(),
        preset_id: None,
    })
    .unwrap_err();
    assert_eq!(error.code, "LIBRARY_NOT_FOUND");

    // A tool that does not exist in the library is a located error (a
    // renamed library tool is a legitimate application: the snapshot is
    // added under its own ID and the assignment rebinds).
    let error = execute(SequenceCommand::ApplyKnifeTool {
        job,
        library: knife_library(),
        operation_id: "knife-1".into(),
        tool_id: "ghost".into(),
        preset_id: None,
    })
    .unwrap_err();
    assert_eq!(error.code, "LIBRARY_NOT_FOUND");
}

#[test]
fn knife_evidence_reports_bound_traces_stock_and_program_hash() {
    // Knife-only: no milling tool exists anywhere in the job; the evidence
    // command still describes the physical stock and the bounded traces.
    let mut job = knife_service_job();
    job["operations"].as_array_mut().unwrap().truncate(1);
    let evidence = execute(SequenceCommand::KnifeEvidence {
        job: job.clone(),
        profile: knife_profile(),
        sample_limit: Some(64),
    })
    .unwrap();
    let report = &evidence["report"];
    assert_eq!(report["artifactKind"], json!("knife_evidence"));
    assert_eq!(report["status"], json!("within"));
    assert_eq!(report["initialHeadingDeg"], json!(90.));
    assert_eq!(report["bladeOffsetMm"], json!(1.));
    let sha = report["programSha256"].as_str().unwrap();
    assert_eq!(sha.len(), 64);
    assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()));
    let samples = report["samples"].as_array().unwrap();
    assert!(!samples.is_empty());
    assert!(samples.len() <= 64);
    for sample in samples {
        assert!(sample["stageId"].is_string());
        assert!(sample["motionIndex"].is_u64());
        assert!(sample["pivotMm"]["x"].is_number());
        assert!(sample["replayedTipMm"]["x"].is_number());
    }
    assert_eq!(evidence["stock"]["thicknessMm"], json!(3.));
    assert!(evidence["stock"]["xy"]["width_mm"].is_number());
    assert_eq!(evidence["stock"]["hasKnifeStages"], json!(true));
    // The evidence summary flows through the same projection the UI uses,
    // with basic checks passed for the complete knife plan.
    assert_eq!(
        evidence["summary"]["basicChecks"]["status"],
        json!("passed")
    );
    assert_eq!(
        evidence["summary"]["operations"][0]["generationStatus"],
        json!("complete")
    );

    // The sample budget bounds detail without changing the outcome: asking
    // for one sample reports truncation with the same maxima and status.
    let small = execute(SequenceCommand::KnifeEvidence {
        job,
        profile: knife_profile(),
        sample_limit: Some(1),
    })
    .unwrap();
    assert_eq!(small["report"]["truncated"], json!(true));
    assert_eq!(small["report"]["samples"].as_array().unwrap().len(), 1);
    assert_eq!(small["report"]["status"], json!("within"));
    assert_eq!(
        small["report"]["maxTipDeviationMm"],
        report["maxTipDeviationMm"]
    );
}
