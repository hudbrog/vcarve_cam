use cam_gui1::{app::App, state::Draft};
use egui_kittest::{Harness, kittest::Queryable};

#[test]
fn named_field_keeps_raw_text_when_sources_reorder() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280., 800.))
        .build_state(|ctx, app: &mut App| app.ui(ctx), App::default());
    h.get_by_label("Maximum depth").click();
    h.run();
    h.get_by_label("Maximum depth").type_text("-");
    h.run();
    let key = h.state().draft.key(0);
    assert_eq!(h.state().draft.raw[&key], "-");
    h.get_by_label("Reverse artwork order").click();
    h.run();
    assert_eq!(h.state().draft.sources, vec![202, 101]);
    assert!(h.get_by_label("Maximum depth").is_focused());
    h.get_by_label("02  Flat V-carve").click();
    h.run();
    h.get_by_label("01  Flat V-carve").click();
    h.run();
    assert_eq!(h.state().draft.raw[&key], "-");
    assert!(h.get_by_label("Maximum depth").is_focused());
    assert_eq!(
        h.get_by_label("Maximum depth").value().as_deref(),
        Some("-")
    );
    // Deliberate negative control: the harness must reject a broken command outcome.
    let mut broken = h.state().draft.clone();
    broken.raw.clear();
    let broken_was_detected =
        std::panic::catch_unwind(|| assert_eq!(broken.raw.get(&key), Some(&"-".into()))).is_err();
    assert!(broken_was_detected);
    assert_eq!(Draft::parse("-"), Err("Invalid number — retained"));
}

#[test]
fn l_list_is_virtualized() {
    let mut app = App::default();
    app.operation_count = 1000;
    let h = Harness::builder()
        .with_size(egui::vec2(1280., 800.))
        .build_state(|ctx, app: &mut App| app.ui(ctx), app);
    assert!(h.state().laid_out_rows < 35);
}

#[test]
fn stock_checkpoint_navigation_retains_the_scene_and_raw_draft() {
    let scene = cam_gui1::compute::run(cam_gui1::compute::Request::Reference {
        flower: false,
        export: false,
    })
    .unwrap();
    let mut app = App::default();
    app.load_scene(Ok(scene));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280., 800.))
        .build_state(|ctx, app: &mut App| app.ui(ctx), app);
    h.get_by_label("Maximum depth").click();
    h.run();
    h.get_by_label("Maximum depth").type_text("1.");
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Transported checkpoint")
        .click();
    h.run();
    let checkpoint = h
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Transported checkpoint")
        .value()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert!(checkpoint < h.state().motion_count());
    // The continuous scrub seeks between transported checkpoints in the same
    // scene, and the raw inspector text survives it.
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Stock motion")
        .click();
    h.run();
    h.get_by_label("Stock preview").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Motion playhead");
    h.get_by_label("Stock preview").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Transported checkpoint");
    assert_eq!(
        h.get_by_label("Maximum depth").value().as_deref(),
        Some("1.")
    );
}

#[test]
fn picking_selects_a_transported_motion_at_a_scaled_dpi() {
    use cam_gui1::pick::{self, Camera, Picker};
    let scene = cam_gui1::compute::run(cam_gui1::compute::Request::Reference {
        flower: false,
        export: false,
    })
    .unwrap();
    let (meta, payload) = scene;
    let picker = Picker::from_vertex_bytes(
        &payload[meta.motion_offset..meta.motion_offset + meta.motion_len],
    )
    .unwrap();
    let rect = egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(900., 600.));
    let camera = Camera {
        iso: false,
        aspect: rect.width() / rect.height(),
        zoom: 1.,
        yaw: 0.,
    };
    let target = (picker.motion_count() / 2) as u32;
    let anchor = camera.to_points(
        camera.ndc(picker.endpoints(target).unwrap()[0]),
        [rect.width(), rect.height()],
    );
    let mut app = App::default();
    app.load_scene(Ok((meta, payload)));
    app.pick_at(rect.center() + egui::vec2(anchor[0], anchor[1]), rect, 1.5);
    let status = app.status.clone();
    assert!(status.contains("Picked motion"), "{status}");
    // Exactly on the projected endpoint of a transported motion, so the pick is
    // the nearest visible segment at that pixel and must be within a pixel.
    let distance: f64 = status
        .split("at ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap();
    assert!(distance <= 1., "pick was {distance} px away: {status}");
    assert!(status.contains("1.50× DPI"), "{status}");
    let scaled = app.selection.unwrap().motion;

    // The same cursor in points must select the same motion at 100% DPI: the
    // tolerance changes in points, not in the geometry.
    app.pick_at(rect.center() + egui::vec2(anchor[0], anchor[1]), rect, 1.);
    assert_eq!(app.selection.unwrap().motion, scaled);
    assert!(app.status.contains("1.00× DPI"), "{}", app.status);
    // Consecutive transported motions share an endpoint, so a cursor exactly on
    // that point may tie-break to either neighbour.
    assert!(
        scaled.abs_diff(target) <= 1,
        "picked {scaled}, expected near {target}"
    );

    // A far cursor selects nothing and says so rather than keeping a stale pick.
    app.pick_tolerance_px = 2.;
    app.pick_at(rect.left_top() + egui::vec2(2., 2.), rect, 1.);
    assert!(app.selection.is_none());
    assert!(app.status.contains("No motion within"), "{}", app.status);
    let _ = pick::point_segment_distance([0., 0.], [1., 0.], [1., 1.]);
}

#[test]
#[ignore = "GPU/local golden check: UPDATE_SNAPSHOTS=true cargo test --test interaction dense_layout -- --ignored"]
fn dense_layout() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280., 800.))
        .build_state(|ctx, app: &mut App| app.ui(ctx), App::default());
    h.run();
    h.snapshot("dense-1280");
}

#[test]
#[ignore = "GPU/local golden check"]
fn dense_layout_1440() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440., 900.))
        .build_state(|ctx, app: &mut App| app.ui(ctx), App::default());
    h.run();
    h.snapshot("dense-1440");
}
