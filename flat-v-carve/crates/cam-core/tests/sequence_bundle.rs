//! H5 output bundles: sequential files split at contiguous tool stages with
//! an ordered manifest, per-file independent numeric readback, recurring
//! tools kept in execution order, and byte-identical one-program behavior
//! with the established export (plan sections 14.5 and 22.8).
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    post::{
        LinuxCncProfile,
        sequence::{
            OutputLayout, PreparedExecution, SequenceProfile, SequenceProgram, SequenceToolMapping,
        },
    },
    project::{
        AnchorFraction, CamJob, ContourSide, CutDirection, FaceArea, FaceMargins, FacePattern,
        FaceSettings, FlatVcarveMode, FlatVcarveRoughSettings, FlatVcarveSettings, HeightRef,
        HeightReference, JobTool, MillingAssignment, Operation, OperationSettings, ProfileContour,
        ProfileSettings, RectXY, SetupSettings, SpindleDirection, StockSetup, ToolCapabilities,
        ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{OperationPlan, PlanLimits, TrustedPlan},
};

const LEGACY_PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");
/// Pocket 5..35 x 5..25 on a 40x30 page at scale 1: one filled component
/// whose outer contour (`pocket-0-outer`) is the profile target.
const ART_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#;

fn tools() -> Vec<JobTool> {
    vec![
        JobTool {
            id: "endmill".into(),
            name: "4mm endmill".into(),
            geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
                diameter_mm: 4.,
                cutting_length_mm: 12.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: Some(false),
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
    ]
}

fn milling(tool: &str, feed: f64, stepover: f64) -> MillingAssignment {
    MillingAssignment {
        tool_id: tool.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(feed),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(8.),
        stepover_mm: Some(stepover),
    }
}

fn face_op(id: &str, rect: RectXY, depth: f64) -> Operation {
    Operation {
        id: id.into(),
        name: "Face".into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area: FaceArea::Rectangle { rect },
            margins: FaceMargins::default(),
            entry_overrun_mm: Some(1.),
            exit_overrun_mm: Some(1.),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -depth,
            },
            stepdown_mm: Some(depth),
            stepover_mm: Some(3.),
            pass_angle_deg: Some(0.),
            pattern: FacePattern::ZigZag,
            assignment: milling("endmill", 300., 1.5),
        }),
    }
}

fn carve_op(id: &str, face_id: Option<&str>, depth: f64) -> Operation {
    Operation {
        id: id.into(),
        name: "Carve".into(),
        enabled: true,
        settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
            component_ids: vec!["pocket::0".into()],
            mode: FlatVcarveMode::Combined,
            top: HeightRef {
                reference: match face_id {
                    Some(id) => HeightReference::FaceResult {
                        operation_id: id.into(),
                    },
                    None => HeightReference::StockTop,
                },
                offset_mm: 0.,
            },
            endmill: milling("endmill", 300., 1.5),
            vbit: milling("vbit", 250., 0.5),
            max_depth_mm: Some(depth),
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
    }
}

fn profile_op(id: &str, face_id: Option<&str>, through: Option<f64>) -> Operation {
    let top = HeightRef {
        reference: match face_id {
            Some(id) => HeightReference::FaceResult {
                operation_id: id.into(),
            },
            None => HeightReference::StockTop,
        },
        offset_mm: 0.,
    };
    Operation {
        id: id.into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours: vec![ProfileContour {
                contour_id: "pocket-0-outer".into(),
                side: ContourSide::Outside,
                traversal: None,
            }],
            assignment: milling("endmill", 350., 1.5),
            top,
            bottom: HeightRef {
                reference: HeightReference::StockBottom,
                offset_mm: -through.unwrap_or(0.),
            },
            stepdown_mm: Some(3.),
            through_cut_allowance_mm: through,
            direction: Some(CutDirection::Climb),
            order: Default::default(),
            start: Default::default(),
            finish: Default::default(),
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    }
}

