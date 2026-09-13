use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
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
}
