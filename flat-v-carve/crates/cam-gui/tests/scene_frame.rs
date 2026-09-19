//! One display frame for the whole scene: the artwork, the stock rectangle and
//! the generated toolpath are normalized against the same bounds, so a facing
//! pass that travels past the stock widens the frame instead of rescaling the
//! artwork drawn inside it. Field report 2026-09-13, reproduced from
//! `real_data/facing_job.json`: "when generating the path the loaded SVG is
//! rendered larger than the stock material", and switching the pass angle to
//! 90 degrees made it stop reproducing.
use cam_core::{
    job::SourceSnapshot,
    project::{
        FaceEntry, FacePattern,
        v5::{ArtworkContent, CamJobV5, FaceSettingsV5, OperationSettingsV5},
    },
};
use cam_gui_runtime::{
    compute, pages,
    session::{self, Command},
};
use cam_service::retained::Retained;

/// Bytes per [`compute::Vertex`]: `[f32; 3] position + [f32; 4] color`, both
/// four-byte aligned, so the payload is read here without any unsafe casting.
const VERTEX_BYTES: usize = 28;

/// A page-sized artwork, like the reported job: the imported geometry fills
/// the whole 200 × 100 mm page, which is also the stock rectangle.
const PAGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm" viewBox="0 0 200 100"><rect id="plate" x="0" y="0" width="200" height="100" fill="#000000"/></svg>"##;

/// The reported job as a fixture: a 200 × 100 × 18 mm stock, its page-sized
/// artwork, and one whole-stock face operation with the 50.2 mm plate that
/// made the toolpath travel a full cutter radius past every row end.
fn facing_job(pass_angle_deg: f64) -> CamJobV5 {
    // The tester's own job file, which is the fixture this test reproduces.
    let mut job: CamJobV5 =
        serde_json::from_str(include_str!("../../../../real_data/facing_job.json")).unwrap();
    // The report's artwork fills the page, so the frame's artwork and stock
    // extents are the same rectangle; the tester's saved file keeps a smaller
    // drawing on the same page.
    job.artwork[0].content = ArtworkContent::Svg(SourceSnapshot {
        filename: "flower_box.svg".into(),
        svg: PAGE.into(),
    });
    let OperationSettingsV5::Face(settings) = &mut job.operations[0].settings else {
        panic!("the fixture carries one face operation")
    };
    settings.pass_angle_deg = Some(pass_angle_deg);
    job
}

fn generate(job: &CamJobV5) -> compute::Scene {
    let (meta, payload) = session::execute(
        &mut Retained::new(),
        Command::generate(job.to_json().unwrap()),
    )
    .unwrap();
    assert!(meta.motions > 0, "the facing operation produces motions");
    compute::Scene {
        meta,
        payload: std::sync::Arc::new(payload),
    }
}

/// Undo the scene's normalization: a vertex is written as
/// `(setup - frame centre) / frame size * 1.6`, so the frame a vertex was
/// normalized against is recoverable from the payload alone.
fn setup_point(position: [f32; 3], bounds: [f64; 4]) -> (f64, f64) {
    let size = (bounds[2] - bounds[0])
        .max(bounds[3] - bounds[1])
        .max(0.001);
    (
        position[0] as f64 / 1.6 * size + (bounds[0] + bounds[2]) / 2.,
        position[1] as f64 / 1.6 * size + (bounds[1] + bounds[3]) / 2.,
    )
}

fn contour_extent(scene: &compute::Scene) -> (f64, f64, f64, f64) {
    let section = scene
        .meta
        .sections
        .iter()
        .find(|s| s.kind == pages::SECTION_CONTOUR)
        .expect("an artwork scene carries the contour section");
    let bytes = &scene.payload[section.offset..section.offset + section.len];
    let count = scene.meta.contour_vertices;
    assert_eq!(
        bytes.len(),
        count * VERTEX_BYTES,
        "the contour section holds exactly the artwork vertices"
    );
    let mut extent = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for index in 0..count {
        let at = index * VERTEX_BYTES;
        let x = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let y = f32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        let (x, y) = setup_point([x, y, 0.], scene.meta.bounds);
        extent = (
            extent.0.min(x),
            extent.1.min(y),
            extent.2.max(x),
            extent.3.max(y),
        );
    }
    extent
}

