//! H3 multi-source machining and identities (plan sections 22.4 and 22.8):
//! cross-source selections resolve through the shared document resolver,
//! the Flat V-carve planner consumes a resolved union, Profile/Knife
//! selections stay per-contour across items, and the semantic machining
//! identity separates machining edits from display/provenance/output-only
//! changes. Migrated documents machine identically to their schema-4
//! originals.
use cam_core::{
    checks::{CheckStatus, check_plan_v5},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    model::VBitSpec,
    project::{
        CAM_JOB_SCHEMA_VERSION, CamJob, ContourOrder, ContourSide, CutDirection, DragKnifeSpec,
        EndmillGeometry, FaceArea, FacePattern, FlatVcarveMode, FlatVcarveSettings, HeightRef,
        HeightReference, JobTool, KnifeAlignment, KnifeAssignment, MillingAssignment, Operation,
        OperationSettings, ProfileContour, ProfileEntry, ProfileSettings, RectXY, SetupSettings,
        SpindleDirection, StockSetup, ToolCapabilities, ToolGeometry, WorkZero, WorkZeroXY,
        WorkZeroZ,
        v5::{
            self, ArtworkItemId, GeometryRef, GeometryRefKind, OperationSettingsV5, OperationV5,
            ReadinessScope,
            artwork::{inspect_artwork, parse_wire_id},
            migrate::migrate_v4,
        },
    },
    sequence::{
        GenerationStatus, OPERATION_PLAN_V5_SCHEMA_VERSION, OperationPlan, OperationPlanV5,
        PlanLimits, StageRole,
    },
    svg::{ImportMode, ImportOptions, Placement},
    toolpath::{MotionEffect, MotionPurpose},
};

/// Source 1: one filled plate plus one stroked centerline.
const PLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="16" height="10" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M25 25 L35 25"/></svg>"##;
/// Source 2: one filled word outline placed to overlap the plate. The rect
/// is 6mm tall so its exclusive placed strip admits at least the first
/// depth-dependent clearing layer of the 3mm endmill.
const WORD: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 20 10"><rect id="word" x="1" y="1" width="8" height="6" fill="#fff"/></svg>"##;
/// The plate source with a sub-fingerprint-grid content edit.
const PLATE_EDITED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="16.001" height="10" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M25 25 L35 25"/></svg>"##;
const M4_CONTACT_LINE: &str = include_str!("../../../fixtures/m4/contact-line.json");

fn tools() -> Vec<v5::JobToolV5> {
    let tool = |id: &str, name: &str, geometry, plunge, ramp| v5::JobToolV5 {
        id: id.into(),
        name: name.into(),
        geometry: Some(geometry),
        capabilities: ToolCapabilities {
            plunge_capable: plunge,
            ramp_capable: ramp,
        },
        library_origin: None,
    };
    vec![
        tool(
            "t1",
            "3mm endmill",
            ToolGeometry::Endmill(EndmillGeometry {
                diameter_mm: 3.,
                cutting_length_mm: 8.,
            }),
            Some(true),
            Some(true),
        ),
        tool(
            "t2",
            "90 degree V-bit",
            ToolGeometry::Vbit(VBitSpec {
                included_angle_deg: 90.,
                tip_diameter_mm: 0.2,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 3.,
            }),
            None,
            None,
        ),
        tool(
            "t3",
            "drag knife",
            ToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.,
                max_cut_depth_mm: 2.,
            }),
            None,
            None,
        ),
    ]
}

fn milling(tool_id: &str) -> v5::MillingAssignmentV5 {
    v5::MillingAssignmentV5 {
        tool_id: tool_id.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(1.),
        stepover_mm: Some(1.5),
        applied_profile: None,
    }
}

fn knife_assignment() -> v5::KnifeAssignmentV5 {
    v5::KnifeAssignmentV5 {
        tool_id: "t3".into(),
        cutting_feed_mm_min: Some(150.),
        plunge_feed_mm_min: Some(50.),
        swivel_feed_mm_min: Some(75.),
        max_stepdown_mm: Some(1.),
        applied_profile: None,
    }
}

fn setup() -> SetupSettings {
    SetupSettings {
        stock: StockSetup {
            thickness_mm: Some(8.),
            xy: Some(RectXY {
                min_x_mm: 0.,
                min_y_mm: 0.,
                width_mm: 40.,
                length_mm: 30.,
            }),
        },
        work_zero: WorkZero::default(),
        clearance_above_stock_mm: Some(5.),
        start_xy_mm: Some(Point::new(0., 0.)),
    }
}

fn tolerances() -> PlanningTolerances {
    PlanningTolerances {
        motion_tolerance_mm: Some(0.01),
        verification_tolerance_mm: Some(0.05),
    }
}

