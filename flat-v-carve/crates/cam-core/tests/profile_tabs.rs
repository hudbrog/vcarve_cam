//! E1 tabs: protected footprints (bridge dilated by the cutter radius),
//! rectangular tabs in automatic and manual placement, tab tops measured
//! from the physical stock bottom, every deep pass retaining the bridge,
//! and located errors for impossible placements (plan sections 10.3, 19.1).
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, ContourAnchor, ContourSide, CutDirection, JobTool, MillingAssignment, Operation,
        OperationSettings, ProfileContour, ProfileSettings, RectXY, SetupSettings,
        SpindleDirection, StockSetup, TabPlacement, TabSettings, TabShape, ToolCapabilities,
        ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, NamedOutput, OperationPlan, PlanLimits},
    toolpath::MotionPurpose,
};

const RECT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#;
const CIRCLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><circle id="cut" cx="25" cy="25" r="20"/></svg>"#;

fn endmill() -> JobTool {
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
    }
}

fn assignment() -> MillingAssignment {
    MillingAssignment {
        tool_id: "endmill".into(),
        spindle_rpm: Some(12_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(400.),
        plunge_feed_mm_min: Some(120.),
        max_stepdown_mm: Some(8.),
        stepover_mm: None,
    }
}

fn tabs(count: Option<u32>, height: f64, width: f64) -> Option<TabSettings> {
    Some(TabSettings {
        height_mm: Some(height),
        width_mm: Some(width),
        shape: TabShape::Rectangular,
        placement: TabPlacement::Automatic {
            count,
            spacing_mm: None,
        },
    })
}

fn profile_op(
    id: &str,
    contour: &str,
    stepdown: f64,
    through: Option<f64>,
    tabs: Option<TabSettings>,
) -> Operation {
    Operation {
        id: id.into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours: vec![ProfileContour {
                contour_id: contour.into(),
                side: ContourSide::Outside,
                traversal: None,
            }],
            assignment: assignment(),
            top: Default::default(),
            bottom: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockBottom,
                offset_mm: -through.unwrap_or(0.),
            },
            stepdown_mm: Some(stepdown),
            through_cut_allowance_mm: through,
            direction: Some(CutDirection::Climb),
            order: Default::default(),
            start: Default::default(),
            finish: Default::default(),
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs,
        }),
    }
}

fn job(svg: &str, operation: Operation) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "tabs".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: Default::default(),
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
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![endmill()],
        operations: vec![operation],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

