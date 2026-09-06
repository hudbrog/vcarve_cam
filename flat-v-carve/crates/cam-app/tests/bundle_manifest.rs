#[path = "../build/bundle.rs"]
mod bundle;

use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cam-bundle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("dist/assets/nested")).unwrap();
        fs::create_dir_all(path.join("src")).unwrap();
        let source_names = [
            "index.html",
            "package.json",
            "pnpm-lock.yaml",
            "tsconfig.json",
            "vite.config.ts",
            "src/main.ts",
        ];
        let mut sources = BTreeMap::new();
        for name in source_names {
            fs::write(path.join(name), name).unwrap();
            sources.insert(name, format!("{:x}", Sha256::digest(name)));
        }
        let mut assets = BTreeMap::new();
        for (name, bytes) in [
            ("index.html", b"<html>embedded</html>".as_slice()),
            ("assets/nested/icon.svg", b"<svg/>".as_slice()),
        ] {
            fs::write(path.join("dist").join(name), bytes).unwrap();
            assets.insert(name, format!("{:x}", Sha256::digest(bytes)));
        }
        fs::write(
            path.join("dist/.bundle-manifest.json"),
            json!({"schemaVersion":1,"engineVersion":"test","sources":sources,"assets":assets})
                .to_string(),
        )
        .unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn valid_bundle_embeds_all_assets_including_nested_files() {
    let fixture = Fixture::new();
    let code = bundle::generate(&fixture.0, "test").unwrap();
    assert!(code.contains("assets/nested/icon.svg"));
    assert_eq!(code.matches("include_bytes!").count(), 2);
    assert!(!code.contains(".bundle-manifest.json"));
}

#[test]
fn source_changes_additions_and_deletions_require_a_new_frontend_build() {
    for change in ["changed", "added", "deleted"] {
        let fixture = Fixture::new();
        match change {
            "changed" => fs::write(fixture.0.join("src/main.ts"), "new source").unwrap(),
            "added" => fs::write(fixture.0.join("src/new.ts"), "new file").unwrap(),
            _ => fs::remove_file(fixture.0.join("src/main.ts")).unwrap(),
        }
        assert!(
            bundle::generate(&fixture.0, "test")
                .unwrap_err()
                .to_string()
                .contains("UI sources changed")
        );
    }
}

#[test]
fn damaged_or_incomplete_assets_are_rejected() {
    for change in ["changed", "added", "deleted"] {
        let fixture = Fixture::new();
        match change {
            "changed" => fs::write(fixture.0.join("dist/index.html"), "wrong UI").unwrap(),
            "added" => fs::write(fixture.0.join("dist/old.js"), "old chunk").unwrap(),
            _ => fs::remove_file(fixture.0.join("dist/assets/nested/icon.svg")).unwrap(),
        }
        assert!(
            bundle::generate(&fixture.0, "test")
                .unwrap_err()
                .to_string()
                .contains("UI assets")
        );
    }
}

#[test]
fn missing_manifest_and_different_engine_versions_are_rejected() {
    let fixture = Fixture::new();
    assert!(
        bundle::generate(&fixture.0, "other")
            .unwrap_err()
            .to_string()
            .contains("engine version")
    );
    fs::remove_file(fixture.0.join("dist/.bundle-manifest.json")).unwrap();
    assert!(bundle::generate(&fixture.0, "test").is_err());
}
