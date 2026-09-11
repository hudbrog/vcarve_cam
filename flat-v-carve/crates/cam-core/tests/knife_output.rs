//! F3 knife output integration: actual-byte modal decoding (F3a), the
//! independent replay of the emitted rounded program and its bounded
//! evidence (F3b), and prefix-stock contact gating (F3c). Fixtures plan
//! through the real knife planner, then exercise the export/verify path
//! against mutated bytes.
use cam_core::{
    job::{PlanningTolerances, SourceSnapshot},
    post::{
        Coolant, LengthCompensation, LinuxCncProfile, PathControl,
        sequence::{
            DecodedProgram, KnifeReplayStatus, PreparedExecution, SequenceProfile, SequenceProgram,
            SequenceToolMapping, verify_program,
        },
    },
    project::{
        CamJob, DragKnifeSettings, DragKnifeSpec, EndmillGeometry, FaceArea, FaceMargins,
        FacePattern, FaceSettings, HeightRef, HeightReference, JobTool, KnifeAlignment,
        KnifeAssignment, MillingAssignment, Operation, OperationSettings, RectXY, SetupSettings,
        SpindleDirection, StockSetup, ToolCapabilities, ToolGeometry,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits, StageRole, TrustedPlan},
    svg::{ImportMode, ImportOptions},
};
use sha2::Digest;

const ENDMILL: &str = "endmill";
const KNIFE: &str = "blade";
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 5 L25 5"/></svg>"##;

fn knife_settings() -> DragKnifeSettings {
    DragKnifeSettings {
        chains: vec!["cut-chain-0".into()],
        assignment: KnifeAssignment {
            tool_id: KNIFE.into(),
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
    }
}

fn stock() -> StockSetup {
    StockSetup {
        thickness_mm: Some(3.),
        xy: Some(RectXY {
            min_x_mm: 0.,
            min_y_mm: 0.,
            width_mm: 40.,
            length_mm: 30.,
        }),
    }
}

fn tools() -> Vec<JobTool> {
    vec![
        JobTool {
            id: ENDMILL.into(),
            name: "6 mm endmill".into(),
            geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                diameter_mm: 6.,
                cutting_length_mm: 20.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: None,
            },
        },
        JobTool {
            id: KNIFE.into(),
            name: "drag knife".into(),
            geometry: Some(ToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.,
                max_cut_depth_mm: 2.,
            })),
            capabilities: Default::default(),
        },
    ]
}

fn face_operation(id: &str, area: FaceArea, bottom_offset: f64) -> Operation {
    Operation {
        id: id.into(),
        name: id.into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area,
            margins: FaceMargins::default(),
            entry_overrun_mm: Some(1.),
            exit_overrun_mm: Some(1.),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: bottom_offset,
            },
            stepdown_mm: Some(1.),
            stepover_mm: Some(3.),
            pass_angle_deg: Some(0.),
            pattern: FacePattern::ZigZag,
            assignment: MillingAssignment {
                tool_id: ENDMILL.into(),
                spindle_rpm: Some(10_000.),
                spindle_direction: Some(SpindleDirection::Clockwise),
                cutting_feed_mm_min: Some(300.),
                plunge_feed_mm_min: Some(100.),
                max_stepdown_mm: Some(3.),
                stepover_mm: Some(3.),
            },
        }),
    }
}

fn knife_operation(top: HeightReference) -> Operation {
    let mut settings = knife_settings();
    settings.top = HeightRef {
        reference: top,
        offset_mm: 0.,
    };
    Operation {
        id: "knife-1".into(),
        name: "knife-1".into(),
        enabled: true,
        settings: OperationSettings::DragKnife(settings),
    }
}

fn base_job(operations: Vec<Operation>) -> CamJob {
    let job = CamJob {
        schema_version: 4,
        name: "knife-output".into(),
        source: Some(SourceSnapshot {
            filename: "cut.svg".into(),
            svg: SVG.into(),
        }),
        import: ImportOptions {
            mode: ImportMode::Centerline,
            ..Default::default()
        },
        setup: SetupSettings {
            stock: stock(),
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: tools(),
        operations,
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: None,
        },
        legacy_machine_profile: None,
    };
    job.validate().unwrap();
    job
}

/// Knife-only on intact stock.
fn knife_only_job() -> CamJob {
    base_job(vec![knife_operation(HeightReference::StockTop)])
}