/// The plan-section-19.1 tab fixture: 8 mm stock, cut bottom -8.2 (through
/// allowance 0.2), tab height 2 → the minimum full-height bridge top is -6
/// regardless of the cut bottom. Probes the composed stock history at the
/// four side midpoints: exactly the two tabbed sides stay at -6, the others
/// cut through; bridge width survives on both sides of the tabbed midpoints.
#[test]
fn automatic_tabs_hold_the_bridge_in_every_deep_pass() {
    let op = profile_op(
        "profile-1",
        "pocket-0-outer",
        2.,
        Some(0.2),
        tabs(Some(2), 2., 5.),
    );
    let job = job(RECT_SVG, op);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

    // Resolved placements are stored: two 5 mm bridges at tab top -6.
    let output = plan.operation_results[0]
        .named_outputs
        .iter()
        .find(|o| o.kind == "profile_tabs")
        .expect("tab placements in named outputs");
    let placements = &output.tab_placements;
    assert_eq!(placements.len(), 2);
    assert!(placements.iter().all(|p| (p.top_z_mm - -6.).abs() < 1e-9
        && (p.bridge_end_mm - p.bridge_start_mm - 5.).abs() < 1e-6));
    assert!(placements.iter().all(
        |p| p.restricted_start_mm < p.bridge_start_mm && p.restricted_end_mm > p.bridge_end_mm
    ));

    // Deep passes (-6, -8.2 with stepdown 2... layers end at -2, -4, -6,
    // -8.2) rise onto the tab; the -2 and -4 passes stay unsplit.
    let transitions: Vec<_> = plan
        .motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::TabTransition)
        .collect();
    assert!(
        transitions.len() >= 8,
        "two tabs × two deep passes × rise/traverse/lower, got {}",
        transitions.len()
    );
    assert!(
        transitions
            .iter()
            .all(|m| (m.start.z.max(m.end.z) - -6.).abs() < 1e-9)
    );

    // Composed stock history: corridor midpoints of the four sides. The
    // centerline of the outside cut runs 3 mm inside each stock edge.
    let history = plan.stock_history(None).unwrap();
    let top_at = |x: f64, y: f64| history.material_top_at(Point::new(x, y)).unwrap();
    let midpoints = [
        (3., 15., 0., 1.),
        (37., 15., 0., -1.),
        (20., 3., -1., 0.),
        (20., 27., 1., 0.),
    ];
    let mut held = 0;
    let mut through = 0;
    for &(x, y, dx, dy) in &midpoints {
        let depth = top_at(x, y);
        if (depth - -6.).abs() < 1e-6 {
            held += 1;
            // Bridge width: inside the 5 mm bridge (±1.75 along the edge)
            // stays at the tab top; beyond it (±3.1) cuts through.
            assert!(
                (top_at(x + dx * 1.75, y + dy * 1.75) - -6.).abs() < 1e-6,
                "bridge interior at ({x},{y}) must stay at the tab top"
            );
            assert!(
                (top_at(x + dx * 3.1, y + dy * 3.1) - -8.).abs() < 1e-6,
                "material beyond the bridge at ({x},{y}) must cut through"
            );
        } else {
            assert!(
                (depth - -8.).abs() < 1e-6,
                "untabbed midpoint ({x},{y}) cut through, got {depth}"
            );
            through += 1;
        }
    }
    assert_eq!((held, through), (2, 2), "exactly the two tabbed sides hold");

    // The tabbed plan exports through the independent numeric readback: the
    // vertical rise/lower moves are required motions, never smoothed away.
    let trusted = cam_core::sequence::TrustedPlan::from_generated(plan);
    let legacy: cam_core::post::LinuxCncProfile =
        serde_json::from_str(include_str!("../../../../real_data/machine-profile.json")).unwrap();
    let profile = cam_core::post::sequence::SequenceProfile {
        schema_version: 2,
        id: "printnc".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: cam_core::post::LengthCompensation::MacroManaged,
        path_control: cam_core::post::PathControl::ExactPath,
        tools: vec![cam_core::post::sequence::SequenceToolMapping {
            tool_id: "endmill".into(),
            tool_number: 1,
            length_offset_number: None,
        }],
        spindle_spinup_seconds: 0.5,
        coolant: cam_core::post::Coolant::Off,
        m6: legacy.m6,
    };
    let prepared =
        cam_core::post::sequence::PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();
    assert_eq!(
        export.report.basic_checks.status,
        cam_core::checks::CheckStatus::Passed
    );
    // Tab tops export at machine Z-6.000 (stock-top datum).
    assert!(export.program.gcode.contains("Z-6.000"));
}

/// A requested count that cannot fit reports the deficit, never silently
/// returns fewer tabs.
#[test]
fn impossible_tab_count_reports_the_deficit() {
    let op = profile_op(
        "profile-1",
        "pocket-0-outer",
        3.,
        Some(0.2),
        tabs(Some(500), 2., 5.),
    );
    let plan = OperationPlan::plan_job(&job(RECT_SVG, op), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "PROFILE_TAB_NO_SPACE")
        .expect("deficit diagnostic");
    assert!(diagnostic.message.contains("500"), "{}", diagnostic.message);
    assert!(
        diagnostic.message.contains("only"),
        "{}",
        diagnostic.message
    );
}

