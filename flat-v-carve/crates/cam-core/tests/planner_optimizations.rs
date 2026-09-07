//! Opt-in real-artwork regression for generation, authentication and output.
use cam_core::{
    job::Job,
    motion::{Motion, MotionKind},
    pocket::PlanStatus,
    post::{LinuxCncProfile, ProgramLayout, export_plan, verify_programs},
    vcarve::{CombinedPlan, plan_combined},
    verification::{VerificationOptions, VerificationStatus},
};

fn tiny(m: &Motion) -> bool {
    m.start.xy().distance(m.end.xy()).hypot(m.end.z - m.start.z) < 1e-5
}

#[test]
#[ignore = "real flower planning, saved-plan replay, and full-stock G-code verification"]
fn flower_has_no_grid_tick_moves_and_replays_and_exports() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../real_data");
    let job = Job::from_json(
        &std::fs::read_to_string(data.join("flower_box-svg.job-real.json")).unwrap(),
    )
    .unwrap();
    let plan = plan_combined(&job).unwrap();
    assert_eq!(plan.analysis.status, PlanStatus::Complete);
    assert_eq!(plan.endmill.analysis.status, PlanStatus::Complete);
    assert!(
        !plan.endmill.motions.iter().any(tiny),
        "endmill grid-tick motions remain"
    );
    assert!(
        !plan.vbit_motions.iter().any(tiny),
        "V-bit grid-tick connectors remain"
    );
    let retracts = |motions: &[Motion]| {
        motions
            .iter()
            .filter(|m| m.kind == MotionKind::RapidRetract)
            .count()
    };
    assert!(retracts(&plan.endmill.motions) < 43);
    // Bounded endmill contour simplification changes the residual-floor
    // clipping slightly. The V-bit route uses one additional retract;
    // retain a tight routing budget while independently verifying every cut.
    assert!(retracts(&plan.vbit_motions) <= 215);
    let saved = plan.to_json().unwrap();
    assert_eq!(
        plan_combined(&job).unwrap().to_json().unwrap(),
        saved,
        "fresh generation must be deterministic"
    );
    let replay = CombinedPlan::from_json(&saved).unwrap();
    assert_eq!(replay.to_json().unwrap(), saved);
    let profile = LinuxCncProfile::from_json(
        &std::fs::read_to_string(data.join("machine-profile.json")).unwrap(),
    )
    .unwrap();
    let options = VerificationOptions::default();
    let export = export_plan(&replay, &profile, ProgramLayout::Combined, &options).unwrap();
    assert_eq!(
        export.report.status,
        VerificationStatus::Passed,
        "{:?}",
        export.report.diagnostics
    );
    let readback = verify_programs(
        &replay,
        &profile,
        ProgramLayout::Combined,
        &options,
        &export.programs,
    )
    .unwrap();
    assert_eq!(
        readback.status,
        VerificationStatus::Passed,
        "{:?}",
        readback.diagnostics
    );
    eprintln!(
        "endmill: {} motions / {} retracts; V-bit: {} motions / {} retracts; output precision: {}",
        plan.endmill.motions.len(),
        retracts(&plan.endmill.motions),
        plan.vbit_motions.len(),
        retracts(&plan.vbit_motions),
        export.report.output_decimal_places
    );
    if let Some(dir) = std::env::var_os("CAM_OPTIMIZATION_OUTPUT") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("flower.plan.json"), saved).unwrap();
        std::fs::write(
            dir.join("export-report.json"),
            serde_json::to_string_pretty(&export.report).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("readback.json"),
            serde_json::to_string_pretty(&readback).unwrap(),
        )
        .unwrap();
        for p in export.programs {
            std::fs::write(dir.join(p.filename), p.gcode).unwrap();
        }
    }
}
