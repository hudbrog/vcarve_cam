//! GUI8: a closed-contour profile operation — explicit sides and depths,
//! generated passes, actual heightfield playback, checked export and portable
//! reopen.
use cam_core::project::{
    ContourSide, CutDirection, SpindleDirection, TabShape,
    v5::{self, CamJobV5, OperationSettingsV5},
};
use cam_gui_runtime::{
    app,
    operation_authoring::{self, Kind},
    profile::{self, AnchorKind, TabAnchorAction},
    session::{self, Command},
};
use cam_service::retained::Retained;

/// The established lettering artwork (an L and a holed O) profiled at 1 mm per
/// pass through 6 mm stock with a 3 mm endmill.
fn profile_job() -> CamJobV5 {
    let mut job = profile::import_svg(
        "letters.svg".into(),
        include_str!("../../../fixtures/gui3/lettering.svg").into(),
    )
    .unwrap();
    let id = job.operations[0].id.clone();
    // Every closed boundary, with the importer's advisory side (outside for
    // outer boundaries, inside for holes).
    let rows = profile::contours(&job)
        .unwrap()
        .into_iter()
        .map(|contour| profile::SelectionRow {
            reference: contour.reference,
            side: contour.suggested_side,
            traversal: None,
        })
        .collect::<Vec<_>>();
    job = profile::select_in(&job, &id, &rows).unwrap();
    for (field, value) in [
        (6, 6.),    // stock thickness
        (7, 5.),    // clearance above stock
        (2, 300.),  // cutting feed
        (10, 100.), // plunge feed
        (11, 12_000.),
        (8, 1.),   // stepdown
        (88, 1.),  // tool stepdown limit
        (90, -6.), // bottom: stock bottom (cut through)
    ] {
        job = app::set_value(&job, &id, field, Some(value))
            .unwrap_or_else(|error| panic!("field {field}: {error}"));
    }
    let mut candidate = job.clone();
    cam_gui_runtime::authoring::set_group_in(&mut candidate, &id, 12, &[3., 8.]).unwrap();
    job = candidate;
    profile::set_direction(&mut job, &id, Some(CutDirection::Climb)).unwrap();
    profile::set_spindle_direction(&mut job, &id, Some(SpindleDirection::Clockwise)).unwrap();
    job.validate_structure().unwrap();
    // Checked export needs one applied machine configuration; the reviewed
    // fixture profile maps the endmill this profile uses.
    let (applied, _) = session::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: job.to_json().unwrap(),
            json: session::PROFILE.into(),
        },
    )
    .unwrap();
    session::open(&applied.job).unwrap()
}

/// Generate through one retained service instance; its plan handles belong to
/// that instance and preparing with a different one is refused by design.
fn generate(service: &mut Retained, job: &CamJobV5) -> cam_gui_runtime::compute::SceneMeta {
    session::execute(service, Command::generate(job.to_json().unwrap()))
        .unwrap()
        .0
}

/// The same generation, keeping the transported payload so the test can read
/// the motion stream exactly as the simulator displays it.
fn generate_scene(service: &mut Retained, job: &CamJobV5) -> cam_gui_runtime::compute::Scene {
    let (meta, payload) =
        session::execute(service, Command::generate(job.to_json().unwrap())).unwrap();
    cam_gui_runtime::compute::Scene {
        meta,
        payload: std::sync::Arc::new(payload),
    }
}

/// The displayed motion stream of a generated scene.
fn stream(scene: &cam_gui_runtime::compute::Scene) -> Vec<cam_gui_runtime::sim::Motion> {
    scene
        .sim_input()
        .expect("a decoded simulation")
        .unwrap_or_else(|| {
            panic!(
                "the ramped job produced no simulation: {}",
                scene.meta.report["gui2"]["generationIssues"]
            )
        })
        .motions
}

/// The largest outer contour, which has room for an entry away from a corner.
fn widest_outer(job: &CamJobV5) -> profile::Contour {
    profile::contours(job)
        .unwrap()
        .into_iter()
        .filter(|contour| contour.role == "outer")
        .max_by(|a, b| a.perimeter_mm.total_cmp(&b.perimeter_mm))
        .unwrap()
}

/// The same job with only its outer contours selected: a small hole has no room
/// for a tab, a ramp or a lead, and the planner says so rather than cutting an
/// unprotected part.
fn outers_only(job: &CamJobV5) -> CamJobV5 {
    let id = job.operations[0].id.clone();
    let contours = profile::contours(job).unwrap();
    let rows = profile::selection(job, &id)
        .into_iter()
        .filter(|row| {
            contours
                .iter()
                .any(|contour| contour.reference == row.reference && contour.role == "outer")
        })
        .collect::<Vec<_>>();
    profile::set_selection_rows(job, &id, &rows).unwrap()
}

/// Removed volume at the end of the displayed stock playback.
fn removed(meta: &cam_gui_runtime::compute::SceneMeta) -> f64 {
    meta.stock
        .as_ref()
        .unwrap()
        .frames
        .last()
        .unwrap()
        .stats
        .removed_volume_mm3
}

/// The generated tab placements of the only operation in the plan.
fn placements(meta: &cam_gui_runtime::compute::SceneMeta) -> &Vec<serde_json::Value> {
    meta.report["gui2"]["inspection"]["operations"][0]["tabPlacements"]
        .as_array()
        .unwrap()
}