/// Curved contours have no sufficiently long straight span in this release:
/// a located error, not a tab on a corner.
#[test]
fn curved_contour_cannot_host_straight_span_tabs() {
    let op = profile_op(
        "profile-1",
        "cut-0-outer",
        3.,
        Some(0.2),
        tabs(Some(1), 2., 5.),
    );
    let plan = OperationPlan::plan_job(&job(CIRCLE_SVG, op), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_TAB_NO_SPACE" && d.message.contains("cut-0-outer"))
    );
}

/// Manual anchors (source-geometry fingerprints) map onto the compensated
/// path and take precedence; the anchor's own position keeps its bridge, and
/// an anchor too near a corner is a located error.
#[test]
fn manual_tab_anchor_holds_material_at_the_anchor() {
    let mut op = profile_op(
        "profile-1",
        "pocket-0-outer",
        3.,
        Some(0.2),
        tabs(None, 2., 5.),
    );
    let document = job(RECT_SVG, op.clone());
    let catalogue = cam_core::contours::ContourCatalogue::build(&document).unwrap();
    let contour = catalogue.contour("pocket-0-outer").unwrap();
    // The canonical page ring runs CCW from (5,5); fraction 0.15 is the
    // midpoint of the 30 mm bottom edge (perimeter 100 mm).
    let anchor = ContourAnchor {
        contour_id: "pocket-0-outer".into(),
        source_geometry_fingerprint: contour.source_fingerprint.clone(),
        fraction_along_source_contour: 0.15,
    };
    let resolved = catalogue.resolve_anchor(&anchor).unwrap();
    if let OperationSettings::Profile(settings) = &mut op.settings {
        settings.tabs = Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Rectangular,
            placement: TabPlacement::Manual {
                anchors: vec![anchor],
            },
        });
    }
    let plan = OperationPlan::plan_job(&job(RECT_SVG, op), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    let placements = &plan.operation_results[0]
        .named_outputs
        .iter()
        .find(|o| o.kind == "profile_tabs")
        .unwrap()
        .tab_placements;
    assert_eq!(placements.len(), 1);
    // The anchor sits on the source contour inside the protected corridor:
    // its material survives at the tab top while the rest cuts through.
    let history = plan.stock_history(None).unwrap();
    let at_anchor = history
        .material_top_at(Point::new(resolved.point.x, resolved.point.y))
        .unwrap();
    assert!(
        (at_anchor - -6.).abs() < 1e-6,
        "anchor material at the tab top, got {at_anchor}"
    );

    // A corner anchor (fraction 0.5 lands on the ring's corner) cannot host
    // the bridge: located error, not a tab on the corner.
    let mut corner_op = profile_op(
        "profile-1",
        "pocket-0-outer",
        3.,
        Some(0.2),
        tabs(None, 2., 5.),
    );
    if let OperationSettings::Profile(settings) = &mut corner_op.settings {
        settings.tabs = Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Rectangular,
            placement: TabPlacement::Manual {
                anchors: vec![ContourAnchor {
                    contour_id: "pocket-0-outer".into(),
                    source_geometry_fingerprint: contour.source_fingerprint.clone(),
                    fraction_along_source_contour: 0.5,
                }],
            },
        });
    }
    let plan = OperationPlan::plan_job(&job(RECT_SVG, corner_op), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_TAB_NO_SPACE" && d.message.contains("runs out of room"))
    );
}

/// Tabs whose top lies below every depth pass hold nothing: an explicit
/// ineffectual-tab diagnostic instead of pretend bridges.
#[test]
fn tabs_below_the_cut_are_ineffectual() {
    // Cut bottom -4 (no through permission): no pass reaches the -6 tab top.
    let mut op = profile_op(
        "profile-1",
        "pocket-0-outer",
        2.,
        None,
        tabs(Some(2), 2., 5.),
    );
    if let OperationSettings::Profile(settings) = &mut op.settings {
        settings.bottom = cam_core::project::HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: -4.,
        };
    }
    let plan = OperationPlan::plan_job(&job(RECT_SVG, op), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_TAB_INEFFECTIVE")
    );
}

