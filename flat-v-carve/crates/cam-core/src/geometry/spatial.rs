//! Deterministic bounding-volume hierarchy. Bounds only select candidates;
//! callers retain their independent exact predicates and distance formulas.
use super::{Diagnostic, Point, Result};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Aabb {
    pub min: Point,
    pub max: Point,
}
impl Aabb {
    pub fn new(a: Point, b: Point) -> Self {
        Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }
    pub fn union(self, other: Self) -> Self {
        Self {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }
    fn distance_lower_squared(self, other: Self) -> f64 {
        let x = (self.min.x - other.max.x)
            .max(other.min.x - self.max.x)
            .max(0.);
        let y = (self.min.y - other.max.y)
            .max(other.min.y - self.max.y)
            .max(0.);
        // Comparing squared bounds avoids a square root at every visited box.
        if x == 0. && y == 0. {
            return 0.;
        }
        let magnitude = [self.min, self.max, other.min, other.max]
            .iter()
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(1., f64::max);
        // Subtract the numerical reserve on BOTH axes before squaring. This
        // is at least as conservative as reserving it on the Euclidean norm.
        let reserve = 32. * f64::EPSILON * magnitude;
        let x = (x - reserve).max(0.);
        let y = (y - reserve).max(0.);
        x * x + y * y
    }
}

#[derive(Clone, Debug)]
struct Node {
    bounds: Aabb,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub(crate) struct SpatialIndex {
    boxes: Vec<Aabb>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
impl SpatialIndex {
    pub fn new(boxes: Vec<Aabb>) -> Self {
        let mut index = Self {
            order: (0..boxes.len()).collect(),
            boxes,
            nodes: vec![],
        };
        if !index.order.is_empty() {
            index.build(0, index.order.len());
        }
        index
    }
    /// Nearby boxes occupy contiguous ranges, suitable for balanced unions.
    pub fn into_spatial_order(self) -> Vec<usize> {
        self.order
    }
    fn build(&mut self, start: usize, end: usize) -> usize {
        let bounds = self.order[start..end]
            .iter()
            .map(|&i| self.boxes[i])
            .reduce(Aabb::union)
            .unwrap();
        let id = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let x = bounds.max.x - bounds.min.x >= bounds.max.y - bounds.min.y;
            let middle = (start + end) / 2;
            let boxes = &self.boxes;
            self.order[start..end].select_nth_unstable_by(middle - start, |&a, &b| {
                let center = |i: usize| {
                    if x {
                        boxes[i].min.x + boxes[i].max.x
                    } else {
                        boxes[i].min.y + boxes[i].max.y
                    }
                };
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            let a = self.build(start, middle);
            let b = self.build(middle, end);
            self.nodes[id].children = Some((a, b));
        }
        id
    }
    pub fn visit(&self, bounds: Aabb, mut visitor: impl FnMut(usize) -> Result<()>) -> Result<()> {
        if self.nodes.is_empty() {
            return Ok(());
        }
        // Median splits bound the number of deferred siblings by usize::BITS,
        // even for the largest representable collection. Queries need no heap
        // allocation, including the per-edge visits in boundary validation.
        let mut stack = [0; usize::BITS as usize];
        let mut pending = 1;
        while pending > 0 {
            pending -= 1;
            let id = stack[pending];
            let node = &self.nodes[id];
            if !node.bounds.overlaps(bounds) {
                continue;
            }
            if let Some((a, b)) = node.children {
                stack[pending] = b;
                stack[pending + 1] = a;
                pending += 2;
            } else {
                for &i in &self.order[node.start..node.end] {
                    if self.boxes[i].overlaps(bounds) {
                        visitor(i)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn pairs(&self, mut visitor: impl FnMut(usize, usize) -> Result<()>) -> Result<usize> {
        let mut count = 0;
        for (i, &bounds) in self.boxes.iter().enumerate() {
            self.visit(bounds, |j| {
                if j > i {
                    count += 1;
                    if count > 32_000_000 { return Err(Diagnostic::new("GEOMETRY_PAIR_LIMIT", "more than 32 million overlapping edge pairs; partition or simplify this arrangement")); }
                    visitor(i, j)?;
                }
                Ok(())
            })?;
        }
        Ok(count)
    }
    /// `value` must be bounded below by distance between the query and edge boxes.
    pub fn minimum(&self, bounds: Aabb, mut value: impl FnMut(usize) -> f64) -> f64 {
        if self.nodes.is_empty() {
            return f64::INFINITY;
        }
        let mut best = f64::INFINITY;
        let mut stack = [(0, 0.); usize::BITS as usize];
        let mut pending = 1;
        while pending > 0 {
            pending -= 1;
            let (id, lower) = stack[pending];
            let node = &self.nodes[id];
            if lower > best * best {
                continue;
            }
            if let Some((a, b)) = node.children {
                // Child ordering already computes these immutable bounds.
                // Carry them through the traversal instead of recomputing
                // axis gaps and numerical reserves when nodes are popped.
                let da = self.nodes[a].bounds.distance_lower_squared(bounds);
                let db = self.nodes[b].bounds.distance_lower_squared(bounds);
                if da <= db {
                    stack[pending] = (b, db);
                    stack[pending + 1] = (a, da);
                } else {
                    stack[pending] = (a, da);
                    stack[pending + 1] = (b, db);
                }
                pending += 2;
            } else {
                for &i in &self.order[node.start..node.end] {
                    if self.boxes[i].distance_lower_squared(bounds) <= best * best {
                        best = best.min(value(i));
                    }
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_queries_match_full_scan_at_large_coordinates_and_box_edges() {
        let centers: Vec<_> = (0..5000)
            .map(|i| {
                Point::new(
                    1e9 + (i * 31 % 277) as f64 / 7.,
                    -1e9 + (i * 47 % 321) as f64 / 11.,
                )
            })
            .collect();
        let index = SpatialIndex::new(centers.iter().map(|&p| Aabb::new(p, p)).collect());
        for p in centers.iter().step_by(7).copied().chain([
            Point::new(1e9 - 1., -1e9),
            Point::new(1e9 + 100., -1e9 + 100.),
            Point::new(0., 0.),
        ]) {
            let expected = centers
                .iter()
                .map(|&c| c.distance(p))
                .fold(f64::INFINITY, f64::min);
            assert_eq!(
                index.minimum(Aabb::new(p, p), |i| centers[i].distance(p)),
                expected
            );
        }
        assert!(
            SpatialIndex::new(vec![])
                .minimum(
                    Aabb::new(Point::new(0., 0.), Point::new(0., 0.)),
                    |_| unreachable!()
                )
                .is_infinite()
        );
    }
    #[test]
    fn indexed_pairs_match_brute_force_including_touching_and_vertical_edges() {
        let boxes: Vec<_> = (0..1000)
            .map(|i| {
                Aabb::new(
                    Point::new((i * 31 % 77) as f64, (i * 47 % 121) as f64),
                    Point::new((i * 31 % 77) as f64, (i * 47 % 121) as f64 + 3.),
                )
            })
            .collect();
        let mut actual = vec![];
        SpatialIndex::new(boxes.clone())
            .pairs(|a, b| {
                actual.push((a, b));
                Ok(())
            })
            .unwrap();
        actual.sort_unstable();
        let mut expected = vec![];
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                if boxes[i].overlaps(boxes[j]) {
                    expected.push((i, j));
                }
            }
        }
        assert_eq!(actual, expected);
        let separated: Vec<_> = (0..10000)
            .map(|i| {
                Aabb::new(
                    Point::new(0., i as f64 * 2.),
                    Point::new(0., i as f64 * 2. + 1.),
                )
            })
            .collect();
        assert_eq!(
            SpatialIndex::new(separated).pairs(|_, _| Ok(())).unwrap(),
            0
        );
    }
}
