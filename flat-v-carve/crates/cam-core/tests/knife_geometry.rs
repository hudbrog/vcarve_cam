//! F2 knife geometry (plan section 12): the right-angle corner fixture of
//! plan section 19.1, compensation on many small turns, concave corners,
//! carried-heading entry to disconnected chains, closure/overlap handling
//! once per loop, full-retract heading retention, depth/swivel resolution,
//! and the independent replay as the tip-error gate.
use cam_core::{
    checks::{CheckStatus, check_plan},
    contours::ContourCatalogue,
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    motion::Position,
    operations::drag_knife::replay::{IntendedTip, ReplayStatus, replay},
    post::{
        Coolant, LengthCompensation, PathControl,
        sequence::{PreparedExecution, SequenceProfile, SequenceToolMapping, verify_program},
    },
    project::{
        CamJob, ContourAnchor, DragKnifeSettings, DragKnifeSpec, HeightRef, HeightReference,
        JobTool, KnifeAlignment, KnifeAssignment, Operation, OperationSettings, SetupSettings,
        StartSelection, StockSetup, ToolCapabilities, ToolGeometry,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits, StageRole, TrustedPlan},
    svg::{ImportMode, ImportOptions, Placement},
    toolpath::{MotionEffect, MotionPurpose, PlannedMotion, knife_tip},
};

const KNIFE: &str = "knife";
const HEAD: &str =
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30">"##;
const TAIL: &str = "</svg>";

/// The page Y axis flips on import (30 mm page): page (x, y) -> setup (x, 30-y).
fn setup_point(x: f64, y: f64) -> Point {
    Point::new(x, 30. - y)
}

fn assignment() -> KnifeAssignment {
    KnifeAssignment {
        tool_id: KNIFE.into(),
        cutting_feed_mm_min: Some(150.),
        plunge_feed_mm_min: Some(60.),
        swivel_feed_mm_min: Some(50.),
        max_stepdown_mm: Some(1.),
    }
}

fn knife_settings(chains: &[&str]) -> DragKnifeSettings {
    DragKnifeSettings {
        chains: chains.iter().map(|id| id.to_string()).collect(),
        assignment: assignment(),
        top: HeightRef {
            reference: Default::default(),
            offset_mm: 0.,
        },
        bottom: HeightRef {
            reference: HeightReference::OperationTop,
            offset_mm: -2.,
        },
        stepdown_mm: Some(1.),
        swivel_depth_mm: Some(0.5),
        corner_threshold_deg: Some(30.),
        through_cut_allowance_mm: None,
        start: StartSelection::Automatic,
        closure_overlap_mm: None,
        alignment: KnifeAlignment {
            initial_heading_deg: Some(90.),
        },
    }
}

fn job(svg_body: &str, settings: DragKnifeSettings) -> CamJob {
    let job = CamJob {
        schema_version: 4,
        name: "knife-geometry".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: format!("{HEAD}{svg_body}{TAIL}"),
        }),
        import: ImportOptions {
            placement: Placement::default(),
            mode: ImportMode::Centerline,
            ..Default::default()
        },
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(3.),
                xy: Some(cam_core::project::RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![JobTool {
            id: KNIFE.into(),
            name: "drag knife".into(),
            geometry: Some(ToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.,
                max_cut_depth_mm: 3.,
            })),
            capabilities: ToolCapabilities::default(),
        }],
        operations: vec![Operation {
            id: "knife-1".into(),
            name: "Score".into(),
            enabled: true,
            settings: OperationSettings::DragKnife(settings),
        }],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.01),
        },
        legacy_machine_profile: None,
    };
    job.validate().unwrap();
    job
}

fn complete_plan(svg_body: &str, settings: DragKnifeSettings) -> OperationPlan {
    let plan = OperationPlan::plan_job(&job(svg_body, settings), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "knife operation plans completely: {:?}",
        plan.generation_diagnostics
    );
    let checks = check_plan(&plan).unwrap();
    assert_eq!(checks.status, CheckStatus::Passed, "{:?}", checks.findings);
    plan
}

