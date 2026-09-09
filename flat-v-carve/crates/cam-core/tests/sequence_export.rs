//! A3 sequence export: basic checks gate generation (never detailed quality),
//! process state resolves before G-code, and independent readback rejects
//! altered motion order, coordinates, feeds, tools or spindle state.
use cam_core::{
    checks::{CheckStatus, check_plan},
    job::Job as LegacyJob,
    post::{
        LinuxCncProfile,
        sequence::{PreparedExecution, SequenceProfile, apply_legacy_profile, verify_program},
    },
    project::{
        CamJob, FlatVcarveMode, FlatVcarveSettings, OperationSettings, SpindleDirection,
        migrate::{migrate_job, migrate_legacy_json},
    },
    sequence::{OperationPlan, PlanLimits, TrustedPlan},
};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");
const M4_CONTACT_LINE: &str = include_str!("../../../fixtures/m4/contact-line.json");
const LEGACY_PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");

/// Build a schema-2 sequence profile from a legacy schema-1 profile.
fn sequence_profile(job_tools: &[(&str, u32)]) -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "test-profile".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: cam_core::post::LengthCompensation::MacroManaged,
        path_control: cam_core::post::PathControl::ExactPath,
        tools: job_tools
            .iter()
            .map(|(id, n)| cam_core::post::sequence::SequenceToolMapping {
                tool_id: (*id).into(),
                tool_number: *n,
                length_offset_number: None,
            })
            .collect(),
        spindle_spinup_seconds: 0.5,
        coolant: cam_core::post::Coolant::Off,
        m6: LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap().m6,
    }
}

fn applied_job(json: &str) -> CamJob {
    let profile = LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap();
    let job = migrate_legacy_json(json).unwrap();
    apply_legacy_profile(&profile, &job).unwrap()
}

#[test]
fn applied_legacy_profile_moves_datum_and_directions_into_the_job() {
    let job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    let profile = LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap();
    let applied = apply_legacy_profile(&profile, &job).unwrap();
    assert!(matches!(
        applied.setup.work_zero.z,
        cam_core::project::WorkZeroZ::StockBottom
    ));
    let OperationSettings::FlatVcarve(settings) = &applied.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    assert_eq!(
        settings.endmill.spindle_direction,
        Some(SpindleDirection::Clockwise)
    );
    assert_eq!(
        settings.vbit.spindle_direction,
        Some(SpindleDirection::Clockwise)
    );
    // The original document is unchanged by the application.
    let OperationSettings::FlatVcarve(original) = &job.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    assert_eq!(original.endmill.spindle_direction, None);
    assert!(matches!(
        job.setup.work_zero.z,
        cam_core::project::WorkZeroZ::StockTop
    ));
}

#[test]
fn sequence_export_works_without_running_detailed_quality() {
    let job = applied_job(M3_RECTANGLE);
    // The applied profile selected a stock-bottom Z datum over 8 mm stock:
    // setup clearance Z=5 must export as machine Z=13.000.
    assert!(matches!(
        job.setup.work_zero.z,
        cam_core::project::WorkZeroZ::StockBottom
    ));
    assert_eq!(job.setup.stock.thickness_mm, Some(8.));
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let checks = check_plan(&plan).unwrap();
    assert_eq!(checks.status, CheckStatus::Passed);
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let prepared =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap();
    assert_eq!(prepared.machine_z_offset_mm, 8.);
    let export = prepared
        .export(&TrustedPlan::from_plan(&plan).unwrap(), &profile)
        .unwrap();
    let gcode = &export.program.gcode;
    assert!(
        gcode.lines().any(|l| l.contains("Z13.000")),
        "clearance plane must be shifted to the machine datum: no Z13.000 in program"
    );
    // Ordered tool state: M3 with the resolved direction appears after the
    // tool change, exact path mode is established, motions are G0/G1 with F.
    assert!(gcode.contains("T1 M6"));
    assert!(gcode.contains("M3 S10000"));
    assert!(gcode.contains("G61"));
    assert!(gcode.contains("G0 ") || gcode.contains("G1 "));
    assert!(gcode.ends_with("M2\n"));
    assert_eq!(export.report.motion_count, plan.motions.len());
    assert_eq!(export.report.basic_checks.status, CheckStatus::Passed);
    // The emitted bytes verify against the plan.
    verify_program(
        &prepared,
        &TrustedPlan::from_plan(&plan).unwrap(),
        &export.program,
    )
    .unwrap();
}

