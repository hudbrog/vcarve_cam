//! Narrow display-only port of the retired TypeScript simulator
//! (the former `web/src/sim/engine.ts`). Never used for CAM checks.
//! Keep f64 expression order, tile traversal and downward quantization aligned
//! with the reference. The comparison harness executes the original TypeScript.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const TILE: usize = 256;
pub const MAX_CELLS: usize = 64_000_000;
const PLUNGE_AREA_EPS: f64 = 1e-16;
/// Rapid rate a machine that states none is simulated with. The timings say
/// when this fallback is in use instead of presenting it as machine truth.
/// The inspection readouts assume the same value, so the transport's program
/// time and the per-operation estimates agree.
pub const DEFAULT_RAPID_RATE_MM_MIN: f64 =
    cam_core::project::v5::inspection::ASSUMED_RAPID_RATE_MM_MIN;
/// Chord error the swept envelope accepts when it walks a programmed arc, in mm.
/// The preview field is a raster: this keeps the walked curve inside a fraction
/// of a display cell while the move stays **one** motion, so the display's
/// motion index remains the plan's motion index.
const ARC_CHORD_MM: f64 = 0.002;
/// Upper bound on the chords one arc is swept with, so a tiny full circle cannot
/// ask for unbounded work.
const MAX_ARC_CHORDS: usize = 4096;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stock {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub thickness_mm: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ToolSpec {
    Knife {
        #[serde(rename = "bladeOffsetMm")]
        offset: f64,
        /// How deep the blade may cut. The display stands the blade up by this
        /// much above the tip; nothing else about the blade is modelled.
        #[serde(rename = "maxCutDepthMm")]
        max_cut_depth: f64,
    },
    Endmill {
        #[serde(rename = "diameterMm")]
        diameter: f64,
        /// Length of the cutting portion. The removal model does not need it —
        /// a cylinder cuts the same at every depth within its length — but the
        /// displayed cutter body is only as long as the flutes.
        #[serde(rename = "cuttingLengthMm")]
        cutting_length: f64,
    },
    Vbit {
        #[serde(rename = "includedAngleDeg")]
        angle: f64,
        #[serde(rename = "tipDiameterMm")]
        tip: f64,
        #[serde(rename = "maxCuttingDiameterMm")]
        diameter: f64,
        #[serde(rename = "cuttingHeightMm")]
        height: f64,
    },
}
#[derive(Clone, Copy, Debug)]
pub enum Tool {
    Knife,
    Endmill { radius: f64 },
    Vbit { tip: f64, slope: f64, radius: f64 },
}
fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.
}
impl ToolSpec {
    pub fn normalize(self) -> Result<Tool, String> {
        match self {
            Self::Knife {
                offset,
                max_cut_depth,
            } if positive(offset) && positive(max_cut_depth) => Ok(Tool::Knife),
            Self::Endmill {
                diameter,
                cutting_length,
            } if positive(diameter) && positive(cutting_length) => Ok(Tool::Endmill {
                radius: diameter / 2.,
            }),
            Self::Vbit {
                angle,
                tip,
                diameter,
                height,
            } if angle.is_finite()
                && angle > 0.
                && angle < 180.
                && tip.is_finite()
                && tip >= 0.
                && positive(diameter)
                && diameter >= tip
                && positive(height) =>
            {
                let slope = ((angle / 2.) * std::f64::consts::PI / 180.).tan();
                let radial_reach = (height * slope).min(diameter / 2. - tip / 2.);
                Ok(Tool::Vbit {
                    tip: tip / 2.,
                    slope,
                    radius: tip / 2. + radial_reach,
                })
            }
            _ => Err("Invalid simulator tool geometry".into()),
        }
    }

    /// The cutting radius at `depth` below the tip: what the field removes at
    /// that depth, and therefore what a displayed cutter body has to reach at
    /// the same height. Derived from [`ToolSpec::normalize`], so the display and
    /// the cut cannot disagree (`a_cutter_body_reaches_exactly_as_far_as_the_removal`).
    pub fn radius_at_depth(self, depth: f64) -> Option<f64> {
        match self.normalize().ok()? {
            Tool::Knife => None,
            Tool::Endmill { radius } => Some(radius),
            Tool::Vbit { tip, slope, radius } => Some((tip + depth.max(0.) * slope).min(radius)),
        }
    }

    /// The cutter's body as a revolved profile from the tip upward:
    /// `(height above the tip, radius)`, strictly increasing in height. `None`
    /// for a drag knife, which is a blade rather than a surface of revolution.
    ///
    /// The radii come from the normalized cutting envelope; the heights come
    /// from the tool's own cutting length or height. Nothing above the cutting
    /// portion is drawn until a tool states a shaft and a stickout.
    pub fn profile(self) -> Option<Vec<(f64, f64)>> {
        match self.normalize().ok()? {
            Tool::Knife => None,
            Tool::Endmill { radius } => {
                let Self::Endmill { cutting_length, .. } = self else {
                    return None;
                };
                Some(vec![(0., radius), (cutting_length, radius)])
            }
            Tool::Vbit { tip, slope, radius } => {
                let Self::Vbit { height, .. } = self else {
                    return None;
                };
                let mut profile = vec![(0., tip)];
                let flank = if slope > 0. {
                    (radius - tip) / slope
                } else {
                    0.
                };
                if flank > 0. {
                    profile.push((flank, radius));
                }
                if height > flank {
                    profile.push((height, radius));
                }
                Some(profile)
            }
        }
    }
}
/// How the machine executes a motion. The plan already carries this
/// (`cam_core::toolpath::Interpolation`) and its checks require a positive feed
/// on every linear feed and forbid one on a rapid, so the clock can time each
/// motion from the plan's own numbers instead of guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Rapid,
    Feed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Motion {
    pub kind: String,
    pub tool: usize,
    /// Executed plan stage this motion belongs to. The display colours a cell
    /// by the stage that last deepened it, so the identity has to travel with
    /// the motion rather than being re-derived from the tool.
    #[serde(default)]
    pub stage: u16,
    /// Machine execution of this move; see [`Interpolation`].
    pub interpolation: Interpolation,
    /// Required on a linear feed, absent on a rapid. The clock needs it; the
    /// removal model does not.
    pub feed_mm_min: Option<f64>,
    /// The programmed arc this move is, when the program carries one. The move
    /// stays **one** motion — start, end and the arc between them — so the
    /// display's motion index stays the plan's motion index and every table the
    /// display indexes by it (stages, groups, vertices, headings) keeps lining
    /// up. Expanding an arc into a chord walk here would silently shift all of
    /// them, which is what `the_display_stream_is_one_motion_per_plan_motion`
    /// exists to catch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc: Option<cam_core::toolpath::ArcMove>,
    pub x0: f64,
    pub y0: f64,
    pub z0: f64,
    pub x1: f64,
    pub y1: f64,
    pub z1: f64,
}
impl Motion {
    /// XY endpoints, for the arc geometry that is defined between them.
    pub fn start_xy(&self) -> cam_core::geometry::Point {
        cam_core::geometry::Point::new(self.x0, self.y0)
    }

