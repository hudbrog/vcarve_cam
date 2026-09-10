//! E3 entries/inspection (plan sections 10.4, 19.1): contour ramp entries
//! that descend along the compensated path and never become plunges when
//! blocked, tangent line/arc leads with their own feeds checked against
//! retained geometry and tab volumes, anchor starts, and automatic tab
//! placement excluding the entry/lead neighborhood.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, ContourAnchor, ContourSide, CutDirection, JobTool, LeadSpec, MillingAssignment,
        Operation, OperationSettings, ProfileContour, ProfileEntry, ProfileFinishSettings,
        ProfileSettings, RectXY, SetupSettings, SpindleDirection, StartSelection, StockSetup,
        TabPlacement, TabSettings, TabShape, ToolCapabilities, ToolGeometry, TraversalDirection,
        WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits},
    toolpath::MotionPurpose,
};

const RECT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#;
const CIRCLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><circle id="cut" cx="25" cy="25" r="20"/></svg>"#;
const SMALL_RECT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="16mm" viewBox="0 0 20 16"><path id="part" fill-rule="evenodd" d="M5 5h10v6h-10z"/></svg>"#;

fn endmill(ramp_capable: Option<bool>) -> JobTool {
    JobTool {
        id: "endmill".into(),
        name: "4mm endmill".into(),
        geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
            diameter_mm: 4.,
            cutting_length_mm: 12.,
        })),
        capabilities: ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable,
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

#[allow(clippy::too_many_arguments)]
fn profile_settings(
    entry: ProfileEntry,
    lead_in: LeadSpec,
    lead_out: LeadSpec,
    start: StartSelection,
    tabs: Option<TabSettings>,
    finish: ProfileFinishSettings,
    stepdown: f64,
) -> ProfileSettings {
    ProfileSettings {
        contours: vec![ProfileContour {
            contour_id: "pocket-0-outer".into(),
            side: ContourSide::Outside,
            traversal: None,
        }],
        assignment: assignment(),
        top: Default::default(),
        bottom: cam_core::project::HeightRef {
            reference: cam_core::project::HeightReference::StockBottom,
            offset_mm: 0.,
        },
        stepdown_mm: Some(stepdown),
        through_cut_allowance_mm: None,
        direction: Some(CutDirection::Climb),
        order: Default::default(),
        start,
        finish,
        entry,
        lead_in,
        lead_out,
        tabs,
    }
}

fn job(svg: &str, settings: ProfileSettings) -> CamJob {
    job_with(svg, settings, endmill(Some(true)))
}

fn job_with(svg: &str, settings: ProfileSettings, tool: JobTool) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "entries".into(),
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
        tools: vec![tool],
        operations: vec![Operation {
            id: "profile-1".into(),
            name: "Profile".into(),
            enabled: true,
            settings: OperationSettings::Profile(settings),
        }],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

fn plan_of(svg: &str, settings: ProfileSettings) -> OperationPlan {
    OperationPlan::plan_job(&job(svg, settings), &PlanLimits::default()).unwrap()
}