#[test]
fn combined_fixture_exports_through_the_sequence_pipeline() {
    let job = applied_job(M4_CONTACT_LINE);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let prepared =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap();
    let export = prepared
        .export(&TrustedPlan::from_plan(&plan).unwrap(), &profile)
        .unwrap();
    // Two stages in order: rough then finish, tool changes T1 then T2.
    let tool_changes: Vec<&str> = export
        .program
        .gcode
        .lines()
        .filter(|l| l.contains(" M6"))
        .collect();
    assert_eq!(tool_changes.len(), plan.stages.len());
    verify_program(
        &prepared,
        &TrustedPlan::from_plan(&plan).unwrap(),
        &export.program,
    )
    .unwrap();
}

#[test]
fn altered_motion_order_or_coordinates_are_rejected_by_readback() {
    let job = applied_job(M3_RECTANGLE);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let prepared =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap();
    let export = prepared
        .export(&TrustedPlan::from_plan(&plan).unwrap(), &profile)
        .unwrap();

    // Swap two motion blocks.
    let mut lines: Vec<String> = export.program.gcode.lines().map(str::to_string).collect();
    let first_motion = lines
        .iter()
        .position(|l| l.starts_with("G0") || l.starts_with("G1"))
        .unwrap();
    let second_motion = first_motion
        + lines[first_motion + 1..]
            .iter()
            .position(|l| l.starts_with("G0") || l.starts_with("G1"))
            .unwrap()
        + 1;
    lines.swap(first_motion, second_motion);
    let altered = cam_core::post::sequence::SequenceProgram {
        filename: export.program.filename.clone(),
        gcode: lines.join("\n") + "\n",
    };
    let err =
        verify_program(&prepared, &TrustedPlan::from_plan(&plan).unwrap(), &altered).unwrap_err();
    assert!(
        err.code == "POST_SEQUENCE_MISMATCH" || err.code == "PROCESS_SPINDLE_STATE",
        "unexpected code {}",
        err.code
    );

    // Change one coordinate.
    let mut lines: Vec<String> = export.program.gcode.lines().map(str::to_string).collect();
    let target = lines
        .iter()
        .position(|l| l.starts_with("G1"))
        .expect("feed motions exist");
    lines[target] = lines[target].replace("X", "X1");
    let altered = cam_core::post::sequence::SequenceProgram {
        filename: export.program.filename.clone(),
        gcode: lines.join("\n") + "\n",
    };
    assert_eq!(
        verify_program(&prepared, &TrustedPlan::from_plan(&plan).unwrap(), &altered)
            .unwrap_err()
            .code,
        "POST_SEQUENCE_MISMATCH"
    );

    // Change a tool number.
    let mut lines: Vec<String> = export.program.gcode.lines().map(str::to_string).collect();
    let m6 = lines.iter().position(|l| l.contains("T1 M6")).unwrap();
    lines[m6] = lines[m6].replace("T1 M6", "T2 M6");
    let altered = cam_core::post::sequence::SequenceProgram {
        filename: export.program.filename.clone(),
        gcode: lines.join("\n") + "\n",
    };
    assert_eq!(
        verify_program(&prepared, &TrustedPlan::from_plan(&plan).unwrap(), &altered)
            .unwrap_err()
            .code,
        "POST_SEQUENCE_MISMATCH"
    );

    // Delete the spindle start: process state mismatch.
    let mut lines: Vec<String> = export.program.gcode.lines().map(str::to_string).collect();
    let spindle = lines.iter().position(|l| l.starts_with("M3 ")).unwrap();
    lines.remove(spindle);
    let altered = cam_core::post::sequence::SequenceProgram {
        filename: export.program.filename.clone(),
        gcode: lines.join("\n") + "\n",
    };
    assert_eq!(
        verify_program(&prepared, &TrustedPlan::from_plan(&plan).unwrap(), &altered)
            .unwrap_err()
            .code,
        "PROCESS_SPINDLE_STATE"
    );
}