#[test]
fn the_artwork_keeps_its_size_whatever_frame_the_toolpath_needs() {
    let along_x = generate(&facing_job(0.));
    let along_y = generate(&facing_job(90.));
    // The two pass directions genuinely need different display frames: the
    // 0-degree raster runs along X and reaches a full cutter radius past both
    // ends of the 200 mm axis, while the 90-degree raster leaves the frame
    // through the 100 mm axis.
    assert_ne!(
        along_x.meta.bounds, along_y.meta.bounds,
        "the two pass angles must not produce one frame"
    );
    // The toolpath really does leave the stock in both directions, so the
    // frame is carrying real travel and not a rounding artifact.
    for scene in [&along_x, &along_y] {
        let sim = scene.sim_input().unwrap().unwrap();
        let motions = sim
            .motions
            .iter()
            .flat_map(|m| [(m.x0, m.y0), (m.x1, m.y1)])
            .fold(
                (
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ),
                |extent, (x, y)| {
                    (
                        extent.0.min(x),
                        extent.1.min(y),
                        extent.2.max(x),
                        extent.3.max(y),
                    )
                },
            );
        assert!(
            motions.0 < sim.stock.x0 || motions.1 < sim.stock.y0,
            "the facing pass starts outside the stock: {motions:?}"
        );
        assert!(
            motions.2 > sim.stock.x1 || motions.3 > sim.stock.y1,
            "the facing pass ends outside the stock: {motions:?}"
        );
    }
    // The artwork is the page, and the stock is the page: on both frames the
    // contour vertices must come back as the 200 × 100 mm rectangle the
    // artwork actually occupies. Before the frame was settled ahead of the
    // vertices, the artwork was normalized against the pre-plan frame and came
    // back 25% oversized at 0 degrees (200 -> 250.2 mm) and almost exactly
    // right at 90 degrees (200 -> 202 mm), which is why the report read as
    // intermittent.
    for (angle, scene) in [(0., &along_x), (90., &along_y)] {
        let stock = scene.meta.sim.as_ref().unwrap().stock;
        let (min_x, min_y, max_x, max_y) = contour_extent(scene);
        for (name, got, want) in [
            ("min_x", min_x, stock.x0),
            ("min_y", min_y, stock.y0),
            ("max_x", max_x, stock.x1),
            ("max_y", max_y, stock.y1),
        ] {
            assert!(
                (got - want).abs() < 1e-3,
                "pass angle {angle}: artwork {name} is {got:.6}, stock is {want:.6} \
                 (frame {:?})",
                scene.meta.bounds
            );
        }
        assert!(
            (max_x - min_x - 200.).abs() < 1e-3 && (max_y - min_y - 100.).abs() < 1e-3,
            "pass angle {angle}: the artwork is {} × {} mm",
            max_x - min_x,
            max_y - min_y
        );
    }
}

/// The viewport draws the facing request, the coverage and the entry from the
/// same resolution the panel shows, published with the scene before a plan
/// exists (plan section 11.1 and the W3 preview half).
#[test]
fn the_scene_publishes_the_facing_request_coverage_and_entry() {
    let scene = generate(&facing_job(0.));
    let plans = scene.meta.report["gui2"]["facePlans"]
        .as_array()
        .expect("the scene publishes the facing overlays")
        .clone();
    assert_eq!(plans.len(), 1, "{plans:?}");
    let plan = &plans[0];
    assert_eq!(plan["operationId"], "face-1");
    assert_eq!(plan["axis"], "X");
    // The reported job faces the whole 200 x 100 stock with no margins, so the
    // coverage is the stock rectangle and the request is the same rectangle.
    let coverage = plan["coverage"].as_array().unwrap();
    let values: Vec<f64> = coverage.iter().map(|v| v.as_f64().unwrap()).collect();
    assert_eq!(values, vec![0., 0., 200., 100.]);
    // The allowed envelope is the coverage, the travel and the cutter radius:
    // with no overrun it is one 25.1 mm radius wider on every side.
    let envelope = plan["envelope"].as_array().unwrap();
    let values: Vec<f64> = envelope.iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(
        // The envelope also carries the planner's numerical reserve.
        (values[0] + 25.1).abs() < 1e-4
            && (values[1] + 25.1).abs() < 1e-4
            && (values[2] - (200. + 50.2)).abs() < 1e-4
            && (values[3] - (100. + 50.2)).abs() < 1e-4,
        "{values:?}"
    );
    // Every pass enters at the coverage's X minimum end, inside the cutter
    // radius of the stock edge: the tangent entry the report's job relies on.
    let entries = plan["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    for entry in entries {
        assert!((entry.as_f64().unwrap() - -25.1).abs() < 1e-6, "{entry}");
    }
    for clearance in plan["clearances"].as_array().unwrap() {
        assert!(
            clearance.as_f64().unwrap() >= -1e-9,
            "a tangent entry clears the stock: {clearance}"
        );
    }
    // The viewport draws its travel arrows from these numbers, so they have to
    // be the pass's own span and the two travel values — nothing reconstructed.
    assert!((plan["passLow"].as_f64().unwrap() - -25.1).abs() < 1e-6);
    assert!((plan["passHigh"].as_f64().unwrap() - 225.1).abs() < 1e-6);
    assert_eq!(plan["entryTravel"].as_f64().unwrap(), 0.);
    assert_eq!(plan["exitTravel"].as_f64().unwrap(), 0.);
}