/// Supported mixed program: the whole stock is faced 0.5 mm and the knife
/// cuts below the published plane.
fn face_then_knife_job() -> CamJob {
    base_job(vec![
        face_operation("face-1", FaceArea::EntireStock, -0.5),
        knife_operation(HeightReference::FaceResult {
            operation_id: "face-1".into(),
        }),
    ])
}

/// Cleared air: a deep partial face removes the chain's material above the
/// knife's contact depth while the knife still references the stock top.
fn cleared_contact_job() -> CamJob {
    base_job(vec![
        face_operation(
            "face-1",
            FaceArea::Rectangle {
                rect: RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                },
            },
            -3.,
        ),
        knife_operation(HeightReference::StockTop),
    ])
}

fn profile(start: Option<cam_core::motion::Position>) -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "knife-output-profile".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: start,
        length_compensation: LengthCompensation::MacroManaged,
        path_control: PathControl::Blend {
            tolerance_mm: 0.01,
            naive_cam_tolerance_mm: None,
        },
        tools: vec![
            SequenceToolMapping {
                tool_id: ENDMILL.into(),
                tool_number: 1,
                length_offset_number: None,
            },
            SequenceToolMapping {
                tool_id: KNIFE.into(),
                tool_number: 3,
                length_offset_number: None,
            },
        ],
        spindle_spinup_seconds: 0.5,
        coolant: Coolant::Flood,
        m6: LinuxCncProfile::from_json(include_str!("../../../../real_data/machine-profile.json"))
            .unwrap()
            .m6,
    }
}

fn export(job: &CamJob, profile: &SequenceProfile) -> (TrustedPlan, PreparedExecution, String) {
    let plan = OperationPlan::plan_job(job, &PlanLimits::default()).unwrap();
    let trusted = TrustedPlan::from_generated(plan);
    let prepared = PreparedExecution::prepare(&trusted, profile).unwrap();
    let export = prepared.export(&trusted, profile).unwrap();
    (trusted, prepared, export.program.gcode)
}

fn verify(
    prepared: &PreparedExecution,
    trusted: &TrustedPlan,
    gcode: &str,
) -> cam_core::geometry::Result<cam_core::post::sequence::SequenceExportReport> {
    verify_program(
        prepared,
        trusted,
        &SequenceProgram {
            filename: "sequence.ngc".into(),
            gcode: gcode.into(),
        },
    )
}

/// Replace `count` occurrences of `from` with `to`.
fn replace(gcode: &str, from: &str, to: &str, count: usize) -> String {
    gcode.replacen(from, to, count)
}

/// Drop the first line that contains `needle`.
fn drop_line(gcode: &str, needle: &str) -> String {
    let mut lines: Vec<&str> = gcode.lines().collect();
    let index = lines.iter().position(|l| l.contains(needle)).expect("line");
    lines.remove(index);
    lines.join("\n") + "\n"
}

/// Duplicate the first line that contains `needle`.
fn duplicate_line(gcode: &str, needle: &str) -> String {
    let mut lines: Vec<&str> = gcode.lines().collect();
    let index = lines.iter().position(|l| l.contains(needle)).expect("line");
    let line = lines[index];
    lines.insert(index + 1, line);
    lines.join("\n") + "\n"
}

/// Insert a raw line right before the first knife motion block.
fn inject_before_knife_motion(gcode: &str, line: &str) -> String {
    let mut lines: Vec<String> = gcode.lines().map(str::to_owned).collect();
    let knife_comment = lines
        .iter()
        .position(|l| l.contains("CAM Stage knife"))
        .expect("stage comment");
    let motion_at = (knife_comment + 1..lines.len())
        .find(|i| lines[*i].starts_with("G0 ") || lines[*i].starts_with("G1 "))
        .expect("knife motion");
    lines.insert(motion_at, line.into());
    lines.join("\n") + "\n"
}

/// The index of the first line of the knife stage's own modal group
/// (the second `G21 G17 …` block, after the knife tool change).
fn knife_modal_line(gcode: &str) -> usize {
    let lines: Vec<&str> = gcode.lines().collect();
    let m6s: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains(" M6"))
        .map(|(i, _)| i)
        .collect();
    let knife_m6 = *m6s
        .iter()
        .find(|i| lines[**i].contains("T3"))
        .expect("knife M6");
    (knife_m6 + 1..lines.len())
        .find(|i| lines[*i].starts_with("G21 "))
        .expect("knife modal line")
}

