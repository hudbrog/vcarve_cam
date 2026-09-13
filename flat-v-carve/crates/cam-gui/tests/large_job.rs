//! GUI9a: a larger real carving through the same retained execution, transport
//! and display replay. The review quotes numbers from here instead of
//! extrapolating them from the small reference.
//!
//! Run the evidence pass in release, which is the only build where the
//! measured times mean anything:
//!
//! ```text
//! $env:CAM_GUI9_MEASURE_OUT = 'artifacts/gui/gui9a-large-job.json'
//! cargo test --release -p cam-gui --test large_job --locked -- --nocapture
//! ```
use cam_gui_runtime::session::{self as gui, Command};
use cam_gui_runtime::stock_preview::DisplayPreset;
use cam_service::retained::Retained;
use serde_json::json;
use std::time::Instant;

/// The real `flower_box.svg` artwork placed as a batch of identical carvings on
/// one stock: six copies of the accepted GUI2 job, about 137,000 motions.
const BATCH: &str = include_str!("../../../fixtures/gui9/flower-box-batch.job.json");

/// Apply the reviewed machine profile so the batch job is as complete as the
/// small reference; without it the planner has no controller mapping.
fn with_machine(job: &str) -> String {
    let (meta, _) = gui::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: job.into(),
            json: gui::PROFILE.into(),
        },
    )
    .unwrap();
    assert_eq!(meta.report["gui2"]["kind"], "profile");
    meta.job
}

struct Measurements {
    motions: usize,
    payload_bytes: usize,
    stock_bytes: usize,
    sim_bytes: usize,
    motion_pages: usize,
    stock_checkpoints: usize,
    display_cell_mm: f64,
    reference_cell_mm: f64,
    stock_tiles: usize,
    checkpoint_intervals: Vec<usize>,
    generation_ms: f64,
    seek_ms: Vec<f64>,
    seek_bytes: Vec<usize>,
    /// GUI9b: the same execution rebuilt at every display resolution.
    resolutions: Vec<serde_json::Value>,
}

impl Measurements {
    fn percentile(&self, quantile: f64) -> f64 {
        let mut sorted = self.seek_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if sorted.is_empty() {
            return 0.;
        }
        let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
        sorted[index]
    }
    fn report(&self, label: &str) -> serde_json::Value {
        json!({
            "workload": label,
            "motions": self.motions,
            "generationMs": self.generation_ms,
            "payloadBytes": self.payload_bytes,
            "stockBytes": self.stock_bytes,
            "motionStreamBytes": self.sim_bytes,
            "motionPages": self.motion_pages,
            "stockCheckpoints": self.stock_checkpoints,
            "displayCellMm": self.display_cell_mm,
            "referenceCellMm": self.reference_cell_mm,
            "stockTiles": self.stock_tiles,
            "checkpointIntervals": self.checkpoint_intervals,
            "seeks": self.seek_ms.len(),
            "seekMs": {
                "p50": self.percentile(0.5),
                "p95": self.percentile(0.95),
                "max": self.seek_ms.iter().copied().fold(0_f64, f64::max),
            },
            "seekTransferBytes": {
                "min": self.seek_bytes.iter().copied().min().unwrap_or(0),
                "max": self.seek_bytes.iter().copied().max().unwrap_or(0),
                "total": self.seek_bytes.iter().sum::<usize>(),
            },
            "resolutions": self.resolutions,
        })
    }
}

/// One display-resolution rebuild of the same retained execution.
struct Rebuild {
    preset: &'static str,
    cell_mm: f64,
    reference_cell_mm: f64,
    checkpoints: usize,
    retained_bytes: usize,
    payload_bytes: usize,
    prefix: usize,
    key: String,
    rebuild_ms: f64,
}

