//! Bounded, explicitly coarser GUI1 display. All integration occurs in compute.
use crate::sim::{Field, Input, Stats, Stock};
use serde::{Deserialize, Serialize};

pub const MAX_SIDE: usize = 512;
pub const MAX_PREVIEW_BYTES: usize = 20 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub prefix: usize,
    pub cells: Vec<u32>,
    pub stats: Stats,
    pub checksum: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub stock: Stock,
    pub cols: usize,
    pub rows: usize,
    pub cell_mm: f64,
    pub reference_cell_mm: f64,
    pub frames: Vec<Frame>,
    pub retained_bytes: usize,
}
pub fn build(input: &Input, rough: usize) -> Result<Preview, String> {
    let width = input.stock.x1 - input.stock.x0;
    let height = input.stock.y1 - input.stock.y0;
    // Coarsening is confined to this named display preset. Toolpaths and core
    // checks never consume this field, and the reference parity grid is unchanged.
    let cell = input
        .resolution
        .cell_mm
        .max(width.max(height) / MAX_SIDE as f64);
    let mut field = Field::new(input.stock, &input.tools, cell)?;
    let mut prefixes: Vec<_> = (0..=16).map(|i| input.motions.len() * i / 16).collect();
    prefixes.push(rough);
    prefixes.sort_unstable();
    prefixes.dedup();
    let retained_bytes = field.cols * field.rows * 4 * prefixes.len();
    if retained_bytes > MAX_PREVIEW_BYTES {
        return Err("Stock preview exceeds 20 MiB checkpoint limit".into());
    }
    let mut frames = Vec::new();
    let mut position = 0;
    for prefix in prefixes {
        for motion in &input.motions[position..prefix] {
            field.apply(motion, 0., 1.)?;
        }
        position = prefix;
        frames.push(Frame {
            prefix,
            cells: field.packed_cells(),
            stats: field.stats.clone(),
            checksum: field.checksum(),
        });
    }
    Ok(Preview {
        stock: input.stock,
        cols: field.cols,
        rows: field.rows,
        cell_mm: cell,
        reference_cell_mm: input.resolution.cell_mm,
        frames,
        retained_bytes,
    })
}
