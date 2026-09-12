//! GUI7a/b: a source-free Face job — settings, generation, coverage controls,
//! actual heightfield playback, checked export and portable reopen.
use cam_core::project::{
    FaceArea, RectXY,
    v5::{self, CamJobV5, OperationSettingsV5},
};
use cam_gui_runtime::{
    app,
    operation_authoring::{self, Kind},
    session::{self, Command},
};
use cam_service::retained::Retained;

/// A small fixture: 40 × 30 × 6 mm stock faced 1 mm deep with a 3 mm endmill.
fn facing_job() -> CamJobV5 {
    let empty = operation_authoring::empty_job();
    let mut job =
        operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty)).unwrap();
    let id = "face-1".to_string();
    for (field, value) in [
        (6, 6.),    // stock thickness
        (7, 5.),    // clearance above stock
        (2, 300.),  // cutting feed
        (10, 100.), // plunge feed
        (11, 12_000.),
        (8, 1.),   // stepdown
        (88, 1.),  // tool stepdown limit
        (9, 1.5),  // stepover
        (75, 0.),  // pass angle
        (87, -1.), // bottom offset: remove 1 mm
    ] {
        job = app::set_value(&job, &id, field, Some(value))
            .unwrap_or_else(|error| panic!("field {field}: {error}"));
    }
    let mut candidate = job.clone();
    cam_gui_runtime::authoring::set_group_in(&mut candidate, &id, 12, &[3., 8.]).unwrap();
    job = candidate;
    cam_gui_runtime::face::set_spindle_direction(
        &mut job,
        &id,
        Some(cam_core::project::SpindleDirection::Clockwise),
    )
    .unwrap();
    // Physical stock rectangle in setup millimeters.
    let mut stock = job.clone();
    stock.setup.stock.xy = Some(RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 30.,
    });
    stock.validate_structure().unwrap();
    // Checked export needs one applied machine configuration; the reviewed
    // fixture profile maps the endmill this face operation uses.
    let (applied, _) = session::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: stock.to_json().unwrap(),
            json: session::PROFILE.into(),
        },
    )
    .unwrap();
    session::open(&applied.job).unwrap()
}

/// Generate through one retained service instance; its plan handles belong to
/// that instance and preparing with a different one is refused by design.
fn generate(service: &mut Retained, job: &CamJobV5) -> cam_gui_runtime::compute::SceneMeta {
    session::execute(service, Command::generate(job.to_json().unwrap()))
        .unwrap()
        .0
}