/// The plan-section-19.1 mixed fixture: Face (T1) -> V-carve (T1 rough, T2
/// finish) -> Profile (T1 again), which is also the recurring-tool row of
/// the fixture matrix.
fn mixed_job() -> CamJob {
    let rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 30.,
    };
    CamJob {
        schema_version: 4,
        name: "Face, carve, profile".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: ART_SVG.into(),
        }),
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(rect),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::StockAnchor {
                    x_fraction: AnchorFraction::Center,
                    y_fraction: AnchorFraction::Center,
                },
                z: WorkZeroZ::StockBottom,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        tools: tools(),
        operations: vec![
            face_op("face-1", rect, 0.5),
            carve_op("carve-1", Some("face-1"), 1.5),
            profile_op("profile-1", Some("face-1"), Some(0.2)),
        ],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

fn sequence_profile() -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "printnc".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: cam_core::post::LengthCompensation::MacroManaged,
        path_control: cam_core::post::PathControl::ExactPath,
        tools: vec![
            SequenceToolMapping {
                tool_id: "endmill".into(),
                tool_number: 1,
                length_offset_number: None,
            },
            SequenceToolMapping {
                tool_id: "vbit".into(),
                tool_number: 2,
                length_offset_number: None,
            },
        ],
        spindle_spinup_seconds: 0.5,
        coolant: cam_core::post::Coolant::Off,
        m6: LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap().m6,
    }
}

fn mixed_plan() -> OperationPlan {
    let job = mixed_job();
    OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap()
}

#[test]
fn precision_escalation_uses_the_emitted_grid_for_independent_readback() {
    let plan = TrustedPlan::from_generated(mixed_plan());
    let mut profile = sequence_profile();
    profile.decimal_places = 0;
    let prepared = PreparedExecution::prepare(&plan, &profile).unwrap();
    for layout in [OutputLayout::OneProgram, OutputLayout::SequentialFiles] {
        let bundle = prepared.export_bundle(&plan, &profile, layout).unwrap();
        assert!(bundle.report.output_decimal_places > profile.decimal_places);
        assert_eq!(bundle.report.motion_count, plan.plan().motions.len());
        assert_eq!(
            bundle.report.output_decimal_places,
            bundle.manifest.output_decimal_places
        );
    }
}

/// The recurring-tool fixture splits at contiguous tool stages: T1,T1 share
/// a file, then T2, then T1 again — execution order preserved, nothing
/// regrouped by tool. Every file is a complete standalone program whose own
/// independent readback passes, and the files' motion spans concatenate to
/// exactly the plan's ordered stream.
#[test]
fn sequential_files_split_at_contiguous_tool_stages() {
    let plan = mixed_plan();
    let trusted = TrustedPlan::from_generated(plan.clone());
    let profile = sequence_profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let bundle = prepared
        .export_bundle(&trusted, &profile, OutputLayout::SequentialFiles)
        .unwrap();
    assert_eq!(
        bundle
            .files
            .iter()
            .map(|f| f.filename.as_str())
            .collect::<Vec<_>>(),
        vec![
            "01-face-T1.ngc",
            "02-vcarve-finish-T2.ngc",
            "03-profile-rough-T1.ngc",
        ],
        "execution order with recurring tools: {:?}",
        bundle
            .files
            .iter()
            .map(|f| f.filename.as_str())
            .collect::<Vec<_>>()
    );
    // Adjacent same-tool stages (face + carve rough) share file 1.
    assert_eq!(
        bundle.manifest.files[0].stage_ids,
        vec!["face-1-face", "carve-1-vcarve-rough"]
    );
    assert_eq!(
        bundle.manifest.files[1].stage_ids,
        vec!["carve-1-vcarve-finish"]
    );
    assert_eq!(
        bundle.manifest.files[2].stage_ids,
        vec!["profile-1-profile-rough"]
    );
    // Ordering and stock prerequisites are explicit and cumulative.
    assert!(bundle.manifest.files[0].runs_after.is_empty());
    assert_eq!(
        bundle.manifest.files[1].runs_after,
        vec!["01-face-T1.ngc".to_string()]
    );
    assert_eq!(
        bundle.manifest.files[2].runs_after,
        vec![
            "01-face-T1.ngc".to_string(),
            "02-vcarve-finish-T2.ngc".to_string()
        ]
    );
    assert_eq!(
        bundle.manifest.files[2].stock_through_operation,
        "profile-1"
    );
    // File 1 covers face-1 and carve-1's rough stage, so stock is through
    // carve-1's rough work after it (its finish stage is file 2).
    assert_eq!(bundle.manifest.files[0].stock_through_operation, "carve-1");
    // Every file is a standalone program: modal establishment, its own end,
    // and every tool change inside it selects one and the same tool (one
    // M6 group per stage, exactly as the one-program writer emits).
    for file in &bundle.files {
        assert!(file.gcode.contains("G21 G17 G90 G94 G40 G80 G61"));
        assert_eq!(file.gcode.matches("M2").count(), 1);
        let tools: Vec<_> = ["T1 M6", "T2 M6"]
            .into_iter()
            .filter(|change| file.gcode.contains(change))
            .collect();
        assert_eq!(tools.len(), 1, "one tool number per file");
    }
    // Each file independently verifies against exactly its own motion span.
    for (index, file) in bundle.files.iter().enumerate() {
        let entry = &bundle.manifest.files[index];
        let span = plan
            .stages
            .iter()
            .position(|s| s.stage_id == entry.stage_ids[0])
            .zip(
                plan.stages
                    .iter()
                    .position(|s| s.stage_id == *entry.stage_ids.last().unwrap()),
            )
            .map(|(first, last)| (first, last + 1))
            .unwrap();
        prepared
            .verify_program_span(&trusted, file, span)
            .unwrap_or_else(|e| panic!("file {index} failed its own readback: {e:?}"));
    }
    // Aggregate report binds the whole bundle to the plan's ordered stream.
    assert_eq!(bundle.report.motion_count, plan.motions.len());
    assert_eq!(
        bundle.manifest.execution_fingerprint,
        plan.execution_fingerprint
    );
    let spans: Vec<(usize, usize)> = bundle
        .manifest
        .files
        .iter()
        .map(|file| file.motion_range)
        .collect();
    assert_eq!(spans[0].0, 0);
    assert_eq!(spans[0].1, spans[1].0);
    assert_eq!(spans[1].1, spans[2].0);
    assert_eq!(spans[2].1, plan.motions.len());
}

