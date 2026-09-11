use super::*;
use serde_json::json;

fn instance() -> String {
    "1f".repeat(16)
}
fn hex64(value: &Value) -> &str {
    value.as_str().unwrap()
}
fn parse(reply: &str) -> Value {
    serde_json::from_str(reply).unwrap()
}

#[test]
fn document_open_returns_the_envelope_the_ui_expects() {
    let job_json = include_str!("../../../fixtures/m3/island.json");
    let request = json!({"apiVersion": document::API_VERSION, "requestId": "r-1", "revision": 3,
        "command": {"operation": "open", "json": job_json}});
    let reply = parse(&document(&request.to_string()));
    assert_eq!(reply["ok"]["requestId"], "r-1");
    assert_eq!(reply["ok"]["revision"], 3);
    assert_eq!(reply["ok"]["apiVersion"], document::API_VERSION);
    let data = &reply["ok"]["data"];
    assert!(!data["display"]["components"].as_array().unwrap().is_empty());
    assert_eq!(data["display"]["coordinateSpace"], "source-page-mm-y-up");
    assert_eq!(hex64(&data["documentFingerprint"]).len(), 64);
    assert!(data["missingMachiningFields"].is_array());
}

#[test]
fn document_validate_is_authoritative_for_invalid_jobs() {
    let request = json!({"apiVersion": document::API_VERSION, "requestId": "r-2", "revision": 0,
        "command": {"operation": "validate", "job": {"schema_version": 3, "unexpected": true}}});
    let reply = parse(&document(&request.to_string()));
    let data = &reply["ok"]["data"];
    assert_eq!(data["valid"], false);
    assert_eq!(data["authoritative"], true);
    assert!(data["diagnostics"][0]["severity"] == "error");
    assert!(data["documentFingerprint"].is_null());
}

#[test]
fn document_rejects_api_and_request_identity_mismatch() {
    let request = json!({"apiVersion": "ui-6", "requestId": "r-3", "revision": 0,
        "command": {"operation": "validate", "job": {}}});
    let reply = parse(&document(&request.to_string()));
    assert_eq!(reply["error"]["code"], "API_VERSION");
    let request = json!({"apiVersion": document::API_VERSION, "requestId": "bad id!", "revision": 0,
        "command": {"operation": "validate", "job": {}}});
    let reply = parse(&document(&request.to_string()));
    assert_eq!(reply["error"]["code"], "REQUEST_IDENTITY");
}

fn island_plan_request(revision: u64) -> (String, String) {
    let job_json = include_str!("../../../fixtures/m3/island.json");
    let job = Job::from_json(job_json).unwrap();
    let fingerprint = document::fingerprint(&job);
    (
        json!({"apiVersion": document::API_VERSION, "instanceId": instance(), "requestId": "plan-1",
            "revision": revision, "documentFingerprint": fingerprint, "stage": "endmill",
            "job": serde_json::to_value(&job).unwrap()})
        .to_string(),
        fingerprint,
    )
}

#[test]
fn admit_plan_checks_identity_staleness_and_returns_a_stable_hash() {
    let (request, fingerprint) = island_plan_request(7);
    let admitted = parse(&admit_plan(&request, &instance()));
    assert_eq!(admitted["ok"]["documentFingerprint"], fingerprint);
    let hash = hex64(&admitted["ok"]["requestHash"]);
    assert_eq!(hash.len(), 64);
    assert_eq!(
        parse(&admit_plan(&request, &instance()))["ok"]["requestHash"],
        hash
    );
    let wrong_instance = request.replace(&instance(), &"ab".repeat(16));
    assert_eq!(
        parse(&admit_plan(&wrong_instance, &instance()))["error"]["code"],
        "TASK_INSTANCE"
    );
    let stale = request.replace(&fingerprint, &"0".repeat(64));
    assert_eq!(
        parse(&admit_plan(&stale, &instance()))["error"]["code"],
        "STALE_DOCUMENT"
    );
}