fn knife_motions(plan: &OperationPlan) -> &[PlannedMotion] {
    &plan.motions[plan.stages[0].motion_range.0..plan.stages[0].motion_range.1]
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

fn pos(x: f64, y: f64, z: f64) -> Position {
    Position::new(Point::new(x, y), z)
}

/// The plan-section-19.1 right-angle corner, translated to a tip corner at
/// (15,15) with incoming +X and outgoing +Y and blade offset 1: the holder
/// endpoints are (16,15) and (15,16) with the ideal swivel centered (15,15).
const CORNER: &str =
    r##"<path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 15 L15 15 L15 5"/>"##;

#[test]
fn right_angle_corner_swivel_matches_the_plan_numbers() {
    let plan = complete_plan(CORNER, knife_settings(&["cut-chain-0"]));
    assert_eq!(plan.stages[0].role, StageRole::Knife);
    let motions = knife_motions(&plan);
    // The incoming cut ends at the corner holder endpoint (16,15) at pass
    // depth; a rise, the swivel chords at the resolved swivel depth, and a
    // lower follow; the outgoing cut starts from (15,16).
    let incoming = motions
        .iter()
        .find(|m| {
            m.purpose == MotionPurpose::KnifeCut
                && close(m.end.x, 16., 1e-9)
                && close(m.end.y, 15., 1e-9)
        })
        .expect("incoming cut reaches the corner holder endpoint");
    assert!(close(incoming.end.z, -1., 1e-9), "pass one depth");
    assert_eq!(incoming.blade_heading_deg, Some((180., 180.)));
    let swivels: Vec<&PlannedMotion> = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeSwivel)
        .collect();
    // Chords change XY at the resolved swivel depth; the rise/lower around
    // them are vertical moves at the corner pivots.
    let chords: Vec<&&PlannedMotion> = swivels
        .iter()
        .filter(|m| m.start.xy().distance(m.end.xy()) > 1e-12)
        .collect();
    assert_eq!(chords.len(), 24, "one 90-degree swivel per pass, chorded");
    assert!(
        chords
            .iter()
            .all(|m| close(m.start.z, -0.5, 1e-9) && close(m.end.z, -0.5, 1e-9))
    );
    assert!(
        close(chords[0].start.x, 16., 1e-9) && close(chords[0].start.y, 15., 1e-9),
        "the swivel starts at the incoming holder endpoint"
    );
    let last = **chords.last().unwrap();
    assert!(
        close(last.end.x, 15., 1e-9) && close(last.end.y, 16., 1e-9),
        "the swivel ends at the outgoing holder endpoint"
    );
    // The modeled headings swing 180 -> 270 across the swivel chords.
    assert_eq!(chords[0].blade_heading_deg.unwrap().0, 180.);
    assert_eq!(last.blade_heading_deg.unwrap().1, 270.);
    let outgoing = motions
        .iter()
        .find(|m| {
            m.purpose == MotionPurpose::KnifeCut
                && close(m.start.x, 15., 1e-9)
                && close(m.start.y, 16., 1e-9)
        })
        .expect("outgoing cut starts from the second holder endpoint");
    assert_eq!(outgoing.blade_heading_deg, Some((270., 270.)));
    // Every knife motion's tip derives from its pivot and heading: the tip
    // path is the drawn chain (5,15) -> (15,15) -> (15,25) in setup space.
    for motion in motions
        .iter()
        .filter(|m| m.effect == MotionEffect::KnifeTrace)
    {
        let tip = knife_tip(motion.start.xy(), motion.blade_heading_deg.unwrap().0, 1.);
        let on_path = close(tip.distance(setup_point(5., 15.)), 0., 1e-6)
            || close(tip.distance(setup_point(15., 15.)), 0., 1e-6)
            || close(tip.distance(setup_point(15., 25.)), 0., 1e-6)
            || close(tip.y, 15., 1e-6) && (5. ..=15.).contains(&tip.x)
            || close(tip.x, 15., 1e-6) && (15. ..=25.).contains(&tip.y);
        assert!(
            on_path,
            "tip {:?} of motion {} lies on the drawn chain",
            tip, motion.id
        );
    }
    // Alignment (plan section 12.3): the initial heading (90 deg) plants the
    // tip at (5,15) and turns to the first-cut heading 180 in contact.
    let align: Vec<&PlannedMotion> = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeAlign && m.layer == 0)
        .collect();
    assert_eq!(align.len(), 12, "one alignment swivel, chorded");
    assert!(close(align[0].start.x, 5., 1e-9) && close(align[0].start.y, 14., 1e-9));
    assert_eq!(align[0].blade_heading_deg.unwrap().0, 90.);
    assert_eq!(align.last().unwrap().blade_heading_deg.unwrap().1, 180.);
    // Depth passes: two layers ending exactly at the bottom, the corner
    // swivel held in every pass, and pass two re-aligns from the carried
    // heading (270) rather than teleporting.
    let layers: Vec<usize> = motions.iter().map(|m| m.layer).collect();
    assert!(layers.windows(2).all(|w| w[0] <= w[1]), "passes in order");
    assert_eq!(*layers.last().unwrap(), 1, "two depth passes");
    for (index, layer) in [0usize, 1].iter().enumerate() {
        let expected_z = -((index + 1) as f64);
        let pass: Vec<&PlannedMotion> = motions.iter().filter(|m| m.layer == *layer).collect();
        let cuts = pass
            .iter()
            .filter(|m| m.purpose == MotionPurpose::KnifeCut)
            .count();
        assert_eq!(cuts, 2, "both chain segments cut in pass {index}");
        assert!(pass.iter().all(|m| m.end.z >= expected_z - 1e-9));
        assert!(pass.iter().any(|m| close(m.end.z, expected_z, 1e-9)));
    }
    let second_align: Vec<&PlannedMotion> = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeAlign && m.layer == 1)
        .collect();
    assert_eq!(second_align[0].blade_heading_deg.unwrap().0, 270.);
}

