//! Schema-4 canonical model: structural validation and serialized-shape tests.
use cam_core::project::{
    AnchorFraction, CAM_JOB_SCHEMA_VERSION, CamJob, ContourSide, DragKnifeSettings, FaceArea,
    FaceMargins, FacePattern, FaceSettings, FlatVcarveMode, FlatVcarveRoughSettings,
    FlatVcarveSettings, HeightRef, HeightReference, JobTool, KnifeAlignment, KnifeAssignment,
    MillingAssignment, Operation, OperationSettings, ProfileContour, ProfileSettings, RectXY,
    SetupSettings, StockSetup, ToolCapabilities, ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
};
use cam_core::{geometry::Point, job::PlanningTolerances, svg::ImportOptions};

fn milling(tool_id: &str) -> MillingAssignment {
    MillingAssignment {
        tool_id: tool_id.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: None,
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(1.),
        stepover_mm: Some(1.5),
    }
}

fn face_tool() -> JobTool {
    JobTool {
        id: "t1".into(),
        name: "3mm endmill".into(),
        geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
            diameter_mm: 3.,
            cutting_length_mm: 8.,
        })),
        capabilities: ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: Some(true),
        },
    }
}

fn face_operation(id: &str, complete: bool) -> Operation {
    Operation {
        id: id.into(),
        name: "Face the stock".into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area: FaceArea::Rectangle {
                rect: RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 100.,
                    length_mm: 60.,
                },
            },
            margins: FaceMargins::default(),
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
            stepover_mm: if complete { Some(2.) } else { None },
            pass_angle_deg: Some(0.),
            pattern: FacePattern::ZigZag,
            assignment: milling("t1"),
        }),
    }
}

fn face_job(complete: bool) -> CamJob {
    CamJob {
        schema_version: CAM_JOB_SCHEMA_VERSION,
        name: "Face only".into(),
        source: None,
        import: ImportOptions::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 100.,
                    length_mm: 60.,
                }),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(-5., -5.)),
        },
        tools: vec![face_tool()],
        operations: vec![face_operation("face-1", complete)],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

#[test]
fn face_job_without_source_validates_and_round_trips() {
    let job = face_job(true);
    job.validate().unwrap();
    let json = job.to_json().unwrap();
    let reloaded = CamJob::from_json(&json).unwrap();
    assert_eq!(reloaded, job);
    // No source field is serialized at all for source-free jobs.
    assert!(!json.contains("\"source\""));
}

#[test]
fn incomplete_face_job_still_saves() {
    let job = face_job(false);
    job.validate().unwrap();
    let reloaded = CamJob::from_json(&job.to_json().unwrap()).unwrap();
    let OperationSettings::Face(settings) = &reloaded.operations[0].settings else {
        panic!("face settings expected");
    };
    assert_eq!(settings.stepover_mm, None);
}

#[test]
fn duplicate_operation_ids_are_rejected() {
    let mut job = face_job(true);
    job.operations.push(face_operation("face-1", true));
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_OPERATION_ID");
}

#[test]
fn duplicate_tool_ids_are_rejected() {
    let mut job = face_job(true);
    job.tools.push(face_tool());
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_TOOL_ID");
}

#[test]
fn unknown_tool_reference_is_rejected() {
    let mut job = face_job(true);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.assignment.tool_id = "ghost".into();
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_TOOL_REFERENCE");
    assert!(err.message.contains("face-1"), "located: {}", err.message);
    assert!(err.message.contains("ghost"), "located: {}", err.message);
}

#[test]
fn unknown_face_result_reference_is_rejected() {
    let mut job = face_job(true);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.bottom = HeightRef {
        reference: HeightReference::FaceResult {
            operation_id: "missing-face".into(),
        },
        offset_mm: 0.,
    };
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_HEIGHT_REFERENCE");
}

#[test]
fn forward_face_result_reference_is_rejected() {
    let mut job = face_job(true);
    // A second face whose top references the first face is legal.
    let mut later = face_operation("face-2", true);
    let OperationSettings::Face(later_settings) = &mut later.settings else {
        panic!("face settings expected");
    };
    later_settings.top = HeightRef {
        reference: HeightReference::FaceResult {
            operation_id: "face-1".into(),
        },
        offset_mm: 0.,
    };
    // But the first face cannot reference the second.
    let OperationSettings::Face(first) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    first.bottom = HeightRef {
        reference: HeightReference::FaceResult {
            operation_id: "face-2".into(),
        },
        offset_mm: 0.,
    };
    job.operations.push(later);
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "HEIGHT_REFERENCE_FORWARD");
}