#[test]
fn plan_endmill_returns_a_consistent_summary_motions_and_plan_json() {
    let input = json!({"stage": "endmill", "job": serde_json::from_str::<Value>(
        include_str!("../../../fixtures/m3/island.json")).unwrap().to_string()});
    let reply = parse(&plan(&input.to_string()));
    let output = &reply["ok"];
    let motions = output["motions"].as_array().unwrap();
    assert!(!motions.is_empty());
    assert_eq!(output["summary"]["motionCount"], motions.len());
    assert_eq!(output["summary"]["previewMotionCount"], motions.len());
    assert_eq!(output["summary"]["engineVersion"], ENGINE_VERSION);
    assert!(hex64(&output["summary"]["inputFingerprint"]).len() == 64);
    assert!(
        !output["inspection"]["slices"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(serde_json::from_str::<Value>(output["planJson"].as_str().unwrap()).is_ok());
    assert!(output["verificationReceipt"].is_null());
}

fn combined_output() -> Value {
    let input =
        json!({"stage": "combined", "job": include_str!("../../../fixtures/m4/wide-floor.json")});
    let reply = parse(&plan(&input.to_string()));
    assert!(
        reply["error"].is_null(),
        "combined planning failed: {reply}"
    );
    reply["ok"].clone()
}

#[test]
fn plan_verify_and_export_round_trip_with_matching_identities() {
    let output = combined_output();
    assert!(!output["verificationReceipt"].is_null());
    let identity = json!({"planTaskId": "plan-1",
        "inputFingerprint": output["summary"]["inputFingerprint"],
        "motionFingerprint": output["summary"]["motionFingerprint"],
        "options": cam_core::verification::VerificationOptions::default()});
    let verify_input = json!({"planJson": output["planJson"], "receipt": output["verificationReceipt"],
        "identity": identity});
    let verified = parse(&verify(&verify_input.to_string()))["ok"].clone();
    assert_eq!(verified["summary"]["status"], "passed");
    let report: Value = serde_json::from_str(verified["reportJson"].as_str().unwrap()).unwrap();
    assert_eq!(report["status"], "passed");
    assert_eq!(
        report["verification_fingerprint"],
        verified["summary"]["verificationFingerprint"]
    );

    let profile = cam_core::post::LinuxCncProfile::from_json(include_str!(
        "../../../fixtures/m6/macro-stock-bottom.json"
    ))
    .unwrap();
    let export_identity = json!({"planTaskId": "plan-1",
        "inputFingerprint": output["summary"]["inputFingerprint"],
        "motionFingerprint": output["summary"]["motionFingerprint"],
        "profile": profile, "layout": "combined",
        "options": cam_core::verification::VerificationOptions::default()});
    let export_input = json!({"planJson": output["planJson"], "receipt": output["verificationReceipt"],
        "identity": export_identity});
    let exported = parse(&export_linuxcnc(&export_input.to_string()))["ok"].clone();
    assert_eq!(exported["summary"]["status"], "passed");
    assert_eq!(exported["programs"].as_array().unwrap().len(), 1);
    assert_eq!(exported["programs"][0]["filename"], "combined.ngc");
    assert!(
        !exported["programs"][0]["gcode"]
            .as_str()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn verify_rejects_a_receipt_from_a_different_plan() {
    let output = combined_output();
    let identity = json!({"planTaskId": "plan-1",
        "inputFingerprint": "0".repeat(64),
        "motionFingerprint": output["summary"]["motionFingerprint"],
        "options": cam_core::verification::VerificationOptions::default()});
    let verify_input = json!({"planJson": output["planJson"], "receipt": output["verificationReceipt"],
        "identity": identity});
    let reply = parse(&verify(&verify_input.to_string()));
    assert_eq!(reply["error"]["code"], "VERIFICATION_PLAN_IDENTITY");
}

#[test]
fn admission_for_reports_binds_the_combined_source_plan() {
    let (_, fingerprint) = island_plan_request(9);
    let source = json!({"stage": "endmill", "revision": 9, "documentFingerprint": fingerprint,
        "verification": null, "export": null,
        "summary": {"inputFingerprint": "a".repeat(64), "motionFingerprint": "b".repeat(64)}});
    let verify_start = json!({"apiVersion": document::API_VERSION, "instanceId": instance(),
        "requestId": "verify-1", "revision": 9, "documentFingerprint": fingerprint,
        "verification": {"planTaskId": "plan-1", "inputFingerprint": "a".repeat(64),
            "motionFingerprint": "b".repeat(64),
            "options": cam_core::verification::VerificationOptions::default()}});
    let reply = parse(&admit_verification(
        &verify_start.to_string(),
        &instance(),
        &source.to_string(),
    ));
    assert_eq!(reply["error"]["code"], "VERIFICATION_STAGE");
    let combined_source = json!({"stage": "combined", "revision": 9, "documentFingerprint": fingerprint,
        "verification": null, "export": null,
        "summary": {"inputFingerprint": "a".repeat(64), "motionFingerprint": "b".repeat(64)}});
    let reply = parse(&admit_verification(
        &verify_start.to_string(),
        &instance(),
        &combined_source.to_string(),
    ));
    assert_eq!(hex64(&reply["ok"]["requestHash"]).len(), 64);
    let mismatched = json!({"planTaskId": "plan-1", "inputFingerprint": "a".repeat(64),
        "motionFingerprint": "c".repeat(64),
        "options": cam_core::verification::VerificationOptions::default()});
    let mut wrong = verify_start.clone();
    wrong["verification"] = mismatched;
    let reply = parse(&admit_verification(
        &wrong.to_string(),
        &instance(),
        &combined_source.to_string(),
    ));
    assert_eq!(reply["error"]["code"], "VERIFICATION_PLAN_IDENTITY");
}

#[test]
fn sequence_commands_round_trip_through_the_worker_envelope() {
    let legacy = include_str!("../../../fixtures/m3/rectangle.json");
    let open = json!({
        "apiVersion": "ui-8", "requestId": "seq-1", "revision": 1,
        "command": {"operation": "open", "json": legacy}
    });
    let reply = parse(&super::sequence(&open.to_string(), &instance()));
    let document = reply["ok"]["data"].clone();
    assert_eq!(document["migrated"], json!(true));
    assert_eq!(document["job"]["schema_version"], json!(4));

    let edit = json!({
        "apiVersion": "ui-8", "requestId": "seq-2", "revision": 2,
        "command": {"operation": "edit", "job": document["job"],
            "edits": [{"edit": "duplicate", "id": "flat-v-carve", "newId": "carve-2"}]}
    });
    let reply = parse(&super::sequence(&edit.to_string(), &instance()));
    assert_eq!(
        reply["ok"]["data"]["operations"].as_array().unwrap().len(),
        2
    );

    // Admission mirrors the HTTP route: wrong API version and weak identity.
    let wrong = json!({
        "apiVersion": "ui-7", "requestId": "seq-3", "revision": 3,
        "command": {"operation": "capabilities"}
    });
    let reply = parse(&super::sequence(&wrong.to_string(), &instance()));
    assert_eq!(reply["error"]["code"], "TASK_INSTANCE");
    let weak = json!({
        "apiVersion": "ui-8", "requestId": "", "revision": 3,
        "command": {"operation": "capabilities"}
    });
    let reply = parse(&super::sequence(&weak.to_string(), &instance()));
    assert_eq!(reply["error"]["code"], "REQUEST_IDENTITY");

    let plan = json!({
        "apiVersion": "ui-8", "requestId": "seq-4", "revision": 4,
        "command": {"operation": "plan", "job": document["job"], "scope": {"kind": "allEnabled"}}
    });
    let reply = parse(&super::sequence(&plan.to_string(), &instance()));
    let summary = &reply["ok"]["data"]["summary"];
    assert_eq!(summary["basicChecks"]["status"], json!("passed"));
    assert!(summary["motionCount"].as_u64().unwrap() > 0);

    // Motion paging and the contour catalogue flow through the worker
    // envelope like every other sequence command (D3 timeline/profile UI).
    let total = summary["motionCount"].as_u64().unwrap();
    let motions = json!({
        "apiVersion": "ui-8", "requestId": "seq-5", "revision": 5,
        "command": {"operation": "motions", "job": document["job"],
            "scope": {"kind": "allEnabled"}, "offset": 0}
    });
    let reply = parse(&super::sequence(&motions.to_string(), &instance()));
    let page = &reply["ok"]["data"]["motions"];
    assert_eq!(page["offset"], json!(0));
    assert_eq!(page["total"], json!(total));
    assert!(!page["motions"].as_array().unwrap().is_empty());
    let beyond = json!({
        "apiVersion": "ui-8", "requestId": "seq-6", "revision": 6,
        "command": {"operation": "motions", "job": document["job"],
            "scope": {"kind": "allEnabled"}, "offset": total + 1}
    });
    let reply = parse(&super::sequence(&beyond.to_string(), &instance()));
    assert_eq!(
        reply["ok"]["diagnostic"]["code"],
        json!("SEQUENCE_MOTION_OFFSET")
    );

    let contours = json!({
        "apiVersion": "ui-8", "requestId": "seq-7", "revision": 7,
        "command": {"operation": "contours", "job": document["job"]}
    });
    let reply = parse(&super::sequence(&contours.to_string(), &instance()));
    let catalogue = reply["ok"]["data"]["contours"].as_array().unwrap();
    assert!(!catalogue.is_empty());
    assert!(catalogue[0]["id"].as_str().unwrap().ends_with("-outer"));
}

#[test]
fn knife_commands_round_trip_through_the_worker_envelope() {
    // F3d: typed knife creation, targeted preset application and bounded
    // emitted-output evidence flow through the WASM boundary with the same
    // semantics as the HTTP route.
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40mm\" height=\"30mm\" viewBox=\"0 0 40 30\"><path id=\"cut\" fill=\"none\" stroke=\"#000\" stroke-width=\"0.4\" d=\"M5 5 L25 5\"/></svg>";
    let job = json!({
        "schema_version": 4,
        "name": "knife-wasm",
        "source": {"filename": "cut.svg", "svg": svg},
        "import": {"mode": "centerline", "geometry_tolerance_mm": 0.001,
            "placement": {"origin_mm": {"x": 0.0, "y": 0.0}, "rotation_deg": 0.0, "scale": 1.0}},
        "setup": {
            "stock": {"thickness_mm": 3.0, "xy": {"min_x_mm": 0.0, "min_y_mm": 0.0, "width_mm": 40.0, "length_mm": 30.0}},
            "work_zero": {"xy": {"kind": "setup_origin"}, "z": "stock_top"},
            "clearance_above_stock_mm": 5.0,
        },
        "tools": [{"id": "blade", "name": "drag knife", "capabilities": {},
            "geometry": {"kind": "drag_knife", "dimensions": {"blade_offset_mm": 1.0, "max_cut_depth_mm": 2.0}}}],
        "operations": [{
            "id": "knife-1", "name": "knife-1", "enabled": true,
            "settings": {"kind": "drag_knife", "settings": {
                "chains": ["cut-chain-0"],
                "assignment": {"tool_id": "blade", "cutting_feed_mm_min": 150.0,
                    "plunge_feed_mm_min": 60.0, "swivel_feed_mm_min": 50.0, "max_stepdown_mm": 1.0},
                "top": {"reference": {"kind": "stock_top"}, "offset_mm": 0.0},
                "bottom": {"reference": {"kind": "operation_top"}, "offset_mm": -1.0},
                "stepdown_mm": 1.0, "swivel_depth_mm": 0.2, "corner_threshold_deg": 30.0,
                "start": {"kind": "automatic"}, "alignment": {"initial_heading_deg": 90.0},
            }},
        }],
        "tolerances": {"motion_tolerance_mm": 0.01},
    });
    let open = json!({
        "apiVersion": "ui-8", "requestId": "knife-1", "revision": 1,
        "command": {"operation": "open", "json": job.to_string()}
    });
    let reply = parse(&super::sequence(&open.to_string(), &instance()));
    let document = reply["ok"]["data"].clone();
    assert!(
        document["operations"].is_array(),
        "open failed: {}",
        reply["ok"]["diagnostic"]
    );
    assert_eq!(document["operations"][0]["kind"], json!("drag_knife"));

    // Typed knife creation through the edit command.
    let add = json!({
        "apiVersion": "ui-8", "requestId": "knife-2", "revision": 2,
        "command": {"operation": "edit", "job": document["job"],
            "edits": [{"edit": "add", "id": "knife-2", "name": "Second",
                "kind": "drag_knife", "toolId": "blade"}]}
    });
    let reply = parse(&super::sequence(&add.to_string(), &instance()));
    let data = reply["ok"]["data"].clone();
    assert_eq!(data["operations"].as_array().unwrap().len(), 2);
    assert_eq!(data["operations"][1]["kind"], json!("drag_knife"));

    // Targeted preset application: the sibling assignment is untouched.
    let library = json!({
        "schema_version": 1, "revision": 1,
        "tools": [{"id": "blade", "name": "Drag knife",
            "geometry": {"kind": "drag_knife", "dimensions": {"blade_offset_mm": 1.5, "max_cut_depth_mm": 3.0}},
            "ramp_capable": null, "plunge_capable": null, "cutting_presets": [],
            "knife_cutting_presets": [{"id": "cardboard", "name": "Cardboard",
                "material": "cardboard", "machine": null,
                "cutting_feed_mm_min": 120.0, "plunge_feed_mm_min": 40.0,
                "swivel_feed_mm_min": 30.0, "max_stepdown_mm": 0.8}]}],
    });
    let apply = json!({
        "apiVersion": "ui-8", "requestId": "knife-3", "revision": 3,
        "command": {"operation": "applyKnifeTool", "job": data["job"],
            "library": library, "operation_id": "knife-2",
            "tool_id": "blade", "preset_id": "cardboard"}
    });
    let reply = parse(&super::sequence(&apply.to_string(), &instance()));
    assert!(
        reply["ok"]["data"]["job"].is_object(),
        "applyKnifeTool failed: {}",
        reply
    );
    let applied = reply["ok"]["data"]["job"].clone();
    assert_eq!(
        applied["operations"][1]["settings"]["settings"]["assignment"]["cutting_feed_mm_min"],
        json!(120.0)
    );
    assert_eq!(
        applied["operations"][0]["settings"]["settings"]["assignment"]["cutting_feed_mm_min"],
        json!(150.0)
    );

    // Bounded emitted-output evidence: the worker replays the actual bytes
    // and reports the traces with the same status the native route returns.
    let legacy =
        serde_json::from_str::<Value>(include_str!("../../../../real_data/machine-profile.json"))
            .unwrap();
    let profile = json!({
        "schema_version": 2,
        "id": "wasm-knife",
        "work_offset": "G54",
        "clearance_z_mm": 5.0,
        "decimal_places": 3,
        "program_start_position_mm": null,
        "length_compensation": "macro_managed",
        "path_control": {"kind": "exact_path"},
        "tools": [{"tool_id": "blade", "tool_number": 3, "length_offset_number": null}],
        "spindle_spinup_seconds": 0.5,
        "coolant": "off",
        "m6": legacy["m6"].clone()
    });
    let evidence = json!({
        "apiVersion": "ui-8", "requestId": "knife-4", "revision": 4,
        "command": {"operation": "knifeEvidence", "job": document["job"],
            "profile": profile, "sample_limit": 16}
    });
    let reply = parse(&super::sequence(&evidence.to_string(), &instance()));
    assert!(
        reply["ok"]["data"]["report"].is_object(),
        "knifeEvidence failed: {}",
        reply
    );
    let report = &reply["ok"]["data"]["report"];
    assert_eq!(report["status"], json!("within"));
    assert_eq!(report["initialHeadingDeg"], json!(90.0));
    assert!(report["samples"].as_array().unwrap().len() <= 16);
    assert_eq!(reply["ok"]["data"]["stock"]["hasKnifeStages"], json!(true));

    // The advertised capabilities include the emitted evidence feature.
    let capabilities = json!({
        "apiVersion": "ui-8", "requestId": "knife-5", "revision": 5,
        "command": {"operation": "capabilities"}
    });
    let reply = parse(&super::sequence(&capabilities.to_string(), &instance()));
    assert_eq!(
        reply["ok"]["data"]["features"]["knifeEmittedEvidence"],
        json!(true)
    );
}
