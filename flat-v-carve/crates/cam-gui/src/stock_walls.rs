//! Interior walls of the simulated stock: the vertical faces a heightfield
//! cannot represent, derived in the display process from the packed field it
//! already holds (plan `stock-display-plan.md` §4).
//!
//! Every wall is one instance the stock pass expands into a quad. A step counts
//! as a wall only when it is deep enough that drawing it is useful: a V-bit
//! flank is a chain of tiny steps, and turning those into a staircase would be
//! both wrong and expensive. Runs merge along the wall, so a facing pass or a
//! straight pocket side is a handful of instances, and the stock's own edge is
//! the same detector's special case rather than a second code path.
//!
//! The budget policy merges first, then raises the threshold, then keeps the
//! deepest steps — and always reports what it left out.
use crate::sim::TILE;

/// Bytes of one wall instance; the shader's `Wall` mirrors this layout.
pub const WALL_BYTES: usize = 32;
/// Most wall instances one display state may build.
pub const WALL_BUDGET_INSTANCES: usize = 131_072;
/// Identity of a wall whose material no cutter has touched. `stage | tool << 8`
/// can never produce it.
pub const NO_CUTTER: u32 = u32::MAX;
/// How steep a step has to be before it is a wall rather than a slope.
///
/// A step height alone cannot tell a vertical face from a V-bit flank: a 90°
/// V-bit's flank descends cell by cell at a 45° gradient, and every one of those
/// steps would pass a `0.25 × cell` height test, which is exactly the staircase
/// the tester reported. A step counts as a wall only when it rises faster than
/// this many millimetres per cell — about 56°, so a V-bit's 45° flank stays a
/// shaded slope while an endmill's vertical face (the whole depth in one cell)
/// is a wall.
pub const MIN_WALL_SLOPE: f64 = 1.5;

/// Step height at which a step counts as a wall, from the style's minimum and
/// the slope rule. `stock.wgsl` uses the same number to decide whether a cell
/// draws as a smooth slope or as a sharp step, so the two must agree.
pub fn effective_threshold(cell_mm: f64, style_mm: f64) -> f64 {
    style_mm.max(cell_mm * MIN_WALL_SLOPE).max(1e-6)
}

/// Deepest removed fraction anywhere in the field: the cut's own depth range,
/// which is what a useful depth ramp has to span. A 2.5 mm carve in an 18 mm
/// stock reaches only 14% of the thickness, so ramping over the thickness would
/// paint it in one colour.
pub fn max_depth(grid: &Grid) -> f64 {
    let mut deepest = 0_f64;
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            deepest = deepest.max(grid.depth(col, row));
        }
    }
    deepest
}

/// One wall quad, in grid-cell units so the shader multiplies by its own cell
/// size. Axis 0 stands at a fixed x and runs along y; axis 1 at a fixed y and
/// runs along x. `top` and `bottom` are fractions of the stock thickness below
/// the stock top, `top` being the higher surface.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Wall {
    pub start: [f32; 2],
    pub length: f32,
    pub axis: u32,
    pub top: f32,
    pub bottom: f32,
    /// `stage | tool << 8`, or [`NO_CUTTER`].
    pub identity: u32,
    pub _pad: f32,
}

/// The walls of one displayed field state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WallSet {
    pub walls: Vec<Wall>,
    /// The threshold the policy actually used; it rises when the budget does.
    pub threshold_mm: f64,
    /// Steps the budget policy would not draw. Reported, never hidden.
    pub dropped: usize,
}

/// The displayed field, in the packed tile-major form the stock pass reads.
pub struct Grid<'a> {
    pub cells: &'a [u8],
    pub cols: usize,
    pub rows: usize,
    pub tiles_x: usize,
    pub cell_mm: f64,
    pub thickness_mm: f64,
    /// Stock extent in cells: the grid usually covers a partial last cell.
    pub width_cells: f64,
    pub length_cells: f64,
}

