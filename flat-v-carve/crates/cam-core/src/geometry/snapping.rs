//! Certify grid snapping, including contraction of consecutive vertices that
//! share one grid point. Nonlocal contacts, crossings and ring loss still fail.
use super::polygon::{orient, raw_intersects};
use super::precision::{MAX_EDGES, cross, intersects};
use super::spatial::{Aabb, SpatialIndex};
use super::{Diagnostic, Grid, GridPoint, Point, Result};

struct Edge {
    a: Point,
    b: Point,
    qa: GridPoint,
    qb: GridPoint,
    groups: [usize; 2],
    vertices: [usize; 2],
}

pub(super) fn check(grid: Grid, input: &[Vec<Point>]) -> Result<()> {
    if input.iter().map(Vec::len).sum::<usize>() > MAX_EDGES {
        return Err(Diagnostic::new(
            "GEOMETRY_LIMIT",
            "at most two million boundary vertices are supported",
        ));
    }
    let mut edges = vec![];
    let mut base = 0;
    let mut vertex_base = 0;
    for raw in input {
        let mut points = raw.clone();
        points.dedup();
        if points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        let q = points
            .iter()
            .map(|&p| grid.quantize(p))
            .collect::<Result<Vec<_>>>()?;
        if q.is_empty() {
            continue;
        }
        let mut groups = vec![base];
        let mut group = base;
        for i in 1..q.len() {
            if q[i] != q[i - 1] {
                group += 1;
            }
            groups.push(group);
        }
        let wrap = group > base && q.first() == q.last();
        let distinct = group - base + 1 - usize::from(wrap);
        if distinct < 3 {
            return Err(Diagnostic::new(
                "QUANTIZATION_COLLAPSE",
                "snapping would erase a closed boundary; refine the grid",
            ));
        }
        if wrap {
            for value in groups.iter_mut().rev().take_while(|v| **v == group) {
                *value = base;
            }
        }
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            edges.push(Edge {
                a: points[i],
                b: points[j],
                qa: q[i],
                qb: q[j],
                groups: [groups[i], groups[j]],
                vertices: [vertex_base + i, vertex_base + j],
            });
        }
        base = group + 1;
        vertex_base += points.len();
    }
    let index = SpatialIndex::new(
        edges
            .iter()
            .map(|e| Aabb::new(e.a, e.b).union(Aabb::new(grid.point(e.qa), grid.point(e.qb))))
            .collect(),
    );
    let mut arrangement_bound = edges.len();
    index.pairs(|i, j| {
        let a = &edges[i];
        let b = &edges[j];
        let raw_hit = raw_intersects(a.a, a.b, b.a, b.b);
        let grid_hit = intersects(a.qa, a.qb, b.qa, b.qb);
        if raw_hit && !a.vertices.iter().any(|v| b.vertices.contains(v)) {
            // Bound potential fill-arrangement growth before asking the polygon
            // backend to allocate intersection vertices. Shared contacts count
            // conservatively even when they will not create new vertices.
            arrangement_bound += 2;
            if arrangement_bound > MAX_EDGES {
                return Err(Diagnostic::new("GEOMETRY_ARRANGEMENT_LIMIT", "potential SVG fill arrangement exceeds two million vertices; partition overlapping artwork"));
            }
        }
        let opposite = |x: f64, y: f64| (x < 0. && y > 0.) || (x > 0. && y < 0.);
        let raw_proper = opposite(orient(a.a, a.b, b.a), orient(a.a, a.b, b.b))
            && opposite(orient(b.a, b.b, a.a), orient(b.a, b.b, a.b));
        let grid_proper = cross(a.qa, a.qb, b.qa).signum() * cross(a.qa, a.qb, b.qb).signum() < 0
            && cross(b.qa, b.qb, a.qa).signum() * cross(b.qa, b.qb, a.qb).signum() < 0;
        // The only new permitted contact is at a vertex reached by contracting
        // a contiguous chain of this same ring. Equal nonadjacent grid points
        // and points belonging to different rings have different group IDs.
        let local_contraction =
            !raw_hit && grid_hit && a.groups.iter().any(|g| b.groups.contains(g));
        if raw_hit && a.groups.iter().any(|g| b.groups.contains(g)) && !a.vertices.iter().any(|v| b.vertices.contains(v)) {
            return Err(Diagnostic::new("QUANTIZATION_TOPOLOGY", "a contracted chain has a nonadjacent source contact; refine the grid to retain this feature"));
        }
        if (raw_hit != grid_hit && !local_contraction) || raw_proper != grid_proper {
            return Err(Diagnostic::new(
                "QUANTIZATION_TOPOLOGY",
                format!(
                    "snapping changed an SVG edge crossing or nonlocal contact (edges {i} and {j})"
                ),
            ));
        }
        Ok(())
    })?;
    Ok(())
}