fn centerline_item(id: &str, name: &str, svg: &str, placement: Placement) -> v5::ArtworkItem {
    v5::ArtworkItem {
        id: ArtworkItemId(id.into()),
        name: name.into(),
        content: v5::ArtworkContent::Svg(SourceSnapshot {
            filename: format!("{name}.svg"),
            svg: svg.into(),
        }),
        import_settings: v5::SvgInterpretation {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            mode: ImportMode::Centerline,
        },
        placement,
    }
}

fn identity() -> Placement {
    Placement {
        origin_mm: Point::new(0., 0.),
        scale: 1.,
        rotation_deg: 0.,
    }
}

/// Resolve the current reference of one catalogue entry (selections bind
/// the item's live revision exactly like the H2 assignment commands).
fn reference_of(job: &v5::CamJobV5, item: &str, kind: GeometryRefKind) -> GeometryRef {
    let combined = inspect_artwork(job).unwrap();
    combined
        .item(&ArtworkItemId(item.into()))
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.kind == kind)
        .unwrap()
        .reference
        .clone()
}

fn carve_operation(components: Vec<GeometryRef>) -> OperationV5 {
    OperationV5 {
        id: "carve-1".into(),
        name: "Carve".into(),
        enabled: true,
        settings: OperationSettingsV5::FlatVcarve(v5::FlatVcarveSettingsV5 {
            components,
            mode: FlatVcarveMode::EndmillOnly,
            endmill: milling("t1"),
            vbit: milling("t2"),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            max_depth_mm: Some(1.5),
            wall_allowance_mm: Some(0.2),
            max_floor_ridge_mm: None,
            max_detail_residual_mm: None,
            rough: Some(cam_core::project::FlatVcarveRoughSettings {
                strategy: cam_core::pocket::ClearingStrategy::DepthDependent,
                entry: cam_core::pocket::EntryStrategy::Plunge,
                max_layers: 32,
                max_loops_per_layer: 512,
                max_motions: 100_000,
            }),
            finish: None,
        }),
    }
}

fn profile_operation(contours: Vec<(GeometryRef, ContourSide)>) -> OperationV5 {
    OperationV5 {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettingsV5::Profile(v5::ProfileSettingsV5 {
            contours: contours
                .into_iter()
                .map(|(geometry, side)| v5::ProfileContourV5 {
                    geometry,
                    side,
                    traversal: None,
                })
                .collect(),
            assignment: milling("t1"),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::OperationTop,
                offset_mm: -2.,
            },
            stepdown_mm: Some(2.),
            through_cut_allowance_mm: None,
            direction: Some(CutDirection::Climb),
            order: ContourOrder::InnerBeforeOuter,
            start: Default::default(),
            finish: Default::default(),
            entry: ProfileEntry::Plunge,
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    }
}

fn knife_operation(chains: Vec<GeometryRef>) -> OperationV5 {
    OperationV5 {
        id: "knife-1".into(),
        name: "Knife".into(),
        enabled: true,
        settings: OperationSettingsV5::DragKnife(v5::DragKnifeSettingsV5 {
            chains,
            assignment: knife_assignment(),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -1.,
            },
            stepdown_mm: Some(1.),
            swivel_depth_mm: Some(0.5),
            corner_threshold_deg: Some(90.),
            through_cut_allowance_mm: None,
            start: Default::default(),
            closure_overlap_mm: None,
            alignment: KnifeAlignment {
                initial_heading_deg: Some(180.),
            },
        }),
    }
}

/// The plate at identity (placed region (2,18)-(18,28)) plus the word
/// outline translated so its placed region (14,20)-(22,26) overlaps it.
fn two_source_job(operations: Vec<OperationV5>) -> v5::CamJobV5 {
    v5::CamJobV5 {
        schema_version: v5::CAM_JOB_V5_SCHEMA_VERSION,
        name: "collection".into(),
        setup: setup(),
        artwork: vec![
            centerline_item("plate", "Plate", PLATE, identity()),
            centerline_item(
                "word",
                "Lettering",
                WORD,
                Placement {
                    origin_mm: Point::new(-13., -17.),
                    scale: 1.,
                    rotation_deg: 0.,
                },
            ),
        ],
        tools: tools(),
        operations,
        tolerances: tolerances(),
        machine_configuration: None,
        legacy_machine_profile: None,
    }
}

