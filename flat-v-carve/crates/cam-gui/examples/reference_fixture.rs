//! Rebuild the canonical GUI2 regression fixture from the historical reference.
//! This is test-data preparation; GUI2 itself accepts schema 5 only.
fn main() {
    let legacy = cam_core::project::migrate::migrate_legacy_json(include_str!(
        "../../../../real_data/flower_box-svg.job-real.json"
    ))
    .unwrap();
    let profile = cam_core::post::LinuxCncProfile::from_json(include_str!(
        "../../../../real_data/machine-profile.json"
    ))
    .unwrap();
    let applied = cam_core::post::sequence::apply_legacy_profile(&profile, &legacy).unwrap();
    let job = cam_core::project::v5::migrate::migrate_v4(&applied).unwrap();
    // No machine snapshot: the review workflow applies it explicitly.
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/gui2/flower.job.json"),
        job.to_json().unwrap(),
    )
    .unwrap();
}