/// A ramp entry descends along the compensated contour within the commanded
/// angle, and the traversal wraps past the seam so the ramped section is
/// flattened at full depth in every layer (plan section 10.4).
#[test]
fn ramp_entry_descends_along_the_contour_within_the_angle() {
    let settings = profile_settings(
        ProfileEntry::Ramp {
            max_angle_deg: Some(30.),
            feed_mm_min: Some(150.),
        },
        LeadSpec::None,
        LeadSpec::None,
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        2.,
    );
    let plan = plan_of(RECT_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

    let ramp_len = 2. / 30f64.to_radians().tan();
    let layers: Vec<f64> = vec![-2., -4., -6., -8.];
    for (layer_index, &cut_z) in layers.iter().enumerate() {
        let layer: Vec<&cam_core::toolpath::PlannedMotion> = plan
            .motions
            .iter()
            .filter(|m| m.layer == layer_index && m.purpose != MotionPurpose::Approach)
            .collect();
        let entries: Vec<&&cam_core::toolpath::PlannedMotion> = layer
            .iter()
            .filter(|m| m.purpose == MotionPurpose::Entry)
            .collect();
        assert!(
            entries
                .iter()
                .all(|m| (m.feed_mm_min.unwrap() - 150.).abs() < 1e-9),
            "ramp entries carry the ramp feed"
        );
        // Every entry segment is a descent within the angle budget; vertical
        // plunges at cut depth (blocked-ramp fallbacks) would fail this.
        let mut horizontal = 0f64;
        for motion in &entries {
            let dz = motion.start.z - motion.end.z;
            let dxy = Point::new(motion.start.x, motion.start.y)
                .distance(Point::new(motion.end.x, motion.end.y));
            assert!(
                dxy > 1e-6,
                "ramp entries move along the path, not vertically"
            );
            assert!(
                dz / dxy <= 30f64.to_radians().tan() + 1e-6,
                "slope within the ramp angle"
            );
            horizontal += dxy;
        }
        assert!(
            (horizontal - ramp_len).abs() < 0.05,
            "layer {layer_index} ramps over ~{ramp_len:.3} mm, got {horizontal:.3}"
        );
        // The layer's first entry starts at the seam above the previous
        // layer, and the traversal wraps past the seam by the ramp length:
        // the last cutting motion ends exactly where the descent did.
        let first = *entries.first().expect("a ramp entry per layer");
        assert!(
            (first.start.z
                - if layer_index == 0 {
                    0.
                } else {
                    layers[layer_index - 1]
                })
            .abs()
                < 1e-9
        );
        let ramp_end = Point::new(entries.last().unwrap().end.x, entries.last().unwrap().end.y);
        let seam = Point::new(first.start.x, first.start.y);
        assert!(
            ramp_end.distance(seam) > ramp_len * 0.8,
            "the descent travels a meaningful arc along the loop (chord {:.3} of ramp {ramp_len:.3})",
            ramp_end.distance(seam)
        );
        let rough: Vec<&&cam_core::toolpath::PlannedMotion> = layer
            .iter()
            .filter(|m| {
                matches!(
                    m.purpose,
                    MotionPurpose::Rough | MotionPurpose::TabTransition
                )
            })
            .collect();
        let last = *rough.last().expect("rough traversal");
        assert!(
            Point::new(last.end.x, last.end.y).distance(ramp_end) < 0.06,
            "layer {layer_index} wraps the seam and flattens the ramped section"
        );
        assert!((last.end.z - cut_z).abs() < 1e-9);
    }
    // The wrapped re-cut flattens the ramped floor: corridor points along the
    // ramped span (the seam's corner arc on this fixture) reach the cut
    // bottom after the deepest layer.
    let history = plan.stock_history(None).unwrap();
    let seam = plan
        .motions
        .iter()
        .find(|m| m.purpose == MotionPurpose::Entry)
        .unwrap()
        .start;
    let seam_point = Point::new(seam.x, seam.y);
    let travel = {
        let rough = plan
            .motions
            .iter()
            .find(|m| m.purpose == MotionPurpose::Rough)
            .unwrap();
        let d = Point::new(rough.end.x - rough.start.x, rough.end.y - rough.start.y);
        let len = d.distance(Point::new(0., 0.));
        Point::new(d.x / len, d.y / len)
    };
    for offset in [0.5, 1.5, 2.5] {
        let probe = Point::new(
            seam_point.x + travel.x * offset,
            seam_point.y + travel.y * offset,
        );
        let top = history.material_top_at(probe).unwrap();
        assert!(
            (top - -8.).abs() < 1e-6,
            "ramp region at ({:.2},{:.2}) cut through, got {top}",
            probe.x,
            probe.y
        );
    }
}

/// A blocked ramp is a located error, never a silent plunge: a manual tab on
/// the ramp span, and a ramp longer than one revolution, are both rejected.
#[test]
fn blocked_ramps_do_not_become_plunges() {
    // Pin the seam onto the bottom edge with an anchor start (fraction 0.10
    // of the 100 mm source ring = setup (15,5), projecting to loop (15,3)),
    // then place a manual tab anchor 3 mm further along: its restricted
    // interval overlaps the 30-degree ramp span.
    let document = job(
        RECT_SVG,
        profile_settings(
            ProfileEntry::Ramp {
                max_angle_deg: Some(30.),
                feed_mm_min: Some(150.),
            },
            LeadSpec::None,
            LeadSpec::None,
            StartSelection::Automatic,
            None,
            ProfileFinishSettings::default(),
            2.,
        ),
    );
    let catalogue = cam_core::contours::ContourCatalogue::build(&document).unwrap();
    let contour = catalogue.contour("pocket-0-outer").unwrap();
    let settings = profile_settings(
        ProfileEntry::Ramp {
            max_angle_deg: Some(30.),
            feed_mm_min: Some(150.),
        },
        LeadSpec::None,
        LeadSpec::None,
        StartSelection::Anchor(Box::new(ContourAnchor {
            contour_id: "pocket-0-outer".into(),
            source_geometry_fingerprint: contour.source_fingerprint.clone(),
            fraction_along_source_contour: 0.10,
        })),
        Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Rectangular,
            placement: TabPlacement::Manual {
                anchors: vec![ContourAnchor {
                    contour_id: "pocket-0-outer".into(),
                    source_geometry_fingerprint: contour.source_fingerprint.clone(),
                    fraction_along_source_contour: 0.13,
                }],
            },
        }),
        ProfileFinishSettings::default(),
        2.,
    );
    let plan = plan_of(RECT_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "PROFILE_RAMP_NO_SPACE")
        .expect("ramp diagnostic");
    assert!(
        diagnostic.message.contains("pocket-0-outer"),
        "{}",
        diagnostic.message
    );
    assert!(
        plan.motions.is_empty(),
        "a blocked ramp must not fall back to plunge motions"
    );

    // A ramp longer than the whole loop cannot wrap one revolution.
    let mut shallow = profile_settings(
        ProfileEntry::Ramp {
            max_angle_deg: Some(2.),
            feed_mm_min: Some(150.),
        },
        LeadSpec::None,
        LeadSpec::None,
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        2.,
    );
    shallow.contours = vec![ProfileContour {
        contour_id: "part-0-outer".into(),
        side: ContourSide::Outside,
        traversal: None,
    }];
    shallow.bottom = cam_core::project::HeightRef {
        reference: cam_core::project::HeightReference::StockTop,
        offset_mm: -4.,
    };
    let plan = plan_of(SMALL_RECT_SVG, shallow);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "PROFILE_RAMP_NO_SPACE")
        .expect("revolution diagnostic");
    assert!(
        diagnostic.message.contains("part-0-outer"),
        "{}",
        diagnostic.message
    );
}

