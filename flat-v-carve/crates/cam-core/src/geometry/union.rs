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
        if self.vertices >= 32768 || self.pending.len() >= 256 {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut region = Region::union_all(self.grid, &self.pending.iter().collect::<Vec<_>>())?;
        self.pending.clear();
        self.vertices = 0;
        let mut level = 0;
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
