//! D1 drill tool model: validated `DrillSpec` geometry (tip angle, tip-length
//! math), the job/library `drill` geometry kind, and the library→job copy
//! that snapshots a drill with plunge capability defaulted on.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    model::{Depth, Drill, DrillSpec},
    project::{
        RectXY, SetupSettings, StockSetup, ToolCapabilities, ToolGeometry, WorkZero,
        v5::{self, ArtworkContent, ArtworkItemId, CamJobV5},
    },
    svg::Placement,
    tool_library::{LibraryGeometry, LibraryTool, ToolLibrary},
};
use serde_json::json;

fn spec() -> DrillSpec {
    DrillSpec {
        diameter_mm: 5.,
        tip_angle_deg: 118.,
        cutting_length_mm: 30.,
    }
}

#[test]
fn a_drill_validates_and_computes_its_tip_extension() {
    let drill = Drill::try_from(spec()).unwrap();
    // Tip extension (D/2)/tan(θ/2): 2.5/tan(59°) ≈ 1.502 mm ≈ 0.30×D at 118°.
    assert!((drill.tip_length().mm() - 2.5 / 59f64.to_radians().tan()).abs() < 1e-12);
    assert!((drill.tip_length().mm() - 1.502_152).abs() < 1e-5);
    assert_eq!(drill.radius().mm(), 2.5);
    assert_eq!(drill.angle().degrees(), 118.);
    drill.validate_depth(Depth::new(29.).unwrap()).unwrap();
    assert_eq!(
        Drill::try_from(spec())
            .unwrap()
            .validate_depth(Depth::new(31.).unwrap())
            .unwrap_err()
            .code,
        "DRILL_CUTTING_LENGTH"
    );
}

#[test]
fn drill_specs_reject_impossible_dimensions() {
    for bad in [
        DrillSpec {
            diameter_mm: 0.,
            ..spec()
        },
        DrillSpec {
            diameter_mm: -3.,
            ..spec()
        },
        DrillSpec {
            diameter_mm: f64::NAN,
            ..spec()
        },
        DrillSpec {
            cutting_length_mm: 0.,
            ..spec()
        },
    ] {
        assert_eq!(
            Drill::try_from(bad.clone()).unwrap_err().code,
            "INVALID_DRILL",
            "accepted {bad:?}"
        );
    }
}

/// The 0<angle<180 contract comes from the shared included-angle type.
fn angle_check(angle_deg: f64, code: &str) {
    let spec = DrillSpec {
        tip_angle_deg: angle_deg,
        ..spec()
    };
    assert_eq!(Drill::try_from(spec).unwrap_err().code, code);
}

#[test]
fn drill_angles_must_be_strictly_inside_the_open_interval() {
    angle_check(0., "INVALID_ANGLE");
    angle_check(180., "INVALID_ANGLE");
    angle_check(-118., "INVALID_ANGLE");
    // A denormal angle's slope underflows so the tip extension overflows.
    angle_check(1e-308, "INCONSISTENT_DRILL");
}

#[test]
fn drill_geometry_serializes_with_its_own_kind_tag() {
    let geometry = ToolGeometry::Drill(spec());
    let value = serde_json::to_value(&geometry).unwrap();
    assert_eq!(
        value,
        json!({"kind": "drill", "dimensions": {
            "diameter_mm": 5.0, "tip_angle_deg": 118.0, "cutting_length_mm": 30.0
        }})
    );
    assert_eq!(
        serde_json::from_value::<ToolGeometry>(value).unwrap(),
        geometry
    );
}

fn library() -> ToolLibrary {
    ToolLibrary {
        tools: vec![LibraryTool {
            spindle_direction: None,
            id: "drill-5".into(),
            name: "5mm twist drill".into(),
            geometry: LibraryGeometry::Drill(spec()),
            assembly: Default::default(),
            ramp_capable: None,
            plunge_capable: None,
            cutting_presets: vec![],
            knife_cutting_presets: vec![],
        }],
        ..ToolLibrary::default()
    }
}

fn job() -> CamJobV5 {
    CamJobV5 {
        schema_version: v5::CAM_JOB_V5_SCHEMA_VERSION,
        name: "drill".into(),
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
            work_zero: WorkZero::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        artwork: vec![v5::ArtworkItem {
            id: ArtworkItemId("plate".into()),
            name: "Plate".into(),
            content: ArtworkContent::Svg(SourceSnapshot {
                filename: "plate.svg".into(),
                svg: "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40mm\" height=\"30mm\" viewBox=\"0 0 40 30\"><rect x=\"2\" y=\"2\" width=\"10\" height=\"10\" fill=\"#fff\"/></svg>".into(),
            }),
            import_settings: v5::SvgInterpretation {
                geometry_tolerance_mm: 0.001,
                ticks_per_mm: None,
            },
            placement: Placement {
                origin_mm: Point::new(0., 0.),
                scale: 1.,
                rotation_deg: 0.,
            },
        }],
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
            arc_fit_tolerance_mm: None,
        },
        machine_configuration: None,
    }
}

#[test]
fn drill_library_tools_validate_round_trip_and_never_carry_knife_presets() {
    let base = library();
    base.validate().unwrap();
    let saved = base.to_json().unwrap();
    assert_eq!(
        ToolLibrary::from_json(&saved).unwrap().to_json().unwrap(),
        saved
    );

    let mut bad = library();
    let LibraryGeometry::Drill(spec) = &mut bad.tools[0].geometry else {
        unreachable!()
    };
    spec.tip_angle_deg = 0.;
    assert_eq!(bad.validate().unwrap_err().code, "INVALID_ANGLE");

    let mut knife_preset = library();
    knife_preset.tools[0]
        .knife_cutting_presets
        .push(cam_core::tool_library::KnifeCuttingPreset {
            id: "kp".into(),
            name: "Knife preset".into(),
            material: None,
            machine: None,
            cutting_feed_mm_min: None,
            plunge_feed_mm_min: None,
            swivel_feed_mm_min: None,
            max_stepdown_mm: None,
        });
    assert_eq!(
        knife_preset.validate().unwrap_err().code,
        "LIBRARY_TOOL_KIND"
    );
}

#[test]
fn adding_a_drill_library_tool_snapshots_drill_geometry_with_plunge_defaulted_on() {
    let (outcome, tool_id) =
        v5::resources::add_library_tool(&job(), &library(), "lib", "drill-5").unwrap();
    let added = outcome.job.tools.iter().find(|t| t.id == tool_id).unwrap();
    assert_eq!(added.geometry, Some(ToolGeometry::Drill(spec())));
    assert_eq!(
        added.capabilities,
        ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: None,
        }
    );
    assert_eq!(added.library_origin.as_ref().unwrap().tool_id, "drill-5");
}
