use cam_core::{
    job::Job,
    post::{
        LengthCompensation, LinuxCncProfile, M6Return, Program, ProgramLayout, export_plan,
        verify_programs,
    },
    vcarve::{CombinedPlan, plan_combined},
    verification::{VerificationOptions, VerificationStatus},
};
use std::sync::OnceLock;

fn plan() -> &'static CombinedPlan {
    static PLAN: OnceLock<CombinedPlan> = OnceLock::new();
    PLAN.get_or_init(|| {
        let mut job =
            Job::from_json(include_str!("../../../fixtures/m4/narrow-channel.json")).unwrap();
        job.source.svg = job.source.svg.replace("M0 0h3v20h-3z", "M0 0h12v10h-12z");
        plan_combined(&job).unwrap()
    })
}
fn profile() -> LinuxCncProfile {
    LinuxCncProfile::from_json(include_str!("../../../fixtures/m6/macro-stock-bottom.json"))
        .unwrap()
}
fn options() -> VerificationOptions {
    VerificationOptions::default()
}
fn baseline() -> &'static Vec<Program> {
    static PROGRAMS: OnceLock<Vec<Program>> = OnceLock::new();
    PROGRAMS.get_or_init(|| {
        let result = export_plan(plan(), &profile(), ProgramLayout::Combined, &options()).unwrap();
        assert_eq!(
            result.report.status,
            VerificationStatus::Passed,
            "{:?}",
            result.report.diagnostics
        );
        result.programs
    })
}
fn rejected(programs: &[Program]) {
    let report = verify_programs(
        plan(),
        &profile(),
        ProgramLayout::Combined,
        &options(),
        programs,
    )
    .unwrap();
    assert_eq!(report.status, VerificationStatus::Failed);
    assert!(!report.diagnostics.is_empty());
}

#[test]
fn macro_tlo_and_stock_bottom_translate_and_verify_every_output_motion() {
    let report = verify_programs(
        plan(),
        &profile(),
        ProgramLayout::Combined,
        &options(),
        baseline(),
    )
    .unwrap();
    assert_eq!(report.status, VerificationStatus::Passed);
    assert_eq!(report.machine_z_offset_mm, 8.);
    assert_eq!(
        report.programs[0].motion_count,
        plan().endmill.motions.len() + plan().vbit_motions.len()
    );
    assert_eq!(report.programs[0].tool_changes, 2);
    assert!(report.emitted_motion_fingerprint.is_some());
    assert_eq!(
        report.emitted_verification.unwrap().status,
        VerificationStatus::Passed
    );
    let text = &baseline()[0].gcode;
    assert!(!text.contains("G43"));
    assert!(!text.contains("G49"));
    assert!(!text.contains("G53"));
    assert_eq!(
        text.matches("G0 Z150.000000\nG0 X0.000000 Y0.000000 Z150.000000\nG0 Z13.000000")
            .count(),
        2
    );
    assert!(text.contains("T1 M6"));
    assert!(text.contains("T2 M6"));
    // Stock top is Z8 and the two-millimeter depth cap is Z6, not Z-2.
    for word in text
        .lines()
        .filter(|s| s.starts_with("G1 "))
        .flat_map(str::split_whitespace)
        .filter(|s| s.starts_with('Z'))
    {
        assert!(word[1..].parse::<f64>().unwrap() >= 6.);
    }
}

#[test]
fn post_managed_offsets_and_nondefault_cutting_state_are_restored() {
    let p = LinuxCncProfile::from_json(include_str!(
        "../../../fixtures/m6/tool-table-synthetic.json"
    ))
    .unwrap();
    let result = export_plan(plan(), &p, ProgramLayout::Combined, &options()).unwrap();
    assert_eq!(
        result.report.status,
        VerificationStatus::Passed,
        "{:?}",
        result.report.diagnostics
    );
    let text = &result.programs[0].gcode;
    assert!(text.contains("G43 H11\nG0 Z5.000000"));
    assert!(text.contains("G43 H12\nG0 Z5.000000"));
    assert!(text.contains("G97 S12000\nM4\nG4 P1.5\nM8"));
    assert_eq!(text.matches("G55\n").count(), 3);
    assert!(!text.contains("G49"));
}