#[test]
fn knife_only_export_decodes_and_replays_within_budget() {
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    // The knife stage re-establishes exact path under a blending profile.
    let knife_modal = knife_modal_line(&gcode);
    assert!(gcode.lines().nth(knife_modal).unwrap().contains("G61"));
    // The emitted bytes decode with every modal group and verify exactly.
    let decoded = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    assert_eq!(decoded.motions.len(), trusted.plan().motions.len());
    assert_eq!(decoded.tool_changes.len(), 1);
    assert_eq!(decoded.tool_changes[0].0, 3);
    verify(&prepared, &trusted, &gcode).unwrap();
    // Every knife motion decodes under exact path with spindle and coolant
    // off and the prepared work frame.
    for motion in &decoded.motions {
        assert!(matches!(
            motion.path_control,
            cam_core::post::sequence::DecodedPathControl::ExactPath
        ));
        assert!(matches!(
            motion.spindle,
            cam_core::post::sequence::DecodedSpindle::Off
        ));
        assert_eq!(motion.work_offset, "G54");
        assert!(motion.line > 0);
    }
}

#[test]
fn emitted_replay_gates_the_export_report() {
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    // The precision-increase mechanism ran until the replayed rounded
    // output stayed inside the tip budget; the report records the outcome.
    let report = verify(&prepared, &trusted, &gcode).unwrap();
    // verify_program does not rerun the replay; the export report does.
    let export_again = prepared.export(&trusted, &profile).unwrap();
    let replay = export_again.report.knife_replay.expect("knife stages");
    assert_eq!(replay.status, KnifeReplayStatus::Within);
    assert!(replay.max_tip_deviation_mm <= replay.tip_budget_mm);
    assert!(report.motion_count > 0);
}

#[test]
fn evidence_report_binds_program_hash_and_reports_bounded_traces() {
    use cam_core::operations::drag_knife::evidence::{KNIFE_EVIDENCE_MAX_SAMPLES, build_evidence};
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    let decoded = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    let sha = format!("{:x}", sha2::Sha256::digest(gcode.as_bytes()));
    let report = build_evidence(trusted.plan(), &prepared, &decoded, &sha, 10_000).unwrap();
    assert_eq!(report.artifact_kind, "knife_evidence");
    assert_eq!(report.schema_version, 1);
    assert_eq!(report.program_sha256, sha);
    assert_eq!(
        report.execution_fingerprint,
        trusted.plan().execution_fingerprint
    );
    assert_eq!(
        report.status,
        cam_core::operations::drag_knife::evidence::EvidenceStatus::Within
    );
    assert!(report.max_tip_deviation_mm <= report.tip_budget_mm);
    assert_eq!(report.initial_heading_deg, 90.);
    assert_eq!(report.blade_offset_mm, 1.);
    assert!(!report.samples.is_empty());
    assert_eq!(report.samples.len(), report.total_samples.min(10_000));
    assert!(!report.truncated);

    // Ownership: samples carry the plan's stage/pass identity and the
    // modeled headings of the motion they belong to.
    let plan = trusted.plan();
    for sample in &report.samples {
        let motion = &plan.motions[sample.motion_index];
        assert_eq!(sample.stage_id, motion.stage_id);
        assert_eq!(sample.operation_id, motion.operation_id);
        assert_eq!(sample.pass_id, motion.pass_id);
        if let Some((start, _)) = motion.blade_heading_deg
            && sample.at_start
        {
            assert_eq!(sample.modeled_heading_deg, Some(start));
        }
        // Pivot/tip contract (plan section 22.10): tip = q + d*u(theta),
        // degrees CCW from +X pointing pivot toward tip. Contact samples
        // carry the integrated heading; lifted samples before the first
        // contact legitimately have none.
        if let Some(heading) = sample.replayed_heading_deg {
            let derived =
                cam_core::toolpath::knife_tip(sample.pivot_mm, heading, report.blade_offset_mm);
            assert!(derived.distance(sample.replayed_tip_mm) < 1e-9);
        } else {
            assert!(
                sample.deviation_mm.is_none(),
                "lifted samples have no deviation"
            );
        }
    }
    // A planted sample reconstructs the intended tip from the modeled pair:
    // the pivot/tip numbers of plan section 19.1.
    let planted = report
        .samples
        .iter()
        .find(|s| {
            s.intended_tip_mm
                .as_ref()
                .is_some_and(|(_, end)| end.is_none() && s.deviation_mm.is_some())
        })
        .expect("a planted contact sample");
    let (tip, _) = planted.intended_tip_mm.unwrap();
    let derived = cam_core::toolpath::knife_tip(
        planted.pivot_mm,
        planted.modeled_heading_deg.unwrap(),
        report.blade_offset_mm,
    );
    assert!(derived.distance(tip) < 1e-9);

    // The sampling budget bounds detail, never the outcome.
    let small = build_evidence(trusted.plan(), &prepared, &decoded, &sha, 1).unwrap();
    assert!(small.truncated);
    assert_eq!(small.samples.len(), 1);
    assert_eq!(small.total_samples, report.total_samples);
    assert_eq!(small.max_tip_deviation_mm, report.max_tip_deviation_mm);
    assert!(small.sample_budget <= KNIFE_EVIDENCE_MAX_SAMPLES);
}