/// Tangent line leads blend into and out of the contour at the traversal
/// seam with their own feeds, and the lead-in corridor removes material.
#[test]
fn tangent_line_leads_blend_with_their_own_feeds() {
    let settings = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::TangentLine {
            length_mm: Some(4.),
            feed_mm_min: Some(180.),
        },
        LeadSpec::TangentLine {
            length_mm: Some(3.),
            feed_mm_min: Some(200.),
        },
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    let plan = plan_of(RECT_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

    let layer: Vec<&cam_core::toolpath::PlannedMotion> =
        plan.motions.iter().filter(|m| m.layer == 0).collect();
    // The seam and travel direction are derived from the traversal itself
    // (the canonical start is a property of the rasterized offset ring).
    let first_cut = layer
        .iter()
        .find(|m| m.purpose == MotionPurpose::Rough)
        .expect("loop traversal");
    let seam = Point::new(first_cut.start.x, first_cut.start.y);
    let travel = {
        let d = Point::new(
            first_cut.end.x - first_cut.start.x,
            first_cut.end.y - first_cut.start.y,
        );
        let len = d.distance(Point::new(0., 0.));
        Point::new(d.x / len, d.y / len)
    };
    // The approach stands above the lead-in start: one lead length back along
    // the travel direction; one lead-in feeds into the seam; one lead-out
    // departs along the travel direction past the seam.
    let approach = layer
        .iter()
        .find(|m| m.purpose == MotionPurpose::Approach)
        .expect("approach");
    let lead_in: Vec<&&cam_core::toolpath::PlannedMotion> = layer
        .iter()
        .filter(|m| m.purpose == MotionPurpose::LeadIn)
        .collect();
    assert_eq!(lead_in.len(), 1);
    let lead_in = *lead_in[0];
    assert!((lead_in.feed_mm_min.unwrap() - 180.).abs() < 1e-9);
    let expected_start = Point::new(seam.x - travel.x * 4., seam.y - travel.y * 4.);
    assert!(
        Point::new(approach.end.x, approach.end.y).distance(expected_start) < 0.06,
        "lead-in start at the seam minus the lead along travel, got ({}, {})",
        approach.end.x,
        approach.end.y
    );
    assert!(
        Point::new(lead_in.end.x, lead_in.end.y).distance(seam) < 0.06,
        "lead-in ends at the seam"
    );
    assert!(
        (Point::new(lead_in.start.x, lead_in.start.y)
            .distance(Point::new(lead_in.end.x, lead_in.end.y))
            - 4.)
            .abs()
            < 0.06
    );
    assert!((lead_in.end.z - lead_in.start.z).abs() < 1e-9);
    let lead_out: Vec<&&cam_core::toolpath::PlannedMotion> = layer
        .iter()
        .filter(|m| m.purpose == MotionPurpose::LeadOut)
        .collect();
    assert_eq!(lead_out.len(), 1);
    let lead_out = *lead_out[0];
    assert!((lead_out.feed_mm_min.unwrap() - 200.).abs() < 1e-9);
    assert!(
        Point::new(lead_out.start.x, lead_out.start.y).distance(seam) < 0.06,
        "lead-out attaches at the traversal end (the seam without a ramp)"
    );
    assert!(
        (Point::new(lead_out.start.x, lead_out.start.y)
            .distance(Point::new(lead_out.end.x, lead_out.end.y))
            - 3.)
            .abs()
            < 0.06
    );
    // The retract happens after the lead-out, not at the seam.
    let retract = layer
        .iter()
        .find(|m| m.purpose == MotionPurpose::Clearance)
        .expect("retract");
    assert!((retract.start.x - lead_out.end.x).abs() < 1e-9);
    // The lead-in corridor actually removes stock down to the cut bottom.
    let history = plan.stock_history(None).unwrap();
    let probe = Point::new(seam.x - travel.x * 2., seam.y - travel.y * 2.);
    let top = history.material_top_at(probe).unwrap();
    assert!(
        (top - -8.).abs() < 1e-6,
        "lead-in corridor at ({:.2},{:.2}) cut through, got {top}",
        probe.x,
        probe.y
    );
}

/// The plan-section-19.1 narrow-entry fixture: an inside circle cut has no
/// room beside its wall, so a tangent line lead is rejected with a located
/// error instead of gouging the retained material.
#[test]
fn narrow_invalid_entry_is_rejected() {
    let mut settings = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::TangentLine {
            length_mm: Some(4.),
            feed_mm_min: Some(180.),
        },
        LeadSpec::None,
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    settings.contours = vec![ProfileContour {
        contour_id: "cut-0-outer".into(),
        side: ContourSide::Inside,
        traversal: None,
    }];
    settings.bottom = cam_core::project::HeightRef {
        reference: cam_core::project::HeightReference::StockTop,
        offset_mm: -4.,
    };
    let plan = plan_of(CIRCLE_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "PROFILE_LEAD_NO_SPACE")
        .expect("lead diagnostic");
    assert!(
        diagnostic.message.contains("cut-0-outer") && diagnostic.message.contains("lead-in"),
        "{}",
        diagnostic.message
    );
    assert!(plan.motions.is_empty());
}

/// Tangent arc leads arrive tangent at the seam, bulge to the scrap side,
/// stay within the chord-error budget, and never touch the retained wall.
#[test]
fn arc_leads_bulge_to_the_scrap_side_within_tolerance() {
    let mut settings = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::TangentArc {
            radius_mm: Some(5.),
            sweep_deg: Some(90.),
            feed_mm_min: Some(170.),
        },
        LeadSpec::TangentArc {
            radius_mm: Some(5.),
            sweep_deg: Some(90.),
            feed_mm_min: Some(190.),
        },
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    settings.contours = vec![ProfileContour {
        contour_id: "cut-0-outer".into(),
        side: ContourSide::Outside,
        traversal: None,
    }];
    settings.bottom = cam_core::project::HeightRef {
        reference: cam_core::project::HeightReference::StockTop,
        offset_mm: -4.,
    };
    let plan = plan_of(CIRCLE_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

    let layer: Vec<&cam_core::toolpath::PlannedMotion> =
        plan.motions.iter().filter(|m| m.layer == 0).collect();
    let lead_in: Vec<&&cam_core::toolpath::PlannedMotion> = layer
        .iter()
        .filter(|m| m.purpose == MotionPurpose::LeadIn)
        .collect();
    assert!(lead_in.len() >= 2, "arc leads linearize to polylines");
    assert!(
        lead_in
            .iter()
            .all(|m| (m.feed_mm_min.unwrap() - 170.).abs() < 1e-9)
    );
    // Tangent continuity into the seam: the last lead-in segment direction
    // matches the loop's travel direction at the seam.
    let seam = Point::new(lead_in.last().unwrap().end.x, lead_in.last().unwrap().end.y);
    let first_cut = layer
        .iter()
        .find(|m| m.purpose == MotionPurpose::Rough)
        .expect("loop traversal");
    let travel = Point::new(
        first_cut.end.x - first_cut.start.x,
        first_cut.end.y - first_cut.start.y,
    );
    let travel_len = travel.distance(Point::new(0., 0.));
    let travel = Point::new(travel.x / travel_len, travel.y / travel_len);
    let last_seg = lead_in.last().unwrap();
    let seg = Point::new(
        last_seg.end.x - last_seg.start.x,
        last_seg.end.y - last_seg.start.y,
    );
    let seg_len = seg.distance(Point::new(0., 0.));
    let seg = Point::new(seg.x / seg_len, seg.y / seg_len);
    assert!(
        seg.x * travel.x + seg.y * travel.y > 0.99,
        "lead-in arrives tangent to the travel direction"
    );
    // The arc center sits one radius to the scrap side of the seam; every
    // vertex and chord midpoint lies on that circle within the chord budget
    // and no closer to the part wall than the cutter radius.
    let scrap = Point::new(travel.y, -travel.x); // right of travel = outside
    let radius = 5f64;
    let center = Point::new(seam.x + scrap.x * radius, seam.y + scrap.y * radius);
    let mut vertices = vec![Point::new(lead_in[0].start.x, lead_in[0].start.y)];
    for motion in &lead_in {
        vertices.push(Point::new(motion.end.x, motion.end.y));
    }
    for window in vertices.windows(2) {
        let midpoint = Point::new(
            (window[0].x + window[1].x) / 2.,
            (window[0].y + window[1].y) / 2.,
        );
        let error = (midpoint.distance(center) - radius).abs();
        assert!(
            error <= 0.005 + 1e-9,
            "chord error {error} within the linearization budget"
        );
    }
    let part_center = Point::new(25., 25.);
    for vertex in &vertices {
        let wall = vertex.distance(part_center) - 20.;
        assert!(
            wall >= 2. - 0.06,
            "arc lead stays no closer to the retained wall than the loop itself (got {wall:.3})"
        );
    }
}

/// An anchor start moves the seam to the source-geometry anchor: leads and
/// the first cut align with the new start, and machining is unchanged.
#[test]
fn anchor_start_moves_the_seam() {
    let document = job(
        RECT_SVG,
        profile_settings(
            ProfileEntry::Plunge,
            LeadSpec::None,
            LeadSpec::None,
            StartSelection::Automatic,
            None,
            ProfileFinishSettings::default(),
            4.,
        ),
    );
    let catalogue = cam_core::contours::ContourCatalogue::build(&document).unwrap();
    let contour = catalogue.contour("pocket-0-outer").unwrap();
    let anchor = ContourAnchor {
        contour_id: "pocket-0-outer".into(),
        source_geometry_fingerprint: contour.source_fingerprint.clone(),
        // Fraction 0.15 of the 100 mm source ring: 15 mm along the bottom
        // edge, i.e. setup (20, 5) with the +X tangent.
        fraction_along_source_contour: 0.15,
    };
    let resolved = catalogue.resolve_anchor(&anchor).unwrap();
    assert!((resolved.point.x - 20.).abs() < 1e-6 && (resolved.point.y - 5.).abs() < 1e-6);

    let settings = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::TangentLine {
            length_mm: Some(3.),
            feed_mm_min: Some(180.),
        },
        LeadSpec::None,
        StartSelection::Anchor(Box::new(anchor)),
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    let plan = plan_of(RECT_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);
    // The lead-in ends at the anchor's projected loop point (20, 3) and the
    // traversal continues along the edge (+X), not the canonical corner.
    let layer: Vec<&cam_core::toolpath::PlannedMotion> =
        plan.motions.iter().filter(|m| m.layer == 0).collect();
    let lead_in = layer
        .iter()
        .find(|m| m.purpose == MotionPurpose::LeadIn)
        .expect("lead-in");
    assert!(
        Point::new(lead_in.end.x, lead_in.end.y).distance(Point::new(20., 3.)) < 0.06,
        "seam moved to the anchor, got ({}, {})",
        lead_in.end.x,
        lead_in.end.y
    );
    assert!(lead_in.end.x - lead_in.start.x > 2.9, "arrival along +X");

    // Foreign anchors and stale fingerprints are located errors.
    let mut foreign = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::None,
        LeadSpec::None,
        StartSelection::Anchor(Box::new(ContourAnchor {
            contour_id: "ghost-0-outer".into(),
            source_geometry_fingerprint: "stale".into(),
            fraction_along_source_contour: 0.1,
        })),
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    foreign.contours = vec![ProfileContour {
        contour_id: "pocket-0-outer".into(),
        side: ContourSide::Outside,
        traversal: None,
    }];
    let bad = plan_of(RECT_SVG, foreign);
    assert_eq!(
        bad.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        bad.generation_diagnostics
            .iter()
            .any(|d| d.code == "CONTOUR_REFERENCE")
    );
    let mut stale = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::None,
        LeadSpec::None,
        StartSelection::Anchor(Box::new(ContourAnchor {
            contour_id: "pocket-0-outer".into(),
            source_geometry_fingerprint: "stale".into(),
            fraction_along_source_contour: 0.1,
        })),
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    stale.contours = vec![ProfileContour {
        contour_id: "pocket-0-outer".into(),
        side: ContourSide::Outside,
        traversal: None,
    }];
    let bad = plan_of(RECT_SVG, stale);
    assert_eq!(
        bad.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        bad.generation_diagnostics
            .iter()
            .any(|d| d.code == "CONTOUR_ANCHOR_UNRESOLVED")
    );
}

/// The plan-section-19.1 combined fixture: tabs + finish + ramp + leads —
/// every deep rough and finish pass protects the tabs, no lead or ramp
/// passes through a bridge, and the program exports through readback.
#[test]
fn tabs_finish_ramp_and_leads_coexist() {
    // A 45-degree ramp (2 mm over 2 mm) completes inside the 3 mm lead-in,
    // so the descent, the lead-in remainder at depth, and the lead-out all
    // carry their own feeds into the exported program.
    let settings = profile_settings(
        ProfileEntry::Ramp {
            max_angle_deg: Some(45.),
            feed_mm_min: Some(140.),
        },
        LeadSpec::TangentLine {
            length_mm: Some(3.),
            feed_mm_min: Some(180.),
        },
        LeadSpec::TangentLine {
            length_mm: Some(2.),
            feed_mm_min: Some(200.),
        },
        StartSelection::Automatic,
        Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Rectangular,
            placement: TabPlacement::Automatic {
                count: Some(2),
                spacing_mm: None,
            },
        }),
        ProfileFinishSettings {
            enabled: true,
            radial_allowance_mm: Some(0.5),
            feed_mm_min: Some(300.),
        },
        2.,
    );
    let plan = plan_of(RECT_SVG, settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

    // Both stages still protect the bridges at the tab top in deep passes.
    let stages: Vec<&str> = plan
        .stages
        .iter()
        .map(|stage| stage.stage_id.as_str())
        .collect();
    assert_eq!(
        stages,
        vec!["profile-1-profile-rough", "profile-1-profile-finish"]
    );
    for stage in &plan.stages {
        let transitions = plan
            .motions
            .iter()
            .filter(|m| m.stage_id == stage.stage_id && m.purpose == MotionPurpose::TabTransition)
            .count();
        assert!(
            transitions > 0,
            "{} carries tab transitions",
            stage.stage_id
        );
    }

    // No entry, ramp or lead motion crosses a protected bridge: sample each
    // motion's XY path against the resolved bridge footprints.
    let placements = &plan.operation_results[0]
        .named_outputs
        .iter()
        .find(|o| o.kind == "profile_tabs")
        .expect("tab placements")
        .tab_placements;
    assert_eq!(placements.len(), 2);
    for motion in &plan.motions {
        if !matches!(
            motion.purpose,
            MotionPurpose::Entry | MotionPurpose::LeadIn | MotionPurpose::LeadOut
        ) {
            continue;
        }
        let steps = (Point::new(motion.start.x, motion.start.y)
            .distance(Point::new(motion.end.x, motion.end.y))
            / 0.2)
            .ceil() as usize;
        for step in 0..=steps.max(1) {
            let t = step as f64 / steps.max(1) as f64;
            let point = Point::new(
                motion.start.x + (motion.end.x - motion.start.x) * t,
                motion.start.y + (motion.end.y - motion.start.y) * t,
            );
            for placement in placements {
                let footprint = &placement.footprint_mm;
                assert_eq!(footprint.len(), 4, "resolved footprints are drawable quads");
                assert!(
                    !point_in_quad(point, footprint),
                    "{:?} motion {} crosses the bridge of {}",
                    motion.purpose,
                    motion.id,
                    placement.contour_id
                );
            }
        }
    }

    // The composed stock history holds both bridges at the tab top while the
    // other sides cut through, exactly like the tab-only fixture.
    let history = plan.stock_history(None).unwrap();
    let top_at = |x: f64, y: f64| history.material_top_at(Point::new(x, y)).unwrap();
    let mut held = 0;
    for &(x, y, dx, dy) in &[
        (3.5, 15., 0., 1.),
        (36.5, 15., 0., -1.),
        (20., 3.5, -1., 0.),
        (20., 26.5, 1., 0.),
    ] {
        let depth = top_at(x, y);
        if (depth - -6.).abs() < 1e-6 {
            held += 1;
            assert!(
                (top_at(x + dx * 1.75, y + dy * 1.75) - -6.).abs() < 1e-6,
                "bridge interior holds at the tab top"
            );
        } else {
            assert!(
                (depth - -8.).abs() < 1e-6,
                "untabbed side cut through, got {depth}"
            );
        }
    }
    assert_eq!(held, 2, "exactly the two tabbed sides hold");

    // The mixed plan exports through the independent numeric readback with
    // its ramp, lead and finishing feeds preserved.
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
    for feed in [140., 180., 200., 300.] {
        assert!(
            export.program.gcode.contains(&format!("F{feed}")),
            "feed {feed} survives into the program"
        );
    }
}

/// Automatic placement leaves the entry/lead neighborhood free; an
/// impossible count under that restriction reports the reason.
#[test]
fn automatic_tabs_exclude_the_entry_neighborhood() {
    let ramp_len = 2. / 20f64.to_radians().tan();
    let settings = profile_settings(
        ProfileEntry::Ramp {
            max_angle_deg: Some(20.),
            feed_mm_min: Some(140.),
        },
        LeadSpec::TangentLine {
            length_mm: Some(2.),
            feed_mm_min: Some(180.),
        },
        LeadSpec::TangentLine {
            length_mm: Some(3.),
            feed_mm_min: Some(200.),
        },
        StartSelection::Automatic,
        Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Rectangular,
            placement: TabPlacement::Automatic {
                count: Some(4),
                spacing_mm: None,
            },
        }),
        ProfileFinishSettings::default(),
        2.,
    );
    let plan = plan_of(RECT_SVG, settings.clone());
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
    assert_eq!(placements.len(), 4);
    let ramp_on_loop = ramp_len - 2.;
    let lead_out = 3.;
    let lead_in = 2.;
    let perimeter = 116.;
    for placement in placements {
        let restricted = (placement.restricted_start_mm, placement.restricted_end_mm);
        let overlaps = |interval: (f64, f64)| {
            restricted.0 < interval.1 - 1e-9 && restricted.1 > interval.0 + 1e-9
        };
        assert!(
            !overlaps((0., ramp_on_loop + lead_out + 0.01)),
            "placements keep the ramp/lead-out prefix free"
        );
        assert!(
            !overlaps((perimeter - lead_in - 0.01, perimeter)),
            "placements keep the lead-in tail free"
        );
    }

    // An impossible count under the exclusion reports the neighborhood, not
    // a silently smaller set of tabs.
    let mut crowded = settings.clone();
    crowded.tabs = Some(TabSettings {
        height_mm: Some(2.),
        width_mm: Some(5.),
        shape: TabShape::Rectangular,
        placement: TabPlacement::Automatic {
            count: Some(500),
            spacing_mm: None,
        },
    });
    let plan = plan_of(RECT_SVG, crowded);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_TAB_NO_SPACE" && d.message.contains("entry/lead"))
    );
}

