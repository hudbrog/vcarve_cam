//! One display frame for the whole scene: the artwork, the stock rectangle and
//! the generated toolpath are normalized against the same bounds, so a facing
//! pass that travels past the stock widens the frame instead of rescaling the
//! artwork drawn inside it. Field report 2026-09-13, reproduced from
//! `real_data/facing_job.json`: "when generating the path the loaded SVG is
//! rendered larger than the stock material", and switching the pass angle to
//! 90 degrees made it stop reproducing.
use cam_core::{
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, EndmillGeometry, FaceArea, FaceMargins, FacePattern, FaceSettings, HeightRef,
        HeightReference, JobTool, MillingAssignment, Operation, OperationSettings, RectXY,
        SetupSettings, SpindleDirection, StockSetup, ToolCapabilities, ToolGeometry, WorkZero,
        WorkZeroXY, WorkZeroZ,
        v5::{CamJobV5, migrate::migrate_v4},
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
    let job = CamJob {
        schema_version: 4,
        name: "facing_job".into(),
        source: Some(SourceSnapshot {
            filename: "flower_box.svg".into(),
            svg: PAGE.into(),
        }),
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(18.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 200.,
                    length_mm: 100.,
                }),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![JobTool {
            id: "tool-4".into(),
            name: "50.2 plate".into(),
            geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                diameter_mm: 50.2,
                cutting_length_mm: 12.,
            })),
            // The reported tool was marked unable to plunge; the frame must
            // not depend on that, so the fixture declares the capability and
            // leaves the entry question to the facing tests.
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: Some(false),
            },
        }],
        operations: vec![Operation {
            id: "face-1".into(),
            name: "Face".into(),
            enabled: true,
            settings: OperationSettings::Face(FaceSettings {
                area: FaceArea::EntireStock,
                margins: FaceMargins::default(),
                entry_overrun_mm: None,
                exit_overrun_mm: None,
                top: HeightRef {
                    reference: HeightReference::StockTop,
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::StockBottom,
                    offset_mm: 14.,
                },
                stepdown_mm: Some(1.),
                stepover_mm: Some(25.),
                pass_angle_deg: Some(pass_angle_deg),
                pattern: FacePattern::ZigZag,
                assignment: MillingAssignment {
                    tool_id: "tool-4".into(),
                    spindle_rpm: Some(16_000.),
                    spindle_direction: Some(SpindleDirection::Clockwise),
                    cutting_feed_mm_min: Some(2400.),
                    plunge_feed_mm_min: Some(600.),
                    max_stepdown_mm: Some(1.),
                    stepover_mm: Some(25.),
                },
            }),
        }],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    };
    migrate_v4(&job).unwrap()
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