#[test]
fn per_tool_files_have_independent_setup_and_explicit_stock_history() {
    let result = export_plan(plan(), &profile(), ProgramLayout::PerTool, &options()).unwrap();
    assert_eq!(
        result.report.status,
        VerificationStatus::Passed,
        "{:?}",
        result.report.diagnostics
    );
    assert_eq!(result.programs.len(), 2);
    assert!(result.report.programs[0].prerequisites.is_empty());
    assert_eq!(result.report.programs[1].prerequisites.len(), 1);
    for program in &result.programs {
        assert_eq!(program.gcode.matches(" M6\n").count(), 1);
        assert_eq!(
            program.gcode.matches("G21 G17 G90 G94 G40 G80 G61").count(),
            2
        );
        assert!(program.gcode.ends_with("M5\nM9\nM2\n"));
    }
    let mut swapped = result.programs.clone();
    swapped.swap(0, 1);
    assert_eq!(
        verify_programs(
            plan(),
            &profile(),
            ProgramLayout::PerTool,
            &options(),
            &swapped
        )
        .unwrap()
        .status,
        VerificationStatus::Failed
    );
    let mut omitted = result.programs;
    omitted.pop();
    assert_eq!(
        verify_programs(
            plan(),
            &profile(),
            ProgramLayout::PerTool,
            &options(),
            &omitted
        )
        .unwrap()
        .status,
        VerificationStatus::Failed
    );
}

#[test]
fn altered_modes_tool_selection_compensation_and_spindle_cannot_pass_readback() {
    for (old, new) in [
        ("G90", "G91"),
        ("G21", "G20"),
        ("G61", "G64"),
        ("G94", "G93"),
        ("G54\n", "G55\n"),
        ("T2 M6", "T1 M6"),
        ("G97 S12000", "G97 S100"),
        ("M3\n", "M4\n"),
        ("G92.1\n", "G49\n"),
        ("G4 P0", "G4 P5"),
    ] {
        let mut changed = baseline().clone();
        assert!(changed[0].gcode.contains(old));
        changed[0].gcode = changed[0].gcode.replacen(old, new, 1);
        rejected(&changed);
    }
}

#[test]
fn omitted_post_m6_setup_and_unsafe_retract_sequences_are_rejected() {
    for (old, new) in [
        (
            "T2 M6\nM5\nM9\nG21 G17 G90 G94 G40 G80 G61\n",
            "T2 M6\nM5\nM9\n",
        ),
        ("G0 Z150.000000\n", ""),
        ("G0 Z150.000000\n", "G0 Z1.000000\n"),
        (
            "G0 X0.000000 Y0.000000 Z150.000000\n",
            "G0 X0.000000 Y0.000000 Z1.000000\n",
        ),
    ] {
        let mut changed = baseline().clone();
        changed[0].gcode = changed[0].gcode.replacen(old, new, 1);
        rejected(&changed);
    }
}

#[test]
fn altered_numeric_path_feed_or_program_tail_is_rejected() {
    let mut changed = baseline().clone();
    let line = changed[0]
        .gcode
        .lines()
        .find(|s| s.starts_with("G1 "))
        .unwrap()
        .to_owned();
    let modified = line
        .split_whitespace()
        .map(|w| if w.starts_with('X') { "X999" } else { w })
        .collect::<Vec<_>>()
        .join(" ");
    changed[0].gcode = changed[0].gcode.replacen(&line, &modified, 1);
    rejected(&changed);
    let mut changed = baseline().clone();
    changed[0].gcode = changed[0].gcode.replacen(" F100", " F101", 1);
    rejected(&changed);
    for suffix in [
        "G0 X0 Y0 Z0\n",
        "#1=1\n",
        "(MSG, ignored?)\n",
        "G2 X0 Y0 I1 J0\n",
    ] {
        let mut changed = baseline().clone();
        changed[0].gcode.push_str(suffix);
        rejected(&changed);
    }
}

#[test]
fn comments_are_not_motion_authority_and_no_cached_pass_is_trusted() {
    let mut programs = baseline().clone();
    programs[0].gcode = programs[0]
        .gcode
        .lines()
        .filter(|s| !s.starts_with('('))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        verify_programs(
            plan(),
            &profile(),
            ProgramLayout::Combined,
            &options(),
            &programs
        )
        .unwrap()
        .status,
        VerificationStatus::Passed
    );
    let mut altered = plan().clone();
    altered.vbit_motions[3].end.z -= 0.2;
    assert!(export_plan(&altered, &profile(), ProgramLayout::Combined, &options()).is_err());
}

