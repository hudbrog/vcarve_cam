//! The viewport's WGSL is compiled by wgpu when the renderer is created, so a
//! syntax or type error reaches the user as a failed viewport rather than as a
//! failing build. Parse and validate every shader here with the same front end
//! and validator wgpu uses, offline and without a GPU.
use naga::valid::{Capabilities, ValidationFlags, Validator};

fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name} does not parse: {error:?}"));
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    validator
        .validate(&module)
        .unwrap_or_else(|error| panic!("{name} does not validate: {error:?}"));
}

#[test]
fn every_viewport_shader_parses_and_validates() {
    validate("scene.wgsl", include_str!("../src/scene.wgsl"));
    validate("stock.wgsl", include_str!("../src/stock.wgsl"));
}