/// The one-program layout is byte-identical with the established export:
/// same bytes, same aggregate digest and report.
#[test]
fn one_program_bundle_is_byte_identical_with_export() {
    let plan = mixed_plan();
    let trusted = TrustedPlan::from_generated(plan);
    let profile = sequence_profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();
    let bundle = prepared
        .export_bundle(&trusted, &profile, OutputLayout::OneProgram)
        .unwrap();
    assert_eq!(bundle.files.len(), 1);
    assert_eq!(bundle.files[0].filename, "sequence.ngc");
    assert_eq!(bundle.files[0].gcode, export.program.gcode);
    assert_eq!(bundle.report.program_sha256, export.report.program_sha256);
    assert_eq!(bundle.report.motion_count, export.report.motion_count);
    // The manifest still describes the single file.
    assert_eq!(bundle.manifest.files.len(), 1);
    assert_eq!(
        bundle.manifest.files[0].sha256,
        export.report.program_sha256
    );
    assert_eq!(bundle.manifest.layout, OutputLayout::OneProgram);
}

/// An edited file cannot smuggle itself through: mutating one sequential
/// file's bytes fails that file's independent numeric comparison even
/// though every other byte of the bundle matches.
#[test]
fn mutated_sequential_file_is_rejected_by_its_own_readback() {
    let plan = mixed_plan();
    let trusted = TrustedPlan::from_generated(plan);
    let profile = sequence_profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let bundle = prepared
        .export_bundle(&trusted, &profile, OutputLayout::SequentialFiles)
        .unwrap();
    // Delete one motion line from the second file: its own readback fails.
    let mutated = SequenceProgram {
        filename: bundle.files[1].filename.clone(),
        gcode: {
            let mut lines: Vec<&str> = bundle.files[1].gcode.lines().collect();
            let motion_line = lines
                .iter()
                .position(|l| l.starts_with("G1 ") || l.starts_with("G0 "))
                .expect("file has motion lines");
            lines.remove(motion_line);
            lines.join("\n") + "\n"
        },
    };
    let span = (2usize, 3usize);
    let error = prepared
        .verify_program_span(&trusted, &mutated, span)
        .unwrap_err();
    assert_eq!(error.code, "POST_SEQUENCE_MISMATCH", "{error:?}");
    // The pristine bundle still verifies — only the edited bytes fail.
    prepared
        .verify_program_span(&trusted, &bundle.files[1], span)
        .expect("pristine file verifies");
}