#[test]
fn mutation_fixtures_cannot_evade_output_checks() {
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    let modal = knife_modal_line(&gcode);
    let modal_text = gcode.lines().nth(modal).unwrap().to_string();

    let cases: Vec<(String, &str)> = vec![
        // Remove the knife stage's exact-path word: its motions would run
        // under the profile's G64 blend inherited from the header.
        (
            {
                let mutated = modal_text.replace(" G61", "");
                replace(&gcode, &modal_text, &mutated, 1)
            },
            "POST_PATH_CONTROL_STATE",
        ),
        // Exact stop is a different machine mode than exact path.
        (
            replace(&gcode, &modal_text, &modal_text.replace("G61", "G61.1"), 1),
            "POST_PATH_CONTROL_STATE",
        ),
        // Substituted blend tolerance in place of the knife's exact path.
        (
            replace(&gcode, &modal_text, "G21 G17 G90 G94 G40 G80 G64 P0.01", 1),
            "POST_PATH_CONTROL_STATE",
        ),
        // Removing the initial modal word leaves every motion without a
        // path-control mode at all (knife-only: the header word is the
        // profile's blend; the stage word is its exact path).
        (
            gcode.replacen(" G64 P0.01", "", 1).replacen(" G61", "", 1),
            "POST_MODAL_STATE",
        ),
        // Unrecognized/mode-changing blocks are rejected, not skipped.
        (
            inject_before_knife_motion(&gcode, "G20"),
            "POST_MODAL_STATE",
        ),
        (
            inject_before_knife_motion(&gcode, "G91"),
            "POST_MODAL_STATE",
        ),
        (
            inject_before_knife_motion(&gcode, "G41"),
            "POST_MODAL_STATE",
        ),
        (
            inject_before_knife_motion(&gcode, "G28.1"),
            "POST_MODAL_STATE",
        ),
        (
            inject_before_knife_motion(&gcode, "M30"),
            "POST_GCODE_SUBSET",
        ),
        // A different work frame than the prepared one. Every work-offset
        // selection is replaced: the writer re-establishes the frame per
        // stage, and only a selection that actually governs a motion can be
        // caught (an intermediate G55 immediately superseded before any
        // motion is semantically invisible, and correctly stays accepted).
        (
            replace(&gcode, "\nG54\n", "\nG55\n", usize::MAX),
            "POST_MODAL_STATE",
        ),
        // Spindle/coolant words during knife cuts.
        (
            inject_before_knife_motion(&gcode, "M3 S1000"),
            "PROCESS_SPINDLE_STATE",
        ),
        (
            inject_before_knife_motion(&gcode, "M8"),
            "PROCESS_COOLANT_STATE",
        ),
        // A motion after the program end.
        (
            gcode.trim_end().to_string() + "\nG0 X0 Y0 Z0\nM2\n",
            "POST_GCODE_SUBSET",
        ),
        // Missing program end.
        (drop_line(&gcode, "M2"), "POST_GCODE_SUBSET"),
    ];
    for (case, (mutated, expected)) in cases.into_iter().enumerate() {
        let error = match verify(&prepared, &trusted, &mutated) {
            Err(error) => error,
            Ok(_) => panic!("mutation case {case} was accepted: {mutated}"),
        };
        let error = error.to_string();
        assert!(
            error.contains(expected),
            "expected {expected} for mutation, got: {error}"
        );
    }

    // Altered swivel geometry and deleted entries fail through the motion
    // comparison even though every other byte matches.
    let lines: Vec<&str> = gcode.lines().collect();
    let swivel = lines
        .iter()
        .position(|l| l.starts_with("G1 ") && l.contains("F50"))
        .expect("a swivel chord");
    let nudged = nudge_first_number(lines[swivel], 'X');
    assert_ne!(nudged, lines[swivel], "the chord must change");
    let mut copy = lines.clone();
    copy[swivel] = &nudged;
    let error = verify(&prepared, &trusted, &(copy.join("\n") + "\n"))
        .expect_err("altered swivel coordinate");
    assert!(error.to_string().contains("POST_SEQUENCE_MISMATCH"));

    let error =
        verify(&prepared, &trusted, &drop_line(&gcode, "F50")).expect_err("deleted swivel motion");
    assert!(error.to_string().contains("POST_SEQUENCE_MISMATCH"));
}