/// Nonsensical heights and unshipped ramped shoulders are explicit errors.
#[test]
fn nonsensical_heights_and_ramped_shapes_are_rejected() {
    let tall = profile_op(
        "profile-1",
        "pocket-0-outer",
        3.,
        Some(0.2),
        tabs(Some(2), 8., 5.),
    );
    let plan = OperationPlan::plan_job(&job(RECT_SVG, tall), &PlanLimits::default()).unwrap();
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_TAB_RANGE")
    );

    let mut ramped = profile_op(
        "profile-1",
        "pocket-0-outer",
        3.,
        Some(0.2),
        tabs(Some(2), 2., 5.),
    );
    if let OperationSettings::Profile(settings) = &mut ramped.settings {
        settings.tabs = Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Ramped,
            placement: TabPlacement::Automatic {
                count: Some(2),
                spacing_mm: None,
            },
        });
    }
    let plan = OperationPlan::plan_job(&job(RECT_SVG, ramped), &PlanLimits::default()).unwrap();
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_TAB_SHAPE_UNSUPPORTED")
    );
    // Missing tab values are located missing fields, not planning errors.
    let mut unmeasured = profile_op(
        "profile-1",
        "pocket-0-outer",
        3.,
        Some(0.2),
        tabs(Some(2), 2., 5.),
    );
    if let OperationSettings::Profile(settings) = &mut unmeasured.settings {
        settings.tabs = Some(TabSettings {
            height_mm: None,
            width_mm: None,
            shape: TabShape::Rectangular,
            placement: TabPlacement::Automatic {
                count: Some(2),
                spacing_mm: None,
            },
        });
    }
    let unmeasured_job = job(RECT_SVG, unmeasured);
    let OperationSettings::Profile(unmeasured_settings) = &unmeasured_job.operations[0].settings
    else {
        unreachable!()
    };
    let missing = cam_core::operations::profile::missing_fields(
        &unmeasured_job,
        "profile-1",
        unmeasured_settings,
    );
    assert!(missing.iter().any(|d| {
        d.field_path
            .as_deref()
            .unwrap_or("")
            .contains("tabs.height_mm")
    }));
}

/// NamedOutput stays strict on the wire: unknown payload fields are rejected.
#[test]
fn tab_placement_output_serializes_strictly() {
    let output = NamedOutput {
        kind: "profile_tabs".into(),
        source_operation_id: Some("op".into()),
        z_mm: None,
        covered: None,
        tab_placements: vec![cam_core::sequence::TabPlacementOutput {
            contour_id: "c".into(),
            bridge_start_mm: 0.,
            bridge_end_mm: 5.,
            restricted_start_mm: -2.,
            restricted_end_mm: 7.,
            top_z_mm: -6.,
            footprint_mm: vec![(0., 2.), (5., 2.), (5., -2.), (0., -2.)],
        }],
    };
    let json = serde_json::to_string(&output).unwrap();
    assert!(json.contains("\"bridgeStartMm\""));
    assert!(json.contains("\"footprintMm\""));
    let parsed: NamedOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.tab_placements.len(), 1);
    assert_eq!(parsed.tab_placements[0].footprint_mm.len(), 4);
    assert!(
        serde_json::from_str::<NamedOutput>(&json.replace("bridgeStartMm", "bridge_start"))
            .is_err()
    );
    // The footprint stays optional on parse so older plan documents load.
    let legacy = serde_json::to_string(&cam_core::sequence::TabPlacementOutput {
        contour_id: "c".into(),
        bridge_start_mm: 0.,
        bridge_end_mm: 5.,
        restricted_start_mm: -2.,
        restricted_end_mm: 7.,
        top_z_mm: -6.,
        footprint_mm: vec![],
    })
    .unwrap();
    let parsed: cam_core::sequence::TabPlacementOutput = serde_json::from_str(&legacy).unwrap();
    assert!(parsed.footprint_mm.is_empty());
}