    pub fn end_xy(&self) -> cam_core::geometry::Point {
        cam_core::geometry::Point::new(self.x1, self.y1)
    }
    /// Path length of the move, which is what a feed rate is measured against:
    /// a plunge or a ramp is timed by its real path, not by its XY projection,
    /// and an arc by the arc it is rather than by its chord.
    pub fn length_mm(&self) -> f64 {
        let dx = self.x1 - self.x0;
        let dy = self.y1 - self.y0;
        let dz = self.z1 - self.z0;
        let xy = match self.arc {
            // A degenerate arc (a coincident centre, a zero radius) falls back
            // to the chord: the move is still a move.
            Some(arc) => arc
                .length(self.start_xy(), self.end_xy())
                .unwrap_or_else(|| (dx * dx + dy * dy).sqrt()),
            None => (dx * dx + dy * dy).sqrt(),
        };
        (xy * xy + dz * dz).sqrt()
    }
    /// Program time this move takes on a machine with `rapid_rate_mm_min`.
    /// A zero-length move takes no time: no state changes across it, and a
    /// floor would invent minutes on a job made of micro-segments.
    pub fn duration_seconds(&self, rapid_rate_mm_min: f64) -> Result<f64, String> {
        let length = self.length_mm();
        match self.interpolation {
            Interpolation::Rapid => {
                if !rapid_rate_mm_min.is_finite() || rapid_rate_mm_min <= 0. {
                    return Err("Simulator needs a positive rapid rate".into());
                }
                Ok(length / rapid_rate_mm_min * 60.)
            }
            Interpolation::Feed => {
                let feed = self
                    .feed_mm_min
                    .filter(|feed| feed.is_finite() && *feed > 0.)
                    .ok_or("Simulator motion stream has a linear feed without a feed rate")?;
                Ok(length / feed * 60.)
            }
        }
    }
    /// The cutter tip while the animation is inside this move.
    pub fn point_at(&self, fraction: f64) -> [f64; 3] {
        let t = fraction.clamp(0., 1.);
        let z = self.z0 + (self.z1 - self.z0) * t;
        // Inside an arc the tool follows the curve, not the chord: the marker,
        // the trail and the material all read their position from here.
        if let Some(arc) = self.arc
            && let Some(point) = arc.point_at(self.start_xy(), self.end_xy(), t)
        {
            return [point.x, point.y, z];
        }
        [
            self.x0 + (self.x1 - self.x0) * t,
            self.y0 + (self.y1 - self.y0) * t,
            z,
        ]
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub cell_mm: f64,
    pub capped_by_texels: bool,
    pub capped_by_budget: bool,
}
pub fn choose_resolution(
    width: f64,
    height: f64,
    detail: f64,
    texels: f64,
    budget: f64,
) -> Result<Resolution, String> {
    if !positive(width)
        || !positive(height)
        || !texels.is_finite()
        || texels < 1.
        || !budget.is_finite()
        || budget < 1.
    {
        return Err("Invalid simulator resolution bounds/budget".into());
    }
    let mut cell = 0.1_f64.min(if detail > 0. { detail / 4. } else { 0.1 });
    let cap = width.max(height) / texels;
    let capped_by_texels = cap > cell;
    if capped_by_texels {
        cell = cap;
    }
    let capped_by_budget = (width / cell).ceil() * (height / cell).ceil() > budget;
    if capped_by_budget {
        cell = cell.max((width * height / budget).sqrt());
    }
    Ok(Resolution {
        cell_mm: cell,
        capped_by_texels,
        capped_by_budget,
    })
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub applied_motions: usize,
    pub cutting_motions: usize,
    pub dirty_cells: usize,
    pub removed_volume_mm3: f64,
    pub stage_removed_mm3: [f64; 2],
}
#[derive(Clone)]
struct Tile {
    depth: Vec<u16>,
    /// Stage that last deepened the cell, and the job tool it used. Both are
    /// indices into the executed plan the display is showing.
    stage: Vec<u8>,
    tool: Vec<u8>,
}
#[derive(Clone)]
pub struct Field {
    pub stock: Stock,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    pub quantum: f64,
    inv_quantum: f64,
    tools: Vec<Tool>,
    pub tiles_x: usize,
    tiles: Vec<Option<Arc<Tile>>>,
    pub versions: Vec<u32>,
    pub stats: Stats,
}
impl Field {
    pub fn new(stock: Stock, tools: &[ToolSpec], cell: f64) -> Result<Self, String> {
        if ![stock.x0, stock.y0, stock.x1, stock.y1]
            .iter()
            .all(|v| v.is_finite())
            || !positive(stock.x1 - stock.x0)
            || !positive(stock.y1 - stock.y0)
            || !positive(stock.thickness_mm)
            || !positive(cell)
        {
            return Err("Invalid simulator stock/grid".into());
        }
        let cols = ((stock.x1 - stock.x0) / cell).ceil() as usize;
        let rows = ((stock.y1 - stock.y0) / cell).ceil() as usize;
        let tiles_x = cols.div_ceil(TILE);
        let tiles_y = rows.div_ceil(TILE);
        // The reference's resolution estimate precedes rounding and stock margin.
        // Enforce an actual allocation admission here instead of silently coarsening.
        if cols.checked_mul(rows).is_none_or(|n| n > MAX_CELLS)
            || tiles_x
                .checked_mul(tiles_y)
                .and_then(|n| n.checked_mul(TILE * TILE))
                .is_none_or(|n| n > MAX_CELLS + 2 * 8192 * TILE)
        {
            return Err("Simulator exceeds the 64 million cell experiment limit".into());
        }
        let tools = tools
            .iter()
            .map(|s| s.normalize())
            .collect::<Result<Vec<_>, _>>()?;
        if tools.is_empty()
            || tools.iter().any(|t| match t {
                Tool::Endmill { radius } | Tool::Vbit { radius, .. } => !positive(*radius),
                Tool::Knife => false,
            })
        {
            return Err("Simulator requires positive-radius tools".into());
        }
        Ok(Self {
            stock,
            cell,
            cols,
            rows,
            quantum: stock.thickness_mm / 65535.,
            inv_quantum: 65535. / stock.thickness_mm,
            tools,
            tiles_x,
            tiles: vec![None; tiles_x * tiles_y],
            versions: vec![0; tiles_x * tiles_y],
            stats: Stats::default(),
        })
    }
    pub fn allocated_bytes(&self) -> usize {
        self.tiles.iter().filter(|t| t.is_some()).count() * TILE * TILE * 4
            + self.versions.len() * 4
    }
    pub fn tile_allocated(&self, tile: usize) -> bool {
        self.tiles.get(tile).is_some_and(|t| t.is_some())
    }
    /// `(removed depth, stage, tool)` for one cell; stage and tool are 0 for
    /// material no cutter has touched.
    pub fn cell_at(&self, col: usize, row: usize) -> (u16, u8, u8) {
        if col >= self.cols || row >= self.rows {
            return (0, 0, 0);
        }
        let index = ((row & 255) << 8) | (col & 255);
        self.tiles[(row >> 8) * self.tiles_x + (col >> 8)]
            .as_ref()
            .map_or((0, 0, 0), |t| {
                (t.depth[index], t.stage[index], t.tool[index])
            })
    }
    /// Tile-major packed grid for incremental tile upload: every 256×256 tile
    /// is one contiguous 256 KiB block, so a dirty tile is a single copy.
    /// One `u32` per cell: `depth | stage << 16 | tool << 24`.
    pub fn packed_tile_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; self.tiles.len() * TILE * TILE * 4];
        for tile in 0..self.tiles.len() {
            let base = Self::tile_byte_offset(tile);
            // An unallocated tile keeps the zeroes the buffer was filled with.
            let _ = self.pack_tile(tile, &mut bytes[base..base + TILE * TILE * 4]);
        }
        bytes
    }
    /// Pack one tile into `out`, which must be exactly
    /// [`TILE`] × [`TILE`] × 4 bytes. The animation keeps the raster bytes the
    /// renderer draws and patches only the tiles an advance actually changed,
    /// so a frame that removes material copies kilobytes instead of the raster.
    pub fn pack_tile(&self, tile: usize, out: &mut [u8]) -> Result<(), String> {
        if out.len() != TILE * TILE * 4 {
            return Err("Simulator tile buffer has the wrong size".into());
        }
        let Some(data) = self.tiles.get(tile).and_then(|tile| tile.as_ref()) else {
            out.fill(0);
            return Ok(());
        };
        for local in 0..TILE * TILE {
            let packed = data.depth[local] as u32
                | ((data.stage[local] as u32) << 16)
                | ((data.tool[local] as u32) << 24);
            out[local * 4..local * 4 + 4].copy_from_slice(&packed.to_le_bytes());
        }
        Ok(())
    }
    /// Byte offset of a tile inside [`Field::packed_tile_bytes`].
    pub fn tile_byte_offset(tile: usize) -> usize {
        tile * TILE * TILE * 4
    }
    /// Rebuild a field from a transported packed grid (`u32` per cell: low 16
    /// bits depth, next 8 bits stage, top 8 bits tool) and its per-tile
    /// versions. Used by
    /// the display process so a checkpoint arrives as bytes, not as a JSON
    /// array, and can be extended into an interactive scrub range.
    pub fn from_packed(
        stock: Stock,
        tools: &[ToolSpec],
        cell: f64,
        packed: &[u8],
        versions: &[u32],
        allocated: &[u32],
        stats: Stats,
    ) -> Result<Self, String> {
        let mut field = Self::new(stock, tools, cell)?;
        if packed.len() != field.tiles.len() * TILE * TILE * 4 {
            return Err("Packed stock grid length does not match the tile grid".into());
        }
        if versions.len() != field.versions.len() {
            return Err("Packed stock versions do not match the tile grid".into());
        }
        // Tiles that exist but hold no cut are not visible in the packed cells;
        // the transported mask keeps allocation, versions and checksums equal.
        for tile in allocated {
            let index = *tile as usize;
            if index >= field.tiles.len() {
                return Err("Packed stock allocation mask is out of range".into());
            }
            field.tiles[index].get_or_insert_with(|| {
                Arc::new(Tile {
                    depth: vec![0; TILE * TILE],
                    stage: vec![0; TILE * TILE],
                    tool: vec![0; TILE * TILE],
                })
            });
        }
        let tiles_y = field.versions.len() / field.tiles_x.max(1);
        for tile in 0..field.tiles.len() {
            let base = Self::tile_byte_offset(tile);
            let tiles_y = tiles_y.max(1);
            let _ = tiles_y;
            let rows = field
                .rows
                .saturating_sub((tile / field.tiles_x.max(1)) * TILE)
                .min(TILE);
            let cols = field
                .cols
                .saturating_sub((tile % field.tiles_x.max(1)) * TILE)
                .min(TILE);
            for local_row in 0..rows {
                for local_col in 0..cols {
                    let at = base + (local_row * TILE + local_col) * 4;
                    let value = u32::from_le_bytes(packed[at..at + 4].try_into().unwrap());
                    let depth = (value & 0xffff) as u16;
                    let stage = ((value >> 16) & 0xff) as u8;
                    let tool = ((value >> 24) & 0xff) as u8;
                    if depth == 0 && stage == 0 && tool == 0 {
                        continue;
                    }
                    let data = field.tiles[tile].get_or_insert_with(|| {
                        Arc::new(Tile {
                            depth: vec![0; TILE * TILE],
                            stage: vec![0; TILE * TILE],
                            tool: vec![0; TILE * TILE],
                        })
                    });
                    let data = Arc::make_mut(data);
                    let local = local_row * TILE + local_col;
                    data.depth[local] = depth;
                    data.stage[local] = stage;
                    data.tool[local] = tool;
                }
            }
        }
        field.versions = versions.to_vec();
        field.stats = stats;
        Ok(field)
    }
    pub fn checksum(&self) -> String {
        let mut hash = 2166136261u32;
        let mut mix = |v: u32| {
            for b in v.to_le_bytes() {
                hash = (hash ^ b as u32).wrapping_mul(16777619);
            }
        };
        for (i, t) in self.tiles.iter().enumerate() {
            let Some(t) = t else {
                continue;
            };
            mix(i as u32);
            mix(self.versions[i]);
            for (j, &d) in t.depth.iter().enumerate() {
                if d != 0 {
                    mix(j as u32);
                    mix(d as u32);
                    mix(t.stage[j] as u32);
                    mix(t.tool[j] as u32);
                }
            }
        }
        format!("{hash:x}")
    }
    /// Explicit little-endian depth + stage + tool per cell, for full-cell
    /// parity tests.
    pub fn cell_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.cols * self.rows * 4);
        for row in 0..self.rows {
            for col in 0..self.cols {
                let (d, stage, tool) = self.cell_at(col, row);
                out.extend_from_slice(&d.to_le_bytes());
                out.push(stage);
                out.push(tool);
            }
        }
        out
    }
    /// GPU storage: low 16 bits depth, next 8 bits stage, top 8 bits tool.
    pub fn packed_cells(&self) -> Vec<u32> {
        let mut cells = Vec::with_capacity(self.cols * self.rows);
        for row in 0..self.rows {
            for col in 0..self.cols {
                let (depth, stage, tool) = self.cell_at(col, row);
                cells.push(depth as u32 | ((stage as u32) << 16) | ((tool as u32) << 24));
            }
        }
        cells
    }
    /// Apply the part of `motion` between two fractions of its length.
    ///
    /// A fractional window is how the animation advances inside one move, and
    /// the window is exact: any partition of a motion removes the same material
    /// as applying it in one piece
    /// (`a_partitioned_motion_removes_the_same_material_as_one_shot`). The
    /// motion counters therefore increment when a window starts at the
    /// beginning of a move, so progressive frames of one motion count once.
    pub fn apply(&mut self, motion: &Motion, start: f64, end: f64) -> Result<(), String> {
        let cutting = matches!(motion.kind.as_str(), "cut" | "plunge" | "ramp");
        if cutting
            && (![
                motion.x0, motion.y0, motion.z0, motion.x1, motion.y1, motion.z1, start, end,
            ]
            .iter()
            .all(|v| v.is_finite())
                || motion.tool >= self.tools.len())
        {
            return Err("Simulator needs finite motion coordinates and a known tool".into());
        }
        let start = start.clamp(0., 1.);
        let end = end.clamp(0., 1.);
        if start <= 0. {
            self.stats.applied_motions += 1;
        }
        if !cutting || matches!(self.tools.get(motion.tool), Some(Tool::Knife)) {
            return Ok(());
        }
        if start <= 0. {
            self.stats.cutting_motions += 1;
        }
        if end <= start {
            return Ok(());
        }
        // A programmed arc is swept as chords of the *whole* arc, at fixed
        // fractions of it, so any partition of the move still lands on the same
        // chords and removes the same material. The move itself stays one
        // motion: expanding it here keeps the display's index space aligned.
        if let Some(arc) = motion.arc
            && let Some(chords) = arc_chord_count(arc, motion)
        {
            for chord in 0..chords {
                let (from, to) = (
                    chord as f64 / chords as f64,
                    (chord + 1) as f64 / chords as f64,
                );
                let (lo, hi) = (start.max(from), end.min(to));
                if hi <= lo {
                    continue;
                }
                let span = to - from;
                let segment = Motion {
                    arc: None,
                    x0: motion.point_at(from)[0],
                    y0: motion.point_at(from)[1],
                    z0: motion.point_at(from)[2],
                    x1: motion.point_at(to)[0],
                    y1: motion.point_at(to)[1],
                    z1: motion.point_at(to)[2],
                    ..motion.clone()
                };
                self.sweep(&segment, (lo - from) / span, (hi - from) / span)?;
            }
            return Ok(());
        }
        self.sweep(motion, start, end)
    }

    /// The straight-line sweep itself: the geometry the reference simulator
    /// defines, applied over a window of one linear move.
    fn sweep(&mut self, motion: &Motion, start: f64, end: f64) -> Result<(), String> {
        let a = motion.point_at(start);
        let b = motion.point_at(end);
        if -a[2] <= 0. && -b[2] <= 0. {
            return Ok(());
        }
        let tool = self.tools[motion.tool];
        let (radius, role) = match tool {
            Tool::Knife => return Ok(()),
            Tool::Endmill { radius } => (radius, 1),
            Tool::Vbit { radius, .. } => (radius, 2),
        };
        let stage = motion.stage.min(u8::MAX as u16) as u8;
        let tool_index = motion.tool.min(u8::MAX as usize) as u8;
        let Some((c0, c1)) = range(
            self.stock.x0,
            self.cell,
            self.cols,
            a[0].min(b[0]) - radius,
            a[0].max(b[0]) + radius,
        ) else {
            return Ok(());
        };
        let Some((r0, r1)) = range(
            self.stock.y0,
            self.cell,
            self.rows,
            a[1].min(b[1]) - radius,
            a[1].max(b[1]) + radius,
        ) else {
            return Ok(());
        };
        // The geometry frame is the **whole motion's**, not the window's: every
        // cell then gets the same covered interval however the move is split,
        // so the animation's frames cannot disagree with a replay of the same
        // move in one piece (see `a_partitioned_motion_removes_the_same_material_as_one_shot`).
        // For a whole motion (`start` 0, `end` 1) these values are exactly the
        // ones the reference simulator computes, so parity is unchanged.
        let origin = [motion.x0, motion.y0, motion.z0];
        let dx = motion.x1 - motion.x0;
        let dy = motion.y1 - motion.y0;
        let dz = motion.z1 - motion.z0;
        let a2 = dx * dx + dy * dy;
        let inv2a = if a2 > PLUNGE_AREA_EPS {
            1. / (2. * a2)
        } else {
            0.
        };
        for tr in (r0 >> 8)..=(r1 >> 8) {
            for tc in (c0 >> 8)..=(c1 >> 8) {
                let tile = tr * self.tiles_x + tc;
                for row in r0.max(tr << 8)..=r1.min((tr << 8) | 255) {
                    let py = self.stock.y0 + (row as f64 + 0.5) * self.cell - origin[1];
                    for col in c0.max(tc << 8)..=c1.min((tc << 8) | 255) {
                        let px = self.stock.x0 + (col as f64 + 0.5) * self.cell - origin[0];
                        let qq = px * px + py * py;
                        let b_raw = -2. * (px * dx + py * dy);
                        let Some((t0, t1)) = coverage(qq - radius * radius, a2, b_raw, inv2a)
                        else {
                            continue;
                        };
                        // The window restricts the motion's own covered
                        // interval instead of re-deriving one from its endpoints.
                        let t0 = t0.max(start);
                        let t1 = t1.min(end);
                        if t0 > t1 {
                            continue;
                        }
                        let depth = match tool {
                            Tool::Knife => unreachable!("knife never removes stock"),
                            Tool::Endmill { .. } => {
                                let d0 = -origin[2] - dz * t0;
                                let d1 = -origin[2] - dz * t1;
                                if d0 > d1 { d0 } else { d1 }
                            }
                            Tool::Vbit { tip, slope, .. } => {
                                let inv_slope = 1. / slope;
                                let denominator = inv_slope * inv_slope * a2 - dz * dz;
                                let stationary_valid = inv2a != 0. && denominator > 0.;
                                let stationary = if stationary_valid {
                                    (-dz * ((4. * a2 * qq - b_raw * b_raw) / denominator).sqrt()
                                        - b_raw)
                                        * inv2a
                                } else {
                                    0.
                                };
                                let mut surface = f64::INFINITY;
                                for candidate in 0..6 {
                                    let t = match candidate {
                                        0 => t0,
                                        1 => t1,
                                        2 if inv2a != 0. => {
                                            ((px * dx + py * dy) / a2).clamp(t0, t1)
                                        }
                                        3 | 4 if inv2a != 0. => {
                                            let disc = b_raw * b_raw - 4. * a2 * (qq - tip * tip);
                                            if disc < 0. {
                                                continue;
                                            }
                                            let root = disc.sqrt();
                                            let t = if candidate == 3 {
                                                (-b_raw - root) * inv2a
                                            } else {
                                                (-b_raw + root) * inv2a
                                            };
                                            if t < t0 || t > t1 {
                                                continue;
                                            }
                                            t
                                        }
                                        5 if stationary_valid => {
                                            if stationary < t0 || stationary > t1 {
                                                continue;
                                            }
                                            stationary
                                        }
                                        _ => continue,
                                    };
                                    let dist = (a2 * t * t + b_raw * t + qq).sqrt();
                                    let height = if dist <= tip {
                                        0.
                                    } else {
                                        (dist - tip) * inv_slope
                                    };
                                    let z = origin[2] + dz * t + height;
                                    // An explicit comparison preserves JS NaN handling.
                                    if z < surface {
                                        surface = z;
                                    }
                                }
                                -surface
                            }
                        };
                        if depth <= 0. {
                            continue;
                        }
                        let level = if depth >= self.stock.thickness_mm {
                            65535
                        } else {
                            (depth * self.inv_quantum) as u16
                        };
                        let data = self.tiles[tile].get_or_insert_with(|| {
                            Arc::new(Tile {
                                depth: vec![0; TILE * TILE],
                                stage: vec![0; TILE * TILE],
                                tool: vec![0; TILE * TILE],
                            })
                        });
                        let local = ((row & 255) << 8) | (col & 255);
                        let previous = data.depth[local];
                        if level > previous {
                            let data = Arc::make_mut(data);
                            data.depth[local] = level;
                            data.stage[local] = stage;
                            data.tool[local] = tool_index;
                            let delta =
                                (level - previous) as f64 * self.quantum * (self.cell * self.cell);
                            self.stats.dirty_cells += usize::from(previous == 0);
                            self.stats.removed_volume_mm3 += delta;
                            self.stats.stage_removed_mm3[role as usize - 1] += delta;
                            self.versions[tile] = self.versions[tile].wrapping_add(1);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
/// How many chords one arc is swept with, bounded so the sagitta stays inside
/// [`ARC_CHORD_MM`]. `2*acos(1 - e/r)` is the exact step angle whose sagitta is
/// `e`. `None` when the arc geometry is degenerate: the chord between the ends
/// is then all there is, and the straight sweep already handles it.
fn arc_chord_count(arc: cam_core::toolpath::ArcMove, motion: &Motion) -> Option<usize> {
    let radius = arc.radius(motion.start_xy())?;
    let sweep = arc.sweep_rad(motion.start_xy(), motion.end_xy())?;
    let step = if ARC_CHORD_MM >= radius {
        std::f64::consts::PI
    } else {
        2. * (1. - ARC_CHORD_MM / radius).acos()
    };
    Some(((sweep / step).ceil() as usize).clamp(1, MAX_ARC_CHORDS))
}

fn coverage(c: f64, a2: f64, b: f64, inv2a: f64) -> Option<(f64, f64)> {
    if inv2a == 0. {
        return if c > 0. { None } else { Some((0., 1.)) };
    }
    let disc = b * b - 4. * a2 * c;
    if disc < 0. {
        return None;
    }
    let root = disc.sqrt();
    let t0 = ((-b - root) * inv2a).max(0.);
    let t1 = ((-b + root) * inv2a).min(1.);
    if t0 > t1 { None } else { Some((t0, t1)) }
}
fn range(origin: f64, cell: f64, count: usize, min: f64, max: f64) -> Option<(usize, usize)> {
    let lo = ((min - origin) / cell).floor().max(0.);
    let hi = ((max - origin) / cell).floor().min(count as f64 - 1.);
    if lo > hi {
        None
    } else {
        Some((lo as usize, hi as usize))
    }
}

/// Bounded copy-on-write checkpoints. Seeking never subtracts raster cuts;
/// it restores a prior exact prefix and replays the retained motion stream.
pub struct Playback {
    pub field: Field,
    pristine: Field,
    /// Motions fully applied to `field`. `field` is the exact state at this
    /// prefix plus `motions[position]` applied over `[0, fraction]`.
    pub position: usize,
    /// Fraction of the in-flight motion already applied. Zero means `field` is
    /// exactly the prefix state.
    fraction: f64,
    /// `(prefix, field, pinned)`. Transported seed frames are pinned so a
    /// bounded replay window survives interactive seeking instead of being
    /// churned away by the user's own positions.
    checkpoints: Vec<(usize, Field, bool)>,
    budget: usize,
    limit: usize,
}

/// What one seek actually did. The display quotes this instead of guessing how
/// much stock work a scrub caused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeekReport {
    /// Prefix the replay started from (a checkpoint, the pristine field or the
    /// current position).
    pub from: usize,
    /// Motions re-integrated after that prefix.
    pub replayed: usize,
}

impl Playback {
    pub fn new(field: Field, checkpoint_budget: usize) -> Self {
        Self {
            pristine: field.clone(),
            field,
            position: 0,
            fraction: 0.,
            checkpoints: Vec::new(),
            budget: checkpoint_budget,
            limit: 4,
        }
    }
    /// Adopt checkpoint fields transported from the disposable compute process.
    /// The seed frames bound replay work for interactive scrubbing; the limit
    /// keeps them until the user's own seeks replace them.
    pub fn seed(
        field: Field,
        pristine: Field,
        points: Vec<(usize, Field)>,
        checkpoint_budget: usize,
    ) -> Self {
        let limit = points.len().max(4);
        let position = points.last().map_or(0, |(prefix, _)| *prefix);
        Self {
            pristine,
            field,
            position,
            fraction: 0.,
            checkpoints: points
                .into_iter()
                .map(|(prefix, field)| (prefix, field, true))
                .collect(),
            budget: checkpoint_budget,
            limit,
        }
    }
    /// Fraction of the in-flight motion the field already holds.
    pub fn fraction(&self) -> f64 {
        self.fraction
    }
    /// Prefixes that bound the current replay window.
    pub fn checkpoint_prefixes(&self) -> Vec<usize> {
        self.checkpoints
            .iter()
            .map(|(prefix, _, _)| *prefix)
            .collect()
    }
    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints
            .iter()
            .map(|(_, f, _)| f.allocated_bytes())
            .sum()
    }
    /// Seek to `target`, starting from whichever exact prefix costs the least
    /// replay work. A forward jump across a checkpoint restores it instead of
    /// re-integrating every motion in between; a backward jump always restores
    /// the nearest earlier checkpoint. Either way the restored field is an
    /// exact prefix state, so the result is identical to a cold replay.
    pub fn seek(&mut self, motions: &[Motion], target: usize) -> Result<SeekReport, String> {
        if target > motions.len() {
            return Err("Simulator playhead exceeds retained motions".into());
        }
        let saved = self
            .checkpoints
            .iter()
            .filter(|(p, _, _)| *p <= target)
            .max_by_key(|(p, _, _)| *p);
        let saved_prefix = saved.map_or(0, |(p, _, _)| *p);
        // A field holding an unfinished motion is not an exact prefix state, and
        // a raster cannot be subtracted: restore an exact state and replay
        // instead of pretending the prefix is already there.
        let mid_motion = self.fraction > 0.;
        if mid_motion || target < self.position || saved_prefix > self.position {
            let (p, f) = saved.map_or((0, &self.pristine), |(p, f, _)| (*p, f));
            self.field = f.clone();
            self.position = p;
        }
        self.fraction = 0.;
        let from = self.position;
        for motion in &motions[self.position..target] {
            self.field.apply(motion, 0., 1.)?;
            self.position += 1;
        }
        let bytes = self.field.allocated_bytes();
        if bytes <= self.budget && !self.checkpoints.iter().any(|(p, _, _)| *p == target) {
            // Keep room for a few user positions on top of the transported
            // seeds; those seeds are what bound the replay distance.
            let capacity = self.limit + 4;
            while !self.checkpoints.is_empty()
                && (self.checkpoints.len() >= capacity
                    || self.checkpoint_bytes() + bytes > self.budget)
            {
                let Some(victim) = self.checkpoints.iter().position(|(_, _, pinned)| !pinned)
                else {
                    // Only transported seeds are left: never evict them for a
                    // transient position unless the budget itself demands it.
                    break;
                };
                self.checkpoints.remove(victim);
            }
            if self.checkpoints.len() < capacity && self.checkpoint_bytes() + bytes <= self.budget {
                self.checkpoints.push((target, self.field.clone(), false));
            }
        }
        Ok(SeekReport {
            from,
            replayed: target - from,
        })
    }

    /// Move to `(prefix, fraction)` by applying only the windows in between.
    ///
    /// Returns `false` when the target sits behind the current state, or when
    /// it is further than `limit` motions ahead: the caller restores an exact
    /// state through [`Playback::seek`] — or asks the compute process for one —
    /// and advances again. A fraction of 1.0 means the move is finished and is
    /// normalised to the next prefix, so a position never carries a fraction
    /// of exactly one.
    pub fn advance_to(
        &mut self,
        motions: &[Motion],
        prefix: usize,
        fraction: f64,
        limit: usize,
    ) -> Result<bool, String> {
        if prefix > motions.len() {
            return Err("Simulator playhead exceeds retained motions".into());
        }
        let mut target = prefix;
        let mut target_fraction = fraction.clamp(0., 1.);
        if target_fraction >= 1. && target < motions.len() {
            target += 1;
            target_fraction = 0.;
        }
        if target == motions.len() {
            target_fraction = 0.;
        }
        if target < self.position || (target == self.position && target_fraction < self.fraction) {
            return Ok(false);
        }
        if target - self.position > limit {
            return Ok(false);
        }
        if target == self.position {
            if target_fraction > self.fraction && target < motions.len() {
                self.field
                    .apply(&motions[target], self.fraction, target_fraction)?;
            }
            self.fraction = target_fraction;
            return Ok(true);
        }
        // Finish the move the animation is inside, then consume whole motions.
        let first = if self.fraction > 0. {
            self.field
                .apply(&motions[self.position], self.fraction, 1.)?;
            self.position + 1
        } else {
            self.position
        };
        for motion in &motions[first..target] {
            self.field.apply(motion, 0., 1.)?;
        }
        self.position = target;
        self.fraction = 0.;
        if target_fraction > 0. && target < motions.len() {
            self.field.apply(&motions[target], 0., target_fraction)?;
            self.fraction = target_fraction;
        }
        Ok(true)
    }

    /// Restore an exact state at `(prefix, fraction)`: seek to the prefix, then
    /// apply the unfinished move. This is what a scrub to a time uses, and it
    /// is bit-identical to a cold replay of the same position.
    pub fn seek_position(
        &mut self,
        motions: &[Motion],
        prefix: usize,
        fraction: f64,
    ) -> Result<SeekReport, String> {
        let report = self.seek(motions, prefix)?;
        self.advance_to(motions, prefix, fraction, 0)?;
        Ok(report)
    }
}

/// Binary motion stream. `Motion` is field tuples and a tool index, so the
/// display process can rebuild the exact replay input without parsing a JSON
/// array of positions.
/// kind u8 | stage u16 | interpolation u8 | tool u32 | feed f64 |
/// arc centre x f64 | arc centre y f64 | arc flags u8 | pad 7 |
/// six f64 coordinates.
pub const MOTION_BYTES: usize = 88;

pub fn interpolation_code(interpolation: Interpolation) -> u8 {
    match interpolation {
        Interpolation::Rapid => 0,
        Interpolation::Feed => 1,
    }
}

pub fn code_interpolation(code: u8) -> Interpolation {
    match code {
        1 => Interpolation::Feed,
        _ => Interpolation::Rapid,
    }
}

pub fn kind_code(kind: &str) -> u8 {
    match kind {
        "cut" => 0,
        "plunge" => 1,
        "ramp" => 2,
        _ => 3,
    }
}

pub fn code_kind(code: u8) -> &'static str {
    match code {
        0 => "cut",
        1 => "plunge",
        2 => "ramp",
        _ => "rapid",
    }
}

pub fn encode_motions(motions: &[Motion]) -> Vec<u8> {
    let mut out = Vec::with_capacity(motions.len() * MOTION_BYTES);
    for motion in motions {
        out.push(kind_code(&motion.kind));
        out.extend_from_slice(&motion.stage.to_le_bytes());
        out.push(interpolation_code(motion.interpolation));
        out.extend_from_slice(&(motion.tool as u32).to_le_bytes());
        out.extend_from_slice(&motion.feed_mm_min.unwrap_or(f64::NAN).to_le_bytes());
        // A programmed arc travels with its move: the centre and the direction
        // here, the endpoints in the coordinates below.
        let (center_x, center_y) = motion
            .arc
            .map_or((f64::NAN, f64::NAN), |arc| (arc.center.x, arc.center.y));
        out.extend_from_slice(&center_x.to_le_bytes());
        out.extend_from_slice(&center_y.to_le_bytes());
        out.push(match motion.arc {
            None => 0,
            Some(arc) if arc.clockwise => 1,
            Some(_) => 2,
        });
        out.extend_from_slice(&[0u8; 7]);
        for value in [
            motion.x0, motion.y0, motion.z0, motion.x1, motion.y1, motion.z1,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

pub fn decode_motions(bytes: &[u8], tools: usize) -> Result<Vec<Motion>, String> {
    if !bytes.len().is_multiple_of(MOTION_BYTES) {
        return Err("Motion stream is not a whole number of records".into());
    }
    let mut motions = Vec::with_capacity(bytes.len() / MOTION_BYTES);
    for record in bytes.chunks_exact(MOTION_BYTES) {
        let kind = code_kind(record[0]).to_string();
        let stage = u16::from_le_bytes(record[1..3].try_into().unwrap());
        let interpolation = code_interpolation(record[3]);
        let tool = u32::from_le_bytes(record[4..8].try_into().unwrap()) as usize;
        let feed = f64::from_le_bytes(record[8..16].try_into().unwrap());
        let center_x = f64::from_le_bytes(record[16..24].try_into().unwrap());
        let center_y = f64::from_le_bytes(record[24..32].try_into().unwrap());
        let arc_flags = record[32];
        let mut values = [0_f64; 6];
        for (i, value) in values.iter_mut().enumerate() {
            let at = 40 + i * 8;
            *value = f64::from_le_bytes(record[at..at + 8].try_into().unwrap());
        }
        if tool >= tools.max(1) {
            return Err("Motion stream references an unknown tool".into());
        }
        if !values.iter().all(|v| v.is_finite()) {
            return Err("Motion stream contains non-finite coordinates".into());
        }
        motions.push(Motion {
            kind,
            tool,
            stage,
            interpolation,
            // A rapid carries no feed, and the stream writes NaN for it. A feed
            // motion without one is kept as it is: the clock refuses to time it
            // rather than the display refusing to show the plan at all.
            feed_mm_min: feed.is_finite().then_some(feed),
            arc: match arc_flags {
                1 | 2 if center_x.is_finite() && center_y.is_finite() => {
                    Some(cam_core::toolpath::ArcMove {
                        center: cam_core::geometry::Point::new(center_x, center_y),
                        clockwise: arc_flags == 1,
                    })
                }
                _ => None,
            },
            x0: values[0],
            y0: values[1],
            z0: values[2],
            x1: values[3],
            y1: values[4],
            z1: values[5],
        });
    }
    Ok(motions)
}

/// Program time of every motion boundary, so the transport can move between
/// time and `(prefix, fraction)` without integrating the stream again.
///
/// The table is derived from the plan's own numbers: a linear feed costs its
/// length at its `feed_mm_min`, a rapid costs its length at the machine's rapid
/// rate. A zero-length move costs nothing, which keeps a job made of
/// micro-segments honest instead of inventing a floor per segment.
pub struct TimeTable {
    /// `cumulative[i]` is the program time before motion `i`; one longer than
    /// the motion stream, so the final entry is the total.
    cumulative: Vec<f64>,
    rapid_rate_mm_min: f64,
    assumed_rapid_rate: bool,
}

impl TimeTable {
    pub fn build(motions: &[Motion], rapid_rate_mm_min: Option<f64>) -> Result<Self, String> {
        let (rate, assumed) = match rapid_rate_mm_min {
            Some(rate) if rate.is_finite() && rate > 0. => (rate, false),
            _ => (DEFAULT_RAPID_RATE_MM_MIN, true),
        };
        let mut cumulative = Vec::with_capacity(motions.len() + 1);
        let mut seconds = 0.;
        cumulative.push(0.);
        for motion in motions {
            seconds += motion.duration_seconds(rate)?;
            cumulative.push(seconds);
        }
        Ok(Self {
            cumulative,
            rapid_rate_mm_min: rate,
            assumed_rapid_rate: assumed,
        })
    }
    pub fn motions(&self) -> usize {
        self.cumulative.len() - 1
    }
    pub fn total_seconds(&self) -> f64 {
        self.cumulative.last().copied().unwrap_or(0.)
    }
    pub fn rapid_rate_mm_min(&self) -> f64 {
        self.rapid_rate_mm_min
    }
    /// True when the machine stated no rapid rate and the display fallback is
    /// what times the G0 moves. The transport says so rather than presenting
    /// the fallback as machine truth.
    pub fn assumes_rapid_rate(&self) -> bool {
        self.assumed_rapid_rate
    }
    /// Program time one motion takes.
    pub fn duration_of(&self, index: usize) -> f64 {
        if index + 1 >= self.cumulative.len() {
            return 0.;
        }
        self.cumulative[index + 1] - self.cumulative[index]
    }
    /// Program time of an exact prefix.
    pub fn seconds_at_prefix(&self, prefix: usize) -> f64 {
        self.cumulative[prefix.min(self.motions())]
    }
    /// Program time of `(prefix, fraction)`.
    pub fn seconds_at(&self, prefix: usize, fraction: f64) -> f64 {
        if prefix >= self.motions() {
            return self.total_seconds();
        }
        self.cumulative[prefix] + self.duration_of(prefix) * fraction.clamp(0., 1.)
    }
    /// The position at a program time: the motion it falls inside and how much
    /// of it is done. Zero-length moves are skipped — no state changes across
    /// them and no time passes — so the result never carries a fraction of
    /// exactly one.
    pub fn position_at(&self, seconds: f64) -> (usize, f64) {
        let seconds = seconds.clamp(0., self.total_seconds());
        let index = self.cumulative.partition_point(|t| *t <= seconds);
        if index >= self.cumulative.len() {
            return (self.motions(), 0.);
        }
        let prefix = index.saturating_sub(1);
        let duration = self.duration_of(prefix);
        let fraction = if duration > 0. {
            ((seconds - self.cumulative[prefix]) / duration).clamp(0., 1.)
        } else {
            0.
        };
        (prefix, fraction)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub stock: Stock,
    pub tools: Vec<ToolSpec>,
    pub resolution: Resolution,
    pub motions: Vec<Motion>,
    pub prefixes: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion(kind: &str, tool: usize, x: f64) -> Motion {
        let rapid = kind == "rapid_x_y";
        Motion {
            kind: kind.into(),
            tool,
            stage: 0,
            interpolation: if rapid {
                Interpolation::Rapid
            } else {
                Interpolation::Feed
            },
            feed_mm_min: if rapid { None } else { Some(120.) },
            arc: None,
            x0: x,
            y0: x + 1.,
            z0: -x,
            x1: x + 2.,
            y1: x + 3.,
            z1: -(x + 4.),
        }
    }

    #[test]
    fn motion_stream_round_trips_every_field() {
        let motions = vec![
            motion("cut", 0, 0.5),
            motion("plunge", 1, 1.25),
            motion("ramp", 1, 2.),
            motion("rapid_x_y", 0, 3.5),
        ];
        let bytes = encode_motions(&motions);
        assert_eq!(bytes.len(), motions.len() * MOTION_BYTES);
        let decoded = decode_motions(&bytes, 2).unwrap();
        assert_eq!(decoded.len(), motions.len());
        for (decoded, original) in decoded.iter().zip(&motions) {
            assert_eq!(decoded.tool, original.tool);
            assert_eq!(decoded.stage, original.stage);
            assert_eq!(decoded.interpolation, original.interpolation);
            assert_eq!(decoded.feed_mm_min, original.feed_mm_min);
            // Non-cutting kinds collapse to "rapid"; the simulator treats every
            // other kind identically.
            assert_eq!(
                matches!(decoded.kind.as_str(), "cut" | "plunge" | "ramp"),
                matches!(original.kind.as_str(), "cut" | "plunge" | "ramp")
            );
            for (a, b) in [
                (decoded.x0, original.x0),
                (decoded.y0, original.y0),
                (decoded.z0, original.z0),
                (decoded.x1, original.x1),
                (decoded.y1, original.y1),
                (decoded.z1, original.z1),
            ] {
                assert_eq!(a.to_bits(), b.to_bits());
            }
        }
        assert!(decode_motions(&bytes[..MOTION_BYTES - 1], 2).is_err());
        assert!(decode_motions(&bytes, 1).is_err());
    }

    /// The whole display indexes motions by the plan's index — the stages, the
    /// timeline spans, the path vertices, the picker, the knife headings. A
    /// programmed arc therefore travels **inside** one motion: it is timed by
    /// its arc, positioned along its arc, and swept as chords of its arc, and
    /// none of that may add or remove a motion.
    #[test]
    fn an_arc_travels_with_its_motion_instead_of_expanding_it() {
        use cam_core::geometry::Point;
        use cam_core::toolpath::ArcMove;

        let quarter = ArcMove {
            center: Point::new(0., 0.),
            clockwise: false,
        };
        let arc = Motion {
            kind: "cut".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(600.),
            arc: Some(quarter),
            x0: 10.,
            y0: 0.,
            z0: -1.,
            x1: 0.,
            y1: 10.,
            z1: -1.,
        };
        // The move is timed and measured by its arc, not by its chord.
        let arc_length = std::f64::consts::FRAC_PI_2 * 10.;
        assert!((arc.length_mm() - arc_length).abs() < 1e-9);
        let chord = ((10_f64).powi(2) + (10_f64).powi(2)).sqrt();
        assert!(arc.length_mm() > chord);
        // Halfway through it is on the circle, not on the chord.
        let middle = arc.point_at(0.5);
        let on_circle = (middle[0] * middle[0] + middle[1] * middle[1]).sqrt();
        assert!((on_circle - 10.).abs() < 1e-9, "{middle:?}");
        assert!((middle[2] + 1.).abs() < 1e-9, "z stays linear: {middle:?}");

        // The wire carries the arc, so the display process rebuilds the same
        // motion — and the same count.
        let bytes = encode_motions(std::slice::from_ref(&arc));
        let decoded = decode_motions(&bytes, 1).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].arc, Some(quarter));
        assert_eq!(decoded[0].length_mm().to_bits(), arc.length_mm().to_bits());

        // A flat quarter-circle cut: the material follows the curve. A cell on
        // the arc is cut; a cell just inside the chord — which a straight move
        // would have removed — is not.
        let stock = Stock {
            x0: -16.,
            y0: -16.,
            x1: 16.,
            y1: 16.,
            thickness_mm: 4.,
        };
        let tools = [ToolSpec::Endmill {
            diameter: 0.6,
            cutting_length: 12.,
        }];
        let mut field = Field::new(stock, &tools, 0.05).unwrap();
        field.apply(&arc, 0., 1.).unwrap();
        let cut_at = |field: &Field, x: f64, y: f64| {
            let col = ((x - stock.x0) / field.cell).floor() as usize;
            let row = ((y - stock.y0) / field.cell).floor() as usize;
            field.cell_at(col, row).0 > 0
        };
        // (7.07, 7.07) is on the arc; the chord runs through (5, 5), which the
        // arc does not touch.
        assert!(cut_at(&field, 7.07, 7.07), "the arc is cut");
        assert!(!cut_at(&field, 5., 5.), "the chord is not cut");
    }

    /// The clock is the plan's own arithmetic: a feed move costs its length at
    /// its programmed feed, a rapid costs its length at the machine's rate, and
    /// the facing job's pass/plunge ratio comes out of the numbers rather than
    /// out of a per-motion step.
    #[test]
    fn a_feed_move_is_timed_by_its_feed_and_a_rapid_by_the_machine_rate() {
        let pass = Motion {
            kind: "cut".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(2400.),
            arc: None,
            x0: 0.,
            y0: 0.,
            z0: -1.,
            x1: 300.,
            y1: 0.,
            z1: -1.,
        };
        let plunge = Motion {
            kind: "cut".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(600.),
            arc: None,
            x0: 0.,
            y0: 0.,
            z0: 0.,
            x1: 0.,
            y1: 0.,
            z1: -14.,
        };
        let rapid = Motion {
            kind: "rapid_xy".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Rapid,
            feed_mm_min: None,
            arc: None,
            x0: 0.,
            y0: 0.,
            z0: 5.,
            x1: 300.,
            y1: 0.,
            z1: 5.,
        };
        // 300 mm at 2400 mm/min is 7.5 s; 14 mm at 600 mm/min is 1.4 s.
        assert!((pass.duration_seconds(5000.).unwrap() - 7.5).abs() < 1e-9);
        assert!((plunge.duration_seconds(5000.).unwrap() - 1.4).abs() < 1e-9);
        // The rapid ignores the feeds and uses the machine rate: 300 mm at
        // 5000 mm/min is 3.6 s, and the rate does not affect the feed moves.
        assert!((rapid.duration_seconds(5000.).unwrap() - 3.6).abs() < 1e-9);
        assert!((rapid.duration_seconds(10000.).unwrap() - 1.8).abs() < 1e-9);
        assert!((pass.duration_seconds(10000.).unwrap() - 7.5).abs() < 1e-9);
        // A zero-length move takes no time: no state changes across it.
        let stalled = Motion {
            x1: 0.,
            ..rapid.clone()
        };
        assert_eq!(stalled.duration_seconds(5000.).unwrap(), 0.);
    }

    #[test]
    fn a_feed_without_a_rate_is_refused_by_the_clock_not_by_the_display() {
        let motions = vec![
            Motion {
                kind: "cut".into(),
                tool: 0,
                stage: 0,
                interpolation: Interpolation::Feed,
                feed_mm_min: None,
                arc: None,
                x0: 0.,
                y0: 0.,
                z0: -1.,
                x1: 1.,
                y1: 0.,
                z1: -1.,
            },
            motion("rapid_x_y", 0, 4.),
        ];
        // The stream itself still decodes, so a plan with an unfinished feed is
        // still visible; only the timing refuses to invent one.
        let bytes = encode_motions(&motions);
        let decoded = decode_motions(&bytes, 1).unwrap();
        assert_eq!(decoded[0].feed_mm_min, None);
        assert!(TimeTable::build(&decoded, Some(5000.)).is_err());
        // A rapid without a machine rate is timed by the stated fallback
        // instead of being refused, and the table says which it used.
        let assumed = TimeTable::build(&[motion("rapid_x_y", 0, 4.)], None).unwrap();
        assert!(assumed.assumes_rapid_rate());
        assert_eq!(assumed.rapid_rate_mm_min(), DEFAULT_RAPID_RATE_MM_MIN);
        let rapid = motion("rapid_x_y", 0, 4.);
        let stated = TimeTable::build(std::slice::from_ref(&rapid), Some(8000.)).unwrap();
        assert!(!stated.assumes_rapid_rate());
        assert!((stated.total_seconds() * 8000. / 60. - rapid.length_mm()).abs() < 1e-9);
    }

    /// Time and position are two views of one table: every position maps to a
    /// time and back to the same position, and a zero-length move is skipped
    /// rather than given invented minutes.
    #[test]
    fn the_time_table_moves_between_time_and_position() {
        let cut = |x: f64, feed: f64| Motion {
            kind: "cut".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(feed),
            arc: None,
            x0: x,
            y0: 0.,
            z0: -1.,
            x1: x + 10.,
            y1: 0.,
            z1: -1.,
        };
        let stalled = Motion {
            kind: "cut".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(100.),
            arc: None,
            x0: 5.,
            y0: 0.,
            z0: -1.,
            x1: 5.,
            y1: 0.,
            z1: -1.,
        };
        let table =
            TimeTable::build(&[cut(0., 600.), stalled.clone(), cut(50., 1200.)], None).unwrap();
        assert_eq!(table.motions(), 3);
        assert_eq!(table.duration_of(0), 1.);
        assert_eq!(table.duration_of(1), 0.);
        assert_eq!(table.duration_of(2), 0.5);
        assert!((table.total_seconds() - 1.5).abs() < 1e-9);
        for (prefix, fraction) in [(0, 0.), (0, 0.25), (2, 0.5), (3, 0.)] {
            let seconds = table.seconds_at(prefix, fraction);
            assert_eq!(
                table.position_at(seconds),
                (prefix, fraction),
                "position -> time -> position at ({prefix}, {fraction})"
            );
        }
        // A position never carries a fraction of exactly one: the end of a move
        // is the start of the next, and with no motion left it is the end.
        assert_eq!(table.position_at(table.seconds_at(0, 1.)), (2, 0.));
        // The stalled move occupies no time, so the time reached at the end of
        // the first move is the start of the third.
        assert_eq!(table.position_at(1.), (2, 0.));
        assert_eq!(table.seconds_at_prefix(2), 1.);
        assert_eq!(table.position_at(-5.), (0, 0.));
        assert_eq!(table.position_at(99.), (3, 0.));
    }

    /// The fractional window is the whole basis of the animation, so the
    /// material state has to be partition-independent: any split of one move
    /// removes the same cells, records the same stage and tool per cell,
    /// reports the same volume, and counts the motion once.
    ///
    /// Tile *versions* are deliberately not compared: they count deepening
    /// events per tile so the renderer knows what to re-upload, and a move
    /// drawn over several frames touches a tile more times than the same move
    /// applied in one piece. Material and identity are the state.
    #[test]
    fn a_partitioned_motion_removes_the_same_material_as_one_shot() {
        let stock = Stock {
            x0: -6.,
            y0: -6.,
            x1: 6.,
            y1: 6.,
            thickness_mm: 5.,
        };
        let tools = [
            ToolSpec::Endmill {
                diameter: 2.,
                cutting_length: 12.,
            },
            ToolSpec::Vbit {
                angle: 90.,
                tip: 0.4,
                diameter: 6.,
                height: 3.,
            },
        ];
        // A flat pass, a dive and a diagonal ramp: the sloped moves are where
        // the cutter envelope's stationary-point search could disagree with a
        // partition, so they are the cases worth pinning.
        let motion = |tool: usize, z0: f64, z1: f64| Motion {
            kind: "cut".into(),
            tool,
            stage: 1,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(240.),
            arc: None,
            x0: -3.,
            y0: -1.,
            z0,
            x1: 3.,
            y1: 1.,
            z1,
        };
        for (tool, z0, z1) in [
            (0, -1., -1.),
            (0, 0.5, -2.),
            (1, -2.5, -2.5),
            (1, -0.5, -3.),
        ] {
            let mut one_shot = Field::new(stock, &tools, 0.05).unwrap();
            one_shot.apply(&motion(tool, z0, z1), 0., 1.).unwrap();
            let mut partitioned = Field::new(stock, &tools, 0.05).unwrap();
            let mut from = 0.;
            for step in 1..=10 {
                let to = step as f64 / 10.;
                partitioned.apply(&motion(tool, z0, z1), from, to).unwrap();
                from = to;
            }
            assert_eq!(
                partitioned.cell_bytes(),
                one_shot.cell_bytes(),
                "tool {tool}, z {z0} -> {z1}: ten pieces must equal one shot"
            );
            assert_eq!(
                partitioned.stats.cutting_motions, one_shot.stats.cutting_motions,
                "a motion is counted once however many frames it takes"
            );
            assert!(
                (partitioned.stats.removed_volume_mm3 - one_shot.stats.removed_volume_mm3).abs()
                    < 1e-9
            );
        }
    }

    /// Advancing into a motion is the display's half of the clock: the field it
    /// ends up holding must equal a cold replay to the same position, and a
    /// target behind the current state asks for an exact restore instead.
    #[test]
    fn advancing_into_a_motion_matches_a_cold_replay() {
        let stock = Stock {
            x0: -8.,
            y0: -8.,
            x1: 8.,
            y1: 8.,
            thickness_mm: 4.,
        };
        let tools = [ToolSpec::Endmill {
            diameter: 1.5,
            cutting_length: 12.,
        }];
        let motions: Vec<Motion> = (0..12)
            .map(|i| Motion {
                kind: "cut".into(),
                tool: 0,
                stage: 0,
                interpolation: Interpolation::Feed,
                feed_mm_min: Some(300.),
                arc: None,
                x0: -6. + i as f64,
                y0: -6.,
                z0: -1.,
                x1: -6. + i as f64,
                y1: 6.,
                z1: -1.,
            })
            .collect();
        let pristine = Field::new(stock, &tools, 0.1).unwrap();
        let mut playback = Playback::new(pristine.clone(), usize::MAX);
        let steps = [(0, 0.3), (0, 0.9), (1, 0.0), (3, 0.5), (7, 0.25), (12, 0.0)];
        for (prefix, fraction) in steps {
            assert!(
                playback
                    .advance_to(&motions, prefix, fraction, usize::MAX)
                    .unwrap()
            );
            let mut cold = pristine.clone();
            for motion in &motions[..prefix] {
                cold.apply(motion, 0., 1.).unwrap();
            }
            if fraction > 0. {
                cold.apply(&motions[prefix], 0., fraction).unwrap();
            }
            assert_eq!(
                playback.field.cell_bytes(),
                cold.cell_bytes(),
                "cold replay at ({prefix}, {fraction})"
            );
        }
        // A target behind the current position is refused: the caller restores
        // an exact state and then advances.
        assert!(!playback.advance_to(&motions, 3, 0.5, usize::MAX).unwrap());
        let report = playback.seek_position(&motions, 3, 0.5).unwrap();
        assert_eq!(playback.position, 3);
        assert!((playback.fraction() - 0.5).abs() < 1e-9);
        let mut cold = pristine.clone();
        for motion in &motions[..3] {
            cold.apply(motion, 0., 1.).unwrap();
        }
        cold.apply(&motions[3], 0., 0.5).unwrap();
        assert_eq!(playback.field.cell_bytes(), cold.cell_bytes());
        assert!(report.replayed <= 3);
        // And a forward jump further than the caller allows asks for a restore
        // rather than replaying the whole stream inside one frame.
        assert!(!playback.advance_to(&motions, 12, 0., 2).unwrap());
    }

    #[test]
    fn packed_tile_round_trip_preserves_tiles_and_versions() {
        let stock = Stock {
            x0: -3.,
            y0: -3.,
            x1: 3.,
            y1: 3.,
            thickness_mm: 4.,
        };
        let tools = [ToolSpec::Endmill {
            diameter: 1.5,
            cutting_length: 12.,
        }];
        let mut field = Field::new(stock, &tools, 0.05).unwrap();
        for i in 0..40 {
            let x = -2.5 + i as f64 * 0.12;
            field
                .apply(
                    &Motion {
                        kind: "cut".into(),
                        tool: 0,
                        stage: 0,
                        interpolation: Interpolation::Feed,
                        feed_mm_min: Some(100.),
                        arc: None,
                        x0: x,
                        y0: -2.5,
                        z0: -0.5,
                        x1: x + 0.05,
                        y1: 2.5,
                        z1: -0.5,
                    },
                    0.,
                    1.,
                )
                .unwrap();
        }
        let allocated: Vec<u32> = (0..field.versions.len())
            .filter(|tile| field.tile_allocated(*tile))
            .map(|tile| tile as u32)
            .collect();
        let rebuilt = Field::from_packed(
            stock,
            &tools,
            0.05,
            &field.packed_tile_bytes(),
            &field.versions,
            &allocated,
            field.stats.clone(),
        )
        .unwrap();
        assert_eq!(rebuilt.checksum(), field.checksum());
        assert_eq!(rebuilt.cell_bytes(), field.cell_bytes());
        assert_eq!(rebuilt.allocated_bytes(), field.allocated_bytes());
    }

    /// The cell identity the colour modes resolve against: the stage and tool
    /// that removed the deepest material in a cell, carried through the packed
    /// grid the display uploads.
    #[test]
    fn packed_cells_carry_the_stage_and_tool_that_created_the_surface() {
        let stock = Stock {
            x0: -2.,
            y0: -2.,
            x1: 2.,
            y1: 2.,
            thickness_mm: 6.,
        };
        let tools = [
            ToolSpec::Endmill {
                diameter: 2.,
                cutting_length: 12.,
            },
            ToolSpec::Vbit {
                angle: 90.,
                tip: 0.,
                diameter: 6.,
                height: 3.,
            },
        ];
        let mut field = Field::new(stock, &tools, 0.25).unwrap();
        let cut = |tool: usize, stage: u16, z: f64| Motion {
            kind: "cut".into(),
            tool,
            stage,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(100.),
            arc: None,
            x0: -1.5,
            y0: -1.5,
            z0: z,
            x1: 1.5,
            y1: -1.5,
            z1: z,
        };
        // Stage 3 cuts 1 mm with tool 0, then stage 7 cuts deeper with tool 1.
        field.apply(&cut(0, 3, -1.), 0., 1.).unwrap();
        // Cell (8, 3) sits inside the first pass's band.
        let cell = (8, 3);
        let shallow = field.cell_at(cell.0, cell.1);
        assert_eq!(shallow.1, 3, "the endmill stage owns the first floor");
        assert_eq!(shallow.2, 0);
        field.apply(&cut(1, 7, -2.5), 0., 1.).unwrap();
        let deep = field.cell_at(cell.0, cell.1);
        assert_eq!(deep.1, 7, "the deeper cut takes the floor");
        assert_eq!(deep.2, 1);
        assert!(deep.0 > shallow.0);
        let allocated: Vec<u32> = (0..field.versions.len())
            .filter(|tile| field.tile_allocated(*tile))
            .map(|tile| tile as u32)
            .collect();
        let rebuilt = Field::from_packed(
            stock,
            &tools,
            0.25,
            &field.packed_tile_bytes(),
            &field.versions,
            &allocated,
            field.stats.clone(),
        )
        .unwrap();
        assert_eq!(rebuilt.cell_at(cell.0, cell.1), deep);
        assert_eq!(rebuilt.packed_cells(), field.packed_cells());
        // The packed word the shader reads: depth | stage << 16 | tool << 24.
        let word = field.packed_cells()[cell.1 * field.cols + cell.0];
        assert_eq!((word & 0xffff) as u16, deep.0);
        assert_eq!((word >> 16) as u8, deep.1);
        assert_eq!((word >> 24) as u8, deep.2);
    }

    /// A forward jump that crosses a checkpoint restores it instead of
    /// re-integrating every motion between the playhead and the target. Both
    /// paths must land on the same field.
    #[test]
    fn a_forward_jump_restores_the_nearest_checkpoint_before_replaying() {
        let stock = Stock {
            x0: -10.,
            y0: -10.,
            x1: 10.,
            y1: 10.,
            thickness_mm: 4.,
        };
        let tools = [ToolSpec::Endmill {
            diameter: 1.,
            cutting_length: 12.,
        }];
        let motions: Vec<Motion> = (0..400)
            .map(|i| Motion {
                kind: "cut".into(),
                tool: 0,
                stage: 0,
                interpolation: Interpolation::Feed,
                feed_mm_min: Some(100.),
                arc: None,
                x0: -8. + (i % 20) as f64 * 0.8,
                y0: -8. + (i / 20) as f64 * 0.8,
                z0: -0.5,
                x1: -8. + (i % 20) as f64 * 0.8 + 0.4,
                y1: -8. + (i / 20) as f64 * 0.8,
                z1: -0.5,
            })
            .collect();
        let mut cold = Field::new(stock, &tools, 0.2).unwrap();
        let pristine = cold.clone();
        let mut frames = vec![(0, cold.clone())];
        for (index, motion) in motions.iter().enumerate() {
            cold.apply(motion, 0., 1.).unwrap();
            // Deliberately not every 100: the ladder has to be reached by
            // choosing among the checkpoints that exist.
            if matches!(index + 1, 200 | 300 | 400) {
                frames.push((index + 1, cold.clone()));
            }
        }
        let latest = frames.last().unwrap().1.clone();
        let mut playback = Playback::seed(latest, pristine, frames, usize::MAX);
        assert_eq!(playback.position, 400);

        // Backward to 100: the playhead itself becomes the cheap start.
        let back = playback.seek(&motions, 100).unwrap();
        assert_eq!((back.from, back.replayed), (0, 100));
        // Forward to 350 crosses the 200 and 300 checkpoints. Replaying from
        // the playhead would re-integrate 250 motions; the checkpoint at 300
        // leaves 50.
        let forward = playback.seek(&motions, 350).unwrap();
        assert_eq!(
            (forward.from, forward.replayed),
            (300, 50),
            "the nearest earlier checkpoint is the cheapest exact start"
        );
        assert_eq!(playback.position, 350);

        let mut reference = Field::new(stock, &tools, 0.2).unwrap();
        for motion in &motions[..350] {
            reference.apply(motion, 0., 1.).unwrap();
        }
        assert_eq!(playback.field.checksum(), reference.checksum());
        assert_eq!(playback.field.cell_bytes(), reference.cell_bytes());
    }
}
