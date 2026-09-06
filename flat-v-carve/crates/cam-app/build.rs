use std::process::Command;
#[path = "build/bundle.rs"]
mod bundle;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/bundle.rs");
    println!("cargo:rerun-if-env-changed=RUSTC");
    let rustc = Command::new(std::env::var_os("RUSTC").expect("Cargo provides RUSTC"))
        .arg("--version")
        .output()
        .expect("read build toolchain");
    assert!(rustc.status.success(), "rustc --version failed");
    println!(
        "cargo:rustc-env=CAM_RUSTC={}",
        String::from_utf8(rustc.stdout)
            .expect("Rust version is UTF-8")
            .trim()
    );
    println!(
        "cargo:rustc-env=CAM_TARGET={}",
        std::env::var("TARGET").expect("Cargo provides TARGET")
    );
    if std::env::var_os("CARGO_FEATURE_BUNDLED_UI").is_some() {
        let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let web = root
            .join("../../web")
            .canonicalize()
            .expect("workspace web directory");
        // Directory watches also catch added/deleted files; hashes catch stale builds.
        println!("cargo:rerun-if-changed={}", web.display());
        println!("cargo:rerun-if-changed=../../Cargo.toml");
        let assets = bundle::generate(&web, &std::env::var("CARGO_PKG_VERSION").unwrap())
            .unwrap_or_else(|e| {
                panic!("Cannot embed UI: {e}. Run pnpm build in flat-v-carve/web first.")
            });
        std::fs::write(
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
                .join("bundled_assets.rs"),
            assets,
        )
        .expect("write embedded asset table");
    }
}