#[test]
fn profile_job_generates_playback_output_and_reopens() {
    let job = profile_job();
    let mut service = Retained::new();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert!(meta.motions > 0, "a profile produces real motions");
    assert_eq!(report["checks"]["exportReady"], true, "{report}");
    assert_eq!(report["groups"][0]["role"], "Profile rough");
    assert_eq!(report["groups"][0]["jump"], "After profile rough");
    // The resolved heights come from the plan: stock top down to the bottom
    // reference, six passes below.
    let heights = &report["inspection"]["operations"][0]["heights"];
    assert!(
        (heights["topZMm"].as_f64().unwrap() - 0.).abs() < 1e-9,
        "{heights}"
    );
    assert!(
        (heights["bottomZMm"].as_f64().unwrap() - -6.).abs() < 1e-9,
        "{heights}"
    );
    // Real material is removed, and the contour band is what changed.
    let preview = meta.stock.as_ref().unwrap();
    assert_eq!(preview.frames[0].stats.removed_volume_mm3, 0.);
    let removed = preview.frames.last().unwrap().stats.removed_volume_mm3;
    assert!(removed > 0., "removed {removed} mm³");
    // Export the retained prefix and reopen the saved document.
    let handle = report["handle"].as_str().unwrap().to_owned();
    let (prepared, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: job.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    assert!(
        prepared.report["gui2"]["file"]["gcode"]
            .as_str()
            .is_some_and(|gcode| gcode.lines().count() > 5),
        "{}",
        prepared.report["gui2"]["file"]
    );
    let reopened = session::open(&prepared.job).unwrap();
    assert_eq!(reopened.operations.len(), 1);
    let OperationSettingsV5::Profile(settings) = &reopened.operations[0].settings else {
        panic!("the reopened operation is a profile");
    };
    assert_eq!(settings.contours.len(), 3, "two outers and one hole");
    assert!(
        settings
            .contours
            .iter()
            .any(|c| c.side == ContourSide::Inside),
        "the O's hole is cut inside"
    );
    assert_eq!(settings.direction, Some(CutDirection::Climb));
}

#[test]
fn profile_side_depth_and_through_allowance_change_the_cut() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let mut service = Retained::new();
    let outside = generate(&mut service, &job);
    let outside_removed = outside
        .stock
        .as_ref()
        .unwrap()
        .frames
        .last()
        .unwrap()
        .stats
        .removed_volume_mm3;
    // Cutting the same contours from the inside leaves the material on the
    // other side of the boundary, so the removed volume changes.
    let mut inside = job.clone();
    let rows = profile::selection(&inside, &id)
        .into_iter()
        .map(|row| profile::SelectionRow {
            side: ContourSide::Inside,
            ..row
        })
        .collect::<Vec<_>>();
    inside = profile::set_selection_rows(&inside, &id, &rows).unwrap();
    let inside = generate(&mut service, &inside);
    let inside_removed = inside
        .stock
        .as_ref()
        .unwrap()
        .frames
        .last()
        .unwrap()
        .stats
        .removed_volume_mm3;
    assert!(
        (inside_removed - outside_removed).abs() > 1.,
        "inside {inside_removed} vs outside {outside_removed} mm³"
    );
    // A deeper bottom than the through-cut allowance permits is rejected with
    // a located reason instead of silently cutting air.
    let mut below = job.clone();
    below = app::set_value(&below, &id, 90, Some(-8.)).unwrap();
    let meta = generate(&mut service, &below);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], false);
    assert!(
        report["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_THROUGH_ALLOWANCE")),
        "{}",
        report["generationIssues"]
    );
    // The allowance permits travel past the stock bottom and restores export.
    let mut allowed = below.clone();
    allowed = app::set_value(&allowed, &id, 70, Some(2.)).unwrap();
    let meta = generate(&mut service, &allowed);
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], true);
}

#[test]
fn an_incomplete_profile_reports_its_own_missing_fields() {
    // A freshly created profile operation has no selection and no values.
    let empty = operation_authoring::empty_job();
    let job = operation_authoring::apply(&empty, operation_authoring::add(Kind::Profile, &empty))
        .unwrap();
    let id = job.operations[0].id.clone();
    assert!(profile::selection(&job, &id).is_empty());
    let issues = v5::inspection::inspect_profile_fields(&job, &id).unwrap();
    for path in [
        "contours",
        "stepdown_mm",
        "assignment.cutting_feed_mm_min",
        "assignment.plunge_feed_mm_min",
        "assignment.spindle_rpm",
        "assignment.max_stepdown_mm",
        "assignment.spindle_direction",
    ] {
        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path.as_deref()
                    == Some(&format!("operations[{id}].{path}"))),
            "missing {path} in {issues:?}"
        );
    }
    // Direction becomes required once a selection retains a side.
    let carved = profile_job();
    let carved_id = carved.operations[0].id.clone();
    let mut carved = carved;
    profile::set_direction(&mut carved, &carved_id, None).unwrap();
    let issues = v5::inspection::inspect_profile_fields(&carved, &carved_id).unwrap();
    assert!(
        issues.iter().any(|issue| issue
            .field_path
            .as_deref()
            .is_some_and(|path| path.ends_with(".direction"))),
        "direction is required for a retained side: {issues:?}"
    );
    // Selection is refused for a contour the catalogue does not carry.
    let stale = profile::SelectionRow {
        reference: v5::GeometryRef {
            artwork_item_id: v5::ArtworkItemId("artwork-1".into()),
            kind: v5::GeometryRefKind::ClosedContour,
            local_geometry_id: "letter-l::nope".into(),
            source_revision: v5::SourceRevision {
                content_digest: "not-the-current-source".into(),
                algorithm_version: 1,
            },
        },
        side: ContourSide::Outside,
        traversal: None,
    };
    assert!(profile::select_in(&job, &id, &[stale]).is_err());
}