#[test]
fn many_small_curve_turns_keep_compensation_and_tip_accuracy() {
    // A semicircle of 24 chords (7.5-degree turns, all below the 30-degree
    // threshold): no turn may discard compensation, and the accumulated tip
    // error stays within the budget — verified by the replay gate inside
    // `plan` and by deriving every cut motion's tip from its heading.
    let radius = 10.;
    let center = (20., 15.);
    let mut path = String::from("M");
    for step in 0..=24 {
        let angle = (180. - step as f64 * 7.5).to_radians();
        let x = center.0 + radius * angle.cos();
        let y = center.1 + radius * angle.sin();
        // Setup -> page: flip Y back.
        path.push_str(&format!(" {} {}", x, 30. - y));
    }
    let svg = format!(
        r##"<polyline id="arc" fill="none" stroke="#000" stroke-width="0.4" points="{}"/>"##,
        path.trim_start_matches('M').trim()
    );
    let plan = complete_plan(&svg, knife_settings(&["arc-chain-0"]));
    let motions = knife_motions(&plan);
    assert!(
        motions
            .iter()
            .all(|m| !matches!(m.purpose, MotionPurpose::KnifeSwivel)),
        "small turns stay continuous cuts; no explicit swivels"
    );
    let cuts = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeCut)
        .count();
    assert!(cuts > 24, "the whole curve is cut chord by chord: {cuts}");
    for motion in motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeCut)
    {
        let tip_start = knife_tip(motion.start.xy(), motion.blade_heading_deg.unwrap().0, 1.);
        let expected_radius =
            ((tip_start.x - center.0).hypot(tip_start.y - center.1) - radius).abs();
        assert!(
            expected_radius < 1e-6,
            "cut tip starts on the drawn arc: {:?}",
            tip_start
        );
    }
    // Compensation exists: the holder rides ahead of the tip, never on it.
    let first_cut = motions
        .iter()
        .find(|m| m.purpose == MotionPurpose::KnifeCut)
        .unwrap();
    let tip = knife_tip(
        first_cut.start.xy(),
        first_cut.blade_heading_deg.unwrap().0,
        1.,
    );
    assert!(tip.distance(first_cut.start.xy()) > 0.9);
}