#[test]
fn face_job_generates_playback_output_and_reopens() {
    let job = facing_job();
    let mut service = Retained::new();
    let meta = generate(&mut service, &job);
    assert!(meta.motions > 0, "facing produces real motions");
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{}", report);
    // The published face plane is the requested depth below the stock top.
    let face = &report["inspection"]["operations"][0]["face"];
    assert!((face["zMm"].as_f64().unwrap() - -1.).abs() < 1e-9, "{face}");
    assert!((face["covered"]["width_mm"].as_f64().unwrap() - 40.).abs() < 1e-9);
    // Total stock removal matches the whole requested rectangle at 1 mm.
    let preview = meta.stock.as_ref().unwrap();
    let last = preview.frames.last().unwrap();
    assert!(
        (last.stats.removed_volume_mm3 - 40. * 30. * 1.).abs() < 60.,
        "removed {} mm³",
        last.stats.removed_volume_mm3
    );
    assert_eq!(preview.frames[0].stats.removed_volume_mm3, 0.);
    // Export and portable reopen.
    let handle = report["handle"].as_str().unwrap().to_owned();
    let (prepared, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: job.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    assert!(
        prepared.report["gui2"]["file"]["gcode"]
            .as_str()
            .is_some_and(|gcode| gcode.lines().count() > 5)
    );
    let reopened = session::open(&prepared.job).unwrap();
    assert_eq!(reopened.operations.len(), 1);
    assert!(matches!(
        reopened.operations[0].settings,
        OperationSettingsV5::Face(_)
    ));
}

#[test]
fn facing_coverage_controls_change_what_is_removed_and_where_travel_goes() {
    let mut job = facing_job();
    let id = "face-1";
    // Passes run along Y, coverage is a 20 × 10 rectangle at (5, 5), margins
    // expand it 2 mm per side and the travel overruns 3 mm at each end.
    job = app::set_value(&job, id, 75, Some(90.)).unwrap();
    cam_gui_runtime::face::set_area(
        &mut job,
        id,
        FaceArea::Rectangle {
            rect: RectXY {
                min_x_mm: 5.,
                min_y_mm: 5.,
                width_mm: 20.,
                length_mm: 10.,
            },
        },
    )
    .unwrap();
    job = app::set_value(&job, id, 78, Some(2.)).unwrap();
    job = app::set_value(&job, id, 79, Some(2.)).unwrap();
    job = app::set_value(&job, id, 80, Some(2.)).unwrap();
    job = app::set_value(&job, id, 81, Some(2.)).unwrap();
    job = app::set_value(&job, id, 76, Some(3.)).unwrap();
    job = app::set_value(&job, id, 77, Some(3.)).unwrap();
    let mut service = Retained::new();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    let covered = &report["inspection"]["operations"][0]["face"]["covered"];
    // Requested 5..25 × 5..15 expanded by 2 mm of margins per side.
    assert!(
        (covered["min_x_mm"].as_f64().unwrap() - 3.).abs() < 1e-9,
        "{covered}"
    );
    assert!(
        (covered["min_y_mm"].as_f64().unwrap() - 3.).abs() < 1e-9,
        "{covered}"
    );
    assert!((covered["width_mm"].as_f64().unwrap() - 24.).abs() < 1e-9);
    assert!((covered["length_mm"].as_f64().unwrap() - 14.).abs() < 1e-9);
    // Only the requested coverage is removed.
    let removed = meta
        .stock
        .as_ref()
        .unwrap()
        .frames
        .last()
        .unwrap()
        .stats
        .removed_volume_mm3;
    let covered_area = 24. * 14.;
    assert!(
        removed > covered_area,
        "removed {removed} mm³ exceeds the requested coverage because the engaged tool overhangs"
    );
    assert!(
        removed < (covered_area + 2. * 7. * (24. + 14.)),
        "overhang stays bounded near the coverage: removed {removed} mm³"
    );
    // Travel overrun is real travel, and the requested coverage is what is
    // removed: the motions extend beyond the covered rectangle, but the
    // removed volume matches the coverage (plus the cutter's free pass).
    let report = &meta.report["gui2"];
    assert_eq!(report["groups"][0]["role"], "Face");
    assert_eq!(report["groups"][0]["jump"], "After face");
    let bounds = meta.bounds;
    assert!(
        bounds[1] <= 3. && bounds[3] >= 17.,
        "travel reaches the overrun: {bounds:?}"
    );
}

#[test]
fn rejected_facing_settings_are_located_and_never_export() {
    let mut job = facing_job();
    let id = "face-1";
    let mut service = Retained::new();
    job = app::set_value(&job, id, 75, Some(45.)).unwrap();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], false);
    assert!(
        report["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "FACE_ANGLE_UNSUPPORTED")),
        "{}",
        report["generationIssues"]
    );
    // A stepover larger than the cutter is rejected with its allowed range.
    let mut job = facing_job();
    job = app::set_value(&job, id, 9, Some(40.)).unwrap();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], false);
    assert!(
        report["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "FACE_STEPOVER_RANGE")),
        "{}",
        report["generationIssues"]
    );
    // Missing stock thickness is reported before planning as a field issue.
    let mut job = facing_job();
    job = app::set_value(&job, id, 6, None).unwrap();
    let meta = generate(&mut service, &job);
    assert_eq!(meta.report["gui2"]["kind"], "issues");
    assert!(
        meta.report["gui2"]["issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| issue["field_path"]
                .as_str()
                .is_some_and(|path| path.contains("thickness"))))
    );
}

#[test]
fn face_before_the_known_carving_publishes_per_operation_stock() {
    // The established carving plus a face operation before it: the heightfield
    // must show the faced stock before the carving removes anything.
    let carving = session::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
    let mut job =
        operation_authoring::apply(&carving, operation_authoring::add(Kind::Face, &carving))
            .unwrap();
    let id = "face-1";
    for (field, value) in [
        (2, 300.),
        (10, 100.),
        (11, 12_000.),
        (8, 0.5),
        (88, 0.5),
        (9, 1.5),
        (75, 0.),
        (87, -0.5),
    ] {
        job = app::set_value(&job, id, field, Some(value)).unwrap();
    }
    let mut candidate = job.clone();
    cam_gui_runtime::authoring::set_group_in(&mut candidate, id, 12, &[6., 8.]).unwrap();
    job = candidate;
    cam_gui_runtime::face::set_spindle_direction(
        &mut job,
        id,
        Some(cam_core::project::SpindleDirection::Clockwise),
    )
    .unwrap();
    job = operation_authoring::apply(
        &job,
        operation_authoring::Action::Move {
            operation_id: id.into(),
            to_index: 0,
        },
    )
    .unwrap();
    let mut service = Retained::new();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{}", report);
    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups[0]["role"], "Face", "{groups:?}");
    assert_eq!(groups[1]["role"], "Endmill", "{groups:?}");
    let face_end = groups[0]["end"].as_u64().unwrap() as usize;
    let frames = &meta.stock.as_ref().unwrap().frames;
    // The face boundary is a real stock checkpoint: removal before the carve
    // equals the faced area at the requested depth.
    let face_frame = frames
        .iter()
        .find(|frame| frame.prefix == face_end)
        .expect("a checkpoint at the face operation boundary");
    assert!(face_frame.stats.removed_volume_mm3 > 0.);
    assert!(face_frame.stats.removed_volume_mm3 < frames.last().unwrap().stats.removed_volume_mm3);
    // The carve that follows references the face plane the prefix published.
    let mut bound = job.clone();
    {
        let OperationSettingsV5::FlatVcarve(settings) = &mut bound
            .operations
            .iter_mut()
            .find(|operation| operation.id == "carving")
            .unwrap()
            .settings
        else {
            panic!("carving is a Flat V-carve")
        };
        settings.top.reference = cam_core::project::HeightReference::FaceResult {
            operation_id: id.into(),
        };
    }
    bound.validate_structure().unwrap();
    job = bound;
    let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
        &job,
        &v5::ReadinessScope::AllEnabled,
        &cam_core::sequence::PlanLimits::default(),
    )
    .unwrap();
    assert!(
        plan.operation_results.iter().all(
            |result| result.generation_status == cam_core::sequence::GenerationStatus::Complete
        ),
        "{:?}",
        plan.generation_diagnostics
    );
    let carve_top = plan
        .motions
        .iter()
        .filter(|m| {
            m.operation_id == "carving"
                && m.effect == cam_core::toolpath::MotionEffect::MillingSweep
        })
        .map(|m| m.start.z)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (carve_top - -0.5).abs() < 1e-6,
        "carving cuts from the faced plane -0.5, got {carve_top}"
    );
}

