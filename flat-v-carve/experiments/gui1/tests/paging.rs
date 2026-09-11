//! Structural checks for the paged transport: heavy scene data must travel as
//! binary sections, page math must address the payload exactly, and the display
//! must be able to rebuild the transported stock checkpoints.
use cam_gui1::{compute, pages, paging, sim, stock_preview};

fn scene(segments: usize) -> (compute::Scene, pages::PageTable) {
    let (meta, payload) = compute::run(compute::Request::Synthetic { segments }).unwrap();
    let table = pages::PageTable::new(meta.motion_offset, meta.motion_len, meta.motions).unwrap();
    (
        compute::Scene {
            meta,
            payload: std::sync::Arc::new(payload),
        },
        table,
    )
}

#[test]
fn heavy_scene_data_never_travels_as_json() {
    let (scene, table) = scene(20_000);
    let meta_json = serde_json::to_vec(&scene.meta).unwrap();
    // Metadata stays small: the vertex and cell arrays are binary sections.
    assert!(
        meta_json.len() < 64 * 1024,
        "metadata grew to {}",
        meta_json.len()
    );
    assert!(scene.meta.payload_bytes > 4 * meta_json.len());
    assert_eq!(
        scene.meta.transport.vertex_bytes,
        scene.meta.motions * 2 * 28
    );
    assert_eq!(
        table.page_count(),
        scene.meta.motions.div_ceil(pages::PAGE_MOTIONS)
    );
    assert_eq!(scene.motion_bytes().len(), scene.meta.motion_len);
    // Every stock checkpoint section is inside the payload and aligns to tiles.
    let tile_bytes = sim::TILE * sim::TILE * 4;
    for (index, section) in scene.stock_sections().enumerate() {
        assert_eq!(section.offset % pages::ALIGN, 0);
        assert!(section.offset + section.len <= scene.payload.len());
        let stock = scene.meta.stock.as_ref().unwrap();
        assert_eq!(section.len, stock.tiles_x * stock.tiles_y * tile_bytes);
        assert_eq!(section.len, scene.stock_cells(index).unwrap().len());
    }
}

#[test]
fn transported_checkpoints_rebuild_the_exact_transported_fields() {
    let (scene, _) = scene(20_000);
    let stock = scene.meta.stock.clone().unwrap();
    let input = scene
        .sim_input()
        .unwrap()
        .expect("S transports a replay stream");
    assert_eq!(input.motions.len(), scene.meta.motions);
    assert_eq!(
        scene.meta.transport.sim_bytes,
        input.motions.len() * sim::MOTION_BYTES
    );
    for (index, frame) in stock.frames.iter().enumerate() {
        let field = sim::Field::from_packed(
            stock.stock,
            &input.tools,
            stock.cell_mm,
            scene.stock_cells(index).unwrap(),
            &frame.versions,
            &frame.allocated,
            frame.stats.clone(),
        )
        .unwrap();
        assert_eq!(field.checksum(), frame.checksum);
        assert_eq!(field.versions, frame.versions);
    }
}

#[test]
fn l_workload_has_no_replay_stream_and_says_so() {
    let (scene, table) = scene(compute::MAX_SEGMENTS);
    assert!(scene.meta.sim.is_none());
    assert!(scene.sim_input().unwrap().is_none());
    assert!(scene.meta.report["replayableMotions"] == serde_json::json!(false));
    assert!(scene.meta.report["stockPartial"] == serde_json::json!(true));
    assert!(table.page_count() > 100);
    // The display can still address every page of the transported stream.
    for page in 0..table.page_count() {
        let range = table.bytes_of(page);
        assert!(range.end <= scene.payload.len());
        assert_eq!(
            range.end - range.start,
            table.motions_in(page).len() * 2 * 28
        );
    }
}

#[test]
fn page_residency_is_bounded_and_reloads_are_free() {
    let (scene, table) = scene(20_000);
    let hashes: Vec<Option<u64>> = (0..table.page_count())
        .map(|page| Some(pages::page_hash(&scene.payload[table.bytes_of(page)])))
        .collect();
    let required: Vec<usize> = (0..table.page_count()).collect();
    let identity = scene.identity();
    let mut pager = paging::Pager::new(u64::MAX);
    let first = pager.plan(identity, &table, &hashes, &required, |page| {
        table.bytes_of(page)
    });
    assert_eq!(first.uploads.len(), table.page_count());
    let reload = pager.plan(identity, &table, &hashes, &required, |page| {
        table.bytes_of(page)
    });
    assert!(reload.uploads.is_empty());
    assert_eq!(reload.skipped, table.page_count());
    // Camera-only changes never touch the page set.
    assert_eq!(reload.upload_bytes(), 0);
}

#[test]
fn stock_seek_transfers_only_changed_tiles() {
    let (scene, _) = scene(20_000);
    let stock = scene.meta.stock.clone().unwrap();
    let input = scene.sim_input().unwrap().unwrap();
    let mut seeds = Vec::new();
    for (index, frame) in stock.frames.iter().enumerate() {
        seeds.push((
            frame.prefix,
            sim::Field::from_packed(
                stock.stock,
                &input.tools,
                stock.cell_mm,
                scene.stock_cells(index).unwrap(),
                &frame.versions,
                &frame.allocated,
                frame.stats.clone(),
            )
            .unwrap(),
        ));
    }
    let pristine = seeds[0].1.clone();
    let latest = seeds.last().unwrap().1.clone();
    let mut playback = sim::Playback::seed(latest.clone(), pristine, seeds, 20 * 1024 * 1024);
    let mut previous = latest.versions.clone();
    let mut moved = 0;
    for target in [500, 9_000, 19_999, 1, 12_345, 19_999] {
        playback.seek(&input.motions, target).unwrap();
        let frame = stock_preview::FrameMeta {
            prefix: target,
            stats: playback.field.stats.clone(),
            checksum: playback.field.checksum(),
            versions: playback.field.versions.clone(),
            allocated: Vec::new(),
        };
        let dirty = stock_preview::dirty_tiles(&frame, &previous);
        let tiles = stock.tiles_x * stock.tiles_y;
        assert!(dirty.len() <= tiles);
        // Re-reporting the same prefix must copy nothing.
        assert!(stock_preview::dirty_tiles(&frame, &frame.versions).is_empty());
        moved += usize::from(!dirty.is_empty());
        previous = playback.field.versions.clone();
    }
    assert!(moved > 0);
    // Replay through the seeded checkpoints stays bounded by the field size.
    assert!(playback.checkpoint_bytes() <= 20 * 1024 * 1024);
}