#[test]
fn tabs_hold_material_and_follow_their_manual_anchors() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let mut service = Retained::new();
    // Plain through cut: every contour is cut free.
    let through = generate(&mut service, &job);
    let through_removed = removed(&through);
    assert!(placements(&through).is_empty());

    // Automatic tabs on every contour: 0.5 mm high and 3 mm wide. Four tabs do
    // not fit the letter O's small hole, and the planner says so with a located
    // reason instead of cutting an unprotected part.
    let mut tabbed = job.clone();
    profile::set_tabs_enabled(&mut tabbed, &id, true).unwrap();
    for (field, value) in [(93, 0.5), (94, 3.)] {
        tabbed = app::set_value(&tabbed, &id, field, Some(value)).unwrap();
    }
    let meta = generate(&mut service, &tabbed);
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], false);
    assert!(
        meta.report["gui2"]["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_TAB_NO_SPACE")),
        "{}",
        meta.report["gui2"]["generationIssues"]
    );

    // Tabs on the outer contours only: four bridges each, which is what the
    // automatic placement is for.
    let outers = profile::selection(&job, &id)
        .into_iter()
        .filter(|row| {
            profile::contours(&job)
                .unwrap()
                .iter()
                .any(|contour| contour.reference == row.reference && contour.role == "outer")
        })
        .collect::<Vec<_>>();
    let mut tabbed = profile::set_selection_rows(&job, &id, &outers).unwrap();
    let plain = generate(&mut service, &tabbed);
    let plain_removed = removed(&plain);
    profile::set_tabs_enabled(&mut tabbed, &id, true).unwrap();
    for (field, value) in [(93, 0.5), (94, 3.)] {
        tabbed = app::set_value(&tabbed, &id, field, Some(value)).unwrap();
    }
    let meta = generate(&mut service, &tabbed);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{report}");
    let placed = placements(&meta).clone();
    assert!(!placed.is_empty(), "the planner placed tabs: {report}");
    for placement in &placed {
        assert_eq!(
            placement["footprintMm"].as_array().unwrap().len(),
            4,
            "each bridge publishes its exact protected quad: {placement}"
        );
        assert!(
            placement["topZMm"].as_f64().unwrap() < 0.,
            "the bridge top is below the stock top: {placement}"
        );
    }
    // The bridges are real: less material is removed than the free cut.
    assert!(
        removed(&meta) < plain_removed,
        "tabs retain material: {} vs {plain_removed} mm³",
        removed(&meta)
    );
    // Cutting every contour free removes more material than the tabbed outer
    // contours alone.
    assert!(plain_removed < through_removed);

    // Manual anchors: one tab on the first outer contour, moved and removed by
    // the same commands the numeric row and the drag use.
    // The largest outer contour has room for a manual tab away from its
    // corners; a manual anchor at a corner is refused by the planner.
    let outer = profile::contours(&job)
        .unwrap()
        .into_iter()
        .filter(|contour| contour.role == "outer")
        .max_by(|a, b| a.perimeter_mm.total_cmp(&b.perimeter_mm))
        .unwrap();
    let mut manual = profile::set_selection_rows(&job, &id, &outers).unwrap();
    profile::set_tabs_enabled(&mut manual, &id, true).unwrap();
    profile::set_tab_placement_mode(&mut manual, &id, false).unwrap();
    for (field, value) in [(93, 0.5), (94, 2.)] {
        manual = app::set_value(&manual, &id, field, Some(value)).unwrap();
    }
    manual = profile::tab_anchor(
        &manual,
        &id,
        &TabAnchorAction::Add {
            wire_id: outer.wire_id.clone(),
            fraction: 0.125,
        },
    )
    .unwrap();
    let rows = profile::tab_rows(&manual, &id);
    assert_eq!(rows.len(), 1, "one manual anchor");
    assert!(rows[0].resolved, "the anchor resolves against its source");
    assert_eq!(rows[0].scope, outer.wire_id);
    // The same anchor cannot be added twice to one contour.
    assert!(
        profile::tab_anchor(
            &manual,
            &id,
            &TabAnchorAction::Add {
                wire_id: outer.wire_id.clone(),
                fraction: 0.4,
            },
        )
        .is_err()
    );
    let first = placements(&generate(&mut service, &manual)).clone();
    assert_eq!(first.len(), 1);
    // Moving the anchor moves the generated bridge.
    profile::set_anchor_fraction(&mut manual, &id, AnchorKind::Tab, &outer.wire_id, 0.625).unwrap();
    let moved = placements(&generate(&mut service, &manual)).clone();
    assert_eq!(moved.len(), 1);
    assert_ne!(
        first[0]["bridgeStartMm"], moved[0]["bridgeStartMm"],
        "the anchor's fraction decides where the bridge is"
    );
    // Removing the anchor returns the operation to a free cut.
    manual = profile::tab_anchor(
        &manual,
        &id,
        &TabAnchorAction::Remove {
            scope: outer.wire_id.clone(),
        },
    )
    .unwrap();
    assert!(placements(&generate(&mut service, &manual)).is_empty());

    // Rejections stay located: an unsupported shape and a tab that leaves no
    // stock below it.
    let mut ramped = tabbed.clone();
    profile::set_tab_shape(&mut ramped, &id, TabShape::Ramped).unwrap();
    let meta = generate(&mut service, &ramped);
    assert!(
        meta.report["gui2"]["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_TAB_SHAPE_UNSUPPORTED")),
        "{}",
        meta.report["gui2"]["generationIssues"]
    );
    let mut tall = tabbed.clone();
    tall = app::set_value(&tall, &id, 93, Some(6.)).unwrap();
    let meta = generate(&mut service, &tall);
    assert!(
        meta.report["gui2"]["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_TAB_RANGE")),
        "{}",
        meta.report["gui2"]["generationIssues"]
    );
}

