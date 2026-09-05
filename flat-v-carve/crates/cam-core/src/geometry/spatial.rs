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
    fn distance_lower(self, other: Self) -> f64 {
        let x = (self.min.x - other.max.x)
            .max(other.min.x - self.max.x)
            .max(0.);
        let y = (self.min.y - other.max.y)
            .max(other.min.y - self.max.y)
            .max(0.);
        let magnitude = [self.min, self.max, other.min, other.max]
            .iter()
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(1., f64::max);
        (x.hypot(y) - 32. * f64::EPSILON * magnitude).max(0.)
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
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if !node.bounds.overlaps(bounds) {
                continue;
            }
            if let Some((a, b)) = node.children {
                stack.push(b);
                stack.push(a);
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
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if node.bounds.distance_lower(bounds) > best {
                continue;
            }
            if let Some((a, b)) = node.children {
                if self.nodes[a].bounds.distance_lower(bounds)
                    <= self.nodes[b].bounds.distance_lower(bounds)
                {
                    stack.push(b);
                    stack.push(a);
                } else {
                    stack.push(a);
                    stack.push(b);
                }
            } else {
                for &i in &self.order[node.start..node.end] {
                    if self.boxes[i].distance_lower(bounds) <= best {
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