/// Normalize a collection plan's motion contour IDs (owner-qualified wire
/// IDs) down to their local IDs so migrated outputs compare against the
/// schema-4 plan field by field.
fn normalized_motions(plan: &OperationPlanV5) -> Vec<cam_core::toolpath::PlannedMotion> {
    plan.motions
        .iter()
        .map(|motion| {
            let mut motion = motion.clone();
            if let Some(wire) = &motion.contour_id
                && let Some(pick) = parse_wire_id(wire)
            {
                motion.contour_id = Some(pick.local_geometry_id);
            }
            motion
        })
        .collect()
}

#[test]
fn migrated_job_plans_identically_through_both_models() {
    // A schema-4 job with every operation kind (face is exercised through
    // the profile's face-referenced top), planned directly and through the
    // migrated collection document: identical motions, stages and execution.
    let v4 = |operations: Vec<Operation>| CamJob {
        schema_version: CAM_JOB_SCHEMA_VERSION,
        name: "single source".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: PLATE.into(),
        }),
        import: ImportOptions {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            placement: identity(),
            mode: ImportMode::Centerline,
        },
        setup: setup(),
        tools: vec![
            JobTool {
                id: "t1".into(),
                name: "3mm endmill".into(),
                geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                    diameter_mm: 3.,
                    cutting_length_mm: 8.,
                })),
                capabilities: ToolCapabilities {
                    plunge_capable: Some(true),
                    ramp_capable: Some(true),
                },
            },
            JobTool {
                id: "t2".into(),
                name: "90 degree V-bit".into(),
                geometry: Some(ToolGeometry::Vbit(VBitSpec {
                    included_angle_deg: 90.,
                    tip_diameter_mm: 0.2,
                    max_cutting_diameter_mm: 12.,
                    cutting_height_mm: 3.,
                })),
                capabilities: ToolCapabilities::default(),
            },
            JobTool {
                id: "t3".into(),
                name: "drag knife".into(),
                geometry: Some(ToolGeometry::DragKnife(DragKnifeSpec {
                    blade_offset_mm: 1.,
                    max_cut_depth_mm: 2.,
                })),
                capabilities: ToolCapabilities::default(),
            },
        ],
        operations,
        tolerances: tolerances(),
        legacy_machine_profile: None,
    };
    let bare = v4(vec![]);
    let catalogue = cam_core::contours::ContourCatalogue::build(&bare).unwrap();
    let contour = catalogue.contours[0].id.clone();
    let chain = catalogue.open_chains[0].id.clone();
    let v4_milling = |tool_id: &str| MillingAssignment {
        tool_id: tool_id.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(1.),
        stepover_mm: Some(1.5),
    };
    let operations = vec![
        Operation {
            id: "profile-1".into(),
            name: "Profile".into(),
            enabled: true,
            settings: OperationSettings::Profile(ProfileSettings {
                contours: vec![ProfileContour {
                    contour_id: contour.clone(),
                    side: ContourSide::Outside,
                    traversal: None,
                }],
                assignment: v4_milling("t1"),
                top: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::OperationTop,
                    offset_mm: -2.,
                },
                stepdown_mm: Some(2.),
                through_cut_allowance_mm: None,
                direction: Some(CutDirection::Climb),
                order: ContourOrder::InnerBeforeOuter,
                start: Default::default(),
                finish: Default::default(),
                entry: ProfileEntry::Plunge,
                lead_in: Default::default(),
                lead_out: Default::default(),
                tabs: None,
            }),
        },
        Operation {
            id: "carve-1".into(),
            name: "Carve".into(),
            enabled: true,
            settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
                component_ids: vec!["plate::0".into()],
                mode: FlatVcarveMode::EndmillOnly,
                endmill: v4_milling("t1"),
                vbit: v4_milling("t2"),
                top: Default::default(),
                max_depth_mm: Some(1.5),
                wall_allowance_mm: Some(0.2),
                max_floor_ridge_mm: None,
                max_detail_residual_mm: None,
                rough: Some(cam_core::project::FlatVcarveRoughSettings {
                    strategy: cam_core::pocket::ClearingStrategy::DepthDependent,
                    entry: cam_core::pocket::EntryStrategy::Plunge,
                    max_layers: 32,
                    max_loops_per_layer: 512,
                    max_motions: 100_000,
                }),
                finish: None,
            }),
        },
        Operation {
            id: "knife-1".into(),
            name: "Knife".into(),
            enabled: true,
            settings: OperationSettings::DragKnife(cam_core::project::DragKnifeSettings {
                chains: vec![chain],
                assignment: KnifeAssignment {
                    tool_id: "t3".into(),
                    cutting_feed_mm_min: Some(150.),
                    plunge_feed_mm_min: Some(50.),
                    swivel_feed_mm_min: Some(75.),
                    max_stepdown_mm: Some(1.),
                },
                top: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: -1.,
                },
                stepdown_mm: Some(1.),
                swivel_depth_mm: Some(0.5),
                corner_threshold_deg: Some(90.),
                through_cut_allowance_mm: None,
                start: Default::default(),
                closure_overlap_mm: None,
                alignment: KnifeAlignment {
                    initial_heading_deg: Some(180.),
                },
            }),
        },
    ];
    let source = v4(operations);
    let v4_plan = OperationPlan::plan_job(&source, &PlanLimits::default()).unwrap();
    assert_eq!(v4_plan.operation_results.len(), 3);
    assert!(
        v4_plan
            .operation_results
            .iter()
            .all(|r| r.generation_status == GenerationStatus::Complete)
    );

    let v5_job = migrate_v4(&source).unwrap();
    let v5_plan =
        OperationPlanV5::plan_job_v5(&v5_job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert_eq!(v5_plan.schema_version, OPERATION_PLAN_V5_SCHEMA_VERSION);
    assert_eq!(v5_plan.operation_results.len(), 3);
    assert!(
        v5_plan
            .operation_results
            .iter()
            .all(|r| r.generation_status == GenerationStatus::Complete)
    );
    // Identical machining: motions (wire IDs normalized to the local IDs the
    // schema-4 plan carries), stages and execution item for item.
    assert_eq!(normalized_motions(&v5_plan), v4_plan.motions);
    assert_eq!(v5_plan.stages, v4_plan.stages);
    assert_eq!(v5_plan.execution, v4_plan.execution);
    for (v5, v4) in v5_plan
        .operation_results
        .iter()
        .zip(&v4_plan.operation_results)
    {
        assert_eq!(v5.operation_id, v4.operation_id);
        assert_eq!(v5.generation_status, v4.generation_status);
        assert_eq!(v5.stage_ids, v4.stage_ids);
    }
    assert_eq!(check_plan_v5(&v5_plan).unwrap().status, CheckStatus::Passed);
}