#[test]
fn a_replaced_source_leaves_tab_anchors_unresolved_until_reattached() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let outer = profile::contours(&job)
        .unwrap()
        .into_iter()
        .find(|contour| contour.role == "outer")
        .unwrap();
    let mut manual = job.clone();
    profile::set_tabs_enabled(&mut manual, &id, true).unwrap();
    profile::set_tab_placement_mode(&mut manual, &id, false).unwrap();
    for (field, value) in [(93, 0.5), (94, 2.)] {
        manual = app::set_value(&manual, &id, field, Some(value)).unwrap();
    }
    manual = profile::tab_anchor(
        &manual,
        &id,
        &TabAnchorAction::Add {
            wire_id: outer.wire_id.clone(),
            fraction: 0.5,
        },
    )
    .unwrap();
    // Replacing the source keeps the anchor's stored fingerprint: the row
    // reports itself unresolved instead of silently following the new shape.
    let mut service = Retained::new();
    let (replaced, _) = session::execute(
        &mut service,
        Command::Artwork {
            job: manual.to_json().unwrap(),
            operation_id: id.clone(),
            action: session::ArtworkCommand::Replace {
                item: v5::ArtworkItemId("artwork-1".into()),
                filename: "letters-edited.svg".into(),
                svg: include_str!("../../../fixtures/gui3/lettering.svg")
                    .replace(r#"d="M2 4H7V17H13V22H2Z""#, r#"d="M2 4H7V17H13V24H2Z""#),
            },
        },
    )
    .unwrap();
    let replaced = session::open(&replaced.job).unwrap();
    let rows = profile::tab_rows(&replaced, &id);
    assert_eq!(rows.len(), 1);
    assert!(
        !rows[0].resolved,
        "the stored fingerprint no longer matches"
    );
    // The generation pre-check reports the unresolved anchor as a located
    // blocker (with the field path the editor routes to) instead of planning
    // around it.
    let meta = generate(&mut service, &replaced);
    let report = &meta.report["gui2"];
    assert_eq!(report["kind"], "issues", "{report}");
    // The exact code distinguishes a changed source revision from a changed
    // shape; either way the anchor is unresolved and needs reattachment.
    let blocker = report["issues"]
        .as_array()
        .and_then(|issues| {
            issues.iter().find(|issue| {
                issue["field_path"].as_str() == Some("tabs.placement.anchors[0]")
                    && issue["code"].as_str().is_some_and(|code| {
                        code.starts_with("ARTWORK_") || code == "CONTOUR_ANCHOR_UNRESOLVED"
                    })
            })
        })
        .unwrap_or_else(|| panic!("no unresolved-anchor blocker in {report}"));
    assert_eq!(blocker["field_path"], "tabs.placement.anchors[0]");
    // Reattaching to an explicit current contour is a document command.
    let current = profile::contours(&replaced)
        .unwrap()
        .into_iter()
        .find(|contour| contour.role == "outer")
        .unwrap();
    let reattached = profile::reattach(
        &replaced,
        &id,
        v5::commands::AnchorTarget::Tab(0),
        &current.wire_id,
        Some(0.5),
    )
    .unwrap();
    let rows = profile::tab_rows(&reattached, &id);
    assert!(rows[0].resolved, "the reattached anchor resolves");
    // The contour selection still holds the older revision, so generation stays
    // blocked: reattaching one anchor never silently rebinds the selection.
    let blocked = generate(&mut service, &reattached);
    assert_eq!(blocked.report["gui2"]["kind"], "issues");
    // Re-binding the selection explicitly (the table's Select all) and
    // generating again produces the bridge on the edited source.
    let current_rows = profile::contours(&reattached)
        .unwrap()
        .into_iter()
        .map(|contour| profile::SelectionRow {
            reference: contour.reference,
            side: contour.suggested_side,
            traversal: None,
        })
        .collect::<Vec<_>>();
    let rebound = profile::select_in(&reattached, &id, &current_rows).unwrap();
    let meta = generate(&mut service, &rebound);
    let report = &meta.report["gui2"];
    assert_eq!(
        placements(&meta).len(),
        1,
        "the reattached anchor places its bridge again: {}",
        report["generationIssues"]
    );
}

