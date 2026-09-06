//! Bounded batches and balanced merges for streamed cutter footprints.
use super::{Grid, Region, Result};

pub(crate) struct UnionAccumulator {
    grid: Grid,
    pending: Vec<Region>,
    vertices: usize,
    levels: Vec<Option<Region>>,
}
impl UnionAccumulator {
    pub fn new(grid: Grid) -> Self {
        Self {
            grid,
            pending: vec![],
            vertices: 0,
            levels: vec![],
        }
    }
    pub fn push(&mut self, region: Region) -> Result<()> {
        self.vertices += region
            .rings()
            .iter()
            .map(|r| r.points().len())
            .sum::<usize>();
        self.pending.push(region);
        // Adjacent fine toolpath capsules overlap almost completely. Large
        // batches create dense intersection arrangements before the union
        // discards their interiors; small batches collapse these early.
        if self.vertices >= 32768 || self.pending.len() >= 8 {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let region = Region::union_all(self.grid, &self.pending.iter().collect::<Vec<_>>())?;
        self.pending.clear();
        self.vertices = 0;
        self.merge_level(region, 0)
    }
    fn merge_level(&mut self, mut region: Region, mut level: usize) -> Result<()> {
        loop {
            if level == self.levels.len() {
                self.levels.push(Some(region));
                break;
            }
            if let Some(previous) = self.levels[level].take() {
                region = Region::union_all(self.grid, &[&previous, &region])?;
                level += 1;
            } else {
                self.levels[level] = Some(region);
                break;
            }
        }
        Ok(())
    }
    /// Join independently constructed, batch-aligned subtrees without finishing
    /// or re-normalizing them. This retains the sequential Boolean tree exactly.
    pub fn append_aligned(&mut self, other: Self) -> Result<()> {
        if self.grid != other.grid || !self.pending.is_empty() {
            return Err(super::Diagnostic::new(
                "UNION_ALIGNMENT",
                "stock union subtrees must share a grid and a completed batch boundary",
            ));
        }
        for (level, region) in other.levels.into_iter().enumerate().rev() {
            let Some(region) = region else { continue };
            if self.levels.iter().take(level).any(Option::is_some) {
                return Err(super::Diagnostic::new(
                    "UNION_ALIGNMENT",
                    "stock union subtree is not aligned with the sequential merge tree",
                ));
            }
            while self.levels.len() < level {
                self.levels.push(None);
            }
            self.merge_level(region, level)?;
        }
        for region in other.pending {
            self.push(region)?;
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<Region> {
        self.flush()?;
        Region::union_all(
            self.grid,
            &self
                .levels
                .iter()
                .filter_map(Option::as_ref)
                .collect::<Vec<_>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;
    #[test]
    fn misaligned_subtrees_are_rejected() {
        let grid = Grid::new(0.001, 100.).unwrap();
        let region = Region::from_rings(
            grid,
            &[vec![
                Point::new(0., 0.),
                Point::new(1., 0.),
                Point::new(1., 1.),
                Point::new(0., 1.),
            ]],
        )
        .unwrap();
        let build = |n| {
            let mut a = UnionAccumulator::new(grid);
            for _ in 0..n {
                a.push(region.clone()).unwrap();
            }
            a
        };
        assert_eq!(
            build(1).append_aligned(build(8)).unwrap_err().code,
            "UNION_ALIGNMENT"
        );
        assert_eq!(
            build(8).append_aligned(build(64)).unwrap_err().code,
            "UNION_ALIGNMENT"
        );
    }
    #[test]
    fn batched_footprints_preserve_analytic_union_across_merge_levels() {
        let grid = Grid::new(0.001, 2000.).unwrap();
        let mut union = UnionAccumulator::new(grid);
        for i in 0..1300 {
            let x = i as f64;
            union
                .push(
                    Region::from_rings(
                        grid,
                        &[vec![
                            Point::new(x, 0.),
                            Point::new(x + 2., 0.),
                            Point::new(x + 2., 3.),
                            Point::new(x, 3.),
                        ]],
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let result = union.finish().unwrap();
        assert_eq!(result.component_count(), 1);
        assert_eq!(result.hole_count(), 0);
        assert_eq!(result.area_mm2(), 3903.);
    }
}