/// W3 acceptance: changing any facing parameter may change the travel and the
/// frame it needs, but never the stock rectangle or the artwork inside it.
#[test]
fn facing_parameters_never_move_the_stock_or_the_artwork() {
    fn settings(job: &mut CamJobV5) -> &mut FaceSettingsV5 {
        let OperationSettingsV5::Face(settings) = &mut job.operations[0].settings else {
            panic!("the fixture carries one face operation")
        };
        settings
    }
    fn extent_of(stock: Option<&cam_gui_runtime::sim::Stock>) -> (f64, f64, f64, f64) {
        let stock = stock.expect("a generated scene has a simulated stock");
        (stock.x0, stock.y0, stock.x1, stock.y1)
    }

    let reference = generate(&facing_job(0.));
    let want_artwork = contour_extent(&reference);
    let want_stock = extent_of(reference.meta.sim.as_ref().map(|sim| &sim.stock));
    /// One facing parameter change, named for the failure message.
    type Variation = (&'static str, fn(&mut FaceSettingsV5));
    let variations: [Variation; 10] = [
        ("coverage margin min X", |s| s.margins.min_x_mm = Some(5.)),
        ("coverage margin max X", |s| s.margins.max_x_mm = Some(5.)),
        ("coverage margin min Y", |s| s.margins.min_y_mm = Some(5.)),
        ("coverage margin max Y", |s| s.margins.max_y_mm = Some(5.)),
        ("entry travel", |s| s.entry_overrun_mm = Some(20.)),
        ("exit travel", |s| s.exit_overrun_mm = Some(20.)),
        ("pass angle", |s| s.pass_angle_deg = Some(90.)),
        ("pattern", |s| s.pattern = FacePattern::OneWay),
        ("entry at the high end", |s| s.entry = FaceEntry::Max),
        ("explicit entry", |s| {
            s.entry = FaceEntry::At {
                coordinate_mm: -60.,
            }
        }),
    ];
    for (name, edit) in variations {
        let mut job = facing_job(0.);
        edit(settings(&mut job));
        let scene = generate(&job);
        // The payload holds normalized `f32` vertices, so the same setup point
        // rounds a little differently under a different frame; the geometry
        // itself must not move beyond display precision.
        let got = contour_extent(&scene);
        for (index, (got, want)) in [
            (got.0, want_artwork.0),
            (got.1, want_artwork.1),
            (got.2, want_artwork.2),
            (got.3, want_artwork.3),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                (got - want).abs() < 1e-3,
                "{name} moved the artwork: {got} vs {want} at {index} ({got:?} vs {want_artwork:?})"
            );
        }
        assert_eq!(
            extent_of(scene.meta.sim.as_ref().map(|sim| &sim.stock)),
            want_stock,
            "{name} moved the stock"
        );
    }
}