#[test]
fn radial_finishing_adds_a_finish_pass_at_the_allowance_and_feed() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let mut service = Retained::new();
    let mut finished = job.clone();
    profile::set_finish_enabled(&mut finished, &id, true).unwrap();
    for (field, value) in [(91, 0.4), (92, 150.)] {
        finished = app::set_value(&finished, &id, field, Some(value))
            .unwrap_or_else(|error| panic!("field {field}: {error}"));
    }
    // Radial finishing leaves this operation's one assignment alone: only the
    // finishing feed is added. Feeds, speed, stepdown limit and the tool stay.
    let settings = profile::settings_in(&finished, &id).unwrap();
    assert_eq!(settings.assignment.cutting_feed_mm_min, Some(300.));
    assert_eq!(settings.assignment.plunge_feed_mm_min, Some(100.));
    assert_eq!(settings.assignment.spindle_rpm, Some(12_000.));
    assert_eq!(settings.assignment.max_stepdown_mm, Some(1.));
    assert_eq!(settings.finish.radial_allowance_mm, Some(0.4));
    let meta = generate(&mut service, &finished);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{report}");
    let stages = report["inspection"]["stages"].as_array().unwrap();
    let rough: usize = stages
        .iter()
        .filter(|stage| stage["role"] == "profile_rough")
        .map(|stage| stage["motionCount"].as_u64().unwrap() as usize)
        .sum();
    let finish: usize = stages
        .iter()
        .filter(|stage| stage["role"] == "profile_finish")
        .map(|stage| stage["motionCount"].as_u64().unwrap() as usize)
        .sum();
    assert!(rough > 0 && finish > 0, "both passes generate: {stages:?}");
    // The timeline states the passes in execution order, with distinct labels.
    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups[0]["role"], "Profile rough");
    assert_eq!(groups[1]["role"], "Profile finish");
    assert_eq!(groups[0]["label"], "Profile rough paths (1 of 3)");
    assert_eq!(groups[1]["jump"], "After profile finish (1 of 3)");
    // The first checkpoint is the stock after the first rough pass and the last
    // is the finished stock: the finishing pass removes the allowance the rough
    // pass left standing.
    let preview = meta.stock.as_ref().unwrap();
    let after_first_rough = preview.frames[1].stats.removed_volume_mm3;
    let final_removed = preview.frames.last().unwrap().stats.removed_volume_mm3;
    assert!(
        final_removed > after_first_rough,
        "the finishing pass cuts material: {after_first_rough} then {final_removed} mm³"
    );
    // The emitted program carries both feeds: roughing at the assignment feed
    // and finishing at the finishing feed.
    let handle = report["handle"].as_str().unwrap().to_owned();
    let (prepared, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: finished.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    let gcode = prepared.report["gui2"]["file"]["gcode"].as_str().unwrap();
    assert!(gcode.contains("F300"), "rough feed in the program");
    assert!(gcode.contains("F150"), "finishing feed in the program");
    // A positive allowance on an on-contour selection is refused with a
    // located reason instead of an offset that does not exist.
    let mut on_contour = finished.clone();
    let rows = profile::selection(&on_contour, &id)
        .into_iter()
        .map(|row| profile::SelectionRow {
            side: ContourSide::On,
            traversal: Some(cam_core::project::TraversalDirection::Forward),
            ..row
        })
        .collect::<Vec<_>>();
    on_contour = profile::set_selection_rows(&on_contour, &id, &rows).unwrap();
    let meta = generate(&mut service, &on_contour);
    assert!(
        meta.report["gui2"]["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_FINISH_ALLOWANCE")),
        "{}",
        meta.report["gui2"]["generationIssues"]
    );
}

