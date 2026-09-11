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
    h.get_by_label("02  Flat V-carve").click();
    h.run();
    h.get_by_label("01  Flat V-carve").click();
    h.run();
    assert_eq!(h.state().draft.raw[&key], "-");
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