#[test]
fn face_result_reference_to_non_face_is_rejected() {
    let mut job = face_job(true);
    let mut profile = profile_operation("profile-1");
    let OperationSettings::Profile(settings) = &mut profile.settings else {
        panic!("profile settings expected");
    };
    settings.top = HeightRef {
        reference: HeightReference::FaceResult {
            operation_id: "profile-1".into(),
        },
        offset_mm: 0.,
    };
    job.operations.push(profile);
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_HEIGHT_REFERENCE");
}

#[test]
fn operation_top_is_only_legal_for_bottoms() {
    let mut job = face_job(true);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.top = HeightRef {
        reference: HeightReference::OperationTop,
        offset_mm: 0.,
    };
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "HEIGHT_REFERENCE_INVALID");
}

fn profile_operation(id: &str) -> Operation {
    Operation {
        id: id.into(),
        name: "Cutout".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours: vec![ProfileContour {
                contour_id: "c1".into(),
                side: ContourSide::Outside,
                traversal: None,
            }],
            assignment: milling("t1"),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockBottom,
                offset_mm: 0.,
            },
            stepdown_mm: Some(2.),
            through_cut_allowance_mm: Some(0.2),
            direction: None,
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

#[test]
fn on_contour_selection_requires_explicit_traversal() {
    let mut job = face_job(true);
    let mut profile = profile_operation("profile-1");
    let OperationSettings::Profile(settings) = &mut profile.settings else {
        panic!("profile settings expected");
    };
    settings.contours[0].side = ContourSide::On;
    job.operations.push(profile);
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_PARAMETER");
    assert!(err.message.contains("traversal"));
}

#[test]
fn knife_assignment_requires_knife_geometry() {
    let mut job = face_job(true);
    job.operations.push(Operation {
        id: "knife-1".into(),
        name: "Score".into(),
        enabled: true,
        settings: OperationSettings::DragKnife(DragKnifeSettings {
            chains: vec![],
            assignment: KnifeAssignment {
                tool_id: "t1".into(), // endmill geometry, not a knife
                cutting_feed_mm_min: Some(200.),
                plunge_feed_mm_min: Some(60.),
                swivel_feed_mm_min: Some(60.),
                max_stepdown_mm: Some(0.5),
            },
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -0.5,
            },
            stepdown_mm: Some(0.5),
            swivel_depth_mm: Some(0.2),
            corner_threshold_deg: Some(30.),
            through_cut_allowance_mm: None,
            start: Default::default(),
            closure_overlap_mm: None,
            alignment: KnifeAlignment {
                initial_heading_deg: Some(0.),
            },
        }),
    });
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_TOOL_KIND");
}

#[test]
fn unknown_schema_versions_are_rejected() {
    let json = face_job(true).to_json().unwrap();
    for version in [3u64, 5] {
        let stale = json.replace(
            &format!("\"schema_version\": {CAM_JOB_SCHEMA_VERSION}"),
            &format!("\"schema_version\": {version}"),
        );
        let err = CamJob::from_json(&stale).unwrap_err();
        assert_eq!(err.code, "CAM_JOB_SCHEMA_VERSION", "version {version}");
    }
}

#[test]
fn unknown_operation_kind_and_fields_are_rejected() {
    let json = face_job(true).to_json().unwrap();
    let unknown_kind = json.replace("\"kind\": \"face\"", "\"kind\": \"pocket\"");
    assert_eq!(
        CamJob::from_json(&unknown_kind).unwrap_err().code,
        "PROJECT_JSON"
    );
    let unknown_field = json.replace(
        "\"name\": \"Face only\"",
        "\"name\": \"Face only\", \"future_field\": 1}",
    );
    assert_eq!(
        CamJob::from_json(&unknown_field).unwrap_err().code,
        "PROJECT_JSON"
    );
}

