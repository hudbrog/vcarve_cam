//! GUI8: a closed-contour profile operation — explicit sides and depths,
//! generated passes, actual heightfield playback, checked export and portable
//! reopen.
use cam_core::project::{
    ContourSide, CutDirection, SpindleDirection,
    v5::{self, CamJobV5, OperationSettingsV5},
};
use cam_gui_runtime::{
    app,
    operation_authoring::{self, Kind},
    profile,
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
