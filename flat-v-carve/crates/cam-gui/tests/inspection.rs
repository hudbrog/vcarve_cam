//! GUI9b: the display raster can be re-derived at another resolution from the
//! same retained execution, and a raster from another simulation key is never
//! mixed with the one on screen.
use cam_core::project::{CutDirection, SpindleDirection};
use cam_gui_runtime::{
    app,
    profile::{self},
    session::{self as gui, Command},
    stock_preview::DisplayPreset,
};
use cam_service::retained::Retained;

/// A small checked profile job: one closed contour, cut through 6 mm stock.
fn profile_job() -> String {
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
    // A stock large enough that the display cap, not the plan's requested
    // resolution, decides the cell size: this is where another preset buys real
    // detail.
    job.setup.stock.xy = Some(cam_core::project::RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 200.,
        length_mm: 100.,
    });
    job.validate_structure().unwrap();
    let (applied, _) = gui::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: job.to_json().unwrap(),
            json: gui::PROFILE.into(),
        },
    )
    .unwrap();
    applied.job
}

#[test]
fn another_display_resolution_rebuilds_the_same_execution_at_a_new_key() {
    let job = profile_job();
    let mut service = Retained::new();
    let (scene, payload) = gui::execute(
        &mut service,
        Command::generate_at(job.clone(), DisplayPreset::Standard),
    )
    .unwrap();
    let handle = scene.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let standard = scene
        .stock
        .clone()
        .expect("generated scene keeps its stock");
    assert_eq!(standard.preset, DisplayPreset::Standard);
    let motion_count = scene.motions;
    let position = standard.frames.last().unwrap().prefix;
    assert_eq!(position, motion_count);
    let execution = scene.report["gui2"]["executionFingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    // The replayed display input, shared by both resolutions.
    let sim = cam_gui_runtime::compute::Scene {
        meta: scene,
        payload: std::sync::Arc::new(payload),
    }
    .sim_input()
    .unwrap()
    .unwrap();

    let (fine_meta, fine_payload) = gui::execute(
        &mut service,
        Command::DisplayPreset {
            handle: handle.clone(),
            preset: DisplayPreset::Fine,
        },
    )
    .unwrap();
    let fine = fine_meta.stock.clone().expect("the rebuild returns stock");
    assert_eq!(fine_meta.report["gui2"]["kind"], "preset");
    assert_eq!(fine_meta.report["gui2"]["preset"], "fine");
    assert_eq!(fine_meta.report["gui2"]["changed"], true);
    assert_eq!(
        fine_meta.report["gui2"]["handle"].as_str(),
        Some(handle.as_str()),
        "the rebuild answers for the retained plan it was asked about"
    );
    assert!(
        fine.cell_mm < standard.cell_mm,
        "the finer preset must produce smaller cells: {} vs {}",
        fine.cell_mm,
        standard.cell_mm
    );
    assert_ne!(
        fine.key, standard.key,
        "another resolution is another simulation key"
    );
    assert_ne!(fine.identity(), standard.identity());
    assert_eq!(
        fine.frames.first().map(|frame| frame.prefix),
        Some(position),
        "the rebuild keeps the playhead it was asked to compare at"
    );
    assert!(
        fine.retained_bytes <= DisplayPreset::Fine.budget(),
        "the rebuilt checkpoints stay inside the preset budget"
    );
    assert!(
        fine.ladder_frames >= 3,
        "the response reports the retained ladder, not just the frame it carries: {}",
        fine.ladder_frames
    );

    // The rebuilt raster is the same cut at a finer grid: replay the same
    // prefix from the pristine field at the new cell and compare exactly.
    let frame = fine.frames.first().unwrap();
    let response = cam_gui_runtime::compute::Scene {
        meta: fine_meta,
        payload: std::sync::Arc::new(fine_payload),
    };
    let mut replay = cam_gui_runtime::sim::Field::new(sim.stock, &sim.tools, fine.cell_mm).unwrap();
    for motion in &sim.motions[..position] {
        replay.apply(motion, 0., 1.).unwrap();
    }
    assert_eq!(response.stock_cells(0).unwrap(), replay.packed_tile_bytes());
    assert_eq!(frame.versions, replay.versions);

    // The execution itself is untouched: the same handle still resolves and a
    // later seek carries the new key.
    let retained = service.generated_plan(&handle).unwrap();
    assert_eq!(retained.trusted.plan().motions.len(), motion_count);
    assert_eq!(
        retained.trusted.plan().execution_fingerprint,
        execution,
        "changing the display resolution never replans"
    );
    let (seek_meta, _) = gui::execute(
        &mut service,
        Command::Seek {
            handle: handle.clone(),
            prefix: position / 2,
        },
    )
    .unwrap();
    assert_eq!(
        seek_meta.stock.as_ref().map(|stock| stock.key.as_str()),
        Some(fine.key.as_str()),
        "a seek after the rebuild stays on the new key"
    );

    // Asking for the preset that is already displayed changes nothing.
    let (same_meta, _) = gui::execute(
        &mut service,
        Command::DisplayPreset {
            handle: handle.clone(),
            preset: DisplayPreset::Fine,
        },
    )
    .unwrap();
    assert_eq!(same_meta.report["gui2"]["changed"], false);
    assert_eq!(
        same_meta.stock.as_ref().map(|stock| stock.key.clone()),
        Some(fine.key.clone())
    );

    // A handle the worker no longer retains is refused, not silently answered
    // with the current raster.
    let stale = gui::execute(
        &mut Retained::new(),
        Command::DisplayPreset {
            handle,
            preset: DisplayPreset::Coarse,
        },
    )
    .unwrap_err();
    assert!(!stale.is_empty());

    // Coarser than the reference resolution is the honest display limit: the
    // cell size is reported next to the plan's own resolution.
    assert!(fine.reference_cell_mm <= fine.cell_mm);
    assert!(standard.reference_cell_mm <= standard.cell_mm);
}

/// GUI9b's other half: the profile/tab fixture is inspected at another display
/// resolution, and the generated geometry the viewport draws stays exactly what
/// the plan produced — the raster never becomes the source of a boundary.
#[test]
fn a_profile_with_tabs_inspects_at_a_finer_resolution_without_changing_the_plan() {
    // Profile the outer contours only, so a 3 mm tab fits each of them.
    let mut job = cam_gui_runtime::profile::import_svg(
        "letters.svg".into(),
        include_str!("../../../fixtures/gui3/lettering.svg").into(),
    )
    .unwrap();
    let id = job.operations[0].id.clone();
    let rows = profile::contours(&job)
        .unwrap()
        .into_iter()
        .filter(|contour| contour.suggested_side == cam_core::project::ContourSide::Outside)
        .map(|contour| profile::SelectionRow {
            reference: contour.reference,
            side: contour.suggested_side,
            traversal: None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2, "the fixture has two outer contours");
    job = profile::select_in(&job, &id, &rows).unwrap();
    for (field, value) in [
        (6, 6.),    // stock thickness
        (7, 5.),    // clearance above stock
        (2, 300.),  // cutting feed
        (10, 100.), // plunge feed
        (11, 12_000.),
        (8, 1.),   // stepdown
        (88, 1.),  // tool stepdown limit
        (90, -6.), // cut through
    ] {
        job = app::set_value(&job, &id, field, Some(value))
            .unwrap_or_else(|error| panic!("field {field}: {error}"));
    }
    let mut candidate = job.clone();
    cam_gui_runtime::authoring::set_group_in(&mut candidate, &id, 12, &[3., 8.]).unwrap();
    job = candidate;
    profile::set_direction(&mut job, &id, Some(CutDirection::Climb)).unwrap();
    profile::set_spindle_direction(&mut job, &id, Some(SpindleDirection::Clockwise)).unwrap();
    // A large stock so the display cap, not the plan's requested resolution,
    // decides the display cell size.
    job.setup.stock.xy = Some(cam_core::project::RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 200.,
        length_mm: 100.,
    });
    profile::set_tabs_enabled(&mut job, &id, true).unwrap();
    for (field, value) in [(93, 0.5), (94, 3.)] {
        job = app::set_value(&job, &id, field, Some(value)).unwrap();
    }
    job.validate_structure().unwrap();
    let (applied, _) = gui::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: job.to_json().unwrap(),
            json: gui::PROFILE.into(),
        },
    )
    .unwrap();
    let job = gui::open(&applied.job).unwrap();

    let mut service = Retained::new();
    let (scene, _) = gui::execute(
        &mut service,
        Command::generate_at(job.to_json().unwrap(), DisplayPreset::Standard),
    )
    .unwrap();
    let handle = scene.report["gui2"]["handle"].as_str().unwrap().to_owned();
    assert_eq!(scene.report["gui2"]["checks"]["exportReady"], true);
    // The plan's own tab geometry, which the viewport draws as an exact overlay.
    let tabs = scene.report["gui2"]["inspection"]["operations"][0]["tabPlacements"].clone();
    assert!(
        tabs.as_array().is_some_and(|list| !list.is_empty()),
        "the tabbed profile placed bridges: {}",
        scene.report["gui2"]["inspection"]
    );
    // Calculated machine time per operation and stage, with the rapid rate
    // the estimate rests on: what the "Inspect result" panel prints.
    let operations = scene.report["gui2"]["inspection"]["operations"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!operations.is_empty());
    for operation in &operations {
        assert!(
            operation["estimatedSeconds"]
                .as_f64()
                .is_some_and(|s| s > 0.),
            "operation carries a positive time: {operation}"
        );
    }
    assert!(
        scene.report["gui2"]["inspection"]["rapidRateMmMin"]
            .as_f64()
            .is_some_and(|rate| rate > 0.)
    );
    let stages = scene.report["gui2"]["inspection"]["stages"]
        .as_array()
        .unwrap();
    assert!(!stages.is_empty());
    assert!(
        stages
            .iter()
            .all(|stage| stage["estimatedSeconds"].as_f64().is_some_and(|s| s >= 0.))
    );
    let motions = scene.motions;
    let prefix = scene.stock.as_ref().unwrap().frames.last().unwrap().prefix;
    assert_eq!(prefix, motions);

    // Change only the display resolution.
    let (fine_meta, _) = gui::execute(
        &mut service,
        Command::DisplayPreset {
            handle: handle.clone(),
            preset: DisplayPreset::Fine,
        },
    )
    .unwrap();
    let fine = fine_meta.stock.as_ref().unwrap();
    assert!(fine.cell_mm < scene.stock.as_ref().unwrap().cell_mm);
    assert_eq!(fine.frames.first().map(|frame| frame.prefix), Some(prefix));
    // The scene the viewport holds is untouched: same plan, same tab geometry,
    // same motion count. The display rebuild never becomes a plan source.
    let retained = service.generated_plan(&handle).unwrap();
    assert_eq!(retained.trusted.plan().motions.len(), motions);
    assert_eq!(
        scene.report["gui2"]["inspection"]["operations"][0]["tabPlacements"], tabs,
        "the generated tab geometry is what the overlay draws, at either resolution"
    );
}