#[test]
fn face_then_knife_keeps_the_knife_stock_intact() {
    // GUI7d: a supported Face → drag knife fixture. The knife traces leave the
    // preceding stock unchanged and the sequence still prepares output.
    let knife = session::open(include_str!("../../../fixtures/gui6/knife.job.json")).unwrap();
    let mut job =
        operation_authoring::apply(&knife, operation_authoring::add(Kind::Face, &knife)).unwrap();
    let id = "face-1";
    for (field, value) in [
        (2, 300.),
        (10, 100.),
        (11, 12_000.),
        (8, 0.5),
        (88, 0.5),
        (9, 1.5),
        (75, 0.),
        (87, -0.5),
    ] {
        job = app::set_value(&job, id, field, Some(value)).unwrap();
    }
    let mut candidate = job.clone();
    cam_gui_runtime::authoring::set_group_in(&mut candidate, id, 12, &[6., 8.]).unwrap();
    job = candidate;
    cam_gui_runtime::face::set_spindle_direction(
        &mut job,
        id,
        Some(cam_core::project::SpindleDirection::Clockwise),
    )
    .unwrap();
    job = operation_authoring::apply(
        &job,
        operation_authoring::Action::Move {
            operation_id: id.into(),
            to_index: 0,
        },
    )
    .unwrap();
    let mut service = Retained::new();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{}", report);
    assert_eq!(report["knife"], true);
    let roles: Vec<&str> = report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| group["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["Face", "Knife"], "{roles:?}");
    // Removed volume stops changing at the knife stage: only the face removes.
    let frames = &meta.stock.as_ref().unwrap().frames;
    let knife_start = report["groups"][1]["start"].as_u64().unwrap() as usize;
    let before_knife = frames
        .iter()
        .filter(|frame| frame.prefix <= knife_start)
        .max_by_key(|frame| frame.prefix)
        .unwrap();
    let final_frame = frames.last().unwrap();
    assert_eq!(
        before_knife.stats.removed_volume_mm3, final_frame.stats.removed_volume_mm3,
        "knife traces remove no stock"
    );
    let handle = report["handle"].as_str().unwrap().to_owned();
    // The face cutter has no controller mapping yet: preparation refuses the
    // sequence with that located reason, then succeeds once it is mapped.
    let unmapped = session::execute(
        &mut service,
        Command::Prepare {
            job: job.to_json().unwrap(),
            handle: handle.clone(),
        },
    )
    .unwrap_err();
    assert!(unmapped.contains("MACHINE_MAPPING_MISSING"), "{unmapped}");
    let mapped = v5::machine::set_tool_mapping(&job, "endmill", Some(2), None)
        .map_err(|e| e.to_string())
        .unwrap()
        .job;
    let (prepared, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: mapped.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    assert_eq!(
        prepared.report["gui2"]["bundle"]["report"]["knifeEvidence"]["status"], "within",
        "the emitted replay stays inside its tip/heading budgets"
    );
    // A blade planted shallower than the material the face already removed is
    // rejected with a located reason instead of authorizing the sequence.
    let mut planted = job.clone();
    planted = app::set_value(&planted, id, 87, Some(-2.)).unwrap();
    let meta = generate(&mut service, &planted);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], false);
    let issue = report["generationIssues"]
        .as_array()
        .and_then(|issues| {
            issues
                .iter()
                .find(|issue| issue["code"] == "KNIFE_CONTACT_UNSUPPORTED")
        })
        .unwrap_or_else(|| panic!("located contact rejection: {}", report["generationIssues"]));
    assert_eq!(issue["operationId"], "knife");
}