#[test]
fn real_combined_fixture_plans_identically_after_migration() {
    let legacy = cam_core::job::Job::from_json(M4_CONTACT_LINE).unwrap();
    let v4 = cam_core::project::migrate::migrate_job(&legacy).unwrap();
    let v4_plan = OperationPlan::plan_job(&v4, &PlanLimits::default()).unwrap();
    let v5_job = migrate_v4(&v4).unwrap();
    let v5_plan =
        OperationPlanV5::plan_job_v5(&v5_job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert!(
        v5_plan
            .operation_results
            .iter()
            .all(|r| r.generation_status == GenerationStatus::Complete)
    );
    assert_eq!(normalized_motions(&v5_plan), v4_plan.motions);
    assert_eq!(v5_plan.stages, v4_plan.stages);
    assert_eq!(v5_plan.execution, v4_plan.execution);
}

#[test]
fn cross_source_carve_unions_selected_regions() {
    // One carve selecting the plate component from item 1 and the word
    // component from item 2: the union spans both placed regions including
    // their overlap, and material outside both stays untouched.
    let mut job = two_source_job(vec![]);
    job.operations = vec![carve_operation(vec![
        reference_of(&job, "plate", GeometryRefKind::FilledComponent),
        reference_of(&job, "word", GeometryRefKind::FilledComponent),
    ])];
    let job = job;
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert_eq!(check_plan_v5(&plan).unwrap().status, CheckStatus::Passed);
    let history = plan.stock_history(None).unwrap();
    // Inside the plate only (8,21): carved to the full depth.
    assert!((history.material_top_at(Point::new(8., 21.)).unwrap() + 1.5).abs() < 0.05);
    // Inside the word only (20,24): the union clears item 2's exclusive
    // strip to the first depth-dependent layer (the narrow strip cannot
    // admit the full-depth layer a wide region can).
    assert!(history.material_top_at(Point::new(20., 24.)).unwrap() < -0.3);
    // Inside the overlap (16,24): cleared once to the plate floor, not
    // machined twice over.
    assert!((history.material_top_at(Point::new(16., 24.)).unwrap() + 1.5).abs() < 0.05);
    // Outside both regions: untouched stock top.
    assert!((history.material_top_at(Point::new(30., 6.)).unwrap() - 0.).abs() < 1e-9);
    // Selecting only the plate carves strictly less: the union adds cutting
    // work over the word's placed area.
    let mut plate_only = job.clone();
    plate_only.artwork.truncate(1);
    plate_only.operations = vec![carve_operation(vec![reference_of(
        &plate_only,
        "plate",
        GeometryRefKind::FilledComponent,
    )])];
    let plate_plan = OperationPlanV5::plan_job_v5(
        &plate_only,
        &ReadinessScope::AllEnabled,
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plate_plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert!(plate_plan.motions.len() < plan.motions.len());
    assert!(
        plan.motions
            .iter()
            .any(|m| m.effect == MotionEffect::MillingSweep && m.start.x > 18.5),
        "union cutting reaches the word item's placed area"
    );
}

#[test]
fn mixed_profile_and_knife_selections_span_sources() {
    // One profile selecting contours from both items (outside on the plate,
    // inside on the word), one knife following the plate's centerline, plus
    // a face establishing the plane the profile tops refer to.
    let mut job = two_source_job(vec![]);
    job.operations = vec![
        OperationV5 {
            id: "face-1".into(),
            name: "Face".into(),
            enabled: true,
            settings: OperationSettingsV5::Face(v5::FaceSettingsV5 {
                area: FaceArea::EntireStock,
                margins: Default::default(),
                entry_overrun_mm: Some(2.),
                exit_overrun_mm: Some(2.),
                top: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: -0.5,
                },
                stepdown_mm: Some(0.5),
                stepover_mm: Some(2.),
                pass_angle_deg: Some(0.),
                pattern: FacePattern::ZigZag,
                assignment: milling("t1"),
            }),
        },
        {
            let mut operation = profile_operation(vec![
                (
                    reference_of(&job, "plate", GeometryRefKind::ClosedContour),
                    ContourSide::Outside,
                ),
                (
                    reference_of(&job, "word", GeometryRefKind::ClosedContour),
                    ContourSide::Inside,
                ),
            ]);
            operation.settings = match operation.settings {
                OperationSettingsV5::Profile(mut s) => {
                    s.top = HeightRef {
                        reference: HeightReference::FaceResult {
                            operation_id: "face-1".into(),
                        },
                        offset_mm: 0.,
                    };
                    OperationSettingsV5::Profile(s)
                }
                other => other,
            };
            operation
        },
        knife_operation(vec![reference_of(
            &job,
            "plate",
            GeometryRefKind::Centerline,
        )]),
    ];
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert_eq!(plan.operation_results.len(), 3);
    assert!(
        plan.operation_results
            .iter()
            .all(|r| r.generation_status == GenerationStatus::Complete),
        "all operations complete: {:?}",
        plan.generation_diagnostics
    );
    assert_eq!(check_plan_v5(&plan).unwrap().status, CheckStatus::Passed);
    // The profile's rough passes hold both contours' compensated loops: the
    // plate outer dilated by the 1.5mm cutter radius reaches x < 0.5 inside
    // stock, and the word's inside cut passes through its placed interior.
    let profile_motions: Vec<_> = plan
        .motions
        .iter()
        .filter(|m| m.operation_id == "profile-1" && m.purpose == MotionPurpose::Rough)
        .collect();
    assert!(
        profile_motions
            .iter()
            .any(|m| { m.start.x.min(m.end.x) <= 0.5 + 1e-6 && m.start.x >= 0.4 })
    );
    let word_interior = profile_motions
        .iter()
        .filter(|m| m.start.x > 14. && m.start.x < 22. && m.start.y > 22. && m.start.y < 26.)
        .count();
    assert!(word_interior > 0, "the word contour cuts inside its ring");
    // The knife follows the plate's placed centerline: cutting XY along
    // y=4 between x=20 and x=30 (identity placement), spindle off.
    let knife_motions: Vec<_> = plan
        .motions
        .iter()
        .filter(|m| m.operation_id == "knife-1" && m.purpose == MotionPurpose::KnifeCut)
        .collect();
    assert!(!knife_motions.is_empty());
    for motion in &knife_motions {
        let y = (motion.start.y + motion.end.y) / 2.;
        assert!((y - 5.).abs() < 1e-6, "knife rides the placed chain at y=5");
    }
    assert_eq!(
        plan.stages.iter().map(|s| s.role).collect::<Vec<_>>(),
        vec![StageRole::Face, StageRole::ProfileRough, StageRole::Knife]
    );
}

#[test]
fn machining_identity_separates_display_from_machining_changes() {
    let base = two_source_job(vec![
        profile_operation(vec![(
            GeometryRef {
                artwork_item_id: ArtworkItemId("plate".into()),
                kind: GeometryRefKind::ClosedContour,
                local_geometry_id: "plate-outer".into(),
                source_revision: v5::SourceRevision {
                    content_digest: "placeholder".into(),
                    algorithm_version: 1,
                },
            },
            ContourSide::Outside,
        )]),
        knife_operation(vec![]),
    ]);
    // Bind live references (the placeholder revisions above only shape the
    // operations; real planning binds the item revisions).
    let mut base = base;
    rebind_live(&mut base);
    let base_plan =
        OperationPlanV5::plan_job_v5(&base, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    let base_identity =
        OperationPlanV5::machining_identity(&base, &ReadinessScope::AllEnabled).unwrap();
    assert_eq!(base_plan.machining_identity, base_identity);
    let assert_current = |mut job: v5::CamJobV5| {
        rebind_live(&mut job);
        let id = OperationPlanV5::machining_identity(&job, &ReadinessScope::AllEnabled).unwrap();
        assert_eq!(id, base_identity);
        let plan =
            OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
                .unwrap();
        assert_eq!(plan.execution_fingerprint, base_plan.execution_fingerprint);
    };
    let assert_stale = |mut job: v5::CamJobV5| {
        rebind_live(&mut job);
        let id = OperationPlanV5::machining_identity(&job, &ReadinessScope::AllEnabled).unwrap();
        assert_ne!(id, base_identity);
    };

    // Display-only edits keep machining current: rename the job, an item,
    // an operation and a tool.
    let mut renamed = base.clone();
    renamed.name = "renamed".into();
    renamed.artwork[0].name = "Renamed plate".into();
    renamed.tools[0].name = "renamed tool".into();
    renamed.operations[0].name = "renamed operation".into();
    assert_current(renamed);

    // Artwork row order and an unreferenced item/tool never enter machining.
    let mut reordered = base.clone();
    reordered.artwork.swap(0, 1);
    assert_current(reordered);
    let mut unreferenced = base.clone();
    unreferenced
        .artwork
        .push(centerline_item("spare", "Spare", WORD, identity()));
    unreferenced.tools.push(v5::JobToolV5 {
        id: "t9".into(),
        name: "unused".into(),
        geometry: None,
        capabilities: ToolCapabilities::default(),
        library_origin: None,
    });
    assert_current(unreferenced);

    // Provenance (library origin, applied profile baseline) is metadata.
    let mut provenance = base.clone();
    provenance.tools[0].library_origin = Some(v5::LibraryOrigin {
        library_id: "lib".into(),
        tool_id: "x".into(),
        copied_revision: 3,
        name_at_copy: "at copy".into(),
    });
    if let OperationSettingsV5::Profile(s) = &mut provenance.operations[0].settings {
        s.assignment.applied_profile = Some(v5::AppliedProfile {
            library_id: "lib".into(),
            library_tool_id: "t1".into(),
            preset_id: "p".into(),
            revision_at_application: 3,
            name_at_application: "roughing".into(),
            baseline: v5::CuttingBaseline::Milling {
                spindle_rpm: Some(10_000.),
                cutting_feed_mm_min: Some(300.),
                plunge_feed_mm_min: Some(100.),
                max_stepdown_mm: Some(1.),
                stepover_mm: Some(1.5),
            },
        });
    }
    assert_current(provenance);

    // Work zero changes output coordinates only, never machining.
    let mut moved_zero = base.clone();
    moved_zero.setup.work_zero = WorkZero {
        xy: WorkZeroXY::CustomPoint {
            x_mm: 20.,
            y_mm: 15.,
        },
        z: WorkZeroZ::StockBottom,
    };
    assert_current(moved_zero);

    // Machining edits stale the identity: a used item's placement, its
    // content, an effective assignment value, operation order.
    let mut placed = base.clone();
    placed.artwork[0].placement = Placement {
        origin_mm: Point::new(-1., 0.),
        scale: 1.,
        rotation_deg: 0.,
    };
    assert_stale(placed);
    let mut edited = base.clone();
    edited.artwork[0].content = v5::ArtworkContent::Svg(SourceSnapshot {
        filename: "plate.svg".into(),
        svg: PLATE_EDITED.into(),
    });
    assert_stale(edited);
    let mut feed = base.clone();
    if let OperationSettingsV5::Profile(s) = &mut feed.operations[0].settings {
        s.assignment.cutting_feed_mm_min = Some(999.);
    }
    assert_stale(feed);
    let mut reordered_ops = base.clone();
    reordered_ops.operations.reverse();
    assert_stale(reordered_ops);
    let mut disabled = base.clone();
    disabled.operations[1].enabled = false;
    assert_stale(disabled);
}

/// Rebind every operation's references to the owning items' live revisions
/// (the same binding the H2 assignment commands produce).
fn rebind_live(job: &mut v5::CamJobV5) {
    let combined = inspect_artwork(job).unwrap();
    let lookup = |item: &ArtworkItemId, kind: GeometryRefKind| -> Option<GeometryRef> {
        combined
            .item(item)?
            .entries
            .iter()
            .find(|entry| entry.kind == kind)
            .map(|entry| entry.reference.clone())
    };
    for operation in &mut job.operations {
        match &mut operation.settings {
            OperationSettingsV5::FlatVcarve(s) => {
                for component in &mut s.components {
                    *component = lookup(&component.artwork_item_id.clone(), component.kind)
                        .expect("referenced item resolves");
                }
            }
            OperationSettingsV5::Profile(s) => {
                for contour in &mut s.contours {
                    contour.geometry = lookup(
                        &contour.geometry.artwork_item_id.clone(),
                        contour.geometry.kind,
                    )
                    .expect("referenced item resolves");
                }
            }
            OperationSettingsV5::DragKnife(s) => {
                for chain in &mut s.chains {
                    *chain = lookup(&chain.artwork_item_id.clone(), chain.kind)
                        .expect("referenced item resolves");
                }
            }
            OperationSettingsV5::Face(_) => {}
        }
    }
}

#[test]
fn prefix_dependencies_follow_used_source_edits_only() {
    let mut job = two_source_job(vec![
        profile_operation(vec![(
            GeometryRef {
                artwork_item_id: ArtworkItemId("plate".into()),
                kind: GeometryRefKind::ClosedContour,
                local_geometry_id: "plate-outer".into(),
                source_revision: v5::SourceRevision {
                    content_digest: "placeholder".into(),
                    algorithm_version: 1,
                },
            },
            ContourSide::Outside,
        )]),
        knife_operation(vec![GeometryRef {
            artwork_item_id: ArtworkItemId("plate".into()),
            kind: GeometryRefKind::Centerline,
            local_geometry_id: "cut".into(),
            source_revision: v5::SourceRevision {
                content_digest: "placeholder".into(),
                algorithm_version: 1,
            },
        }]),
    ]);
    rebind_live(&mut job);
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    let before: Vec<_> = plan
        .operation_results
        .iter()
        .map(|r| (r.stock_before_id.clone(), r.stock_after_id.clone()))
        .collect();
    // A rename leaves every prefix identity in place.
    let mut renamed = job.clone();
    renamed.artwork[0].name = "renamed".into();
    let renamed_plan = OperationPlanV5::plan_job_v5(
        &renamed,
        &ReadinessScope::AllEnabled,
        &PlanLimits::default(),
    )
    .unwrap();
    for (a, b) in plan
        .operation_results
        .iter()
        .zip(&renamed_plan.operation_results)
    {
        assert_eq!(a.stock_before_id, b.stock_before_id);
        assert_eq!(a.stock_after_id, b.stock_after_id);
    }
    // Editing the used source stales the suffix prefixes (the operation's
    // own before-id and every later one).
    let mut edited = job.clone();
    edited.artwork[0].content = v5::ArtworkContent::Svg(SourceSnapshot {
        filename: "plate.svg".into(),
        svg: PLATE_EDITED.into(),
    });
    // The stale revision blocks exactly the referencing operations.
    let edited_plan =
        OperationPlanV5::plan_job_v5(&edited, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    for (original, edited_result) in plan
        .operation_results
        .iter()
        .zip(&edited_plan.operation_results)
    {
        assert_ne!(original.stock_after_id, edited_result.stock_after_id);
        assert_ne!(original.stock_before_id, edited_result.stock_before_id);
    }
    assert_eq!(before.len(), 2);
}

#[test]
fn stale_revision_blocks_only_the_referencing_operations() {
    let mut job = two_source_job(vec![
        OperationV5 {
            id: "face-1".into(),
            name: "Face".into(),
            enabled: true,
            settings: OperationSettingsV5::Face(v5::FaceSettingsV5 {
                area: FaceArea::Rectangle {
                    rect: RectXY {
                        min_x_mm: 0.,
                        min_y_mm: 0.,
                        width_mm: 10.,
                        length_mm: 10.,
                    },
                },
                margins: Default::default(),
                entry_overrun_mm: None,
                exit_overrun_mm: None,
                top: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: -0.5,
                },
                stepdown_mm: Some(0.5),
                stepover_mm: Some(2.),
                pass_angle_deg: Some(0.),
                pattern: FacePattern::ZigZag,
                assignment: milling("t1"),
            }),
        },
        {
            let mut operation = profile_operation(vec![(
                GeometryRef {
                    artwork_item_id: ArtworkItemId("plate".into()),
                    kind: GeometryRefKind::ClosedContour,
                    local_geometry_id: "plate-outer".into(),
                    source_revision: v5::SourceRevision {
                        content_digest: "placeholder".into(),
                        algorithm_version: 1,
                    },
                },
                ContourSide::Outside,
            )]);
            operation.id = "profile-1".into();
            operation
        },
    ]);
    rebind_live(&mut job);
    // Replace the plate content: references go stale, the face stays ready.
    job.artwork[0].content = v5::ArtworkContent::Svg(SourceSnapshot {
        filename: "plate.svg".into(),
        svg: PLATE_EDITED.into(),
    });
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "the face never touches artwork"
    );
    assert_eq!(
        plan.operation_results[1].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|issue| issue.code == "ARTWORK_REVISION_MISMATCH"
                && issue.operation_id.as_deref() == Some("profile-1"))
    );
    // Basic checks fail for the sequence: export is unavailable.
    let report = check_plan_v5(&plan).unwrap();
    assert_eq!(report.status, CheckStatus::Failed);
    assert!(!report.export_ready);
    // The prefix ending before the break still plans completely.
    let prefix = OperationPlanV5::plan_job_v5(
        &job,
        &ReadinessScope::ThroughOperation {
            operation_id: "face-1".into(),
        },
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(prefix.operation_results.len(), 1);
    assert_eq!(
        prefix.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert_eq!(check_plan_v5(&prefix).unwrap().status, CheckStatus::Passed);
}

#[test]
fn precision_gate_rejects_incompatible_coarser_items() {
    let mut job = two_source_job(vec![carve_operation(vec![])]);
    job.artwork[1].import_settings.geometry_tolerance_mm = 0.5;
    job.operations = vec![carve_operation(vec![
        reference_of(&job, "plate", GeometryRefKind::FilledComponent),
        reference_of(&job, "word", GeometryRefKind::FilledComponent),
    ])];
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|issue| issue.code == "ARTWORK_PRECISION_INCOMPATIBLE"
                && issue.message.contains("word")),
        "the coarse item is named: {:?}",
        plan.generation_diagnostics
    );
    // Selecting only the fine item keeps planning: the gate applies to
    // coarser partners in a mixed selection, not to fine artwork.
    job.operations = vec![carve_operation(vec![reference_of(
        &job,
        "plate",
        GeometryRefKind::FilledComponent,
    )])];
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
}