#[test]
fn finishing_keeps_the_tabs_and_their_anchors() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let outer = profile::contours(&job)
        .unwrap()
        .into_iter()
        .filter(|contour| contour.role == "outer")
        .max_by(|a, b| a.perimeter_mm.total_cmp(&b.perimeter_mm))
        .unwrap();
    let outers = profile::selection(&job, &id)
        .into_iter()
        .filter(|row| row.reference.kind == v5::GeometryRefKind::ClosedContour)
        .collect::<Vec<_>>();
    let mut tabbed = profile::set_selection_rows(
        &job,
        &id,
        &outers
            .into_iter()
            .filter(|row| {
                profile::contours(&job)
                    .unwrap()
                    .iter()
                    .any(|c| c.reference == row.reference && c.role == "outer")
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    profile::set_tabs_enabled(&mut tabbed, &id, true).unwrap();
    profile::set_tab_placement_mode(&mut tabbed, &id, false).unwrap();
    for (field, value) in [(93, 0.5), (94, 2.)] {
        tabbed = app::set_value(&tabbed, &id, field, Some(value)).unwrap();
    }
    tabbed = profile::tab_anchor(
        &tabbed,
        &id,
        &TabAnchorAction::Add {
            wire_id: outer.wire_id.clone(),
            fraction: 0.125,
        },
    )
    .unwrap();
    let mut service = Retained::new();
    let plain = generate(&mut service, &tabbed);
    assert_eq!(placements(&plain).len(), 1);
    let plain_removed = removed(&plain);
    // Turning finishing on keeps the tab, its anchor and the assignment.
    profile::set_finish_enabled(&mut tabbed, &id, true).unwrap();
    for (field, value) in [(91, 0.4), (92, 150.)] {
        tabbed = app::set_value(&tabbed, &id, field, Some(value)).unwrap();
    }
    let meta = generate(&mut service, &tabbed);
    assert_eq!(
        meta.report["gui2"]["checks"]["exportReady"], true,
        "{}",
        meta.report["gui2"]["generationIssues"]
    );
    assert_eq!(
        placements(&meta).len(),
        1,
        "the manually anchored tab survives regeneration"
    );
    let rows = profile::tab_rows(&tabbed, &id);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fraction, 0.125);
    assert_eq!(
        profile::settings_in(&tabbed, &id)
            .unwrap()
            .finish
            .radial_allowance_mm,
        Some(0.4)
    );
    // The finished part removes at least as much as the same job cut in one
    // pass, because the finishing pass clears the allowance the rough left.
    assert!(
        removed(&meta) >= plain_removed - 1.,
        "finishing clears the allowance: {} vs {plain_removed} mm³",
        removed(&meta)
    );
}

#[test]
fn moving_the_start_and_choosing_the_entry_change_the_real_motion() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let mut service = Retained::new();
    let automatic_scene = generate_scene(&mut service, &job);
    let automatic = stream(&automatic_scene);
    let first = automatic
        .iter()
        .find(|motion| motion.kind == "cut")
        .expect("the automatic profile cuts");
    assert!(
        (first.x0 - first.x1).hypot(first.y0 - first.y1) < 1e-6,
        "a plunge entry descends without travelling: {first:?}"
    );

    // Anchor the start on the widest outer contour at half of its source
    // length: the first cutting motion now begins at that point.
    let outer = widest_outer(&job);
    let mut anchored = job.clone();
    anchored = profile::start_anchor(&anchored, &id, Some(&outer.wire_id), 0.5).unwrap();
    assert!(
        profile::start_row(&anchored, &id).is_some_and(|row| row.resolved),
        "the start anchor resolves against its source"
    );
    let anchored_scene = generate_scene(&mut service, &anchored);
    let anchored_stream = stream(&anchored_scene);
    let anchor_point = profile::ring_point(&outer.anchor_ring, 0.5).unwrap();
    // A cut that follows a rapid (or opens the stream) is where a pass enters;
    // the segments between are the cut itself. The anchor moves one contour's
    // entry to the requested point; the automatic job enters at its own seams.
    let entries = |motions: &[cam_gui_runtime::sim::Motion]| {
        motions
            .iter()
            .enumerate()
            .filter(|(index, motion)| {
                motion.kind == "cut" && (*index == 0 || motions[index - 1].kind != "cut")
            })
            .map(|(_, motion)| (motion.x0, motion.y0))
            .collect::<Vec<_>>()
    };
    let distance = |point: (f64, f64)| (point.0 - anchor_point[0]).hypot(point.1 - anchor_point[1]);
    let anchored_entries = entries(&anchored_stream);
    assert!(
        anchored_entries.iter().any(|point| distance(*point) < 2.5),
        "a pass enters near the anchor {anchor_point:?}: {anchored_entries:?}"
    );
    let automatic_entries = entries(&automatic);
    assert!(
        automatic_entries.iter().all(|point| distance(*point) > 2.5),
        "the automatic job enters at its own seams, not the anchor {anchor_point:?}: {automatic_entries:?}"
    );

    // A ramp entry descends along the loop instead of straight down, so its
    // first descending motion covers real XY distance.
    // The small hole has no room for a 45° entry, so the ramp fixture cuts the
    // outer contours (the planner refuses the hole with a located reason).
    let mut ramped =
        profile::start_anchor(&outers_only(&job), &id, Some(&outer.wire_id), 0.5).unwrap();
    profile::set_entry(&mut ramped, &id, true).unwrap();
    // 45° keeps the first layer's ramp short enough for every selected
    // contour, including the small hole.
    for (field, value) in [(98, 45.), (99, 80.)] {
        ramped = app::set_value(&ramped, &id, field, Some(value)).unwrap();
    }
    cam_gui_runtime::authoring::tool_mut_in(&mut ramped, &id, false)
        .unwrap()
        .capabilities
        .ramp_capable = Some(true);
    let ramped_stream = stream(&generate_scene(&mut service, &ramped));
    let travel = |motion: &cam_gui_runtime::sim::Motion| {
        (motion.x0 - motion.x1).hypot(motion.y0 - motion.y1)
    };
    let descends = |motion: &cam_gui_runtime::sim::Motion| {
        motion.kind == "cut" && motion.z1 < motion.z0 - 1e-9
    };
    let descent = ramped_stream
        .iter()
        .find(|motion| descends(motion) && travel(motion) > 0.5)
        .unwrap_or_else(|| {
            panic!(
                "the ramp descends while travelling: {:?}",
                ramped_stream
                    .iter()
                    .filter(|m| descends(m))
                    .map(|m| (m.x0, m.y0, m.x1, m.y1, m.z0, m.z1))
                    .take(4)
                    .collect::<Vec<_>>()
            )
        });
    assert!(
        travel(descent) > 0.5,
        "a ramp travels while descending: {descent:?}"
    );
    // The anchored plunge job descends without travelling at all.
    assert!(
        anchored_stream
            .iter()
            .filter(|motion| descends(motion))
            .all(|motion| travel(motion) < 0.5),
        "a plunge descends straight down"
    );

    // A tangent line lead-in adds approach motion before the seam, at its own
    // feed. The small hole has no room beside its void either, so the lead
    // fixture cuts the outer contours.
    let mut lead =
        profile::start_anchor(&outers_only(&job), &id, Some(&outer.wire_id), 0.5).unwrap();
    let without_lead = stream(&generate_scene(&mut service, &lead));
    profile::set_lead(
        &mut lead,
        &id,
        false,
        cam_core::project::LeadSpec::TangentLine {
            length_mm: None,
            feed_mm_min: None,
        },
    )
    .unwrap();
    lead = app::set_value(&lead, &id, 100, Some(4.)).unwrap();
    lead = app::set_value(&lead, &id, 101, Some(150.)).unwrap();
    let lead_stream = stream(&generate_scene(&mut service, &lead));
    assert!(
        lead_stream.len() > without_lead.len(),
        "the lead-in adds motion: {} vs {}",
        lead_stream.len(),
        without_lead.len()
    );
    assert!(
        lead_stream
            .iter()
            .any(|motion| motion.kind == "cut" && (travel(motion) - 4.).abs() < 0.3),
        "the lead-in runs the requested 4 mm before the seam"
    );
}