#[test]
fn one_to_one_tool_numbers_accept_job_ids_and_report_each_mapping_problem() {
    let job = Job::from_json(include_str!("../../../fixtures/m4/narrow-channel.json")).unwrap();
    let mut p = profile();
    assert_eq!(p.tools[0].tool_number, 1);
    assert_eq!(p.tools[1].tool_number, 2);
    p.validate(&job).unwrap();
    p.tools.swap(0, 1);
    p.validate(&job).unwrap();

    let check = |p: &LinuxCncProfile, message: &str| {
        let diagnostic = p.validate(&job).unwrap_err();
        assert_eq!(diagnostic.code, "POST_TOOL_MAPPING");
        assert!(
            diagnostic.message.contains(message),
            "{}",
            diagnostic.message
        );
    };
    let mut p = profile();
    p.tools[0].tool_id = "1".into();
    check(&p, "select endmill ID \"endmill\" or V-bit ID \"vbit\"");
    let mut p = profile();
    p.tools[1].tool_id = p.tools[0].tool_id.clone();
    check(&p, "job tool ID \"endmill\" is mapped more than once");
    let mut p = profile();
    p.tools[1].tool_number = 1;
    check(&p, "LinuxCNC T1 is assigned to more than one job tool");
    for number in [0, 100000] {
        let mut p = profile();
        p.tools[0].tool_number = number;
        check(&p, "LinuxCNC T number must be between 1 and 99999");
    }
    let mut p = profile();
    p.tools[0].length_offset_number = Some(1);
    check(
        &p,
        "macro-managed compensation requires length_offset_number to be null",
    );
    p.length_compensation = LengthCompensation::ToolTable;
    for h in [None, Some(0), Some(100000)] {
        p.tools[1].length_offset_number = h;
        check(
            &p,
            "job tool \"vbit\" (T2): tool-table compensation requires an H number",
        );
    }
    p.tools[1].length_offset_number = Some(2);
    p.validate(&job).unwrap();
}

#[test]
fn profile_contracts_mapping_clearance_and_numeric_precision_are_required() {
    let job = &plan().endmill.job;
    let mut p = profile();
    p.m6.reviewed = false;
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.m6.local_offsets_unused = false;
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.tools[1].tool_number = 1;
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.tools[0].length_offset_number = Some(1);
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.clearance_z_mm = 10.;
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.decimal_places = 10;
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.m6.return_position = M6Return::CallerPosition;
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.work_offset = "G54\nG0 Z-10".into();
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.m6.return_position = M6Return::SafeRetract {
        z_mm: 10.,
        transit_xy_mm: cam_core::geometry::Point::new(0., 0.),
    };
    assert!(p.validate(job).is_err());
    let mut p = profile();
    p.decimal_places = 3;
    let mut job = job.clone();
    job.stock.thickness_mm = Some(8.0001);
    assert!(p.validate(&job).is_err());
}

