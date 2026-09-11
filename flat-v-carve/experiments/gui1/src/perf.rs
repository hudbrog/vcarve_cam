//! Headless S/M/L measurement harness.
//!
//! Everything measured here runs outside the UI thread but on the same code
//! paths the display uses: the same payload, the same page fingerprints, the
//! same residency policy, the same picker and the same bounded replay. Numbers
//! that cannot be observed without a GPU (real device transfer, raster time)
//! are reported as unknown rather than as zero.
use crate::{alloc_probe, compute, overlay, pages, paging, pick, sim, stock_preview};
use serde::Serialize;
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Samples {
    pub count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

impl Samples {
    fn from(mut values: Vec<f64>) -> Self {
        if values.is_empty() {
            return Self {
                count: 0,
                p50_ms: 0.,
                p95_ms: 0.,
                max_ms: 0.,
            };
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let at = |fraction: f64| {
            let index = ((values.len() as f64 - 1.) * fraction).round() as usize;
            values[index]
        };
        Self {
            count: values.len(),
            p50_ms: at(0.5),
            p95_ms: at(0.95),
            max_ms: values[values.len() - 1],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workload {
    pub name: String,
    pub segments: usize,
    pub motions: usize,
    pub sources: usize,
    pub operations: usize,
    // Generation and transport
    pub plan_ms: f64,
    pub metadata_bytes: usize,
    pub payload_bytes: usize,
    pub motion_pages: usize,
    pub stock_checkpoints: usize,
    pub stock_tiles: usize,
    pub sim_stream_bytes: usize,
    pub json_estimate: JsonEstimate,
    // Paging
    pub fingerprint_ms: f64,
    pub upload_full: UploadSample,
    pub upload_reload: UploadSample,
    pub upload_budget_16mib: UploadSample,
    // Picking
    pub pick_build_ms: f64,
    pub pick_index_bytes: usize,
    pub pick_query: Samples,
    pub pick_hit_rate: f64,
    pub pick_brute_force_checked: usize,
    pub pick_brute_force_mismatches: usize,
    // Stock scrubbing
    pub replayable: bool,
    pub stock_field_bytes: usize,
    pub seek_warm: Samples,
    /// Backward seek to a point inside the interval after a transported
    /// checkpoint: bounded by the replay window.
    pub seek_backward: Samples,
    /// Forward seek across one whole checkpoint interval: the worst forward
    /// scrub, bounded by the checkpoint spacing.
    pub seek_forward: Samples,
    /// Small +/-1% playhead steps around a checkpoint, as a drag produces.
    pub seek_scrub: Samples,
    pub seek_cold: Samples,
    pub seek_tiles: Samples,
    pub seek_tile_kib: Samples,
    // Overlay
    pub overlay_us: Samples,
    // Memory
    pub heap_before_bytes: usize,
    pub heap_peak_bytes: usize,
    pub heap_after_bytes: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonEstimate {
    pub sampled_vertices: usize,
    pub json_bytes_per_vertex: f64,
    pub estimated_vertex_bytes: usize,
    pub sampled_cells: usize,
    pub json_bytes_per_cell: f64,
    pub estimated_cell_bytes: usize,
    pub estimated_total_bytes: usize,
    pub payload_ratio: f64,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSample {
    pub pages: usize,
    pub bytes: u64,
    pub staging_copy_ms: f64,
    pub omitted_pages: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub protocol: String,
    pub workloads: Vec<Workload>,
    pub notes: Vec<String>,
    pub heap_peak_all_bytes: usize,
}

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 11) as f64) / ((1u64 << 53) as f64)
}

/// Deterministic checkpoint pair for a scrub sample.
fn checkpoint_pair(
    step: usize,
    seed: &mut u64,
    checkpoints: &[usize],
    motions: usize,
) -> (usize, usize) {
    if checkpoints.is_empty() || motions == 0 {
        return (0, motions);
    }
    let index = (lcg(seed) * checkpoints.len() as f64) as usize;
    let index = index.min(checkpoints.len() - 1);
    let prefix = checkpoints[index];
    let next = checkpoints.get(index + 1).copied().unwrap_or(motions);
    let _ = step;
    (prefix, next.max(prefix))
}

/// Picking queries: 240 cursors a few pixels from real motion endpoints, with a
/// bounded brute-force cross-check on real geometry.
fn pick_queries(picker: &pick::Picker) -> (Vec<f64>, usize, usize, usize, usize) {
    let mut seed = 0x5eed_u64;
    let mut samples = Vec::new();
    let (mut hits, mut checked, mut mismatches) = (0_usize, 0_usize, 0_usize);
    let camera = pick::Camera {
        iso: false,
        aspect: 1280. / 800.,
        zoom: 1.,
        yaw: 0.4,
    };
    let rect = [1280., 800.];
    let queries = 240.min(picker.motion_count());
    for _ in 0..queries {
        let motion = (lcg(&mut seed) * picker.motion_count() as f64) as u32;
        let Some(points) = picker.endpoints(motion.min(picker.motion_count() as u32 - 1)) else {
            continue;
        };
        let anchor = camera.to_points(camera.ndc(points[0]), rect);
        let cursor = [anchor[0] + 6., anchor[1] - 4.];
        let begin = Instant::now();
        let hit = picker.pick(&camera, rect, cursor, 8., 1.25);
        samples.push(begin.elapsed().as_secs_f64() * 1000.);
        if hit.is_some() {
            hits += 1;
        }
        if checked < 40 {
            let brute = picker.brute_force(&camera, rect, cursor, 8., 1.25);
            match (hit, brute) {
                (None, None) => {}
                (Some(hit), Some(brute)) => {
                    // Different motion is only a defect when the projected
                    // distance differs; exact ties may resolve either way.
                    if (hit.distance_pixels - brute.distance_pixels).abs() > 1e-3 {
                        mismatches += 1;
                    }
                }
                _ => mismatches += 1,
            }
            checked += 1;
        }
    }
    (samples, hits, queries, checked, mismatches)
}

/// Overlay build cost for a moving playhead: selection ribbon plus blade glyph.
fn overlay_samples(picker: &pick::Picker) -> Vec<f64> {
    let mut samples = Vec::new();
    if picker.motion_count() == 0 {
        return samples;
    }
    for step in 0..60 {
        let index =
            ((step * picker.motion_count() / 60) as u32).min(picker.motion_count() as u32 - 1);
        let points = picker.endpoints(index);
        let begin = Instant::now();
        let built = overlay::build(
            points.map(|points| (points[0], points[1])),
            0.01,
            &[overlay::Marker {
                glyph: overlay::Glyph::Endmill { radius: 0.02 },
                tip: points.map_or([0., 0., 0.], |points| points[1]),
                top: 0.,
            }],
        );
        samples.push(begin.elapsed().as_secs_f64() * 1000. * 1000.);
        std::hint::black_box(&built);
    }
    samples
}

/// JSON cost per record, measured on a bounded sample and clearly reported as
/// an extrapolation rather than as a serialized copy of the whole scene.
fn json_estimate(payload: &[u8], meta: &compute::SceneMeta) -> JsonEstimate {
    const SAMPLE_VERTICES: usize = 4_096;
    const SAMPLE_CELLS: usize = 16_384;
    let mut estimate = JsonEstimate::default();
    let vertices = meta.contour_vertices + meta.motions * 2;
    if vertices > 0 {
        let taken = SAMPLE_VERTICES.min(vertices);
        let bytes = bytemuck::cast_slice::<u8, compute::Vertex>(
            &payload[meta.motion_offset..meta.motion_offset + taken * pages::VERTEX_BYTES],
        );
        let json_bytes = serde_json::to_vec(bytes).map_or(0, |encoded| encoded.len());
        estimate.sampled_vertices = taken;
        estimate.json_bytes_per_vertex = json_bytes as f64 / taken as f64;
        estimate.estimated_vertex_bytes =
            (vertices as f64 * estimate.json_bytes_per_vertex) as usize;
    }
    if let Some(section) = meta
        .sections
        .iter()
        .find(|s| s.kind == pages::SECTION_STOCK)
    {
        let available = section.len / 4;
        let taken = SAMPLE_CELLS.min(available);
        if taken > 0 {
            let cells: Vec<u32> = payload[section.offset..section.offset + taken * 4]
                .chunks_exact(4)
                .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
                .collect();
            let json_bytes = serde_json::to_vec(&cells).map_or(0, |encoded| encoded.len());
            estimate.sampled_cells = taken;
            estimate.json_bytes_per_cell = json_bytes as f64 / taken as f64;
            let total_cells = meta.stock.as_ref().map_or(0, |stock| {
                stock.tiles_x * stock.tiles_y * sim::TILE * sim::TILE
            }) * meta.stock.as_ref().map_or(0, |stock| stock.frames.len());
            estimate.estimated_cell_bytes =
                (total_cells as f64 * estimate.json_bytes_per_cell) as usize;
        }
    }
    estimate.estimated_total_bytes =
        estimate.estimated_vertex_bytes + estimate.estimated_cell_bytes;
    estimate.payload_ratio = if meta.payload_bytes > 0 {
        estimate.estimated_total_bytes as f64 / meta.payload_bytes as f64
    } else {
        0.
    };
    estimate
}

fn staging_copy(payload: &[u8], uploads: &[paging::PageUpload]) -> (f64, u64) {
    let mut staging = Vec::with_capacity(
        uploads
            .iter()
            .map(|upload| upload.len)
            .sum::<usize>()
            .min(64 * 1024 * 1024),
    );
    let begin = Instant::now();
    let mut bytes = 0_u64;
    for upload in uploads {
        let slice = &payload[upload.offset..upload.offset + upload.len];
        staging.extend_from_slice(slice);
        bytes += slice.len() as u64;
    }
    let ms = begin.elapsed().as_secs_f64() * 1000.;
    std::hint::black_box(&staging);
    (ms, bytes)
}

fn upload_sample(
    pager: &mut paging::Pager,
    identity: u64,
    table: &pages::PageTable,
    hashes: &[Option<u64>],
    required: &[usize],
    payload: &[u8],
    copy: bool,
) -> UploadSample {
    let plan = pager.plan(identity, table, hashes, required, |page| {
        table.bytes_of(page)
    });
    let (staging_copy_ms, bytes) = if copy {
        staging_copy(payload, &plan.uploads)
    } else {
        (0., plan.upload_bytes())
    };
    UploadSample {
        pages: plan.uploads.len(),
        bytes,
        staging_copy_ms,
        omitted_pages: plan.omitted,
    }
}

fn measure_workload(name: &str, segments: usize) -> Result<Workload, String> {
    let heap_before = alloc_probe::snapshot();
    alloc_probe::reset_peak();
    let begin = Instant::now();
    let (meta, payload) = compute::run(compute::Request::Synthetic { segments })?;
    let plan_ms = begin.elapsed().as_secs_f64() * 1000.;
    let table = pages::PageTable::new(meta.motion_offset, meta.motion_len, meta.motions)?;

    // Fingerprints: one pass over the motion pages, as the display does lazily.
    let begin = Instant::now();
    let mut hashes: Vec<Option<u64>> = Vec::with_capacity(table.page_count());
    for page in 0..table.page_count() {
        hashes.push(Some(pages::page_hash(&payload[table.bytes_of(page)])));
    }
    let fingerprint_ms = begin.elapsed().as_secs_f64() * 1000.;

    // Residency: full budget, a reload of the same payload, then a 16 MiB cap.
    let required: Vec<usize> = (0..table.page_count()).collect();
    let mut pager = paging::Pager::new(render_budget());
    let upload_full = upload_sample(&mut pager, 1, &table, &hashes, &required, &payload, true);
    let upload_reload = upload_sample(&mut pager, 1, &table, &hashes, &required, &payload, true);
    let mut capped = paging::Pager::new(16 * 1024 * 1024);
    let upload_budget_16mib =
        upload_sample(&mut capped, 1, &table, &hashes, &required, &payload, true);

    // Picking over the real transported payload.
    let begin = Instant::now();
    let picker = pick::Picker::from_vertex_bytes(
        &payload[meta.motion_offset..meta.motion_offset + meta.motion_len],
    )?;
    let pick_build_ms = begin.elapsed().as_secs_f64() * 1000.;
    let (samples, hits, queries, checked, mismatches) = pick_queries(&picker);

    // Stock scrubbing through the same bounded replay the UI uses.
    let mut seek_warm = Vec::new();
    let mut seek_cold = Vec::new();
    let mut seek_backward = Vec::new();
    let mut seek_forward = Vec::new();
    let mut seek_scrub = Vec::new();
    let mut seek_tiles = Vec::new();
    let mut seek_tile_kib = Vec::new();
    let mut stock_field_bytes = 0;
    let mut replayable = false;
    if let Some(stock) = &meta.stock {
        let scene = compute::Scene {
            meta: meta.clone(),
            payload: std::sync::Arc::new(payload.clone()),
        };
        if let Some(input) = scene.sim_input()? {
            replayable = true;
            let mut seed_points = Vec::new();
            for (index, frame) in stock.frames.iter().enumerate() {
                let section = meta
                    .sections
                    .iter()
                    .filter(|section| section.kind == pages::SECTION_STOCK)
                    .nth(index)
                    .ok_or("Stock section missing")?;
                let field = sim::Field::from_packed(
                    stock.stock,
                    &input.tools,
                    stock.cell_mm,
                    &payload[section.offset..section.offset + section.len],
                    &frame.versions,
                    &frame.allocated,
                    frame.stats.clone(),
                )?;
                seed_points.push((frame.prefix, field));
            }
            // Report the integrated display field, not the empty first frame.
            stock_field_bytes = seed_points.last().unwrap().1.allocated_bytes();
            let pristine = seed_points[0].1.clone();
            let latest = seed_points.last().unwrap().1.clone();
            let prefixes: Vec<usize> = seed_points.iter().map(|(prefix, _)| *prefix).collect();
            let mut warm = sim::Playback::seed(
                latest.clone(),
                pristine.clone(),
                seed_points.clone(),
                20 * 1024 * 1024,
            );
            let mut cold = sim::Playback::new(pristine, 0);
            let mut previous = latest.versions.clone();
            let mut seed = 0x5eed_u64;
            for step in 0..40 {
                // Land on a transported checkpoint, then measure the replay out
                // of it so backward and forward costs are separated honestly.
                let (base, next) = checkpoint_pair(step, &mut seed, &prefixes, input.motions.len());
                warm.seek(&input.motions, base)?;
                let target = base + ((next - base) as f64 * lcg(&mut seed)) as usize;
                let begin = Instant::now();
                warm.seek(&input.motions, target)?;
                let forward = begin.elapsed().as_secs_f64() * 1000.;
                seek_forward.push(forward);
                seek_warm.push(forward);
                if next > base {
                    // Backward seek within the same interval: bounded replay.
                    let back = base + (target - base) / 2;
                    let begin = Instant::now();
                    warm.seek(&input.motions, back)?;
                    seek_backward.push(begin.elapsed().as_secs_f64() * 1000.);
                    seek_warm.push(begin.elapsed().as_secs_f64() * 1000.);
                }
                // A drag produces many small steps around one position.
                for _ in 0..3 {
                    let near = (target + input.motions.len() / 200).min(input.motions.len());
                    let begin = Instant::now();
                    warm.seek(&input.motions, near)?;
                    seek_scrub.push(begin.elapsed().as_secs_f64() * 1000.);
                    let begin = Instant::now();
                    warm.seek(&input.motions, target)?;
                    seek_scrub.push(begin.elapsed().as_secs_f64() * 1000.);
                }
                let begin = Instant::now();
                cold.seek(&input.motions, target.min(input.motions.len()))?;
                seek_cold.push(begin.elapsed().as_secs_f64() * 1000.);
                let versioned = stock_preview::FrameMeta {
                    prefix: target,
                    stats: warm.field.stats.clone(),
                    checksum: warm.field.checksum(),
                    versions: warm.field.versions.clone(),
                    allocated: Vec::new(),
                };
                let dirty = stock_preview::dirty_tiles(&versioned, &previous);
                previous = warm.field.versions.clone();
                seek_tiles.push(dirty.len() as f64);
                seek_tile_kib.push(dirty.len() as f64 * (sim::TILE * sim::TILE * 4) as f64 / 1024.);
            }
        }
    }

    let overlay_us = overlay_samples(&picker);

    let report = meta.report.clone();
    let stock_tiles = meta
        .stock
        .as_ref()
        .map_or(0, |stock| stock.tiles_x * stock.tiles_y);
    let workload = Workload {
        name: name.into(),
        segments,
        motions: meta.motions,
        sources: report.get("sources").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        operations: report
            .get("operations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        plan_ms,
        metadata_bytes: meta.transport.metadata_bytes,
        payload_bytes: meta.payload_bytes,
        motion_pages: table.page_count(),
        stock_checkpoints: meta.transport.stock_checkpoints,
        stock_tiles,
        sim_stream_bytes: meta.transport.sim_bytes,
        json_estimate: json_estimate(&payload, &meta),
        fingerprint_ms,
        upload_full,
        upload_reload,
        upload_budget_16mib,
        pick_build_ms,
        pick_index_bytes: picker.memory_bytes(),
        pick_query: Samples::from(samples),
        pick_hit_rate: if queries > 0 {
            hits as f64 / queries as f64
        } else {
            0.
        },
        pick_brute_force_checked: checked,
        pick_brute_force_mismatches: mismatches,
        replayable,
        stock_field_bytes,
        seek_warm: Samples::from(seek_warm),
        seek_backward: Samples::from(seek_backward),
        seek_forward: Samples::from(seek_forward),
        seek_scrub: Samples::from(seek_scrub),
        seek_cold: Samples::from(seek_cold),
        seek_tiles: Samples::from(seek_tiles),
        seek_tile_kib: Samples::from(seek_tile_kib),
        overlay_us: Samples::from(overlay_us),
        heap_before_bytes: heap_before.current_bytes,
        heap_peak_bytes: alloc_probe::snapshot().peak_bytes,
        heap_after_bytes: alloc_probe::snapshot().current_bytes,
    };
    Ok(workload)
}

fn render_budget() -> u64 {
    crate::render::DEFAULT_PAGE_BUDGET
}

fn measure_flower() -> Result<Workload, String> {
    let heap_before = alloc_probe::snapshot();
    alloc_probe::reset_peak();
    let begin = Instant::now();
    let (meta, payload) = compute::run(compute::Request::Reference {
        flower: true,
        export: false,
    })?;
    let plan_ms = begin.elapsed().as_secs_f64() * 1000.;
    let table = pages::PageTable::new(meta.motion_offset, meta.motion_len, meta.motions)?;
    let mut hashes = Vec::with_capacity(table.page_count());
    for page in 0..table.page_count() {
        hashes.push(Some(pages::page_hash(&payload[table.bytes_of(page)])));
    }
    let required: Vec<usize> = (0..table.page_count()).collect();
    let mut pager = paging::Pager::new(render_budget());
    let upload_full = upload_sample(&mut pager, 1, &table, &hashes, &required, &payload, true);
    let upload_reload = upload_sample(&mut pager, 1, &table, &hashes, &required, &payload, true);
    let mut capped = paging::Pager::new(16 * 1024 * 1024);
    let upload_budget_16mib =
        upload_sample(&mut capped, 1, &table, &hashes, &required, &payload, true);
    let mut workload = Workload {
        name: "flower reference".into(),
        segments: meta.motions,
        motions: meta.motions,
        sources: 1,
        operations: 1,
        plan_ms,
        metadata_bytes: meta.transport.metadata_bytes,
        payload_bytes: meta.payload_bytes,
        motion_pages: table.page_count(),
        stock_checkpoints: meta.transport.stock_checkpoints,
        stock_tiles: meta
            .stock
            .as_ref()
            .map_or(0, |stock| stock.tiles_x * stock.tiles_y),
        sim_stream_bytes: meta.transport.sim_bytes,
        json_estimate: json_estimate(&payload, &meta),
        fingerprint_ms: 0.,
        upload_full,
        upload_reload,
        upload_budget_16mib,
        pick_build_ms: 0.,
        pick_index_bytes: 0,
        pick_query: Samples::from(Vec::new()),
        pick_hit_rate: 0.,
        pick_brute_force_checked: 0,
        pick_brute_force_mismatches: 0,
        replayable: meta.sim.is_some(),
        stock_field_bytes: 0,
        seek_warm: Samples::from(Vec::new()),
        seek_backward: Samples::from(Vec::new()),
        seek_forward: Samples::from(Vec::new()),
        seek_scrub: Samples::from(Vec::new()),
        seek_cold: Samples::from(Vec::new()),
        seek_tiles: Samples::from(Vec::new()),
        seek_tile_kib: Samples::from(Vec::new()),
        overlay_us: Samples::from(Vec::new()),
        heap_before_bytes: heap_before.current_bytes,
        heap_peak_bytes: alloc_probe::snapshot().peak_bytes,
        heap_after_bytes: alloc_probe::snapshot().current_bytes,
    };
    // The flower is the interactive reference; measure its picker and seeks too.
    let begin = Instant::now();
    if let Ok(picker) = pick::Picker::from_vertex_bytes(
        &payload[meta.motion_offset..meta.motion_offset + meta.motion_len],
    ) {
        workload.pick_build_ms = begin.elapsed().as_secs_f64() * 1000.;
        workload.pick_index_bytes = picker.memory_bytes();
        let (samples, hits, queries, checked, mismatches) = pick_queries(&picker);
        workload.pick_query = Samples::from(samples);
        workload.pick_hit_rate = if queries > 0 {
            hits as f64 / queries as f64
        } else {
            0.
        };
        workload.pick_brute_force_checked = checked;
        workload.pick_brute_force_mismatches = mismatches;
        workload.overlay_us = Samples::from(overlay_samples(&picker));
    }
    let scene = compute::Scene {
        meta: meta.clone(),
        payload: std::sync::Arc::new(payload.clone()),
    };
    if let Some(stock) = &meta.stock
        && let Ok(Some(input)) = scene.sim_input()
    {
        let mut seeds = Vec::new();
        for (index, frame) in stock.frames.iter().enumerate() {
            let Some(section) = meta
                .sections
                .iter()
                .filter(|s| s.kind == pages::SECTION_STOCK)
                .nth(index)
            else {
                continue;
            };
            let field = sim::Field::from_packed(
                stock.stock,
                &input.tools,
                stock.cell_mm,
                &payload[section.offset..section.offset + section.len],
                &frame.versions,
                &frame.allocated,
                frame.stats.clone(),
            )?;
            seeds.push((frame.prefix, field));
        }
        if !seeds.is_empty() {
            workload.stock_field_bytes = seeds.last().unwrap().1.allocated_bytes();
            let pristine = seeds[0].1.clone();
            let latest = seeds.last().unwrap().1.clone();
            let prefixes: Vec<usize> = seeds.iter().map(|(prefix, _)| *prefix).collect();
            let mut warm = sim::Playback::seed(latest, pristine.clone(), seeds, 20 * 1024 * 1024);
            let mut cold = sim::Playback::new(pristine, 0);
            let (mut seek_warm, mut seek_cold, mut tiles, mut kib) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let (mut seek_backward, mut seek_forward, mut seek_scrub) =
                (Vec::new(), Vec::new(), Vec::new());
            let mut previous = warm.field.versions.clone();
            let mut seed = 0x5eed_u64;
            for step in 0..40 {
                let (base, next) = checkpoint_pair(step, &mut seed, &prefixes, input.motions.len());
                warm.seek(&input.motions, base)?;
                let target = base + ((next - base) as f64 * lcg(&mut seed)) as usize;
                let begin = Instant::now();
                warm.seek(&input.motions, target)?;
                let measured = begin.elapsed().as_secs_f64() * 1000.;
                seek_warm.push(measured);
                seek_forward.push(measured);
                if next > base {
                    let back = base + (target - base) / 2;
                    let begin = Instant::now();
                    warm.seek(&input.motions, back)?;
                    let measured = begin.elapsed().as_secs_f64() * 1000.;
                    seek_warm.push(measured);
                    seek_backward.push(measured);
                }
                for _ in 0..3 {
                    let near = (target + input.motions.len() / 200).min(input.motions.len());
                    let begin = Instant::now();
                    warm.seek(&input.motions, near)?;
                    seek_scrub.push(begin.elapsed().as_secs_f64() * 1000.);
                    let begin = Instant::now();
                    warm.seek(&input.motions, target)?;
                    seek_scrub.push(begin.elapsed().as_secs_f64() * 1000.);
                }
                let begin = Instant::now();
                cold.seek(&input.motions, target.min(input.motions.len()))?;
                seek_cold.push(begin.elapsed().as_secs_f64() * 1000.);
                let versioned = stock_preview::FrameMeta {
                    prefix: target,
                    stats: warm.field.stats.clone(),
                    checksum: warm.field.checksum(),
                    versions: warm.field.versions.clone(),
                    allocated: Vec::new(),
                };
                let dirty = stock_preview::dirty_tiles(&versioned, &previous);
                previous = warm.field.versions.clone();
                tiles.push(dirty.len() as f64);
                kib.push(dirty.len() as f64 * (sim::TILE * sim::TILE * 4) as f64 / 1024.);
            }
            workload.seek_warm = Samples::from(seek_warm);
            workload.seek_backward = Samples::from(seek_backward);
            workload.seek_forward = Samples::from(seek_forward);
            workload.seek_scrub = Samples::from(seek_scrub);
            workload.seek_cold = Samples::from(seek_cold);
            workload.seek_tiles = Samples::from(tiles);
            workload.seek_tile_kib = Samples::from(kib);
        }
    }
    Ok(workload)
}

pub fn run(output: &Path, include_flower: bool) -> Result<(), String> {
    let mut workloads = Vec::new();
    for (name, segments) in [
        ("S · 20,000", 20_000_usize),
        ("M · 200,000", 200_000),
        ("L · 1,000,000", compute::MAX_SEGMENTS),
    ] {
        workloads.push(measure_workload(name, segments)?);
    }
    if include_flower {
        workloads.push(measure_flower()?);
    }
    let report = Report {
        protocol: compute::PROTOCOL.into(),
        workloads,
        notes: vec![
            "JSON sizes are measured on a bounded sample and extrapolated; they are not a serialized copy of the whole scene.".into(),
            "staging copy is the CPU memcpy of exactly the byte ranges the GPU page copy would stage; real device transfer time needs a GPU run.".into(),
            "scrub targets are drawn from a fixed 0x5eed sequence: backward seeks replay out of a transported checkpoint, forward seeks cross one whole checkpoint interval, scrub steps move +/-0.5% of the stream, cold replays from the initial prefix every time.".into(),
            "picking disagreements are only counted when the projected distance differs; an exact tie between two coincident projections may resolve to either motion.".into(),
            "heap counters wrap the Rust global allocator only; the JS heap, GPU memory and operating-system overhead are not included.".into(),
        ],
        heap_peak_all_bytes: alloc_probe::snapshot().peak_bytes,
    };
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())
}