#[test]
fn concave_right_turn_swivels_clockwise_around_the_corner() {
    // Incoming +X, outgoing -Y: a -90-degree (clockwise) swivel. Tip corner
    // (15,25); holder endpoints (16,25) and (15,24).
    const CONCAVE: &str =
        r##"<path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 5 L15 5 L15 15"/>"##;
    let plan = complete_plan(CONCAVE, knife_settings(&["cut-chain-0"]));
    let motions = knife_motions(&plan);
    let swivel: Vec<&PlannedMotion> = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeSwivel)
        .collect();
    assert!(
        swivel
            .iter()
            .any(|m| close(m.start.x, 16., 1e-9) && close(m.start.y, 25., 1e-9))
    );
    let last = swivel.last().unwrap();
    assert!(close(last.end.x, 15., 1e-9) && close(last.end.y, 24., 1e-9));
    // Heading 180 (trailing +X) -> 90 (trailing -Y): the signed sweep runs
    // clockwise, never the long way around.
    assert_eq!(swivel[0].blade_heading_deg.unwrap().0, 180.);
    assert_eq!(last.blade_heading_deg.unwrap().1, 90.);
    for chord in swivel
        .iter()
        .filter(|m| m.start.xy().distance(m.end.xy()) > 1e-12)
    {
        let (start, end) = chord.blade_heading_deg.unwrap();
        assert!(
            close((start - end).rem_euclid(360.), 7.5, 1e-6),
            "each chord turns 7.5 degrees clockwise: {start} -> {end}"
        );
    }
}

#[test]
fn disconnected_chain_entry_uses_the_carried_heading_in_contact() {
    const TWO: &str = r##"<path id="cuts" fill="none" stroke="#000" stroke-width="0.4" d="M5 15 L15 15 M25 15 L25 5"/>"##;
    let plan = complete_plan(TWO, knife_settings(&["cuts-chain-0", "cuts-chain-1"]));
    let motions = knife_motions(&plan);
    // Chain one ends heading 180 (trailing +X). The lifted rapid to chain
    // two carries that heading unchanged; the blade never rotates in air.
    let rapid = motions
        .iter()
        .find(|m| {
            m.purpose == MotionPurpose::Clearance
                && m.effect == MotionEffect::None
                && close(m.end.x, 26., 1e-9)
                && close(m.end.y, 15., 1e-9)
        })
        .expect("rapid to the second chain's carried-heading pivot");
    assert_eq!(rapid.blade_heading_deg, Some((180., 180.)));
    // The second chain's entry pivots around its planted tip (25,15) from
    // the carried 180 to the first-cut 270 — no teleport to the tangent.
    // Chain two's first pass is the operation's third pass (pass 2).
    let second_entry: Vec<&PlannedMotion> = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeAlign && m.pass_id == 2)
        .collect();
    assert_eq!(second_entry.len(), 12, "one contact alignment per entry");
    assert!(close(second_entry[0].start.x, 26., 1e-9) && close(second_entry[0].start.y, 15., 1e-9));
    assert_eq!(second_entry[0].blade_heading_deg.unwrap().0, 180.);
    assert_eq!(
        second_entry.last().unwrap().blade_heading_deg.unwrap().1,
        270.
    );
    // Pass three (chain two, second pass) carries the matching heading from
    // its own first pass: no second alignment there.
    let chain2_pass2_aligns = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeAlign && m.pass_id == 3)
        .count();
    assert_eq!(chain2_pass2_aligns, 0);
    // The descent into the second chain happens at the carried-heading
    // pivot (26,15): tip planted at (25,15).
    let entry = motions
        .iter()
        .find(|m| {
            m.purpose == MotionPurpose::Entry
                && m.effect == MotionEffect::KnifeTrace
                && close(m.start.x, 26., 1e-9)
                && close(m.start.y, 15., 1e-9)
        })
        .expect("fed descent at the carried-heading pivot");
    let tip = knife_tip(entry.start.xy(), entry.blade_heading_deg.unwrap().0, 1.);
    assert!(close(tip.x, 25., 1e-9) && close(tip.y, 15., 1e-9));
}