/// Bump the first `Axis…` word's numeric value by 0.4 at three decimals.
fn nudge_first_number(line: &str, axis: char) -> String {
    let token = line
        .split_ascii_whitespace()
        .find(|w| w.starts_with(axis))
        .expect("axis word");
    let value: f64 = token[1..].parse().unwrap();
    line.replacen(token, &format!("{axis}{:.3}", value + 0.4), 1)
}

#[test]
fn bridge_blocks_are_accounted_exactly() {
    // A declared machine start position produces concrete bridge blocks the
    // decoder consumes one-for-one; deleting or duplicating one is caught.
    let profile = profile(Some(cam_core::motion::Position::new(
        cam_core::geometry::Point::new(0., 0.),
        50.,
    )));
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    let decoded = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    assert!(
        decoded.bridges.len() >= 2,
        "the declared start above clearance writes Z and XY bridges: {:?}",
        decoded.bridges
    );
    verify(&prepared, &trusted, &gcode).unwrap();

    // Deleting a bridge leaves the stage's positioning unaccounted for.
    let z_bridge = gcode
        .lines()
        .position(|l| l.starts_with("G0 Z"))
        .expect("Z bridge");
    let mut lines: Vec<&str> = gcode.lines().collect();
    lines.remove(z_bridge);
    let error =
        verify(&prepared, &trusted, &(lines.join("\n") + "\n")).expect_err("deleted bridge");
    assert!(error.to_string().contains("POST_SEQUENCE_MISMATCH"));

    // A duplicated bridge is not an excuse for intervening motion: the
    // extra block counts as a motion and breaks the comparison.
    let error = verify(&prepared, &trusted, &duplicate_line(&gcode, "G0 X"))
        .expect_err("duplicated bridge");
    assert!(error.to_string().contains("POST_SEQUENCE_MISMATCH"));
}

#[test]
fn mixed_face_then_knife_passes_and_cleared_air_is_rejected() {
    // Supported mixed program: face -> knife against the faced plane
    // completes, exports, decodes and replays.
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&face_then_knife_job(), &profile);
    let plan = trusted.plan();
    assert_eq!(plan.stages.len(), 2);
    assert_eq!(plan.stages[1].role, StageRole::Knife);
    assert_eq!(
        plan.operation_results[1].generation_status,
        GenerationStatus::Complete
    );
    let replay = prepared
        .export(&trusted, &profile)
        .unwrap()
        .report
        .knife_replay
        .expect("knife stage present");
    assert_eq!(replay.status, KnifeReplayStatus::Within);
    let decoded = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    assert_eq!(
        decoded
            .tool_changes
            .iter()
            .map(|(t, _)| *t)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    verify(&prepared, &trusted, &gcode).unwrap();
    // A milling stage under the same profile decodes as blending, and the
    // recurring tool re-establishes its own state after the knife stage.
    assert!(matches!(
        decoded.motions[0].path_control,
        cam_core::post::sequence::DecodedPathControl::Blend { .. }
    ));
    assert!(matches!(
        decoded.motions[0].spindle,
        cam_core::post::sequence::DecodedSpindle::On { .. }
    ));

    // Cleared air: the deep face removed the chain's corridor above the
    // knife's contact depth; the knife plan is incomplete with the located
    // contact rejection and cannot export.
    let job = cleared_contact_job();
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let knife = &plan.operation_results[1];
    assert_eq!(knife.generation_status, GenerationStatus::Incomplete);
    let issue = plan
        .generation_diagnostics
        .iter()
        .find(|issue| issue.code == "KNIFE_CONTACT_UNSUPPORTED")
        .expect("located contact rejection");
    assert_eq!(issue.operation_id.as_deref(), Some("knife-1"));
    assert!(issue.message.contains("face-1"));
    let trusted = TrustedPlan::from_generated(plan);
    let error = PreparedExecution::prepare(&trusted, &profile)
        .err()
        .or_else(|| prepared_export_error(&trusted, &profile));
    let error = error.expect("cleared-air knife cannot export");
    // The located generation finding blocks preparation; the wrapped
    // diagnostic keeps the plan-level message.
    assert!(
        error.message.contains("did not generate completely"),
        "unexpected error: {error:?}"
    );
}

