//! Deterministic nearest-entry routing. This only orders paths; callers check links.
use crate::geometry::{
    Point,
    spatial::{Aabb, SpatialIndex},
};

/// Entries are (path index, vertex index, entry XY, exit XY). Removing a path removes every
/// entry, including all possible starts of a closed contour. Rebuild after half
/// the entries are consumed so spent paths cannot dominate nearest searches.
pub(crate) fn nearest_order(
    entries: Vec<(usize, usize, Point, Point)>,
    count: usize,
    start: Point,
) -> Vec<(usize, usize)> {
    let mut live = vec![true; count];
    let mut sizes = vec![0; count];
    for &(path, _, _, _) in &entries {
        sizes[path] += 1;
    }
    let mut entries = entries;
    let mut index = SpatialIndex::new(entries.iter().map(|e| Aabb::new(e.2, e.2)).collect());
    let mut remaining = entries.len();
    let mut position = start;
    let mut result = Vec::with_capacity(count);
    while remaining > 0 {
        let mut best: Option<(f64, usize, usize, Point)> = None;
        index.minimum(Aabb::new(position, position), |i| {
            let (path, vertex, p, end) = entries[i];
            if !live[path] {
                return f64::INFINITY;
            }
            let distance = position.distance(p);
            if best.as_ref().is_none_or(|b| {
                distance
                    .total_cmp(&b.0)
                    .then(path.cmp(&b.1))
                    .then(vertex.cmp(&b.2))
                    .is_lt()
            }) {
                best = Some((distance, path, vertex, end));
            }
            distance
        });
        let (_, path, vertex, end) = best.expect("a live entry remains");
        result.push((path, vertex));
        position = end;
        live[path] = false;
        remaining -= sizes[path];
        if remaining > 0 && remaining * 2 <= entries.len() {
            entries.retain(|e| live[e.0]);
            index = SpatialIndex::new(entries.iter().map(|e| Aabb::new(e.2, e.2)).collect());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_routes_match_exhaustive_nearest_entries_and_stable_ties() {
        let entries: Vec<_> = (0..2048)
            .flat_map(|i| {
                let a = Point::new((i * 31 % 127) as f64, (i * 47 % 193) as f64);
                let b = Point::new(a.x + 0.25, a.y + 0.5);
                [(i, 0, a, b), (i, 1, b, a)]
            })
            .collect();
        let start = Point::new(0., 0.);
        let actual = nearest_order(entries.clone(), 2048, start);
        let mut pending = entries;
        let mut previous = start;
        let mut expected = vec![];
        while !pending.is_empty() {
            let &(path, vertex, _, end) = pending
                .iter()
                .min_by(|a, b| {
                    previous
                        .distance(a.2)
                        .total_cmp(&previous.distance(b.2))
                        .then(a.0.cmp(&b.0))
                        .then(a.1.cmp(&b.1))
                })
                .unwrap();
            expected.push((path, vertex));
            pending.retain(|e| e.0 != path);
            previous = end;
        }
        assert_eq!(actual, expected);
        assert!(nearest_order(vec![], 0, start).is_empty());
    }
}