#[test]
fn an_invalid_entry_is_located_and_can_be_repaired() {
    let job = profile_job();
    let id = job.operations[0].id.clone();
    let mut service = Retained::new();
    let outer = widest_outer(&job);
    let mut ramped = profile::start_anchor(&job, &id, Some(&outer.wire_id), 0.5).unwrap();
    profile::set_entry(&mut ramped, &id, true).unwrap();
    for (field, value) in [(98, 10.), (99, 80.)] {
        ramped = app::set_value(&ramped, &id, field, Some(value)).unwrap();
    }
    // The tool has not been marked ramp capable: the requirement is reported as
    // a located missing field before planning.
    let meta = generate(&mut service, &ramped);
    assert_eq!(meta.report["gui2"]["kind"], "issues");
    assert!(
        meta.report["gui2"]["issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| issue["field_path"]
                .as_str()
                .is_some_and(|path| path.contains("ramp_capable")))),
        "{}",
        meta.report["gui2"]["issues"]
    );
    // Marked as not ramp capable: a located generation conflict.
    cam_gui_runtime::authoring::tool_mut_in(&mut ramped, &id, false)
        .unwrap()
        .capabilities
        .ramp_capable = Some(false);
    let meta = generate(&mut service, &ramped);
    assert!(
        meta.report["gui2"]["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_ENTRY_CAPABILITY")),
        "{}",
        meta.report["gui2"]["generationIssues"]
    );
    // Repair: a plunge entry needs no ramp capability.
    let repaired = {
        let mut job = ramped.clone();
        profile::set_entry(&mut job, &id, false).unwrap();
        cam_gui_runtime::authoring::tool_mut_in(&mut job, &id, false)
            .unwrap()
            .capabilities
            .ramp_capable = None;
        job
    };
    let meta = generate(&mut service, &repaired);
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], true);

    // A tangent arc lead needs a retained side: on-contour selections have a
    // zero offset and are refused with a located reason, and choosing a plain
    // lead repairs it.
    let mut on_contour = {
        let mut job = repaired.clone();
        profile::set_entry(&mut job, &id, false).unwrap();
        let rows = profile::selection(&job, &id)
            .into_iter()
            .map(|row| profile::SelectionRow {
                side: ContourSide::On,
                traversal: Some(cam_core::project::TraversalDirection::Forward),
                ..row
            })
            .collect::<Vec<_>>();
        profile::set_selection_rows(&job, &id, &rows).unwrap()
    };
    profile::set_lead(
        &mut on_contour,
        &id,
        false,
        cam_core::project::LeadSpec::TangentArc {
            radius_mm: Some(2.),
            sweep_deg: Some(90.),
            feed_mm_min: Some(120.),
        },
    )
    .unwrap();
    let meta = generate(&mut service, &on_contour);
    assert!(
        meta.report["gui2"]["generationIssues"]
            .as_array()
            .is_some_and(|issues| issues
                .iter()
                .any(|issue| issue["code"] == "PROFILE_LEAD_SIDE")),
        "{}",
        meta.report["gui2"]
    );
    profile::set_lead(
        &mut on_contour,
        &id,
        false,
        cam_core::project::LeadSpec::None,
    )
    .unwrap();
    let meta = generate(&mut service, &on_contour);
    assert_eq!(
        meta.report["gui2"]["checks"]["exportReady"], true,
        "removing the arc lead repairs the entry: {}",
        meta.report["gui2"]["generationIssues"]
    );
}