#[test]
fn work_zero_and_anchors_serialize_strictly() {
    let mut job = face_job(true);
    job.setup.work_zero = WorkZero {
        xy: WorkZeroXY::StockAnchor {
            x_fraction: AnchorFraction::Center,
            y_fraction: AnchorFraction::Max,
        },
        z: WorkZeroZ::StockBottom,
    };
    let json = job.to_json().unwrap();
    assert!(json.contains("\"x_fraction\": 0.5"), "{json}");
    assert!(json.contains("\"y_fraction\": 1.0"), "{json}");
    assert!(json.contains("\"kind\": \"stock_anchor\""), "{json}");
    assert_eq!(CamJob::from_json(&json).unwrap(), job);
    let bad_anchor = json.replace("\"y_fraction\": 1.0", "\"y_fraction\": 0.7");
    let err = CamJob::from_json(&bad_anchor).unwrap_err();
    // Nested Serde conversion failures surface as parse errors carrying the
    // underlying anchor-fraction diagnostic text.
    assert_eq!(err.code, "PROJECT_JSON");
    assert!(err.message.contains("anchor fractions"), "{}", err.message);
}

fn vbit_finish_settings() -> cam_core::vcarve::VBitPlanningSettings {
    cam_core::vcarve::VBitPlanningSettings {
        max_paths: 4096,
        max_motions: 100_000,
        max_curve_segments: 20_000,
        max_depth_passes: 8,
        max_cleanup_iterations: 2,
        quality_sample_spacing_mm: 0.5,
        max_quality_samples: 20_000,
        reachability_max_cells: 4_096,
        stock_slices: 4,
    }
}

#[test]
fn flat_vcarve_mode_and_finish_settings_must_agree() {
    let mut job = face_job(true);
    job.source = Some(cam_core::job::SourceSnapshot {
        filename: "art.svg".into(),
        svg: r#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm" viewBox="0 0 10 10"><rect id="r1" x="1" y="1" width="8" height="8"/></svg>"#.into(),
    });
    job.operations.push(Operation {
        id: "carve-1".into(),
        name: "Carve".into(),
        enabled: true,
        settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
            component_ids: vec![],
            mode: FlatVcarveMode::EndmillOnly,
            endmill: milling("t1"),
            vbit: milling("t2"),
            max_depth_mm: Some(2.),
            wall_allowance_mm: Some(0.),
            max_floor_ridge_mm: Some(0.),
            max_detail_residual_mm: Some(0.),
            rough: Some(FlatVcarveRoughSettings::default()),
            finish: Some(vbit_finish_settings()),
        }),
    });
    job.tools.push(JobTool {
        id: "t2".into(),
        name: "V-bit".into(),
        geometry: Some(ToolGeometry::Vbit(cam_core::model::VBitSpec {
            included_angle_deg: 90.,
            tip_diameter_mm: 0.2,
            max_cutting_diameter_mm: 12.,
            cutting_height_mm: 3.,
        })),
        capabilities: ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: None,
        },
    });
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_OPERATION", "endmill-only carries finish");
    let OperationSettings::FlatVcarve(settings) = &mut job.operations[1].settings else {
        panic!("flat vcarve settings expected");
    };
    settings.finish = None;
    let OperationSettings::FlatVcarve(reloaded) = &job.operations[1].settings else {
        panic!("flat vcarve settings expected");
    };
    assert_eq!(reloaded.finish, None);
    job.validate().unwrap();
    let OperationSettings::FlatVcarve(settings) = &mut job.operations[1].settings else {
        panic!("flat vcarve settings expected");
    };
    settings.mode = FlatVcarveMode::Combined;
    let err = job.validate().unwrap_err();
    assert_eq!(err.code, "PROJECT_OPERATION", "combined lacks finish");
}

#[test]
fn zero_supplied_values_are_preserved_exactly() {
    let mut job = face_job(true);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.margins.min_x_mm = Some(0.);
    settings.bottom.offset_mm = -0.;
    let reloaded = CamJob::from_json(&job.to_json().unwrap()).unwrap();
    let OperationSettings::Face(settings) = &reloaded.operations[0].settings else {
        panic!("face settings expected");
    };
    assert_eq!(settings.margins.min_x_mm, Some(0.));
}