#[test]
fn closed_contour_cuts_the_loop_once_with_a_single_closing_swivel() {
    // A 10x10 square (setup (10,20) (20,20) (20,10) (10,10)) with a 2 mm
    // closure overlap: each pass cuts exactly perimeter + overlap once, the
    // closing corner swivels exactly once, and no second loop is cut.
    const SQUARE: &str = r##"<path id="square" fill="none" stroke="#000" stroke-width="0.4" d="M10 10 L20 10 L20 20 L10 20 Z"/>"##;
    let mut settings = knife_settings(&["square-chain-0"]);
    settings.closure_overlap_mm = Some(2.);
    settings.bottom.offset_mm = -1.5;
    let plan = complete_plan(SQUARE, settings);
    let motions = knife_motions(&plan);
    for (layer_index, layer) in [0usize, 1].iter().enumerate() {
        // Depth 1.5 with stepdown 1: layers end exactly at -1.0 and -1.5.
        let expected_z = [-1.0, -1.5][layer_index];
        let pass: Vec<&PlannedMotion> = motions.iter().filter(|m| m.layer == *layer).collect();
        let mut cut_length = 0f64;
        let mut cut_motions = 0;
        for motion in &pass {
            if motion.purpose == MotionPurpose::KnifeCut {
                cut_length += motion.start.xy().distance(motion.end.xy());
                cut_motions += 1;
            }
        }
        assert_eq!(cut_motions, 5, "four edges plus the overlap tail");
        assert!(
            close(cut_length, 42., 1e-6),
            "perimeter 40 + overlap 2 once: {cut_length}"
        );
        // Corner swivels: three interior corners plus the closing corner,
        // exactly once each — the overlap never re-cuts a full loop.
        let swivel_chords = pass
            .iter()
            .filter(|m| {
                m.purpose == MotionPurpose::KnifeSwivel && m.start.xy().distance(m.end.xy()) > 1e-12
            })
            .count();
        assert_eq!(swivel_chords, 4 * 12, "four swivel actions per pass");
        assert!(
            close(pass.last().unwrap().start.x, 13., 1e-9),
            "the pass ends 2 mm past the seam: tip (12,20), pivot (13,20)"
        );
        assert!(
            close(pass.last().unwrap().start.z, expected_z, 1e-9),
            "the retract leaves from pass depth"
        );
    }
    // The final pass ends exactly at the bottom.
    assert!(motions.iter().any(|m| close(m.end.z, -1.5, 1e-9)));
    // Between passes of a closed chain the heading already matches the next
    // start (the closing swivel realigned it): pass two descends without a
    // second alignment.
    let aligns = motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeAlign)
        .count();
    assert_eq!(aligns, 12, "alignment only before the first pass");
}