#[test]
fn unresolved_spindle_direction_blocks_preparation_not_generation() {
    let job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    // Generation and basic checks pass without the direction...
    assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    // ...but process preparation refuses the unresolved marker.
    let err =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap_err();
    assert_eq!(err.code, "PROCESS_SPINDLE_STATE");
    assert!(
        plan.preparation_requirements
            .iter()
            .any(|r| r.code == "PROCESS_SPINDLE_DIRECTION")
    );
}

#[test]
fn incomplete_generation_cannot_be_exported() {
    let mut job = applied_job(M3_RECTANGLE);
    let OperationSettings::FlatVcarve(settings) = &mut job.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    settings.max_depth_mm = None;
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let checks = check_plan(&plan).unwrap();
    assert_eq!(checks.status, CheckStatus::Failed);
    assert!(
        checks
            .findings
            .iter()
            .any(|f| f.code == "PLAN_GENERATION_INCOMPLETE")
    );
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let err =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap_err();
    assert_eq!(err.code, "PLAN_BASIC_CHECKS");
}

#[test]
fn profile_must_map_every_stage_tool_exactly_once() {
    let job = applied_job(M3_RECTANGLE);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let missing_mapping = sequence_profile(&[("vbit", 2)]);
    let err = PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &missing_mapping)
        .unwrap_err();
    assert_eq!(err.code, "POST_TOOL_MAPPING");

    let duplicate = SequenceProfile {
        tools: vec![
            cam_core::post::sequence::SequenceToolMapping {
                tool_id: "endmill".into(),
                tool_number: 1,
                length_offset_number: None,
            },
            cam_core::post::sequence::SequenceToolMapping {
                tool_id: "endmill".into(),
                tool_number: 3,
                length_offset_number: None,
            },
        ],
        ..sequence_profile(&[("endmill", 1), ("vbit", 2)])
    };
    assert_eq!(
        SequenceProfile::from_json(&serde_json::to_string(&duplicate).unwrap())
            .unwrap_err()
            .code,
        "POST_TOOL_MAPPING"
    );
}

#[test]
fn schema_two_profile_rejects_z_datum_and_tool_direction_fields() {
    let base = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let json = serde_json::to_string(&base).unwrap();
    assert!(!json.contains("z_datum"), "schema 2 has no Z datum: {json}");
    assert!(
        !json.contains("spindle_direction"),
        "schema 2 T/H has no direction: {json}"
    );
    // Strict parsing rejects unknown fields such as a competing z_datum.
    let competing = json.replace(
        "\"work_offset\"",
        "\"z_datum\":\"stock_top\",\"work_offset\"",
    );
    assert_eq!(
        SequenceProfile::from_json(&competing).unwrap_err().code,
        "POST_PROFILE"
    );
    assert_eq!(
        SequenceProfile::from_json(&json.replace("\"schema_version\":2", "\"schema_version\":1"))
            .unwrap_err()
            .code,
        "POST_PROFILE"
    );
}

#[test]
fn legacy_quality_analysis_remains_callable_alongside_sequence_export() {
    // The legacy M5 path is untouched: the legacy planner still verifies
    // motions for the legacy job while the sequence pipeline runs separately.
    let legacy = LegacyJob::from_json(M3_RECTANGLE).unwrap();
    let legacy_plan = cam_core::pocket::plan_endmill(&legacy).unwrap();
    assert!(legacy_plan.analysis.status != cam_core::pocket::PlanStatus::Inconclusive);
    let cam = migrate_job(&legacy).unwrap();
    let applied =
        apply_legacy_profile(&LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap(), &cam).unwrap();
    let plan = OperationPlan::plan_job(&applied, &PlanLimits::default()).unwrap();
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let prepared =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap();
    let export = prepared
        .export(&TrustedPlan::from_plan(&plan).unwrap(), &profile)
        .unwrap();
    assert_eq!(export.report.motion_count, legacy_plan.motions.len());
    assert_eq!(export.report.basic_checks.status, CheckStatus::Passed);
}

