//! Narrow display-only port of web/src/sim/engine.ts. Never used for CAM checks.
//! Keep f64 expression order, tile traversal and downward quantization aligned
//! with the reference. The comparison harness executes the original TypeScript.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const TILE: usize = 256;
pub const MAX_CELLS: usize = 64_000_000;
const PLUNGE_AREA_EPS: f64 = 1e-16;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stock {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub thickness_mm: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ToolSpec {
    Endmill {
        #[serde(rename = "diameterMm")]
        diameter: f64,
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
    Endmill { radius: f64 },
    Vbit { tip: f64, slope: f64, radius: f64 },
}
fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.
}
impl ToolSpec {
    pub fn normalize(self) -> Result<Tool, String> {
        match self {
            Self::Endmill { diameter } if positive(diameter) => Ok(Tool::Endmill {
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Motion {
    pub kind: String,
    pub tool: usize,
    pub x0: f64,
    pub y0: f64,
    pub z0: f64,
    pub x1: f64,
    pub y1: f64,
    pub z1: f64,
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
    owner: Vec<u8>,
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
        self.tiles.iter().filter(|t| t.is_some()).count() * TILE * TILE * 3
            + self.versions.len() * 4
    }
    pub fn cell_at(&self, col: usize, row: usize) -> (u16, u8) {
        if col >= self.cols || row >= self.rows {
            return (0, 0);
        }
        let index = ((row & 255) << 8) | (col & 255);
        self.tiles[(row >> 8) * self.tiles_x + (col >> 8)]
            .as_ref()
            .map_or((0, 0), |t| (t.depth[index], t.owner[index]))
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
                    mix(t.owner[j] as u32);
                }
            }
        }
        format!("{hash:x}")
    }
    /// Explicit little-endian depth + owner per cell, for full-cell parity tests.
    pub fn cell_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.cols * self.rows * 3);
        for row in 0..self.rows {
            for col in 0..self.cols {
                let (d, o) = self.cell_at(col, row);
                out.extend_from_slice(&d.to_le_bytes());
                out.push(o);
            }
        }
        out
    }
    /// GPU storage: low 16 bits depth, next 8 bits cutting role.
    pub fn packed_cells(&self) -> Vec<u32> {
        let mut cells = Vec::with_capacity(self.cols * self.rows);
        for row in 0..self.rows {
            for col in 0..self.cols {
                let (depth, owner) = self.cell_at(col, row);
                cells.push(depth as u32 | ((owner as u32) << 16));
            }
        }
        cells
    }
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
        self.stats.applied_motions += 1;
        if !cutting {
            return Ok(());
        }
        self.stats.cutting_motions += 1;
        let start = start.clamp(0., 1.);
        let end = end.clamp(0., 1.);
        if end <= start {
            return Ok(());
        }
        let a = [
            motion.x0 + (motion.x1 - motion.x0) * start,
            motion.y0 + (motion.y1 - motion.y0) * start,
            motion.z0 + (motion.z1 - motion.z0) * start,
        ];
        let b = [
            motion.x0 + (motion.x1 - motion.x0) * end,
            motion.y0 + (motion.y1 - motion.y0) * end,
            motion.z0 + (motion.z1 - motion.z0) * end,
        ];
        if -a[2] <= 0. && -b[2] <= 0. {
            return Ok(());
        }
        let tool = self.tools[motion.tool];
        let (radius, role) = match tool {
            Tool::Endmill { radius } => (radius, 1),
            Tool::Vbit { radius, .. } => (radius, 2),
        };
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
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let dz = b[2] - a[2];
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
                    let py = self.stock.y0 + (row as f64 + 0.5) * self.cell - a[1];
                    for col in c0.max(tc << 8)..=c1.min((tc << 8) | 255) {
                        let px = self.stock.x0 + (col as f64 + 0.5) * self.cell - a[0];
                        let qq = px * px + py * py;
                        let b_raw = -2. * (px * dx + py * dy);
                        let Some((t0, t1)) = coverage(qq - radius * radius, a2, b_raw, inv2a)
                        else {
                            continue;
                        };
                        let depth = match tool {
                            Tool::Endmill { .. } => {
                                let d0 = -a[2] - dz * t0;
                                let d1 = -a[2] - dz * t1;
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
                                    let z = a[2] + dz * t + height;
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
                                owner: vec![0; TILE * TILE],
                            })
                        });
                        let local = ((row & 255) << 8) | (col & 255);
                        let previous = data.depth[local];
                        if level > previous {
                            let data = Arc::make_mut(data);
                            data.depth[local] = level;
                            data.owner[local] = role;
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
    pub position: usize,
    checkpoints: Vec<(usize, Field)>,
    budget: usize,
}
impl Playback {
    pub fn new(field: Field, checkpoint_budget: usize) -> Self {
        Self {
            pristine: field.clone(),
            field,
            position: 0,
            checkpoints: Vec::new(),
            budget: checkpoint_budget,
        }
    }
    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints
            .iter()
            .map(|(_, f)| f.allocated_bytes())
            .sum()
    }
    pub fn seek(&mut self, motions: &[Motion], target: usize) -> Result<(), String> {
        if target > motions.len() {
            return Err("Simulator playhead exceeds retained motions".into());
        }
        if target < self.position {
            let saved = self
                .checkpoints
                .iter()
                .filter(|(p, _)| *p <= target)
                .max_by_key(|(p, _)| *p);
            let (p, f) = saved.map_or((0, &self.pristine), |(p, f)| (*p, f));
            self.field = f.clone();
            self.position = p;
        }
        for motion in &motions[self.position..target] {
            self.field.apply(motion, 0., 1.)?;
            self.position += 1;
        }
        let bytes = self.field.allocated_bytes();
        if bytes <= self.budget && !self.checkpoints.iter().any(|(p, _)| *p == target) {
            while !self.checkpoints.is_empty()
                && (self.checkpoints.len() >= 4 || self.checkpoint_bytes() + bytes > self.budget)
            {
                self.checkpoints.remove(0);
            }
            self.checkpoints.push((target, self.field.clone()));
        }
        Ok(())
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