#[test]
fn a_profile_takes_its_cutter_and_cutting_values_from_the_tool_library() {
    use cam_core::project::v5::resources::{AssignmentRole as Role, ProfileStatus};
    use cam_gui_runtime::resources::{Catalog, ResourceCommand as R};

    // A profile job with contours, heights and an applied machine, but no
    // cutter data at all: exactly the state a new profile job starts in.
    let mut job = profile::import_svg(
        "letters.svg".into(),
        include_str!("../../../fixtures/gui3/lettering.svg").into(),
    )
    .unwrap();
    let id = job.operations[0].id.clone();
    let rows = profile::contours(&job)
        .unwrap()
        .into_iter()
        .map(|contour| profile::SelectionRow {
            reference: contour.reference,
            side: contour.suggested_side,
            traversal: None,
        })
        .collect::<Vec<_>>();
    job = profile::select_in(&job, &id, &rows).unwrap();
    for (field, value) in [(6, 6.), (7, 5.), (8, 0.5), (90, -6.)] {
        job = app::set_value(&job, &id, field, Some(value)).unwrap();
    }
    profile::set_direction(&mut job, &id, Some(CutDirection::Climb)).unwrap();
    let (applied, _) = session::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: job.to_json().unwrap(),
            json: session::PROFILE.into(),
        },
    )
    .unwrap();
    let job = session::open(&applied.job).unwrap();
    let mut service = Retained::new();
    // Without a cutter the operation cannot plan.
    let meta = generate(&mut service, &job);
    assert_eq!(meta.report["gui2"]["kind"], "issues");

    // The library's cutter, with a cutting profile, is applied to this
    // profile's single milling assignment.
    let mut catalog = Catalog::decode(include_str!("../../../fixtures/gui5/library.json")).unwrap();
    catalog.library.tools[0].spindle_direction = Some(SpindleDirection::Clockwise);
    let action = R::ApplyToolProfile {
        catalog: catalog.clone(),
        tool: "endmill".into(),
        preset: "rough".into(),
        operation: id.clone(),
        role: Role::Milling,
    };
    // The profile's own drafts for every value the command writes are cleared.
    for field in [2, 10, 11, 88, 12, 13] {
        assert!(
            action.clear_fields(&job).contains(&field),
            "field {field} is not refreshed after applying a library profile"
        );
    }
    let job = action.execute(&job).unwrap();
    let settings = profile::settings_in(&job, &id).unwrap();
    assert_eq!(settings.assignment.spindle_rpm, Some(12_000.));
    assert_eq!(settings.assignment.cutting_feed_mm_min, Some(1_200.));
    assert_eq!(settings.assignment.plunge_feed_mm_min, Some(400.));
    assert_eq!(settings.assignment.max_stepdown_mm, Some(0.5));
    assert_eq!(
        settings.assignment.spindle_direction,
        Some(SpindleDirection::Clockwise),
        "the library tool's rotation is copied onto the milling assignment"
    );
    let tool = job
        .tools
        .iter()
        .find(|tool| tool.id == settings.assignment.tool_id)
        .unwrap();
    assert!(
        matches!(
            tool.geometry,
            Some(cam_core::project::ToolGeometry::Endmill(_))
        ),
        "the profile now uses the copied library cutter: {tool:?}"
    );
    let status = cam_core::project::v5::resources::assignment_statuses(&job)
        .into_iter()
        .find(|status| status.operation_id == id && status.role == Role::Milling)
        .unwrap();
    assert_eq!(status.status, ProfileStatus::Applied);
    assert_eq!(
        status
            .applied
            .as_ref()
            .map(|applied| applied.name_at_application.as_str()),
        Some("Lettering rough")
    );

    // The job now plans, generates and prepares with the library values.
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{report}");
    assert!(meta.motions > 0);

    // Editing one applied value shows Modified; Reset restores the baseline.
    let edited = app::set_value(&job, &id, 2, Some(900.)).unwrap();
    let status = cam_core::project::v5::resources::assignment_statuses(&edited)
        .into_iter()
        .find(|status| status.operation_id == id && status.role == Role::Milling)
        .unwrap();
    assert_eq!(status.status, ProfileStatus::Modified);
    let reset = R::Reset {
        operation: id.clone(),
        role: Role::Milling,
    }
    .execute(&edited)
    .unwrap();
    assert_eq!(
        profile::settings_in(&reset, &id)
            .unwrap()
            .assignment
            .cutting_feed_mm_min,
        Some(1_200.)
    );
}

#[test]
fn profile_after_the_known_carving_keeps_each_operation_stock() {
    // The established carving plus a profile that cuts the same artwork after
    // it: both operations publish their own stock prefix.
    let carving = session::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
    let mut job =
        operation_authoring::apply(&carving, operation_authoring::add(Kind::Profile, &carving))
            .unwrap();
    let id = "profile-1".to_string();
    let rows = profile::contours(&job)
        .unwrap()
        .into_iter()
        .map(|contour| profile::SelectionRow {
            reference: contour.reference,
            side: contour.suggested_side,
            traversal: None,
        })
        .collect::<Vec<_>>();
    job = profile::select_in(&job, &id, &rows).unwrap();
    for (field, value) in [
        (2, 300.),
        (10, 100.),
        (11, 12_000.),
        (8, 0.5),
        (88, 0.5),
        (90, -6.),
    ] {
        job = app::set_value(&job, &id, field, Some(value)).unwrap();
    }
    let mut candidate = job.clone();
    cam_gui_runtime::authoring::set_group_in(&mut candidate, &id, 12, &[3., 8.]).unwrap();
    job = candidate;
    profile::set_direction(&mut job, &id, Some(CutDirection::Climb)).unwrap();
    profile::set_spindle_direction(&mut job, &id, Some(SpindleDirection::Clockwise)).unwrap();
    let mut service = Retained::new();
    let meta = generate(&mut service, &job);
    let report = &meta.report["gui2"];
    assert_eq!(report["checks"]["exportReady"], true, "{report}");
    let operations = report["inspection"]["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2, "{operations:?}");
    assert_ne!(
        operations[0]["stockAfterId"], operations[1]["stockAfterId"],
        "each operation publishes its own stock"
    );
}
