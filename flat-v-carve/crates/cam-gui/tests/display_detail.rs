//! Optional saved-job qualification: set CAM_DETAIL_JOB to a schema-5 file,
//! then run `cargo test --release -p cam-gui --test display_detail -- --ignored --nocapture`.
use cam_gui_runtime::{
    session::{self, Command},
    stock_preview::DisplayPreset,
};
use cam_service::retained::Retained;
use std::time::Instant;

#[test]
#[ignore = "requires CAM_DETAIL_JOB; measures a full saved job at physical display spacings"]
fn saved_job_at_physical_display_spacings() {
    let path = std::env::var("CAM_DETAIL_JOB").expect("set CAM_DETAIL_JOB to a saved job");
    let job = std::fs::read_to_string(path).unwrap();
    let mut service = Retained::new();
    let start = Instant::now();
    let (meta, payload) = session::execute(
        &mut service,
        Command::generate_at(job, DisplayPreset::Detail01),
    )
    .unwrap();
    let handle = meta.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let stock = meta.stock.as_ref().unwrap();
    assert_eq!(stock.cell_mm, 0.1);
    assert_eq!(stock.frames.len(), 1);
    let end = stock.frames[0].prefix;
    let expected = stock.frames[0].checksum.clone();
    println!(
        "0.1 mm: {} x {} cells; {} motions; {} bytes transferred; {} checkpoints; {:.2}s generation + simulation",
        stock.cols,
        stock.rows,
        end,
        payload.len(),
        stock.ladder_frames,
        start.elapsed().as_secs_f64()
    );
    assert!(payload.len() + serde_json::to_vec(&meta).unwrap().len() < 128_000_000);
    drop(payload);
    for prefix in [0, end / 2, end] {
        let start = Instant::now();
        let (meta, _) = session::execute(
            &mut service,
            Command::Seek {
                handle: handle.clone(),
                prefix,
            },
        )
        .unwrap();
        let stock = meta.stock.unwrap();
        assert_eq!(stock.frames[0].prefix, prefix);
        if prefix == 0 {
            assert_eq!(stock.frames[0].stats.removed_volume_mm3, 0.);
        }
        if prefix == end {
            assert_eq!(stock.frames[0].checksum, expected);
        }
        println!("seek {prefix}: {:.2}s", start.elapsed().as_secs_f64());
    }
    for preset in [
        DisplayPreset::Detail02,
        DisplayPreset::Detail05,
        DisplayPreset::Fine,
    ] {
        let start = Instant::now();
        let (meta, payload) = session::execute(
            &mut service,
            Command::DisplayPreset {
                handle: handle.clone(),
                preset,
            },
        )
        .unwrap();
        let stock = meta.stock.as_ref().unwrap();
        if let Some(cell) = preset.cell_mm() {
            assert_eq!(stock.cell_mm, cell);
        }
        assert_eq!(stock.frames[0].prefix, end);
        assert_eq!(meta.report["gui2"]["handle"], handle);
        println!(
            "{}: {:.4} mm; {} x {} cells; {} bytes transferred; {:.2}s rebuild",
            preset.label(),
            stock.cell_mm,
            stock.cols,
            stock.rows,
            payload.len(),
            start.elapsed().as_secs_f64()
        );
    }
}