/// Missing ramp/lead values and wrong arc sides are located before planning.
#[test]
fn missing_entry_and_lead_values_are_located() {
    let ramp_no_values = profile_settings(
        ProfileEntry::Ramp {
            max_angle_deg: None,
            feed_mm_min: None,
        },
        LeadSpec::None,
        LeadSpec::None,
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        2.,
    );
    let document = job_with(RECT_SVG, ramp_no_values, endmill(None));
    let OperationSettings::Profile(settings) = &document.operations[0].settings else {
        unreachable!()
    };
    let missing = cam_core::operations::profile::missing_fields(&document, "profile-1", settings);
    for path in [
        "entry.max_angle_deg",
        "entry.feed_mm_min",
        "capabilities.ramp_capable",
    ] {
        assert!(
            missing
                .iter()
                .any(|d| d.field_path.as_deref().unwrap_or("").contains(path)),
            "missing {path}"
        );
    }

    let lead_no_values = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::TangentLine {
            length_mm: None,
            feed_mm_min: None,
        },
        LeadSpec::TangentArc {
            radius_mm: None,
            sweep_deg: None,
            feed_mm_min: None,
        },
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        2.,
    );
    let document = job(RECT_SVG, lead_no_values);
    let OperationSettings::Profile(settings) = &document.operations[0].settings else {
        unreachable!()
    };
    let missing = cam_core::operations::profile::missing_fields(&document, "profile-1", settings);
    for path in [
        "lead_in.length_mm",
        "lead_in.feed_mm_min",
        "lead_out.radius_mm",
        "lead_out.sweep_deg",
        "lead_out.feed_mm_min",
    ] {
        assert!(
            missing
                .iter()
                .any(|d| d.field_path.as_deref().unwrap_or("").contains(path)),
            "missing {path}"
        );
    }

    // Arc leads need a retained side; on-contour selections reject them.
    let mut on_contour = profile_settings(
        ProfileEntry::Plunge,
        LeadSpec::TangentArc {
            radius_mm: Some(5.),
            sweep_deg: Some(90.),
            feed_mm_min: Some(170.),
        },
        LeadSpec::None,
        StartSelection::Automatic,
        None,
        ProfileFinishSettings::default(),
        4.,
    );
    on_contour.contours = vec![ProfileContour {
        contour_id: "cut-0-outer".into(),
        side: ContourSide::On,
        traversal: Some(TraversalDirection::Forward),
    }];
    on_contour.direction = None;
    on_contour.bottom = cam_core::project::HeightRef {
        reference: cam_core::project::HeightReference::StockTop,
        offset_mm: -4.,
    };
    let plan = plan_of(CIRCLE_SVG, on_contour);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_LEAD_SIDE")
    );
}

fn point_in_quad(point: Point, quad: &[(f64, f64)]) -> bool {
    // Winding test with a small inward shrink so boundary contact does not
    // count as crossing.
    let mut centroid = Point::new(0., 0.);
    for corner in quad {
        centroid.x += corner.0 / 4.;
        centroid.y += corner.1 / 4.;
    }
    let shrink = 0.2f64;
    let shrunk: Vec<Point> = quad
        .iter()
        .map(|corner| {
            let corner = Point::new(corner.0, corner.1);
            Point::new(
                corner.x + (centroid.x - corner.x) * shrink,
                corner.y + (centroid.y - corner.y) * shrink,
            )
        })
        .collect();
    let mut inside = false;
    let n = shrunk.len();
    for index in 0..n {
        let a = shrunk[index];
        let b = shrunk[(index + 1) % n];
        if (a.y > point.y) != (b.y > point.y) {
            let cross = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if point.x < cross {
                inside = !inside;
            }
        }
    }
    inside
}