#[test]
fn closure_overlap_must_stay_below_one_loop() {
    const SQUARE: &str = r##"<path id="square" fill="none" stroke="#000" stroke-width="0.4" d="M10 10 L20 10 L20 20 L10 20 Z"/>"##;
    let mut settings = knife_settings(&["square-chain-0"]);
    settings.closure_overlap_mm = Some(40.);
    let plan = OperationPlan::plan_job(&job(SQUARE, settings), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert_eq!(plan.generation_diagnostics[0].code, "KNIFE_CLOSURE_RANGE");
}

#[test]
fn alignment_outside_stock_and_ambiguous_headings_are_explicit() {
    // A chain starting outside the stock rectangle cannot keep the blade
    // engaged for the alignment contact move. The artwork stays inside the
    // page; only the physical stock is smaller than the drawing.
    const INSIDE_PAGE: &str =
        r##"<path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 15 L15 15"/>"##;
    let mut small_stock = job(INSIDE_PAGE, knife_settings(&["cut-chain-0"]));
    small_stock.setup.stock.xy = Some(cam_core::project::RectXY {
        min_x_mm: 10.,
        min_y_mm: 10.,
        width_mm: 20.,
        length_mm: 10.,
    });
    small_stock.validate().unwrap();
    let plan = OperationPlan::plan_job(&small_stock, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = &plan.generation_diagnostics[0];
    assert_eq!(diagnostic.code, "KNIFE_ALIGNMENT_UNAVAILABLE");
    assert!(diagnostic.message.contains("cut-chain-0"));

    // An initial heading opposing the first cut within half a degree has no
    // trustworthy alignment side.
    let mut opposed = knife_settings(&["cut-chain-0"]);
    opposed.alignment.initial_heading_deg = Some(0.);
    // First cut travels +X with trailing heading 180; heading 0 opposes it.
    let plan = OperationPlan::plan_job(&job(CORNER, opposed), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.generation_diagnostics[0].code,
        "KNIFE_ALIGNMENT_UNAVAILABLE"
    );
    assert!(plan.generation_diagnostics[0].message.contains("ambiguous"));
}

#[test]
fn near_reversal_corners_are_rejected_not_guessed() {
    const REVERSAL: &str = r##"<path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M5 15 L25 15 L5.05 14.98"/>"##;
    let plan = OperationPlan::plan_job(
        &job(REVERSAL, knife_settings(&["cut-chain-0"])),
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = &plan.generation_diagnostics[0];
    assert_eq!(diagnostic.code, "KNIFE_CORNER_AMBIGUOUS");
    assert!(diagnostic.message.contains("reversal"));
}

#[test]
fn missing_settings_are_located_not_defaulted() {
    let mut settings = knife_settings(&["cut-chain-0"]);
    settings.alignment.initial_heading_deg = None;
    let plan = OperationPlan::plan_job(&job(CORNER, settings), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(plan.generation_diagnostics.iter().any(|d| {
        d.code == "MISSING_MACHINING_SETTING"
            && plan
                .job_snapshot
                .operations
                .iter()
                .any(|op| d.message.contains(&op.id))
    }));
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.message.contains("initial_heading_deg"))
    );
}

#[test]
fn through_allowance_and_cut_depth_capability_are_enforced() {
    // Bottom below the stock bottom without permission.
    let mut through = knife_settings(&["cut-chain-0"]);
    through.bottom = HeightRef {
        reference: HeightReference::StockBottom,
        offset_mm: -0.2,
    };
    let plan =
        OperationPlan::plan_job(&job(CORNER, through.clone()), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.generation_diagnostics[0].code,
        "KNIFE_THROUGH_ALLOWANCE"
    );
    // With the explicit allowance the same bottom plans completely (this
    // fixture's knife can cut 4 mm deep).
    let mut allowed = through.clone();
    allowed.through_cut_allowance_mm = Some(0.2);
    let mut deeper_tool = job(CORNER, allowed);
    deeper_tool.tools[0].geometry = Some(ToolGeometry::DragKnife(DragKnifeSpec {
        blade_offset_mm: 1.,
        max_cut_depth_mm: 4.,
    }));
    deeper_tool.validate().unwrap();
    let plan = OperationPlan::plan_job(&deeper_tool, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    // Cut depth beyond the knife's declared capability.
    let mut deep = knife_settings(&["cut-chain-0"]);
    deep.bottom.offset_mm = -3.5;
    deep.through_cut_allowance_mm = Some(1.);
    let plan = OperationPlan::plan_job(&job(CORNER, deep), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.generation_diagnostics[0].code,
        "KNIFE_DEPTH_CAPABILITY"
    );
}

#[test]
fn shallow_passes_swivel_at_full_pass_depth() {
    // A single 0.3 mm pass: the resolved swivel depth is min(0.5, 0.3) =
    // 0.3 below the top — the swivel runs at pass depth with no rise/lower.
    let mut settings = knife_settings(&["cut-chain-0"]);
    settings.bottom.offset_mm = -0.3;
    let plan = complete_plan(CORNER, settings);
    let motions = knife_motions(&plan);
    assert!(
        motions
            .iter()
            .filter(|m| m.purpose == MotionPurpose::KnifeSwivel)
            .all(|m| close(m.start.z, -0.3, 1e-9))
    );
    assert!(motions.iter().all(|m| m.end.z >= -0.3 - 1e-9));
}

#[test]
fn anchor_starts_reposition_the_chain() {
    let catalogue =
        ContourCatalogue::build(&job(CORNER, knife_settings(&["cut-chain-0"]))).unwrap();
    let chain = catalogue.chain("cut-chain-0").unwrap();
    // A mid-chain anchor on an open chain cannot become its start.
    let mut mid = knife_settings(&["cut-chain-0"]);
    mid.start = StartSelection::Anchor(Box::new(ContourAnchor {
        contour_id: chain.id.clone(),
        source_geometry_fingerprint: chain.source_fingerprint.clone(),
        fraction_along_source_contour: 0.5,
    }));
    let plan = OperationPlan::plan_job(&job(CORNER, mid), &PlanLimits::default()).unwrap();
    assert_eq!(plan.generation_diagnostics[0].code, "KNIFE_START_RANGE");
    // An anchor resolving at the far endpoint (within the anchor tolerance
    // of fraction 1.0) reverses the chain: the first cut runs from (15,25)
    // toward (15,15), pivot entering at (15,24) with heading 90.
    let mut end = knife_settings(&["cut-chain-0"]);
    end.start = StartSelection::Anchor(Box::new(ContourAnchor {
        contour_id: chain.id.clone(),
        source_geometry_fingerprint: chain.source_fingerprint.clone(),
        fraction_along_source_contour: 1. - 1e-6,
    }));
    let plan = complete_plan(CORNER, end);
    let first_cut = knife_motions(&plan)
        .iter()
        .find(|m| m.purpose == MotionPurpose::KnifeCut)
        .unwrap();
    assert!(close(first_cut.start.x, 15., 1e-6) && close(first_cut.start.y, 24., 1e-6));
    assert_eq!(first_cut.blade_heading_deg.unwrap().0, 90.);
}

#[test]
fn stock_history_ignores_knife_stages() {
    // Knife traces never enter the milling-stock model (plan section 9.1):
    // composing the history of a knife-only plan yields no sweep batches
    // instead of a tool-geometry error.
    let plan = complete_plan(CORNER, knife_settings(&["cut-chain-0"]));
    let history = plan.stock_history(None).unwrap();
    assert!(history.batches.is_empty());
}

#[test]
fn planner_output_exports_through_the_independent_readback() {
    let plan = complete_plan(CORNER, knife_settings(&["cut-chain-0"]));
    let trusted = TrustedPlan::from_generated(plan);
    let profile = SequenceProfile {
        schema_version: 2,
        id: "knife-profile".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: LengthCompensation::MacroManaged,
        path_control: PathControl::Blend {
            tolerance_mm: 0.01,
            naive_cam_tolerance_mm: None,
        },
        tools: vec![SequenceToolMapping {
            tool_id: KNIFE.into(),
            tool_number: 3,
            length_offset_number: None,
        }],
        spindle_spinup_seconds: 0.5,
        coolant: Coolant::Flood,
        m6: cam_core::post::LinuxCncProfile::from_json(include_str!(
            "../../../../real_data/machine-profile.json"
        ))
        .unwrap()
        .m6,
    };
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();
    let knife_process = &prepared.stages[0].process;
    assert!(matches!(
        knife_process.spindle,
        cam_core::post::sequence::PreparedSpindle::Off
    ));
    assert_eq!(knife_process.coolant, Coolant::Off);
    assert_eq!(knife_process.path_control, PathControl::ExactPath);
    let gcode = &export.program.gcode;
    assert!(gcode.lines().any(|l| l.contains("G61")));
    assert!(!gcode.lines().any(|l| l.contains("M3") || l.contains("M4")));
    verify_program(&prepared, &trusted, &export.program).unwrap();
}

// ---------------------------------------------------------------------------
// Independent replay (plan section 12.5)
// ---------------------------------------------------------------------------

fn knife_motion(
    id: usize,
    purpose: MotionPurpose,
    start: Position,
    end: Position,
    heading: Option<(f64, f64)>,
) -> PlannedMotion {
    PlannedMotion {
        id,
        operation_id: "knife-1".into(),
        stage_id: "knife-1-knife".into(),
        tool_id: KNIFE.into(),
        contour_id: None,
        pass_id: 0,
        layer: 0,
        interpolation: cam_core::toolpath::Interpolation::LinearFeed,
        purpose,
        effect: MotionEffect::KnifeTrace,
        start,
        end,
        feed_mm_min: Some(100.),
        blade_heading_deg: heading,
    }
}

#[test]
fn replay_accepts_an_aligned_straight_cut_exactly() {
    // Holder +X line with the blade already trailing (heading 180): the
    // contact equation is at stable equilibrium — deviation ~0.
    let motions = vec![knife_motion(
        0,
        MotionPurpose::KnifeCut,
        pos(11., 0., -1.),
        pos(21., 0., -1.),
        Some((180., 180.)),
    )];
    let intended = vec![IntendedTip::Segment(
        Point::new(10., 0.),
        Point::new(20., 0.),
    )];
    let outcome = replay(&motions, 1., &intended, 0.01, 1_000_000);
    assert_eq!(outcome.status, ReplayStatus::Within);
    assert!(outcome.max_tip_deviation_mm < 1e-9);
    assert!(outcome.max_heading_error_deg < 1e-6);
}

#[test]
fn replay_rejects_a_misaligned_entry_and_shows_convergence() {
    // The blade enters pointing +Y (heading 90) while the holder drives +X:
    // the tip starts ~1 mm off the intended line — the gate must reject it —
    // but over a long line the no-slip heading converges to trailing, which
    // the boundary heading comparison observes as a small error.
    let motions = vec![knife_motion(
        0,
        MotionPurpose::KnifeCut,
        pos(10., 1., -1.),
        pos(110., 1., -1.),
        Some((90., 180.)),
    )];
    let intended = vec![IntendedTip::Segment(
        Point::new(10., 0.),
        Point::new(110., 0.),
    )];
    let outcome = replay(&motions, 1., &intended, 0.01, 1_000_000);
    assert_eq!(outcome.status, ReplayStatus::Exceeded);
    assert!(outcome.max_tip_deviation_mm > 0.5);
    assert!(
        outcome.max_heading_error_deg < 0.01,
        "heading converges to trailing along the line: {:?}",
        outcome
    );
}

#[test]
fn replay_retains_heading_through_lifted_moves_and_budget_is_inconclusive() {
    // A lifted rapid between two contact moves never rotates the blade: the
    // modeled constant heading stays consistent.
    let lifted = PlannedMotion {
        id: 1,
        interpolation: cam_core::toolpath::Interpolation::Rapid,
        effect: MotionEffect::None,
        feed_mm_min: None,
        ..knife_motion(
            1,
            MotionPurpose::Clearance,
            pos(21., 0., 5.),
            pos(41., 0., 5.),
            Some((180., 180.)),
        )
    };
    let motions = vec![
        knife_motion(
            0,
            MotionPurpose::KnifeCut,
            pos(11., 0., -1.),
            pos(21., 0., -1.),
            Some((180., 180.)),
        ),
        lifted,
        knife_motion(
            2,
            MotionPurpose::KnifeCut,
            pos(41., 0., -1.),
            pos(51., 0., -1.),
            Some((180., 180.)),
        ),
    ];
    let intended = vec![
        IntendedTip::Segment(Point::new(10., 0.), Point::new(20., 0.)),
        IntendedTip::Lifted,
        IntendedTip::Segment(Point::new(40., 0.), Point::new(50., 0.)),
    ];
    let outcome = replay(&motions, 1., &intended, 0.01, 1_000_000);
    assert_eq!(outcome.status, ReplayStatus::Within);
    // A step budget of zero can never certify the path: inconclusive, not
    // success (plan section 12.5).
    let outcome = replay(&motions, 1., &intended, 0.01, 0);
    assert_eq!(outcome.status, ReplayStatus::BudgetExhausted);
}

#[test]
fn replay_convergence_with_a_finer_planner_tolerance() {
    // The planner gate at a ten-times tighter tolerance still replays within
    // budget on the corner fixture: linearization error scales down with the
    // tolerance rather than competing with it.
    let mut tighter = job(CORNER, knife_settings(&["cut-chain-0"]));
    tighter.tolerances.motion_tolerance_mm = Some(0.001);
    let plan = OperationPlan::plan_job(&tighter, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    // More chords for the tighter budget.
    let chords = knife_motions(&plan)
        .iter()
        .filter(|m| m.purpose == MotionPurpose::KnifeSwivel)
        .count();
    assert!(chords > 12 * 2, "finer linearization: {chords}");
}