fn measure_rebuild(
    service: &mut Retained,
    handle: &str,
    preset: cam_gui_runtime::stock_preview::DisplayPreset,
    expected_prefix: usize,
) -> Rebuild {
    let started = Instant::now();
    let (meta, payload) = gui::execute(
        service,
        Command::DisplayPreset {
            handle: handle.to_owned(),
            preset,
        },
    )
    .unwrap();
    let rebuild_ms = started.elapsed().as_secs_f64() * 1000.;
    let stock = meta.stock.clone().expect("a rebuild returns stock");
    let frame = stock.frames.first().expect("a rebuild returns its frame");
    assert_eq!(
        meta.report["gui2"]["preset"].as_str(),
        Some(preset.wire()),
        "the response names the preset it built"
    );
    assert_eq!(
        frame.prefix, expected_prefix,
        "the rebuild compares at the same playhead"
    );
    Rebuild {
        preset: preset.wire(),
        cell_mm: stock.cell_mm,
        reference_cell_mm: stock.reference_cell_mm,
        checkpoints: meta.report["gui2"]["checkpoints"].as_u64().unwrap_or(0) as usize,
        retained_bytes: stock.retained_bytes,
        payload_bytes: payload.len(),
        prefix: frame.prefix,
        key: stock.key,
        rebuild_ms,
    }
}

/// A scrub pattern: forward, backward, then jumps that cross several
/// checkpoints. Every target must come back as the displayed frame.
fn scrub_pattern(total: usize) -> Vec<usize> {
    let mut targets = Vec::new();
    for step in 1..=12 {
        targets.push(total * step / 12);
    }
    for step in (0..12).rev() {
        targets.push(total * step / 12);
    }
    for fraction in [1, 3, 7, 11, 2, 9, 5, 8] {
        targets.push(total * fraction / 12);
    }
    targets
}