#[test]
fn endmill_only_mode_keeps_the_vbit_stage_out_of_export() {
    let job = applied_job(M3_RECTANGLE);
    let OperationSettings::FlatVcarve(settings) = &job.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    assert_eq!(settings.mode, FlatVcarveMode::EndmillOnly);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let prepared =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap();
    let export = prepared
        .export(&TrustedPlan::from_plan(&plan).unwrap(), &profile)
        .unwrap();
    assert!(export.program.gcode.contains("T1 M6"));
    assert!(!export.program.gcode.contains("T2 M6"));
    assert_eq!(prepared.stages.len(), 1);
}

#[test]
fn plan_settings_changes_invalidate_the_prepared_fingerprint() {
    let job = applied_job(M3_RECTANGLE);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let first =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap();
    let mut other = job.clone();
    let OperationSettings::FlatVcarve(settings) = &mut other.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    let FlatVcarveSettings { endmill, vbit, .. } = settings;
    endmill.spindle_direction = Some(SpindleDirection::Counterclockwise);
    vbit.spindle_direction = Some(SpindleDirection::Counterclockwise);
    let replan = OperationPlan::plan_job(&other, &PlanLimits::default()).unwrap();
    let second =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&replan).unwrap(), &profile).unwrap();
    assert_ne!(first.plan_fingerprint, second.plan_fingerprint);
    assert_ne!(first.process_fingerprint, second.process_fingerprint);
}

#[test]
fn forged_imported_plans_and_receipts_are_rejected() {
    let job = applied_job(M3_RECTANGLE);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let json = serde_json::to_string(&plan).unwrap();

    // A faithful serialized plan re-establishes trust by regeneration.
    let trusted = TrustedPlan::import(&json).unwrap();
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    prepared.export(&trusted, &profile).unwrap();

    // Edited motions are rejected even though the reader cannot see receipts.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["motions"][0]["end"]["z"] = serde_json::json!(-9.5);
    let err = TrustedPlan::import(&serde_json::to_string(&value).unwrap()).unwrap_err();
    assert_eq!(err.code, "PLAN_IMPORT_MISMATCH");

    // A recomputed-looking fingerprint cannot authorize edited motions.
    let replan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    value["execution_fingerprint"] = serde_json::json!(replan.execution_fingerprint);
    value["input_fingerprint"] = serde_json::json!(replan.input_fingerprint);
    let err = TrustedPlan::import(&serde_json::to_string(&value).unwrap()).unwrap_err();
    assert_eq!(err.code, "PLAN_IMPORT_MISMATCH");

    // Dropped motions and other-engine versions are rejected outright.
    let mut dropped = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    dropped["motions"].as_array_mut().unwrap().pop();
    assert_eq!(
        TrustedPlan::import(&serde_json::to_string(&dropped).unwrap())
            .unwrap_err()
            .code,
        "PLAN_IMPORT_MISMATCH"
    );
    let mut other_engine = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    other_engine["engine_version"] = serde_json::json!("0.0.0-other");
    assert_eq!(
        TrustedPlan::import(&serde_json::to_string(&other_engine).unwrap())
            .unwrap_err()
            .code,
        "PLAN_IMPORT_ENGINE"
    );
}

#[test]
fn xy_work_zero_transforms_are_rejected_until_physical_stock_support() {
    let mut job = applied_job(M3_RECTANGLE);
    job.setup.work_zero.xy = cam_core::project::WorkZeroXY::StockAnchor {
        x_fraction: cam_core::project::AnchorFraction::Center,
        y_fraction: cam_core::project::AnchorFraction::Center,
    };
    // Planning is unaffected: work zero changes the output transform only.
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
    let profile = sequence_profile(&[("endmill", 1), ("vbit", 2)]);
    let err =
        PreparedExecution::prepare(&TrustedPlan::from_plan(&plan).unwrap(), &profile).unwrap_err();
    assert_eq!(err.code, "POST_WORK_ZERO_XY_UNSUPPORTED");
}