/// S1: playback is a machine clock, not a step counter. The reported job's
/// passes and plunges are timed from their own feeds — 2400 and 600 mm/min in
/// `real_data/facing_job.json` — so a long pass costs many times a short plunge
/// instead of one step each, and the whole program takes the time its own
/// numbers say rather than [`FIT_SECONDS`](cam_gui_runtime::viewport) of steps.
#[test]
fn the_facing_job_is_timed_by_its_own_feeds() {
    let scene = generate(&facing_job(0.));
    let sim = scene
        .meta
        .sim
        .as_ref()
        .expect("the scene carries a motion stream");
    let input = scene
        .sim_input()
        .unwrap()
        .expect("the transported stream decodes");
    assert_eq!(input.motions.len(), sim.motions);
    assert_eq!(input.motions.len(), scene.meta.motions);

    let table =
        cam_gui_runtime::sim::TimeTable::build(&input.motions, sim.rapid_rate_mm_min).unwrap();
    assert_eq!(table.motions(), input.motions.len());
    // The tester's job states no rapid rate, so the clock says it is using the
    // display's fallback rather than presenting one as machine truth.
    assert!(table.assumes_rapid_rate());
    assert!(
        (table.rapid_rate_mm_min() - cam_gui_runtime::sim::DEFAULT_RAPID_RATE_MM_MIN).abs() < 1e-9
    );

    // The feed the plan carries reaches the simulator stream, and a rapid
    // carries none: the clock must tell the two apart to time anything.
    let mut feeds: Vec<f64> = input
        .motions
        .iter()
        .filter_map(|motion| motion.feed_mm_min)
        .collect();
    feeds.sort_by(f64::total_cmp);
    feeds.dedup();
    assert_eq!(
        feeds,
        vec![2400.],
        "the job's cutting feed is the only feed in this program"
    );
    assert!(
        input
            .motions
            .iter()
            .filter(|motion| motion.interpolation == cam_gui_runtime::sim::Interpolation::Rapid)
            .all(|motion| motion.feed_mm_min.is_none()),
        "a rapid carries no feed"
    );

    // Per-move time is proportional to length and feed, so the longest feed
    // move takes several times the shortest: the shape the transport shows.
    let mut durations: Vec<f64> = input
        .motions
        .iter()
        .enumerate()
        .filter(|(_, motion)| motion.feed_mm_min.is_some())
        .map(|(index, _)| table.duration_of(index))
        .filter(|seconds| *seconds > 0.)
        .collect();
    durations.sort_by(f64::total_cmp);
    let shortest = durations.first().copied().expect("a timed feed move");
    let longest = durations.last().copied().expect("a timed feed move");
    assert!(
        longest / shortest > 3.,
        "a long pass must not cost the same as a short plunge: {longest:.3} s vs {shortest:.3} s"
    );

    // The program's own motion time, not a fixed playback window.
    let total = table.total_seconds();
    assert!(
        total > 30.,
        "the facing job is minutes of motion, not {total:.1} s"
    );
    assert!(total < 3600., "the facing job is not an hour: {total:.1} s");
    // Every motion contributes its own length at its own rate.
    for (index, motion) in input.motions.iter().enumerate() {
        let expected = match motion.interpolation {
            cam_gui_runtime::sim::Interpolation::Rapid => {
                motion.length_mm() / table.rapid_rate_mm_min() * 60.
            }
            cam_gui_runtime::sim::Interpolation::Feed => {
                motion.length_mm() / motion.feed_mm_min.expect("a linear feed carries one") * 60.
            }
            cam_gui_runtime::sim::Interpolation::Dwell { seconds } => seconds,
        };
        assert!(
            (table.duration_of(index) - expected).abs() < 1e-9,
            "motion {index} is not timed by its own length and rate"
        );
    }
}