#[test]
fn face_referenced_cross_source_carve_admits_the_union_bounds() {
    let mut job = two_source_job(vec![]);
    let mut carve = carve_operation(vec![
        reference_of(&job, "plate", GeometryRefKind::FilledComponent),
        reference_of(&job, "word", GeometryRefKind::FilledComponent),
    ]);
    if let OperationSettingsV5::FlatVcarve(s) = &mut carve.settings {
        s.top = HeightRef {
            reference: HeightReference::FaceResult {
                operation_id: "face-1".into(),
            },
            offset_mm: 0.,
        };
    }
    job.operations = vec![
        OperationV5 {
            id: "face-1".into(),
            name: "Face".into(),
            enabled: true,
            settings: OperationSettingsV5::Face(v5::FaceSettingsV5 {
                area: FaceArea::Rectangle {
                    rect: RectXY {
                        min_x_mm: 0.,
                        min_y_mm: 15.,
                        width_mm: 30.,
                        length_mm: 15.,
                    },
                },
                margins: Default::default(),
                entry_overrun_mm: Some(2.),
                exit_overrun_mm: Some(2.),
                top: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: -0.5,
                },
                stepdown_mm: Some(0.5),
                stepover_mm: Some(2.),
                pass_angle_deg: Some(0.),
                pattern: FacePattern::ZigZag,
                assignment: milling("t1"),
            }),
        },
        carve,
    ];
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    assert!(
        plan.operation_results
            .iter()
            .all(|r| r.generation_status == GenerationStatus::Complete),
        "{:?}",
        plan.generation_diagnostics
    );
    // The deepest cut lies 1.5mm below the faced plane (setup Z = -2.0).
    let deepest = plan
        .motions
        .iter()
        .filter(|m| m.operation_id == "carve-1")
        .map(|m| m.start.z.min(m.end.z))
        .fold(f64::INFINITY, f64::min);
    assert!((deepest + 2.0).abs() < 0.05);
    // Crossing the faced coverage with the union is rejected: shrink the
    // face so the word item lies outside it.
    let mut crossing = job.clone();
    if let OperationSettingsV5::Face(s) = &mut crossing.operations[0].settings {
        s.area = FaceArea::Rectangle {
            rect: RectXY {
                min_x_mm: 0.,
                min_y_mm: 15.,
                width_mm: 5.,
                length_mm: 5.,
            },
        };
    }
    let crossing_plan = OperationPlanV5::plan_job_v5(
        &crossing,
        &ReadinessScope::AllEnabled,
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        crossing_plan.operation_results[1].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        crossing_plan
            .generation_diagnostics
            .iter()
            .any(|issue| issue.code == "SURFACE_REFERENCE_OUTSIDE_COVERAGE")
    );
}