impl Grid<'_> {
    fn packed(&self, col: usize, row: usize) -> u32 {
        if col >= self.cols || row >= self.rows {
            return 0;
        }
        let tile = (row / TILE) * self.tiles_x + (col / TILE);
        let index = tile * TILE * TILE + (row % TILE) * TILE + (col % TILE);
        let at = index * 4;
        match self.cells.get(at..at + 4) {
            Some(bytes) => u32::from_le_bytes(bytes.try_into().unwrap()),
            None => 0,
        }
    }

    fn depth(&self, col: usize, row: usize) -> f64 {
        (self.packed(col, row) & 0xffff) as f64 / 65535.
    }

    fn identity(&self, col: usize, row: usize) -> u32 {
        let packed = self.packed(col, row);
        if packed & 0xffff == 0 {
            return NO_CUTTER;
        }
        ((packed >> 16) & 255) | (((packed >> 24) & 255) << 8)
    }

    fn height_mm(&self, fraction: f64) -> f64 {
        fraction * self.thickness_mm
    }
}

/// Build the walls of one field state.
pub fn build(grid: &Grid, threshold_mm: f64, budget: usize) -> WallSet {
    // The style's threshold is a minimum: a slope that is merely steep is not a
    // wall, however deep the material beside it is.
    let mut threshold = effective_threshold(grid.cell_mm, threshold_mm);
    for _ in 0..4 {
        let mut walls = Vec::new();
        detect(grid, threshold, &mut walls);
        if walls.len() <= budget {
            return WallSet {
                walls,
                threshold_mm: threshold,
                dropped: 0,
            };
        }
        threshold *= 2.;
    }
    let mut walls = Vec::new();
    detect(grid, threshold, &mut walls);
    let dropped = walls.len().saturating_sub(budget);
    if dropped > 0 {
        // Keep the deepest steps: those are what the eye reads as walls.
        walls.sort_by(|a, b| {
            (b.bottom - b.top)
                .partial_cmp(&(a.bottom - a.top))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        walls.truncate(budget);
    }
    WallSet {
        walls,
        threshold_mm: threshold,
        dropped,
    }
}

/// The section of the stock for a true elevation: one quad per column across
/// the screen, from that column's silhouette down to the stock bottom.
///
/// An edge-on view has no visible surface — every cell quad projects to a line —
/// so the material would read as a wireframe. The silhouette is the highest
/// material anywhere along the view direction at that column, and the quads
/// tile the whole profile, giving a filled section instead. `along_x` says the
/// screen's horizontal axis runs along the stock's X (front and back views) or
/// its Y (left and right views).
pub fn build_section(grid: &Grid, along_x: bool) -> Vec<Wall> {
    let count = if along_x { grid.cols } else { grid.rows };
    let limit = if along_x {
        grid.width_cells
    } else {
        grid.length_cells
    };
    let mut walls = Vec::new();
    let mut index = 0;
    while index < count {
        let Some((top, identity)) = silhouette(grid, index, along_x) else {
            // The whole column is cut through: the section opens there.
            index += 1;
            continue;
        };
        let mut run = index + 1;
        while run < count && silhouette(grid, run, along_x) == Some((top, identity)) {
            run += 1;
        }
        let start_along = index as f64;
        let end_along = (run as f64).min(limit);
        if end_along > start_along {
            // Axis 1 runs along X, axis 0 along Y; the fixed coordinate is
            // arbitrary because the plane is seen edge-on.
            let axis = u32::from(along_x);
            let start = if along_x {
                [start_along, 0.]
            } else {
                [0., start_along]
            };
            push(
                &mut walls,
                start,
                end_along - start_along,
                axis,
                Step {
                    top,
                    bottom: 1.,
                    identity,
                },
            );
        }
        index = run;
    }
    walls
}

/// One column's silhouette: the least-cut cell along the view direction, so the
/// highest material, and the cutter that left that surface. `None` when the
/// column holds no material at all.
fn silhouette(grid: &Grid, index: usize, along_x: bool) -> Option<(f64, u32)> {
    let count = if along_x { grid.rows } else { grid.cols };
    let mut best: Option<(f64, u32)> = None;
    for step in 0..count {
        let (col, row) = if along_x {
            (index, step)
        } else {
            (step, index)
        };
        let depth = grid.depth(col, row);
        if best.is_none_or(|(top, _)| depth < top) {
            best = Some((depth, grid.identity(col, row)));
        }
    }
    best.filter(|(top, _)| grid.height_mm(1. - top) > 1e-9)
}

fn detect(grid: &Grid, threshold: f64, walls: &mut Vec<Wall>) {
    // Interior steps. The wall between two neighbouring cells belongs to the
    // boundary they share, so each step is emitted exactly once.
    for col in 0..grid.cols.saturating_sub(1) {
        let x = (col + 1) as f64;
        let mut row = 0;
        while row < grid.rows {
            let Some(step) = between(grid, threshold, (col, row), (col + 1, row)) else {
                row += 1;
                continue;
            };
            let mut run = row + 1;
            while run < grid.rows
                && between(grid, threshold, (col, run), (col + 1, run)) == Some(step)
            {
                run += 1;
            }
            push(
                walls,
                [x.min(grid.width_cells), row as f64],
                (run - row) as f64,
                0,
                step,
            );
            row = run;
        }
    }
    for row in 0..grid.rows.saturating_sub(1) {
        let y = (row + 1) as f64;
        let mut col = 0;
        while col < grid.cols {
            let Some(step) = between(grid, threshold, (col, row), (col, row + 1)) else {
                col += 1;
                continue;
            };
            let mut run = col + 1;
            while run < grid.cols
                && between(grid, threshold, (run, row), (run, row + 1)) == Some(step)
            {
                run += 1;
            }
            push(
                walls,
                [col as f64, y.min(grid.length_cells)],
                (run - col) as f64,
                1,
                step,
            );
            col = run;
        }
    }
    // The stock's own edge: the neighbour outside the grid is air, so the whole
    // remaining thickness at that boundary cell is exposed. A cell cut through
    // has nothing left to stand there and leaves a real gap.
    ring(grid, 0, walls);
    ring(grid, 1, walls);
    ring(grid, 2, walls);
    ring(grid, 3, walls);
}

/// The step between two neighbouring cells: `(top, bottom, identity)` of the
/// wall, where the identity is the cutter that removed the lower side.
#[derive(Clone, Copy, PartialEq)]
struct Step {
    top: f64,
    bottom: f64,
    identity: u32,
}

fn between(grid: &Grid, threshold: f64, a: (usize, usize), b: (usize, usize)) -> Option<Step> {
    let (da, db) = (grid.depth(a.0, a.1), grid.depth(b.0, b.1));
    if grid.height_mm((da - db).abs()) <= threshold {
        return None;
    }
    let lower = if da >= db { a } else { b };
    Some(Step {
        top: da.min(db),
        bottom: da.max(db),
        identity: grid.identity(lower.0, lower.1),
    })
}

/// The exposed face of the boundary cells along one stock edge. `side` is 0 low
/// X, 1 low Y, 2 high X, 3 high Y.
fn ring(grid: &Grid, side: u32, walls: &mut Vec<Wall>) {
    // Side 0 stands at x = 0 and runs along y, side 1 at y = 0 running along x,
    // side 2 at x = width running along y and side 3 at y = length running
    // along x.
    let (axis, fixed, count, limit) = match side {
        0 => (0u32, 0., grid.rows, grid.length_cells),
        1 => (1u32, 0., grid.cols, grid.width_cells),
        2 => (0u32, grid.width_cells, grid.rows, grid.length_cells),
        _ => (1u32, grid.length_cells, grid.cols, grid.width_cells),
    };
    let cell = |index: usize| -> (usize, usize) {
        match side {
            0 => (0, index),
            1 => (index, 0),
            2 => (grid.cols.saturating_sub(1), index),
            _ => (index, grid.rows.saturating_sub(1)),
        }
    };
    let mut index = 0;
    while index < count {
        let (col, row) = cell(index);
        let top = grid.depth(col, row);
        // Nothing is left of a cell cut through, so the edge opens there.
        if grid.height_mm(1. - top) <= 1e-9 {
            index += 1;
            continue;
        }
        let identity = grid.identity(col, row);
        let mut run = index + 1;
        while run < count {
            let (c, r) = cell(run);
            if (grid.depth(c, r) - top).abs() > 1e-12 || grid.identity(c, r) != identity {
                break;
            }
            run += 1;
        }
        let start_along = index as f64;
        let end_along = (run as f64).min(limit);
        if end_along > start_along {
            let start = if axis == 0 {
                [fixed, start_along]
            } else {
                [start_along, fixed]
            };
            push(
                walls,
                start,
                end_along - start_along,
                axis,
                Step {
                    top,
                    bottom: 1.,
                    identity,
                },
            );
        }
        index = run;
    }
}

fn push(walls: &mut Vec<Wall>, start: [f64; 2], length: f64, axis: u32, step: Step) {
    if length <= 0. {
        return;
    }
    walls.push(Wall {
        start: [start[0] as f32, start[1] as f32],
        length: length as f32,
        axis,
        top: step.top as f32,
        bottom: step.bottom as f32,
        identity: step.identity,
        _pad: 0.,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack a single-tile field: `depth` returns the removed fraction, `stage`
    /// the stage index (tool 0). Cells outside the requested grid stay empty.
    fn packed(
        cols: usize,
        rows: usize,
        depth: impl Fn(usize, usize) -> f64,
        stage: impl Fn(usize, usize) -> u8,
    ) -> Vec<u8> {
        let mut bytes = vec![0u8; TILE * TILE * 4];
        for row in 0..rows {
            for col in 0..cols {
                let value = (depth(col, row).clamp(0., 1.) * 65535.).round() as u32;
                let word = (value & 0xffff) | ((stage(col, row) as u32) << 16);
                let at = (row * TILE + col) * 4;
                bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        bytes
    }

    fn grid(cells: &[u8], cols: usize, rows: usize) -> Grid<'_> {
        Grid {
            cells,
            cols,
            rows,
            tiles_x: 1,
            cell_mm: 1.,
            thickness_mm: 10.,
            width_cells: cols as f64,
            length_cells: rows as f64,
        }
    }

    const THRESHOLD: f64 = 0.25;

    #[test]
    fn the_layout_is_the_shaders() {
        assert_eq!(std::mem::size_of::<Wall>(), WALL_BYTES);
        assert_eq!(std::mem::offset_of!(Wall, start), 0);
        assert_eq!(std::mem::offset_of!(Wall, length), 8);
        assert_eq!(std::mem::offset_of!(Wall, axis), 12);
        assert_eq!(std::mem::offset_of!(Wall, top), 16);
        assert_eq!(std::mem::offset_of!(Wall, bottom), 20);
        assert_eq!(std::mem::offset_of!(Wall, identity), 24);
    }

    /// A faced plate: no interior steps at all, and the four edges merge into
    /// one run each, from the faced surface down to the bottom.
    #[test]
    fn a_facing_pass_leaves_only_the_four_edges() {
        let cells = packed(4, 3, |_, _| 0.4, |_, _| 3);
        let set = build(&grid(&cells, 4, 3), THRESHOLD, WALL_BUDGET_INSTANCES);
        assert_eq!(set.walls.len(), 4, "{:?}", set.walls);
        assert_eq!(set.dropped, 0);
        for wall in &set.walls {
            assert!((wall.top - 0.4).abs() < 1e-4, "{wall:?}");
            assert!((wall.bottom - 1.).abs() < 1e-4, "{wall:?}");
            assert_eq!(wall.identity, 3, "the facing stage owns the edge");
        }
        // The north edge runs the full width and stands at the stock's limit.
        let north = set
            .walls
            .iter()
            .find(|w| w.axis == 1 && w.start[1] > 1.)
            .unwrap();
        assert!((north.length - 4.).abs() < 1e-4, "{north:?}");
        assert!((north.start[1] - 3.).abs() < 1e-4);
    }

    /// A rectangular pocket: four walls, one run each, owned by the operation
    /// that removed the pocket's material.
    #[test]
    fn a_pocket_gets_four_walls_from_the_operation_that_cut_it() {
        let cells = packed(
            5,
            5,
            |col, row| {
                if (1..4).contains(&col) && (1..4).contains(&row) {
                    0.3
                } else {
                    0.
                }
            },
            |col, row| u8::from((1..4).contains(&col) && (1..4).contains(&row)) * 7,
        );
        let set = build(&grid(&cells, 5, 5), THRESHOLD, WALL_BUDGET_INSTANCES);
        let pocket: Vec<_> = set.walls.iter().filter(|w| w.identity == 7).collect();
        assert_eq!(pocket.len(), 4, "{:?}", set.walls);
        for wall in pocket {
            assert!(
                (wall.top - 0.).abs() < 1e-6,
                "the surface above is untouched"
            );
            assert!((wall.bottom - 0.3).abs() < 1e-4, "{wall:?}");
            assert!((wall.length - 3.).abs() < 1e-4, "runs merge: {wall:?}");
        }
        // The untouched edge is still there, owned by no cutter.
        assert_eq!(
            set.walls.iter().filter(|w| w.identity == NO_CUTTER).count(),
            4
        );
    }

    /// A cell cut through opens a real gap in the stock's edge.
    #[test]
    fn a_through_cut_leaves_a_gap_in_the_edge() {
        let cells = packed(4, 1, |col, _| if col == 1 { 1. } else { 0. }, |_, _| 1);
        let set = build(&grid(&cells, 4, 1), THRESHOLD, WALL_BUDGET_INSTANCES);
        let south: Vec<_> = set
            .walls
            .iter()
            .filter(|w| w.axis == 1 && w.start[1] == 0.)
            .collect();
        assert_eq!(
            south.len(),
            2,
            "a run either side of the gap: {:?}",
            set.walls
        );
        assert!(
            south.iter().all(|w| w.start[0] != 1.),
            "nothing stands above the cut-through cell"
        );
    }

    /// A V-bit flank is a chain of shallow steps. Below the threshold they stay
    /// a shaded slope instead of becoming a staircase.
    #[test]
    fn a_slope_below_the_threshold_is_not_a_staircase() {
        // A 45° flank at 1 mm cells: every step is 1 mm, four times the style's
        // 0.25 mm threshold, so only the slope rule keeps it off the walls.
        let cells = packed(6, 2, |col, _| col as f64 * 0.1, |_, _| 2);
        let set = build(&grid(&cells, 6, 2), THRESHOLD, WALL_BUDGET_INSTANCES);
        // Only the stock's own edge remains: every wall stands on the boundary,
        // where the cell beside it was cut by the slope's operation.
        assert!(
            set.walls.iter().all(|wall| {
                let fixed = if wall.axis == 0 {
                    wall.start[0]
                } else {
                    wall.start[1]
                };
                fixed == 0. || fixed == 6. || fixed == 2.
            }),
            "no interior staircase, only edges: {:?}",
            set.walls
        );
    }

    /// The budget policy raises the threshold and reports what it dropped: a
    /// pathological checkerboard cannot flood the pass or pass silently.
    #[test]
    fn the_budget_policy_reports_what_it_leaves_out() {
        let cells = packed(
            20,
            20,
            |col, row| if (col + row) % 2 == 0 { 0.5 } else { 0. },
            |_, _| 4,
        );
        let budget = 64;
        let set = build(&grid(&cells, 20, 20), THRESHOLD, budget);
        assert!(set.walls.len() <= budget);
        assert!(
            set.threshold_mm > THRESHOLD || set.dropped > 0,
            "the policy has to leave a trace: {:?}",
            (set.threshold_mm, set.dropped)
        );
    }

    /// A faced plate's section is one run: the silhouette is the faced surface
    /// across the whole width, and it reaches the bottom.
    #[test]
    fn a_faced_plate_sections_as_one_run() {
        let cells = packed(5, 3, |_, _| 0.4, |_, _| 3);
        let grid = grid(&cells, 5, 3);
        let section = build_section(&grid, true);
        assert_eq!(section.len(), 1, "{section:?}");
        assert_eq!(section[0].axis, 1, "the front view runs along X");
        assert!((section[0].length - 5.).abs() < 1e-6);
        assert!((section[0].top - 0.4).abs() < 1e-4);
        assert!((section[0].bottom - 1.).abs() < 1e-4);
        assert_eq!(section[0].identity, 3);
    }

    /// The silhouette is the *highest* material along the view direction, and a
    /// column of different heights becomes its own run — so a pocket shows as a
    /// step in the section, not as a hole.
    #[test]
    fn a_section_follows_the_highest_material_in_each_column() {
        // Front view: the screen runs along X, so each column is a strip of the
        // stock at one X and the silhouette scans Y.
        let cells = packed(
            2,
            3,
            |col, row| if col == 0 && row == 1 { 0.6 } else { 0. },
            |_, _| 5,
        );
        let view = grid(&cells, 2, 3);
        let section = build_section(&view, true);
        // Both columns keep their untouched surface: the cut is inside the
        // material, so the section (the outline) is unchanged.
        assert_eq!(section.len(), 1, "{section:?}");
        assert!((section[0].top - 0.).abs() < 1e-6);
        // Cut the whole of column 0's material away and the section opens.
        let cells = packed(2, 3, |col, _| if col == 0 { 1. } else { 0. }, |_, _| 5);
        let section = build_section(&grid(&cells, 2, 3), true);
        assert_eq!(section.len(), 1, "{section:?}");
        assert!((section[0].start[0] - 1.).abs() < 1e-6, "half is gone");
        assert!((section[0].length - 1.).abs() < 1e-6);
        // And a half-depth cut over the whole column shows up as its own run.
        let cells = packed(2, 3, |col, _| if col == 0 { 0.25 } else { 0. }, |_, _| 5);
        let section = build_section(&grid(&cells, 2, 3), true);
        assert_eq!(section.len(), 2, "{section:?}");
        assert!((section[0].top - 0.25).abs() < 1e-4);
        assert!((section[1].top - 0.).abs() < 1e-6);
    }

    /// Every wall stands where it claims: an edge wall on a stock boundary, an
    /// interior wall on a cell boundary inside the rectangle. This is the check
    /// the misplaced-wall review build needed, applied to the builder that owns
    /// the geometry now.
    #[test]
    fn every_wall_instance_stands_on_the_boundary_it_names() {
        let cells = packed(
            5,
            4,
            |col, row| {
                if (1..4).contains(&col) && (1..3).contains(&row) {
                    0.5
                } else {
                    0.
                }
            },
            |_, _| 6,
        );
        let view = grid(&cells, 5, 4);
        let set = build(&view, THRESHOLD, WALL_BUDGET_INSTANCES);
        assert!(!set.walls.is_empty());
        for wall in &set.walls {
            let fixed = if wall.axis == 0 {
                wall.start[0]
            } else {
                wall.start[1]
            };
            let along = if wall.axis == 0 {
                wall.start[1]
            } else {
                wall.start[0]
            };
            // Axis 0 stands at a fixed x (bounded by the width) and runs along
            // y (bounded by the length); axis 1 the other way round.
            let fixed_limit = if wall.axis == 0 { 5. } else { 4. };
            let along_limit = if wall.axis == 0 { 4. } else { 5. };
            let on_stock_edge = fixed == 0. || fixed == fixed_limit;
            let on_cell_boundary = fixed.fract() == 0. && fixed > 0. && fixed < fixed_limit;
            assert!(
                on_stock_edge || on_cell_boundary,
                "wall {wall:?} stands at {fixed} on neither an edge nor a cell boundary"
            );
            assert!(along.fract() == 0., "wall {wall:?} starts mid-cell");
            assert!(
                wall.length > 0.
                    && along >= 0.
                    && along + wall.length <= along_limit + 1e-4
                    && wall.start[0] >= 0.
                    && wall.start[0] <= 5.
                    && wall.start[1] >= 0.
                    && wall.start[1] <= 4.,
                "wall {wall:?} leaves the stock"
            );
            assert!(wall.top < wall.bottom, "wall {wall:?} has no height");
        }
    }
}