/// S3: the machine checks run in the worker with the execution they describe,
/// over their own coarse raster, and the report names that raster. A holder that
/// would stand inside the job is reported with the motion it happens at; the
/// same job with a stickout that clears it is silent.
#[test]
fn a_holder_that_would_hit_the_job_is_reported_with_the_execution() {
    // A 3 mm deep carve with a 2.5 mm cutter that sticks only 2 mm out of its
    // holder: the 25 mm nut beside the carve is inside the job. The material
    // around the carve is untouched, which is what a wide holder hits.
    let mut tight: CamJobV5 =
        serde_json::from_str(include_str!("../../../fixtures/gui4/lettering.job.json"))
            .expect("the lettering fixture");
    if let OperationSettingsV5::FlatVcarve(settings) = &mut tight.operations[0].settings {
        settings.max_depth_mm = Some(3.);
    }
    tight
        .tools
        .iter_mut()
        .find(|tool| tool.id == "endmill")
        .expect("the fixture's endmill")
        .assembly = cam_core::project::ToolAssembly {
        shaft_diameter_mm: Some(2.5),
        // The carve reaches 1.5 mm deep in this fixture, so a tool that sticks
        // 0.8 mm out of its holder puts the nut inside the material beside it.
        stickout_mm: Some(0.8),
    };
    let tight = with_holder(tight, "er20");
    let scene = generate(&tight);
    let warnings = scene.meta.report["gui2"]["warnings"]
        .as_array()
        .expect("the scene publishes its machine warnings")
        .clone();
    assert!(
        !warnings.is_empty(),
        "a 6 mm stickout cannot clear this job"
    );
    let first = warnings
        .iter()
        .find(|warning| warning["kind"] == "assemblyBelowSurface")
        .unwrap_or_else(|| panic!("the holder stands inside the job: {warnings:?}"));
    assert!(
        first["motion"]
            .as_u64()
            .is_some_and(|motion| motion < scene.meta.motions as u64),
        "the deepest pass reports it: {first:?}"
    );
    // The row names the first motion that shows the problem and how deep it gets:
    // the deepest layer puts the nut's underside 0.7 mm below the untouched
    // surface beside the carve.
    let deepest = first
        .get("maxDepthMm")
        .and_then(|depth| depth.as_f64())
        .unwrap_or(0.);
    assert!(
        (deepest - 0.7).abs() < 0.4,
        "the nut reaches {deepest:.2} mm into the material"
    );
    // The raster the estimate rests on travels with it.
    let cell = scene.meta.report["gui2"]["warningCellMm"]
        .as_f64()
        .expect("the warning raster is named");
    assert!(cell > 0., "{cell}");

    // The same job with a stickout that clears the stock is silent: the nut's
    // bottom face sits above the top of the material.
    let mut clear = tight.clone();
    clear
        .tools
        .iter_mut()
        .find(|tool| tool.id == "endmill")
        .expect("the fixture's endmill")
        .assembly
        .stickout_mm = Some(40.);
    let scene = generate(&clear);
    let warnings = scene.meta.report["gui2"]["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        warnings.is_empty(),
        "a 60 mm stickout clears the 18 mm stock: {warnings:?}"
    );
}

/// Bind a holder to a job the way the GUI does: apply a machine configuration
/// whose profile names the holder.
fn with_holder(job: CamJobV5, holder: &str) -> CamJobV5 {
    let mut profile =
        cam_core::post::sequence::SequenceProfile::from_json(session::PROFILE).unwrap();
    profile.holder = Some(cam_core::post::HolderSelection::catalogue(holder));
    cam_core::project::v5::machine::apply_machine_configuration(&job, &profile, "Checks")
        .unwrap()
        .job
}

/// The other half of S3's holder row: a machine that states its own cylinder
/// instead of a catalogue series reaches the display and the checks the same way.
#[test]
fn a_machine_that_states_its_own_holder_cylinder_is_checked() {
    let mut job: CamJobV5 =
        serde_json::from_str(include_str!("../../../fixtures/gui4/lettering.job.json"))
            .expect("the lettering fixture");
    if let OperationSettingsV5::FlatVcarve(settings) = &mut job.operations[0].settings {
        settings.max_depth_mm = Some(3.);
    }
    job.tools
        .iter_mut()
        .find(|tool| tool.id == "endmill")
        .expect("the fixture's endmill")
        .assembly = cam_core::project::ToolAssembly {
        shaft_diameter_mm: Some(2.5),
        stickout_mm: Some(0.8),
    };
    let mut profile =
        cam_core::post::sequence::SequenceProfile::from_json(session::PROFILE).unwrap();
    profile.holder = Some(cam_core::post::HolderSelection {
        id: "custom".into(),
        segments: vec![cam_core::post::HolderSegment {
            height_mm: 30.,
            lower_diameter_mm: 20.,
            upper_diameter_mm: 20.,
        }],
    });
    let job = cam_core::project::v5::machine::apply_machine_configuration(&job, &profile, "Custom")
        .unwrap()
        .job;
    let scene = generate(&job);
    let sim = scene.meta.sim.as_ref().expect("a motion stream");
    let body = sim
        .holder
        .as_ref()
        .and_then(|holder| holder.body())
        .expect("the machine's own cylinder");
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].lower_diameter_mm, 20.);
    let warnings = scene.meta.report["gui2"]["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let assembly = warnings
        .iter()
        .find(|warning| warning["kind"] == "assemblyBelowSurface")
        .unwrap_or_else(|| panic!("the 20 mm cylinder stands in the material: {warnings:?}"));
    assert!(
        assembly["maxDepthMm"]
            .as_f64()
            .is_some_and(|depth| depth > 0.4),
        "{assembly:?}"
    );
}
