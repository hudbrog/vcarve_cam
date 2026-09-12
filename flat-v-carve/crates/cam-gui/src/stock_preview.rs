//! Bounded, explicitly coarser display grid. All integration happens in
//! `sim::Field`; this module only chooses the display preset, records tile
//! versions for incremental upload and packages the checkpoint cells for the
//! binary payload.
use crate::sim::{Field, Input, Stats, Stock};
use serde::{Deserialize, Serialize};

pub const MAX_SIDE: usize = 512;
pub const MAX_PREVIEW_BYTES: usize = 20 * 1024 * 1024;
pub const CHECKPOINTS: usize = 18;
/// The synthetic S/M/L probes are explicit workload generators with their own
/// stated budget; they are not the interactive reference preset.
pub const PROBE_BUDGET_BYTES: usize = 64 * 1024 * 1024;

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
    pub frames: Vec<FrameMeta>,
    /// Stage boundaries the display budget could not keep as their own
    /// checkpoint. They are still seekable: the display replays forward from
    /// the nearest earlier checkpoint. Zero means every stage boundary is
    /// directly seekable.
    #[serde(default)]
    pub dropped_stage_marks: usize,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub meta: PreviewMeta,
    /// Packed `u32` cells per frame, in the same order as `meta.frames`.
    pub cells: Vec<Vec<u8>>,
}

pub fn build(input: &Input, rough: usize) -> Result<Preview, String> {
    build_with_marks(input, &[rough], CHECKPOINTS, MAX_PREVIEW_BYTES)
}

pub fn build_with(
    input: &Input,
    rough: usize,
    checkpoints: usize,
    budget: usize,
) -> Result<Preview, String> {
    build_with_marks(input, &[rough], checkpoints, budget)
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
        .max(width.max(height) / MAX_SIDE as f64);
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
            frames,
            dropped_stage_marks,
        },
        cells,
    })
}

fn packed_bytes(field: &Field) -> Vec<u8> {
    field.packed_tile_bytes()
}

/// Bytes a full-frame upload would copy, for comparison against dirty tiles.
pub fn frame_bytes(meta: &PreviewMeta) -> usize {
    meta.cols * meta.rows * 4
}

/// Tiles whose version differs from the last uploaded version.
pub fn dirty_tiles(current: &FrameMeta, uploaded: &[u32]) -> Vec<u32> {
    current
        .versions
        .iter()
        .enumerate()
        .filter(|(i, version)| uploaded.get(*i).is_none_or(|last| last != *version))
        .map(|(i, _)| i as u32)
        .collect()
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
        let preview = build_with_marks(&input, &marks, CHECKPOINTS, budget).unwrap();
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
        let all = dirty_tiles(first, &[]);
        assert_eq!(all.len(), first.versions.len());
        // The first frame is empty, so re-reporting it changes nothing.
        assert!(dirty_tiles(first, &first.versions).is_empty());
        let changed = dirty_tiles(last, &first.versions);
        assert!(
            changed
                .iter()
                .all(|t| first.versions[*t as usize] != last.versions[*t as usize])
        );
        assert!(changed.len() < last.versions.len());
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