#[test]
fn resource_exhaustion_blocks_output_and_precision_increases_preserve_motions() {
    let result = export_plan(
        plan(),
        &profile(),
        ProgramLayout::Combined,
        &VerificationOptions {
            max_cells: 1,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(result.report.status, VerificationStatus::Inconclusive);
    assert!(result.programs.is_empty());
    let mut p = profile();
    p.decimal_places = 0;
    let result = export_plan(plan(), &p, ProgramLayout::Combined, &options()).unwrap();
    assert_eq!(result.report.status, VerificationStatus::Passed);
    assert!(result.report.output_decimal_places > p.decimal_places);
    assert_eq!(result.report.profile.decimal_places, 0);
    assert_eq!(
        result.report.diagnostics[0].code,
        "POST_PRECISION_INCREASED"
    );
    assert_eq!(
        result.report.programs[0].motion_count,
        plan().endmill.motions.len() + plan().vbit_motions.len()
    );
    assert_eq!(
        verify_programs(
            plan(),
            &p,
            ProgramLayout::Combined,
            &options(),
            &result.programs
        )
        .unwrap()
        .status,
        VerificationStatus::Passed
    );
}

#[test]
fn retained_export_matches_full_replay_and_rejects_changed_artifacts() {
    use cam_core::vcarve::{export_retained_plan, plan_combined_with_receipt};
    for name in ["narrow-channel", "finite-tip", "resource-limit"] {
        let job = Job::from_json(
            &std::fs::read_to_string(format!("../../fixtures/m4/{name}.json")).unwrap(),
        )
        .unwrap();
        let (plan, receipt) = plan_combined_with_receipt(&job).unwrap();
        let json = plan.to_json().unwrap();
        let opts = if name == "resource-limit" {
            VerificationOptions {
                max_cells: 1,
                ..options()
            }
        } else {
            options()
        };
        for layout in [ProgramLayout::Combined, ProgramLayout::PerTool] {
            let retained =
                export_retained_plan(json.as_bytes(), &receipt, &profile(), layout, &opts).unwrap();
            let replayed = export_plan(&plan, &profile(), layout, &opts).unwrap();
            assert_eq!(
                serde_json::to_value(&retained.report).unwrap(),
                serde_json::to_value(&replayed.report).unwrap(),
                "{name}"
            );
            assert_eq!(
                serde_json::to_value(&retained.programs).unwrap(),
                serde_json::to_value(&replayed.programs).unwrap()
            );
        }
        let original: serde_json::Value = serde_json::from_str(&json).unwrap();
        for pointer in [
            "/endmill/job/tools/1/spindle_rpm",
            "/vbit_motions/0/end/x",
            "/executions/0/pass_depth_mm",
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = serde_json::json!(99.);
            assert_eq!(
                export_retained_plan(
                    serde_json::to_vec(&changed).unwrap().as_slice(),
                    &receipt,
                    &profile(),
                    ProgramLayout::Combined,
                    &options()
                )
                .unwrap_err()
                .code,
                "STALE_PLAN"
            );
        }
    }
}

#[test]
fn empty_endmill_stage_is_omitted_without_inventing_a_tool_change() {
    let job = Job::from_json(include_str!("../../../fixtures/m4/narrow-channel.json")).unwrap();
    let plan = plan_combined(&job).unwrap();
    assert!(plan.endmill.motions.is_empty());
    let result = export_plan(&plan, &profile(), ProgramLayout::PerTool, &options()).unwrap();
    assert_eq!(
        result.report.status,
        VerificationStatus::Passed,
        "{:?}",
        result.report.diagnostics
    );
    assert_eq!(result.programs.len(), 1);
    assert_eq!(result.programs[0].filename, "vbit.ngc");
    assert!(result.report.programs[0].prerequisites.is_empty());
    assert!(!result.programs[0].gcode.contains("T1 M6"));
}

#[test]
fn fractional_stock_datum_and_clearance_are_decoded_before_stock_verification() {
    let mut job = plan().endmill.job.clone();
    job.stock.thickness_mm = Some(8.2);
    job.endmill_planning.as_mut().unwrap().clearance_z_mm = 1.1;
    let plan = plan_combined(&job).unwrap();
    let mut p = profile();
    p.clearance_z_mm = 1.1;
    let result = export_plan(&plan, &p, ProgramLayout::Combined, &options()).unwrap();
    assert_eq!(
        result.report.status,
        VerificationStatus::Passed,
        "{:?}",
        result.report.diagnostics
    );
    assert!(result.programs[0].gcode.contains("G0 Z9.300000"));
    assert!(result.programs[0].gcode.contains("Z8.200000 F"));
    assert_eq!(
        result.report.emitted_verification.unwrap().status,
        VerificationStatus::Passed
    );
}

#[test]
fn output_identity_is_deterministic_and_binds_the_machine_profile() {
    let first = export_plan(plan(), &profile(), ProgramLayout::Combined, &options()).unwrap();
    let second = export_plan(plan(), &profile(), ProgramLayout::Combined, &options()).unwrap();
    assert_eq!(
        serde_json::to_string(&first.report).unwrap(),
        serde_json::to_string(&second.report).unwrap()
    );
    assert_eq!(first.programs[0].gcode, second.programs[0].gcode);
    let mut changed = profile();
    changed.spindle_spinup_seconds = 2.;
    let different = export_plan(plan(), &changed, ProgramLayout::Combined, &options()).unwrap();
    assert_eq!(different.report.status, VerificationStatus::Passed);
    assert_ne!(
        first.report.profile_fingerprint,
        different.report.profile_fingerprint
    );
    assert_ne!(
        first.report.programs[0].sha256,
        different.report.programs[0].sha256
    );
}
