//! Bounded, explicitly coarser display grid. All integration happens in
//! `sim::Field`; this module only chooses the display preset, records tile
//! versions for incremental upload and packages the checkpoint cells for the
//! binary payload.
use crate::sim::{Field, Input, Stats, Stock};
use serde::{Deserialize, Serialize};

pub const MAX_PREVIEW_BYTES: usize = 20 * 1024 * 1024;
pub const CHECKPOINTS: usize = 18;
/// The synthetic S/M/L probes are explicit workload generators with their own
/// stated budget; they are not the interactive reference preset.
pub const PROBE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// Bumped when the raster's meaning changes. It is part of the simulation key,
/// so a retained checkpoint or tile from an older algorithm is never reused.
pub const DISPLAY_ALGORITHM_VERSION: u32 = 2;

/// How much display resolution and retained stock a session buys. Changing the
/// preset re-derives the display raster from the retained execution; it never
/// replans and never changes a machining value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayPreset {
    /// Fastest, for a large job on a small window.
    Coarse,
    /// The historical GUI2–GUI8 preset.
    #[default]
    Standard,
    /// Finer cells for thin features, at four times the retained bytes.
    Fine,
}

impl DisplayPreset {
    pub const ALL: [Self; 3] = [Self::Coarse, Self::Standard, Self::Fine];
    /// Longest side of the display raster, in cells.
    pub fn max_side(self) -> usize {
        match self {
            Self::Coarse => 256,
            Self::Standard => 512,
            Self::Fine => 1024,
        }
    }
    /// Bytes the retained stock checkpoints may occupy.
    pub fn budget(self) -> usize {
        match self {
            Self::Coarse => 8 * 1024 * 1024,
            Self::Standard => MAX_PREVIEW_BYTES,
            Self::Fine => 64 * 1024 * 1024,
        }
    }
    pub fn checkpoints(self) -> usize {
        CHECKPOINTS
    }
    pub fn wire(self) -> &'static str {
        match self {
            Self::Coarse => "coarse",
            Self::Standard => "standard",
            Self::Fine => "fine",
        }
    }
    pub fn from_wire(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.wire() == value)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Standard => "Standard",
            Self::Fine => "Fine",
        }
    }
    /// One line for the control: what the preset costs and what it buys.
    pub fn summary(self) -> &'static str {
        match self {
            Self::Coarse => "256 cells across · 8 MiB checkpoints",
            Self::Standard => "512 cells across · 20 MiB checkpoints",
            Self::Fine => "1024 cells across · 64 MiB checkpoints",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameMeta {
    pub prefix: usize,
    pub stats: Stats,
    pub checksum: String,
    /// Per-tile change counters. The renderer compares these with the versions
    /// it last uploaded and copies only the tiles that actually changed.
    pub versions: Vec<u32>,
    /// Tiles that exist in the field. An allocated but still empty tile is not
    /// recoverable from packed cells alone, so the mask travels with the frame.
    pub allocated: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewMeta {
    pub stock: Stock,
    pub cols: usize,
    pub rows: usize,
    pub cell_mm: f64,
    pub reference_cell_mm: f64,
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub retained_bytes: usize,
    /// Checkpoints the worker retains for this raster. A seek or preset
    /// response carries one frame, so this is the honest ladder size.
    #[serde(default)]
    pub ladder_frames: usize,
    /// Display preset this raster was derived from.
    #[serde(default)]
    pub preset: DisplayPreset,
    /// Simulation key: execution identity, display algorithm version, tool
    /// geometry and the display resolution. Stock tiles and checkpoints for
    /// another key must not be reused even when the stock rectangle matches.
    #[serde(default)]
    pub key: String,
    pub frames: Vec<FrameMeta>,
    /// Stage boundaries the display budget could not keep as their own
    /// checkpoint. They are still seekable: the display replays forward from
    /// the nearest earlier checkpoint. Zero means every stage boundary is
    /// directly seekable.
    #[serde(default)]
    pub dropped_stage_marks: usize,
}

impl PreviewMeta {
    /// Identity the stock renderer keys its resident tiles on. A preset change
    /// produces a different key, so every tile is re-uploaded instead of being
    /// mixed with cells from another resolution.
    pub fn identity(&self) -> u64 {
        u64::from_str_radix(self.key.get(..16).unwrap_or("0"), 16).unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub meta: PreviewMeta,
    /// Packed `u32` cells per frame, in the same order as `meta.frames`.
    pub cells: Vec<Vec<u8>>,
}

pub fn build(input: &Input, rough: usize) -> Result<Preview, String> {
    build_with_marks(input, &[rough], DisplayPreset::Standard, "unspecified")
}

pub fn build_with(input: &Input, rough: usize) -> Result<Preview, String> {
    build_with_marks(input, &[rough], DisplayPreset::Standard, "unspecified")
}

/// Build the display preset with explicit checkpoint prefixes. Every mark is
/// an operation or stage boundary the timeline must be able to seek to;
/// evenly spaced frames fill the rest of the budget. Marks that no longer fit
/// are dropped (nearest to the end first) instead of failing a usable job,
/// because the interactive replay can still reach any prefix between
/// checkpoints.
pub fn build_with_marks(
    input: &Input,
    marks: &[usize],
    preset: DisplayPreset,
    key_seed: &str,
) -> Result<Preview, String> {
    build_with_limits(
        input,
        marks,
        preset,
        key_seed,
        preset.checkpoints(),
        preset.budget(),
    )
}

/// The preset's checkpoint scheme, with the limits exposed so the budget
/// behaviour stays testable without inventing a preset that ships.
fn build_with_limits(
    input: &Input,
    marks: &[usize],
    preset: DisplayPreset,
    key_seed: &str,
    checkpoints: usize,
    budget: usize,
) -> Result<Preview, String> {
    let checkpoints = checkpoints.max(3);
    let width = input.stock.x1 - input.stock.x0;
    let height = input.stock.y1 - input.stock.y0;
    // Coarsening is confined to this named display preset. Toolpaths and core
    // checks never consume this field, and the reference parity grid is unchanged.
    let cell = input
        .resolution
        .cell_mm
        .max(width.max(height) / preset.max_side() as f64);
    let mut field = Field::new(input.stock, &input.tools, cell)?;
    let per_frame = field
        .versions
        .len()
        .max(1)
        .saturating_mul(crate::sim::TILE * crate::sim::TILE * 4);
    let mut marks: Vec<usize> = marks
        .iter()
        .copied()
        .filter(|prefix| *prefix <= input.motions.len())
        .collect();
    marks.sort_unstable();
    marks.dedup();
    // The preferred set is the historical evenly spaced ladder
    // (`checkpoints - 2` divisions, including the initial and final state) plus
    // every named stage boundary. The spacing matches the qualification
    // fixture's checkpoint scheme.
    let total = input.motions.len();
    let spans = checkpoints - 2;
    let mut prefixes: Vec<_> = (0..=spans).map(|i| total * i / spans).collect();
    prefixes.extend(marks.iter().copied());
    prefixes.sort_unstable();
    prefixes.dedup();
    // A profile with finishing publishes several stage boundaries, so the
    // preferred set can exceed what the display budget holds. Frames are then
    // dropped explicitly — evenly spaced frames first, oldest stage boundary
    // next — instead of failing an otherwise usable job, because the interactive
    // replay can still reach any prefix between checkpoints.
    let capacity = (budget / per_frame.max(1)).max(3);
    let mut dropped_stage_marks = 0;
    while prefixes.len() > capacity && prefixes.len() > 3 {
        let interior = prefixes
            .iter()
            .position(|prefix| *prefix != 0 && *prefix != total && !marks.contains(prefix));
        match interior {
            Some(index) => {
                prefixes.remove(index);
            }
            None => {
                let oldest = marks.first().copied();
                match oldest {
                    Some(oldest) if marks.len() > 1 => {
                        marks.remove(0);
                        prefixes.retain(|prefix| *prefix != oldest);
                        dropped_stage_marks += 1;
                    }
                    _ => break,
                }
            }
        }
    }
    let retained_bytes =
        field.versions.len() * crate::sim::TILE * crate::sim::TILE * 4 * prefixes.len();
    if retained_bytes > budget {
        return Err(format!(
            "Stock preview needs {retained_bytes} retained bytes, above the {budget} byte display budget"
        ));
    }
    let mut frames = Vec::with_capacity(prefixes.len());
    let mut cells = Vec::with_capacity(prefixes.len());
    let ladder_frames = prefixes.len();
    let mut position = 0;
    for prefix in prefixes {
        for motion in &input.motions[position..prefix] {
            field.apply(motion, 0., 1.)?;
        }
        position = prefix;
        cells.push(packed_bytes(&field));
        frames.push(FrameMeta {
            prefix,
            stats: field.stats.clone(),
            checksum: field.checksum(),
            versions: field.versions.clone(),
            allocated: field
                .versions
                .iter()
                .enumerate()
                .filter(|(i, _)| field.tile_allocated(*i))
                .map(|(i, _)| i as u32)
                .collect(),
        });
    }
    Ok(Preview {
        meta: PreviewMeta {
            stock: input.stock,
            cols: field.cols,
            rows: field.rows,
            cell_mm: cell,
            reference_cell_mm: input.resolution.cell_mm,
            tiles_x: field.tiles_x,
            tiles_y: field.versions.len() / field.tiles_x.max(1),
            retained_bytes,
            ladder_frames,
            preset,
            key: simulation_key(
                key_seed,
                preset,
                cell,
                field.cols,
                field.rows,
                &input.tools,
                input.stock,
            ),
            frames,
            dropped_stage_marks,
        },
        cells,
    })
}

/// The display simulation key: execution identity, display algorithm version,
/// the display raster's resolution and the tool geometry that produced it. Two
/// rasters with different keys describe different bytes, so neither may reuse
/// the other's checkpoints or resident tiles.
pub fn simulation_key(
    seed: &str,
    preset: DisplayPreset,
    cell_mm: f64,
    cols: usize,
    rows: usize,
    tools: &[crate::sim::ToolSpec],
    stock: Stock,
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for byte in seed.as_bytes() {
        mix(*byte as u64);
    }
    for byte in preset.wire().as_bytes() {
        mix(*byte as u64);
    }
    mix(DISPLAY_ALGORITHM_VERSION as u64);
    mix(cell_mm.to_bits());
    mix(cols as u64);
    mix(rows as u64);
    for value in [stock.x0, stock.y0, stock.x1, stock.y1, stock.thickness_mm] {
        mix(value.to_bits());
    }
    for tool in tools {
        match *tool {
            crate::sim::ToolSpec::Knife { offset } => {
                mix(1);
                mix(offset.to_bits());
            }
            crate::sim::ToolSpec::Endmill { diameter } => {
                mix(2);
                mix(diameter.to_bits());
            }
            crate::sim::ToolSpec::Vbit {
                angle,
                tip,
                diameter,
                height,
            } => {
                mix(3);
                for value in [angle, tip, diameter, height] {
                    mix(value.to_bits());
                }
            }
        }
    }
    format!("{hash:016x}")
}

fn packed_bytes(field: &Field) -> Vec<u8> {
    field.packed_tile_bytes()
}

/// Bytes a full-frame upload would copy, for comparison against dirty tiles.
pub fn frame_bytes(meta: &PreviewMeta) -> usize {
    meta.cols * meta.rows * 4
}

/// Tiles whose version differs from the last uploaded version.
pub fn dirty_tiles(versions: &[u32], uploaded: &[u32]) -> Vec<u32> {
    versions
        .iter()
        .enumerate()
        .filter(|(i, version)| uploaded.get(*i).is_none_or(|last| last != *version))
        .map(|(i, _)| i as u32)
        .collect()
}

/// How many stock tiles one frame may copy. Larger than one tile, small enough
/// that a cold fine-resolution load cannot stall a frame on its own.
pub const TILE_UPLOADS_PER_FRAME: usize = 4;

/// The next batch of changed tiles to copy, in tile order. Tiles outside the
/// batch are not forgotten: their version still differs from the uploaded one,
/// so the next frame asks for them again. This is the headless half of the
/// renderer's per-frame upload bound.
pub fn next_tile_batch(versions: &[u32], uploaded: &[u32], limit: usize) -> Vec<u32> {
    let mut dirty = dirty_tiles(versions, uploaded);
    dirty.truncate(limit.max(1));
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Motion, ToolSpec, choose_resolution};

    fn input(motions: usize) -> Input {
        Input {
            stock: Stock {
                x0: -5.,
                y0: -5.,
                x1: 5.,
                y1: 5.,
                thickness_mm: 6.,
            },
            tools: vec![ToolSpec::Endmill { diameter: 2. }],
            resolution: choose_resolution(10., 10., 0.02, 8192., 64_000_000.).unwrap(),
            motions: (0..motions)
                .map(|i| Motion {
                    kind: "cut".into(),
                    tool: 0,
                    x0: -4. + (i % 80) as f64 * 0.1,
                    y0: -4. + (i / 80) as f64 * 0.1,
                    z0: -0.5,
                    x1: -4. + (i % 80) as f64 * 0.1 + 0.1,
                    y1: -4. + (i / 80) as f64 * 0.1,
                    z1: -0.5,
                })
                .collect(),
            prefixes: vec![],
        }
    }

    #[test]
    fn frames_record_tile_versions_and_stay_inside_the_cap() {
        let preview = build(&input(400), 200).unwrap();
        assert!(preview.meta.retained_bytes <= MAX_PREVIEW_BYTES);
        assert!(
            preview
                .meta
                .frames
                .windows(2)
                .all(|f| f[0].prefix <= f[1].prefix)
        );
        for frame in &preview.meta.frames {
            assert_eq!(
                frame.versions.len(),
                preview.meta.tiles_x * preview.meta.tiles_y
            );
            assert!(
                frame
                    .allocated
                    .iter()
                    .all(|t| (*t as usize) < frame.versions.len())
            );
            assert!(!frame.checksum.is_empty());
        }
        assert!(preview.meta.frames.iter().any(|f| !f.allocated.is_empty()));
    }

    #[test]
    fn more_stage_boundaries_than_the_budget_holds_drop_the_oldest_marks() {
        // A profile's rough and finishing passes publish one boundary per
        // stage; the display keeps the frames it can hold and reports the ones
        // it dropped instead of failing the whole job.
        let input = input(400);
        let marks: Vec<usize> = (1..=12).map(|i| i * 30).collect();
        // Room for exactly three frames: the initial state, one intermediate
        // checkpoint and the final state.
        let budget = 3 * 1024 * 1024 + 512 * 1024;
        let preview = build_with_limits(
            &input,
            &marks,
            DisplayPreset::Standard,
            "test",
            CHECKPOINTS,
            budget,
        )
        .unwrap();
        assert!(preview.meta.retained_bytes <= budget);
        assert!(preview.meta.frames.len() <= 3);
        assert!(
            preview.meta.dropped_stage_marks > 0,
            "dropped boundaries are reported"
        );
        // The frames stay ordered and the final state is still the last one.
        assert!(
            preview
                .meta
                .frames
                .windows(2)
                .all(|f| f[0].prefix <= f[1].prefix)
        );
        assert_eq!(
            preview.meta.frames.last().unwrap().prefix,
            input.motions.len()
        );
    }

    #[test]
    fn dirty_tiles_only_reports_changed_tiles() {
        let preview = build(&input(400), 200).unwrap();
        let first = &preview.meta.frames[0];
        let last = preview.meta.frames.last().unwrap();
        let all = dirty_tiles(&first.versions, &[]);
        assert_eq!(all.len(), first.versions.len());
        // The first frame is empty, so re-reporting it changes nothing.
        assert!(dirty_tiles(&first.versions, &first.versions).is_empty());
        let changed = dirty_tiles(&last.versions, &first.versions);
        assert!(
            changed
                .iter()
                .all(|t| first.versions[*t as usize] != last.versions[*t as usize])
        );
        assert!(changed.len() < last.versions.len());
    }

    /// A cold load copies a bounded batch per frame and still finishes with
    /// every changed tile copied exactly once.
    #[test]
    fn stock_uploads_are_batched_per_frame_without_losing_tiles() {
        let preview = build(&input(400), 200).unwrap();
        let last = preview.meta.frames.last().unwrap();
        let mut uploaded: Vec<u32> = vec![u32::MAX; last.versions.len()];
        let mut frames = 0;
        loop {
            let batch = next_tile_batch(&last.versions, &uploaded, TILE_UPLOADS_PER_FRAME);
            if batch.is_empty() {
                break;
            }
            assert!(
                batch.len() <= TILE_UPLOADS_PER_FRAME,
                "one frame copies at most the batch limit"
            );
            for tile in batch {
                uploaded[tile as usize] = last.versions[tile as usize];
            }
            frames += 1;
            assert!(frames <= last.versions.len(), "the load must finish");
        }
        assert!(dirty_tiles(&last.versions, &uploaded).is_empty());
        assert_eq!(frames, last.versions.len().div_ceil(TILE_UPLOADS_PER_FRAME));
    }

    /// The simulation key is what stops a checkpoint or a resident tile from
    /// another execution, algorithm version or display resolution being reused
    /// just because the stock rectangle matches.
    #[test]
    fn the_simulation_key_separates_executions_and_display_resolutions() {
        let input = input(200);
        let marks = [50, 100];
        let standard =
            build_with_marks(&input, &marks, DisplayPreset::Standard, "execution-1").unwrap();
        let again =
            build_with_marks(&input, &marks, DisplayPreset::Standard, "execution-1").unwrap();
        let fine = build_with_marks(&input, &marks, DisplayPreset::Fine, "execution-1").unwrap();
        let other =
            build_with_marks(&input, &marks, DisplayPreset::Standard, "execution-2").unwrap();
        assert_eq!(
            standard.meta.key, again.meta.key,
            "the same execution and preset keep one key"
        );
        assert_ne!(
            standard.meta.key, fine.meta.key,
            "a resolution change is a new simulation key"
        );
        assert_ne!(
            standard.meta.key, other.meta.key,
            "a different execution is a new simulation key"
        );
        assert_eq!(
            (
                standard.meta.stock.x0,
                standard.meta.stock.y0,
                standard.meta.stock.x1,
                standard.meta.stock.y1,
                standard.meta.stock.thickness_mm
            ),
            (
                fine.meta.stock.x0,
                fine.meta.stock.y0,
                fine.meta.stock.x1,
                fine.meta.stock.y1,
                fine.meta.stock.thickness_mm
            ),
            "the stock rectangle is identical, so the key is the only separation"
        );
        assert!(fine.meta.cell_mm < standard.meta.cell_mm);
        assert_ne!(standard.meta.identity(), fine.meta.identity());
        assert!(
            build_with_marks(&input, &marks, DisplayPreset::Coarse, "execution-1")
                .unwrap()
                .meta
                .cell_mm
                > standard.meta.cell_mm
        );
    }

    #[test]
    fn transported_frames_rebuild_the_exact_field() {
        let input = input(300);
        let preview = build(&input, 150).unwrap();
        let last = preview.meta.frames.last().unwrap();
        let rebuilt = Field::from_packed(
            preview.meta.stock,
            &input.tools,
            preview.meta.cell_mm,
            preview.cells.last().unwrap(),
            &last.versions,
            &last.allocated,
            last.stats.clone(),
        )
        .unwrap();
        assert_eq!(rebuilt.checksum(), last.checksum);
        assert_eq!(rebuilt.allocated_bytes(), {
            let mut direct =
                Field::new(preview.meta.stock, &input.tools, preview.meta.cell_mm).unwrap();
            for motion in &input.motions {
                direct.apply(motion, 0., 1.).unwrap();
            }
            assert_eq!(direct.checksum(), last.checksum);
            assert_eq!(direct.versions, last.versions);
            direct.allocated_bytes()
        });
    }
}