fn measure(job: &str) -> Measurements {
    let mut service = Retained::new();
    let started = Instant::now();
    let (scene, payload) = gui::execute(&mut service, Command::generate(job)).unwrap();
    let generation_ms = started.elapsed().as_secs_f64() * 1000.;
    assert!(
        scene.report["gui2"]["kind"] == "generated",
        "{}",
        scene.report
    );
    let handle = scene.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let stock = scene
        .stock
        .as_ref()
        .expect("generated scene keeps its stock");
    let checkpoints: Vec<usize> = stock.frames.iter().map(|frame| frame.prefix).collect();
    let checkpoint_intervals = checkpoints
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    assert!(scene.transport.sim_bytes > 0, "the motion stream travels");
    assert_eq!(scene.transport.payload_bytes, payload.len());

    let mut seek_ms = Vec::new();
    let mut seek_bytes = Vec::new();
    let mut cold: Option<(usize, Vec<u8>)> = None;
    let targets = scrub_pattern(scene.motions);
    for prefix in targets.iter().copied() {
        let started = Instant::now();
        let (meta, payload) = gui::execute(
            &mut service,
            Command::Seek {
                handle: handle.clone(),
                prefix,
            },
        )
        .unwrap();
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        let frame = meta
            .stock
            .as_ref()
            .and_then(|stock| stock.frames.first())
            .expect("a seek returns the requested stock frame");
        assert_eq!(frame.prefix, prefix, "the frame is the requested prefix");
        // A seek response carries the stock frame only; the scene geometry it
        // already holds is not re-sent.
        assert_eq!(meta.report["gui2"]["kind"], "seek");
        assert_eq!(meta.report["gui2"]["prefix"].as_u64(), Some(prefix as u64));
        seek_ms.push(elapsed);
        seek_bytes.push(payload.len());
        // Keep one transported field for the independent replay check.
        if cold.is_none() {
            let response = cam_gui_runtime::compute::Scene {
                meta,
                payload: std::sync::Arc::new(payload),
            };
            cold = Some((prefix, response.stock_cells(0).unwrap().to_vec()));
        }
    }
    let last_scrub = *targets.last().expect("the pattern has targets");

    // The transported seek is not trusted on its own: replay the same prefix
    // from the pristine field and require identical cells.
    let (prefix, transported) = cold.unwrap();
    let sim = cam_gui_runtime::compute::Scene {
        meta: scene.clone(),
        payload: std::sync::Arc::new(payload),
    }
    .sim_input()
    .unwrap()
    .unwrap();
    let mut replay =
        cam_gui_runtime::sim::Field::new(sim.stock, &sim.tools, stock.cell_mm).unwrap();
    for motion in &sim.motions[..prefix] {
        replay.apply(motion, 0., 1.).unwrap();
    }
    assert_eq!(transported, replay.packed_tile_bytes());

    // GUI9b: the same retained execution at three display resolutions. The plan
    // is not replanned and the playhead does not move.
    let resolutions = DisplayPreset::ALL
        .into_iter()
        .map(|preset| {
            let rebuild = measure_rebuild(&mut service, &handle, preset, last_scrub);
            assert!(rebuild.cell_mm >= rebuild.reference_cell_mm);
            json!({
                "preset": rebuild.preset,
                "cellMm": rebuild.cell_mm,
                "referenceCellMm": rebuild.reference_cell_mm,
                "checkpoints": rebuild.checkpoints,
                "retainedBytes": rebuild.retained_bytes,
                "payloadBytes": rebuild.payload_bytes,
                "prefix": rebuild.prefix,
                "simulationKey": rebuild.key,
                "rebuildMs": rebuild.rebuild_ms,
            })
        })
        .collect::<Vec<_>>();
    // Another resolution is another raster: each one has its own key, so a
    // checkpoint or tile from one can never be shown as the other.
    let keys: Vec<&str> = resolutions
        .iter()
        .map(|entry| entry["simulationKey"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        keys.iter().collect::<std::collections::HashSet<_>>().len(),
        keys.len(),
        "each display resolution has its own simulation key: {keys:?}"
    );

    Measurements {
        motions: scene.motions,
        payload_bytes: scene.payload_bytes,
        stock_bytes: scene.transport.stock_bytes,
        sim_bytes: scene.transport.sim_bytes,
        motion_pages: scene.transport.motion_pages,
        stock_checkpoints: scene.transport.stock_checkpoints,
        display_cell_mm: stock.cell_mm,
        reference_cell_mm: stock.reference_cell_mm,
        stock_tiles: stock.tiles_x * stock.tiles_y,
        checkpoint_intervals,
        generation_ms,
        seek_ms,
        seek_bytes,
        resolutions,
    }
}

#[test]
#[ignore = "GUI9a evidence run: real batch planning plus 32 scrubs, release build"]
fn the_flower_box_batch_generates_and_scrubs_inside_the_display_budget() {
    let batch = measure(&with_machine(BATCH));
    assert!(
        batch.motions > 100_000,
        "the GUI9 fixture must be larger than the old 100,000-motion limit, got {}",
        batch.motions
    );
    assert!(batch.motions <= gui::MOTION_LIMIT);
    assert!(
        batch.motion_pages > 12,
        "a larger job must span many motion pages, got {}",
        batch.motion_pages
    );
    // The plan's M-scrub target: the latest requested view appears within
    // 250 ms when nearby pages/checkpoints are resident. Timing is only
    // meaningful in the release build the review artifacts use.
    if !cfg!(debug_assertions) {
        assert!(
            batch.percentile(0.95) <= 250.,
            "p95 seek {:.1} ms exceeds the 250 ms display budget",
            batch.percentile(0.95)
        );
    }
    assert!(
        batch
            .checkpoint_intervals
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            > 0,
        "transported checkpoints must cover the job"
    );

    let small = measure(&with_machine(gui::FLOWER));
    assert_eq!(small.motions, 22_883, "the small reference is unchanged");

    if let Ok(path) = std::env::var("CAM_GUI9_MEASURE_OUT") {
        let report = json!({
            "protocol": cam_gui_runtime::compute::PROTOCOL,
            "fixture": "fixtures/gui9/flower-box-batch.job.json",
            "small": small.report("flower box reference"),
            "large": batch.report("flower box batch 3x2"),
            "resolutions": batch.resolutions,
        });
        // Relative paths are workspace-relative (`cargo test` runs the binary
        // from the crate directory).
        let path = std::path::Path::new(&path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path)
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}

#[test]
fn a_plan_above_the_display_motion_limit_is_refused_by_name() {
    assert!(gui::admit_motions(gui::MOTION_LIMIT).is_ok());
    let refused = gui::admit_motions(gui::MOTION_LIMIT + 1).unwrap_err();
    assert!(
        refused.contains(&gui::MOTION_LIMIT.to_string()),
        "the refusal names the limit: {refused}"
    );
    assert!(
        refused.contains(&(gui::MOTION_LIMIT + 1).to_string()),
        "the refusal names the plan size: {refused}"
    );
}
