//! Maintenance tool: re-bind the saved selections of a schema-5 job to its
//! artwork items' *current* source revision.
//!
//! A source revision is a digest of the item's content plus its import
//! settings, so any change to those settings invalidates the selections that
//! were bound to the old digest. The importer losing its interpretation mode
//! is exactly that kind of change, and this is how stored fixtures are brought
//! forward: the geometry and the selections are untouched, only the revision
//! they are bound to is refreshed.
//!
//! Usage: `cargo run -p cam-core --example rebind_fixture_revisions -- FILE...`
use cam_core::project::v5::{self, CamJobV5, GeometryRef, OperationSettingsV5};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: rebind_fixture_revisions FILE...");
        std::process::exit(2);
    }
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("read job");
        let mut job = CamJobV5::from_json(&text).expect("parse schema-5 job");
        let catalogue = v5::artwork::inspect_artwork(&job).expect("resolve artwork");
        let live = |reference: &GeometryRef| -> Option<GeometryRef> {
            catalogue
                .item(&reference.artwork_item_id)?
                .entries
                .iter()
                .find(|entry| {
                    entry.kind == reference.kind
                        && entry.reference.local_geometry_id == reference.local_geometry_id
                })
                .map(|entry| entry.reference.clone())
        };
        let mut rebound = 0usize;
        let mut missing = 0usize;
        for operation in &mut job.operations {
            let mut bind = |reference: &mut GeometryRef| match live(reference) {
                Some(current) => {
                    *reference = current;
                    rebound += 1;
                }
                None => missing += 1,
            };
            match &mut operation.settings {
                OperationSettingsV5::FlatVcarve(settings) => {
                    for reference in &mut settings.components {
                        bind(reference);
                    }
                }
                OperationSettingsV5::Profile(settings) => {
                    for contour in &mut settings.contours {
                        bind(&mut contour.geometry);
                    }
                }
                OperationSettingsV5::DragKnife(settings) => {
                    for reference in &mut settings.chains {
                        bind(reference);
                    }
                }
                OperationSettingsV5::Face(_) => {}
            }
        }
        job.validate_structure().expect("job stays valid");
        std::fs::write(&path, job.to_json().expect("serialize")).expect("write job");
        println!("{path}: {rebound} rebound, {missing} not found");
    }
}
