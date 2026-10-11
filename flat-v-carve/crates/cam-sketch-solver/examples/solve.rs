//! Read a SketchDocument from stdin; write its SolveReport to stdout.
use std::io::{self, Read};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin()
        .take(cam_core::sketch::MAX_SKETCH_BYTES as u64 + 1)
        .read_to_string(&mut input)?;
    let output = cam_sketch_solver::solve_json(&input)?;
    println!("{output}");
    Ok(())
}
