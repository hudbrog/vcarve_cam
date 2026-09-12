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
    let reattached = profile::tab_anchor(
        &replaced,
        &id,
        &TabAnchorAction::Reattach {
            index: 0,
            wire_id: current.wire_id.clone(),
            fraction: Some(0.5),
        },
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