fn prepared_export_error(
    trusted: &TrustedPlan,
    profile: &SequenceProfile,
) -> Option<cam_core::geometry::Diagnostic> {
    let prepared = PreparedExecution::prepare(trusted, profile).ok()?;
    prepared.export(trusted, profile).err()
}

#[test]
fn emitted_replay_detects_geometry_deviation_by_itself() {
    // Even a mutation that keeps the motion count identical (so the byte
    // comparison would need exact coordinates to catch it) fails the
    // independent replay when the holder polyline no longer tracks the
    // intended tip path.
    use cam_core::operations::drag_knife::evidence::{EMITTED_REPLAY_STEP_BUDGET, replay_emitted};
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    let mut decoded = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    // Shift one cutting motion's endpoints sideways by 0.5 mm and carry the
    // following motion's start along, keeping the polyline connected.
    let plan = trusted.plan();
    let index = plan
        .motions
        .iter()
        .position(|m| m.purpose == cam_core::toolpath::MotionPurpose::KnifeCut)
        .unwrap();
    decoded.motions[index].end.x += 0.5;
    if let Some(next) = decoded.motions.get_mut(index + 1)
        && let Some(start) = next.start.as_mut()
    {
        start.x += 0.5;
    }
    let replay = replay_emitted(plan, &prepared, &decoded, EMITTED_REPLAY_STEP_BUDGET).unwrap();
    assert_ne!(
        replay.outcome.status,
        cam_core::operations::drag_knife::replay::ReplayStatus::Within,
        "a 0.5 mm holder shift must fail the emitted replay"
    );
    assert!(replay.outcome.max_tip_deviation_mm > replay.tip_budget_mm);
}

#[test]
fn stock_bottom_work_zero_and_nonzero_xy_transform_round_trip() {
    // The decoded program replays against setup coordinates under the
    // stock-bottom/center-anchor output transform, and the evidence records
    // the machine offset it inverted (plan section 22.10 F3b).
    let mut job = knife_only_job();
    job.setup.work_zero.z = cam_core::project::WorkZeroZ::StockBottom;
    job.setup.work_zero.xy = cam_core::project::WorkZeroXY::StockAnchor {
        x_fraction: cam_core::project::AnchorFraction::Center,
        y_fraction: cam_core::project::AnchorFraction::Center,
    };
    job.validate().unwrap();
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&job, &profile);
    assert!((prepared.machine_offset_mm[2] - (-3.)).abs() < 1e-9);
    assert!((prepared.machine_offset_mm[0] - 20.).abs() < 1e-9);
    let decoded = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    // Setup coordinates recovered from the decoded output stay inside the
    // stock rectangle the knife planned against, to output precision (the
    // emitted values are rounded; the offset itself is exact).
    let quantum = 10f64.powi(-(prepared.output_decimal_places as i32));
    for (read, planned) in decoded.motions.iter().zip(&trusted.plan().motions) {
        let recovered_x = read.end.x + prepared.machine_offset_mm[0];
        let recovered_y = read.end.y + prepared.machine_offset_mm[1];
        assert!((recovered_x - planned.end.x).abs() < quantum);
        assert!((recovered_y - planned.end.y).abs() < quantum);
    }
    let replay = prepared
        .export(&trusted, &profile)
        .unwrap()
        .report
        .knife_replay
        .unwrap();
    assert_eq!(replay.status, KnifeReplayStatus::Within);
}

#[test]
fn decode_program_reports_source_lines_and_modal_mapping() {
    let profile = profile(None);
    let (trusted, prepared, gcode) = export(&knife_only_job(), &profile);
    let decoded: DecodedProgram = prepared
        .decode_program(trusted.plan(), prepared.output_decimal_places, &gcode)
        .unwrap();
    // Line mapping: each decoded motion names the line it came from, and
    // that line really carries its coordinates.
    for motion in &decoded.motions {
        let line = gcode.lines().nth(motion.line - 1).unwrap();
        assert!(line.starts_with("G0 ") || line.starts_with("G1 "));
        assert!(line.contains(&format!("Z{:.3}", motion.end.z)));
    }
    // Motion starts continue from the previous block; the first block after
    // the tool change starts from its bridge or is unpositioned (machine
    // boundary), never fabricated.
    assert!(decoded.motions.iter().skip(1).all(|m| m.start.is_some()));
}
