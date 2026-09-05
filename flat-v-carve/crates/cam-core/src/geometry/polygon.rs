use super::precision::{MAX_EDGES, cross, inside, intersects, on_segment, twice_area};
use super::{Diagnostic, Grid, GridPoint, Point, Result, Segment};
use clipper2_rust::{
    ClipType, Clipper64, ClipperOffset, EndType, FillRule, JoinType, Paths64, Point64, PolyTree64,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct Ring {
    pub(crate) points: Vec<GridPoint>,
    pub(crate) parent: Option<usize>,
    pub(crate) is_hole: bool,
}
impl Ring {
    pub fn points(&self) -> &[GridPoint] {
        &self.points
    }
    pub fn parent(&self) -> Option<usize> {
        self.parent
    }
    pub fn is_hole(&self) -> bool {
        self.is_hole
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Region {
    pub(crate) grid: Grid,
    pub(crate) rings: Vec<Ring>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    Union,
    Intersection,
    Difference,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindingRule {
    Nonzero,
    Evenodd,
}

impl Region {
    /// Resolve filled SVG contours before passing nonintersecting boundaries downstream.
    /// Intentional contour crossings are allowed; snapping must preserve their relations.
    pub fn from_filled_paths(grid: Grid, input: &[Vec<Point>], rule: WindingRule) -> Result<Self> {
        if input.iter().map(Vec::len).sum::<usize>() > MAX_EDGES {
            return Err(Diagnostic::new(
                "GEOMETRY_LIMIT",
                "at most 4096 flattened vertices are supported",
            ));
        }
        let mut originals = Vec::new();
        let mut paths = Paths64::new();
        let mut diagnostics = Vec::new();
        for raw in input {
            let mut points = raw.clone();
            points.dedup();
            if points.first() == points.last() && points.len() > 1 {
                points.pop();
            }
            if points.len() != raw.len() {
                diagnostics.push(Diagnostic::new(
                    "DUPLICATE_VERTEX_REMOVED",
                    "removed exact repeated/closing path vertices",
                ));
            }
            if points.len() < 3 {
                return Err(Diagnostic::new(
                    "DEGENERATE_RING",
                    "filled subpath needs at least three distinct vertices",
                ));
            }
            let snapped = points
                .iter()
                .map(|&p| grid.quantize(p))
                .collect::<Result<Vec<_>>>()?;
            if snapped
                .iter()
                .zip(snapped.iter().cycle().skip(1))
                .any(|(a, b)| a == b)
            {
                return Err(Diagnostic::new(
                    "QUANTIZATION_COLLAPSE",
                    "snapping collapsed an SVG boundary edge; reduce tolerance or increase scale",
                ));
            }
            let area: f64 = points[1..]
                .windows(2)
                .map(|ab| orient(points[0], ab[0], ab[1]))
                .sum();
            let snapped_area = twice_area(&snapped);
            if area != 0.0 && (snapped_area == 0 || (area < 0.0) != (snapped_area < 0)) {
                return Err(Diagnostic::new(
                    "QUANTIZATION_ORIENTATION",
                    "SVG contour area vanished or reversed during snapping",
                ));
            }
            if points[1..]
                .windows(2)
                .all(|ab| orient(points[0], ab[0], ab[1]) == 0.0)
            {
                return Err(Diagnostic::new(
                    "DEGENERATE_RING",
                    "SVG contour is collinear",
                ));
            }
            paths.push(snapped.iter().map(|p| Point64::new(p.x, p.y)).collect());
            originals.push(points);
        }
        Self::check_snapping(grid, &originals)?;
        let mut engine = Clipper64::new();
        engine.add_subject(&paths);
        let mut tree = PolyTree64::new();
        let rule = match rule {
            WindingRule::Nonzero => FillRule::NonZero,
            WindingRule::Evenodd => FillRule::EvenOdd,
        };
        if !engine.execute_tree(ClipType::Union, rule, &mut tree, &mut vec![])
            || engine.error_code() != 0
        {
            return Err(Diagnostic::new(
                "SVG_FILL_FAILED",
                "could not resolve SVG filled contours",
            ));
        }
        let mut region = Self::from_tree(grid, tree)?;
        region.diagnostics = diagnostics;
        Ok(region)
    }

    /// Check every original/snapped edge relation, including across separate SVG objects.
    pub(crate) fn check_snapping(grid: Grid, input: &[Vec<Point>]) -> Result<()> {
        let originals: Vec<Vec<Point>> = input
            .iter()
            .map(|raw| {
                let mut p = raw.clone();
                p.dedup();
                if p.len() > 1 && p.first() == p.last() {
                    p.pop();
                }
                p
            })
            .collect();
        if originals.iter().map(Vec::len).sum::<usize>() > MAX_EDGES {
            return Err(Diagnostic::new(
                "GEOMETRY_LIMIT",
                "at most 4096 boundary vertices are supported",
            ));
        }
        for ring in &originals {
            let q = ring
                .iter()
                .map(|&p| grid.quantize(p))
                .collect::<Result<Vec<_>>>()?;
            if q.iter().zip(q.iter().cycle().skip(1)).any(|(a, b)| a == b) {
                return Err(Diagnostic::new(
                    "QUANTIZATION_COLLAPSE",
                    "snapping collapsed an SVG boundary edge",
                ));
            }
        }
        let edges: Vec<_> = originals
            .iter()
            .enumerate()
            .flat_map(|(r, ps)| (0..ps.len()).map(move |i| (r, i)))
            .collect();
        for (at, &(r, i)) in edges.iter().enumerate() {
            for &(s, j) in &edges[at + 1..] {
                let a = originals[r][i];
                let b = originals[r][(i + 1) % originals[r].len()];
                let c = originals[s][j];
                let d = originals[s][(j + 1) % originals[s].len()];
                let raw_hit = raw_intersects(a, b, c, d);
                let qa = grid.quantize(a)?;
                let qb = grid.quantize(b)?;
                let qc = grid.quantize(c)?;
                let qd = grid.quantize(d)?;
                let grid_hit = intersects(qa, qb, qc, qd);
                let opposite = |a: f64, b: f64| (a < 0. && b > 0.) || (a > 0. && b < 0.);
                let raw_proper = opposite(orient(a, b, c), orient(a, b, d))
                    && opposite(orient(c, d, a), orient(c, d, b));
                let grid_proper = cross(qa, qb, qc).signum() * cross(qa, qb, qd).signum() < 0
                    && cross(qc, qd, qa).signum() * cross(qc, qd, qb).signum() < 0;
                if raw_hit != grid_hit || raw_proper != grid_proper {
                    return Err(Diagnostic::new(
                        "QUANTIZATION_TOPOLOGY",
                        "snapping changed an SVG edge crossing or contact",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Split filled components with their immediate hole rings, in geometric order.
    pub fn components(&self) -> Vec<Self> {
        let mut result: Vec<_> = self
            .rings
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.is_hole)
            .map(|(i, r)| {
                let mut outer = r.clone();
                outer.parent = None;
                let mut rings = vec![outer];
                for h in self
                    .rings
                    .iter()
                    .filter(|h| h.is_hole && h.parent == Some(i))
                {
                    let mut h = h.clone();
                    h.parent = Some(0);
                    rings.push(h);
                }
                Self {
                    grid: self.grid,
                    rings,
                    diagnostics: vec![],
                }
            })
            .collect();
        result.sort_by_key(|r| r.rings[0].points.iter().min().copied());
        result
    }
    /// Simple closed rings with even-odd nesting, independent of supplied winding.
    /// This is a geometry fixture API, not the future SVG fill-rule importer.
    pub fn from_rings(grid: Grid, input: &[Vec<Point>]) -> Result<Self> {
        if input.iter().map(Vec::len).sum::<usize>() > MAX_EDGES {
            return Err(Diagnostic::new(
                "GEOMETRY_LIMIT",
                "M0 accepts at most 4096 boundary vertices",
            ));
        }
        let mut diagnostics = vec![];
        let mut rings = vec![];
        let mut originals = vec![];
        for (index, raw) in input.iter().enumerate() {
            let mut clean = raw.clone();
            clean.dedup();
            if clean.len() > 1 && clean.first() == clean.last() {
                clean.pop();
            }
            if clean.len() != raw.len() {
                diagnostics.push(Diagnostic::new(
                    "DUPLICATE_VERTEX_REMOVED",
                    format!(
                        "ring {index}: removed {} exact consecutive/closing duplicates",
                        raw.len() - clean.len()
                    ),
                ));
            }
            if clean.len() < 3 {
                return Err(Diagnostic::new(
                    "DEGENERATE_RING",
                    format!("ring {index} has fewer than 3 vertices"),
                ));
            }
            let points: Vec<_> = clean
                .iter()
                .map(|&p| grid.quantize(p))
                .collect::<Result<_>>()?;
            if points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .any(|(a, b)| a == b)
            {
                return Err(Diagnostic::new(
                    "QUANTIZATION_COLLAPSE",
                    format!("ring {index}: snapping collapsed an edge"),
                ));
            }
            originals.push(clean);
            rings.push(Ring {
                points,
                parent: None,
                is_hole: false,
            });
        }
        validate_boundaries(&rings, Some(&originals))?;
        for i in 0..rings.len() {
            let area = twice_area(&rings[i].points);
            if area == 0 {
                return Err(Diagnostic::new(
                    "DEGENERATE_RING",
                    format!("ring {i} has zero snapped area"),
                ));
            }
            let raw = &originals[i];
            let terms: Vec<_> = raw[1..]
                .windows(2)
                .map(|ab| orient(raw[0], ab[0], ab[1]))
                .collect();
            let original_area: f64 = terms.iter().sum();
            // Allow for accumulation over the entire ring, not a fixed number of terms.
            let area_roundoff = 2.0
                * (terms.len() as f64 + 4.0)
                * f64::EPSILON
                * terms.iter().map(|a| a.abs()).sum::<f64>();
            if original_area.abs() <= area_roundoff || (original_area < 0.0) != (area < 0) {
                return Err(Diagnostic::new(
                    "QUANTIZATION_ORIENTATION",
                    format!("ring {i}: area orientation is unresolved or changed after snapping"),
                ));
            }
            let containers: Vec<usize> = (0..rings.len())
                .filter(|&j| i != j && inside(rings[i].points[0], &rings[j].points))
                .collect();
            rings[i].parent = containers
                .iter()
                .min_by_key(|&&j| twice_area(&rings[j].points).abs())
                .copied();
            rings[i].is_hole = containers.len() % 2 == 1;
            if (area < 0) != rings[i].is_hole {
                rings[i].points.reverse();
            }
        }
        Ok(Self {
            grid,
            rings,
            diagnostics,
        })
    }

    pub fn grid(&self) -> Grid {
        self.grid
    }
    pub fn rings(&self) -> &[Ring] {
        &self.rings
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn component_count(&self) -> usize {
        self.rings.iter().filter(|r| !r.is_hole).count()
    }
    pub fn hole_count(&self) -> usize {
        self.rings.iter().filter(|r| r.is_hole).count()
    }
    pub fn area_mm2(&self) -> f64 {
        self.rings
            .iter()
            .map(|r| twice_area(&r.points))
            .sum::<i128>() as f64
            / (2.0 * self.grid.scale().powi(2))
    }
    pub fn rings_mm(&self) -> Vec<Vec<Point>> {
        self.rings
            .iter()
            .map(|r| r.points.iter().map(|&p| self.grid.point(p)).collect())
            .collect()
    }
    pub fn segments(&self) -> Vec<Segment> {
        self.rings
            .iter()
            .flat_map(|r| {
                r.points
                    .iter()
                    .zip(r.points.iter().cycle().skip(1))
                    .map(|(&a, &b)| Segment {
                        start: self.grid.point(a),
                        end: self.grid.point(b),
                    })
            })
            .collect()
    }
    pub fn boundary_distance_mm(&self, p: Point) -> f64 {
        self.segments()
            .iter()
            .map(|s| s.distance(p))
            .fold(f64::INFINITY, f64::min)
    }

    pub fn boolean(&self, op: BooleanOp, other: &Self) -> Result<Self> {
        if self.grid != other.grid {
            return Err(Diagnostic::new(
                "GRID_MISMATCH",
                "Boolean operands must use the same precision policy",
            ));
        }
        let mut engine = Clipper64::new();
        engine.add_subject(&self.paths());
        engine.add_clip(&other.paths());
        let mut tree = PolyTree64::new();
        let kind = match op {
            BooleanOp::Union => ClipType::Union,
            BooleanOp::Intersection => ClipType::Intersection,
            BooleanOp::Difference => ClipType::Difference,
            BooleanOp::Xor => ClipType::Xor,
        };
        if !engine.execute_tree(kind, FillRule::NonZero, &mut tree, &mut vec![])
            || engine.error_code() != 0
        {
            return Err(Diagnostic::new(
                "POLYGON_BACKEND",
                format!("Boolean operation failed: {}", engine.error_code()),
            ));
        }
        Self::from_tree(self.grid, tree)
    }

    /// Disk erosion: round joins are essential around holes and reflex corners.
    /// Positive-area output only; Target::center_set also retains exact-fit lines/points.
    pub fn erode(&self, radius_mm: f64) -> Result<Self> {
        self.offset(radius_mm, false)
    }

    /// Euclidean outward offset; holes shrink. Approximation follows the shared grid policy.
    pub fn dilate(&self, radius_mm: f64) -> Result<Self> {
        self.offset(radius_mm, true)
    }

    fn offset(&self, radius_mm: f64, outward: bool) -> Result<Self> {
        if !radius_mm.is_finite() || radius_mm < 0.0 {
            return Err(Diagnostic::new(
                "INVALID_OFFSET",
                "offset radius must be finite and nonnegative",
            ));
        }
        if radius_mm == 0.0 {
            return Ok(self.clone());
        }
        if radius_mm > self.grid.max_coordinate_mm() {
            return Err(Diagnostic::new(
                "OFFSET_RANGE",
                "offset exceeds the supported grid range",
            ));
        }
        // Prevent unexpectedly enormous round-join allocation. 2*pi*sqrt(r/(8*e))
        // is a conservative small-sagitta estimate for a full circle's segment count.
        if std::f64::consts::TAU * (radius_mm / (8.0 * self.grid.arc_tolerance_mm())).sqrt()
            > 65536.0
        {
            return Err(Diagnostic::new(
                "GEOMETRY_LIMIT",
                "offset arc resolution exceeds M0 resource limit",
            ));
        }
        let mut engine = ClipperOffset::new(
            2.0,
            self.grid.arc_tolerance_mm() * self.grid.scale(),
            true,
            false,
        );
        engine.add_paths(&self.paths(), JoinType::Round, EndType::Polygon);
        let mut tree = PolyTree64::new();
        engine.execute_tree(
            if outward {
                radius_mm * self.grid.scale()
            } else {
                -radius_mm * self.grid.scale()
            },
            &mut tree,
        );
        if engine.error_code() != 0 {
            return Err(Diagnostic::new(
                "POLYGON_BACKEND",
                format!("offset failed: {}", engine.error_code()),
            ));
        }
        let mut output = Self::from_tree(self.grid, tree)?;
        if output.rings.is_empty() && !self.rings.is_empty() {
            output.diagnostics.push(Diagnostic::new("OFFSET_EMPTY_AREA", "erosion has no positive-area output; this does not rule out exact-fit centerlines or points"));
        }
        Ok(output)
    }

    /// Convex hull of quantized constructed points, with exact integer orientation.
    pub(crate) fn convex_hull(grid: Grid, points: &[Point]) -> Result<Self> {
        let mut points = points
            .iter()
            .map(|&p| grid.quantize(p))
            .collect::<Result<Vec<_>>>()?;
        points.sort();
        points.dedup();
        if points.len() < 3 {
            return Err(Diagnostic::new(
                "CONVEX_HULL_COLLAPSE",
                "constructed sweep collapsed on the grid",
            ));
        }
        let half = |iter: Vec<GridPoint>| {
            let mut hull = vec![];
            for p in iter {
                while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
                    hull.pop();
                }
                hull.push(p);
            }
            hull.pop();
            hull
        };
        let mut hull = half(points.clone());
        points.reverse();
        hull.extend(half(points));
        if hull.len() < 3 {
            return Err(Diagnostic::new(
                "CONVEX_HULL_COLLAPSE",
                "constructed sweep has no area",
            ));
        }
        Ok(Self {
            grid,
            rings: vec![Ring {
                points: hull,
                parent: None,
                is_hole: false,
            }],
            diagnostics: vec![],
        })
    }

    fn paths(&self) -> Paths64 {
        self.rings
            .iter()
            .map(|r| r.points.iter().map(|p| Point64::new(p.x, p.y)).collect())
            .collect()
    }

    fn from_tree(grid: Grid, tree: PolyTree64) -> Result<Self> {
        fn visit(tree: &PolyTree64, node: usize, parent: Option<usize>, rings: &mut Vec<Ring>) {
            let index = rings.len();
            let path = &tree.nodes[node];
            rings.push(Ring {
                points: path
                    .polygon()
                    .iter()
                    .map(|p| GridPoint { x: p.x, y: p.y })
                    .collect(),
                parent,
                is_hole: tree.is_hole(node),
            });
            for &child in path.children() {
                visit(tree, child, Some(index), rings);
            }
        }
        let mut rings = vec![];
        for &child in tree.root().children() {
            visit(&tree, child, None, &mut rings);
        }
        for ring in &mut rings {
            for &p in &ring.points {
                grid.quantize(grid.point(p))?;
            }
            if (twice_area(&ring.points) < 0) != ring.is_hole {
                ring.points.reverse();
            }
        }
        validate_boundaries(&rings, None)?;
        Ok(Self {
            grid,
            rings,
            diagnostics: vec![],
        })
    }
}

/// Exact i128 predicates on snapped coordinates, robust predicates on source f64s.
/// Adjacent input edges may share endpoints. Input rings must be simple and mutually
/// disjoint at their boundaries so nesting is unambiguous. Clipper output has an
/// explicit hierarchy and may retain shared endpoints between components.
fn validate_boundaries(rings: &[Ring], originals: Option<&[Vec<Point>]>) -> Result<()> {
    let mut edges: Vec<_> = rings
        .iter()
        .enumerate()
        .flat_map(|(r, ring)| {
            (0..ring.points.len()).map(move |i| {
                (
                    r,
                    i,
                    ring.points[i],
                    ring.points[(i + 1) % ring.points.len()],
                )
            })
        })
        .collect();
    if edges.len() > MAX_EDGES {
        return Err(Diagnostic::new(
            "GEOMETRY_LIMIT",
            "M0 accepts at most 4096 boundary edges",
        ));
    }
    // Output coordinates are already integer and exact. An x-sorted broad phase
    // preserves every contact/intersection check while avoiding all-pairs work
    // for the many disjoint sweep edges created by stock unions.
    if originals.is_none() {
        edges.sort_by_key(|&(_, _, a, b)| a.x.min(b.x));
    }
    for (index, &(r, i, a, b)) in edges.iter().enumerate() {
        for &(s, j, c, d) in &edges[index + 1..] {
            if originals.is_none() {
                if c.x.min(d.x) > a.x.max(b.x) {
                    break;
                }
                if c.y.min(d.y) > a.y.max(b.y) || c.y.max(d.y) < a.y.min(b.y) {
                    continue;
                }
            }
            let shared = a == c || a == d || b == c || b == d;
            let overlaps = cross(a, b, c) == 0
                && cross(a, b, d) == 0
                && ((on_segment(c, a, b) && c != a && c != b)
                    || (on_segment(d, a, b) && d != a && d != b)
                    || (on_segment(a, c, d) && a != c && a != d)
                    || (on_segment(b, c, d) && b != c && b != d)
                    || (a == c && b == d)
                    || (a == d && b == c));
            if let Some(raw) = originals {
                let (oa, ob, oc, od) = (
                    raw[r][i],
                    raw[r][(i + 1) % raw[r].len()],
                    raw[s][j],
                    raw[s][(j + 1) % raw[s].len()],
                );
                let original_shared = oa == oc || oa == od || ob == oc || ob == od;
                if raw_intersects(oa, ob, oc, od)
                    && (!original_shared || raw_overlaps(oa, ob, oc, od))
                {
                    return Err(Diagnostic::new(
                        "INPUT_INTERSECTION",
                        format!("source edges {r}:{i} and {s}:{j} intersect or overlap"),
                    ));
                }
                let adjacent =
                    r == s && ((i + 1) % raw[r].len() == j || (j + 1) % raw[r].len() == i);
                if original_shared && !adjacent {
                    return Err(Diagnostic::new(
                        "NON_SIMPLE_BOUNDARY",
                        format!(
                            "input edges {r}:{i} and {s}:{j} touch outside adjacent endpoints; resolve filled topology first"
                        ),
                    ));
                }
                if shared && !original_shared {
                    return Err(Diagnostic::new(
                        "QUANTIZATION_INTERSECTION",
                        format!("snapping creates contact between {r}:{i} and {s}:{j}"),
                    ));
                }
            }
            if overlaps || (intersects(a, b, c, d) && !shared) {
                return Err(Diagnostic::new(
                    if originals.is_some() {
                        "QUANTIZATION_INTERSECTION"
                    } else {
                        "OUTPUT_INTERSECTION"
                    },
                    format!("edges {r}:{i} and {s}:{j} intersect or overlap"),
                ));
            }
        }
    }
    Ok(())
}

fn orient(a: Point, b: Point, c: Point) -> f64 {
    let to_coord = |p: Point| robust::Coord { x: p.x, y: p.y };
    robust::orient2d(to_coord(a), to_coord(b), to_coord(c))
}
fn raw_on(p: Point, a: Point, b: Point) -> bool {
    orient(a, b, p) == 0.0
        && p.x >= a.x.min(b.x)
        && p.x <= a.x.max(b.x)
        && p.y >= a.y.min(b.y)
        && p.y <= a.y.max(b.y)
}
fn raw_intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let opposite = |x: f64, y: f64| (x > 0.0 && y < 0.0) || (x < 0.0 && y > 0.0);
    (opposite(orient(a, b, c), orient(a, b, d)) && opposite(orient(c, d, a), orient(c, d, b)))
        || raw_on(a, c, d)
        || raw_on(b, c, d)
        || raw_on(c, a, b)
        || raw_on(d, a, b)
}
fn raw_overlaps(a: Point, b: Point, c: Point, d: Point) -> bool {
    orient(a, b, c) == 0.0
        && orient(a, b, d) == 0.0
        && ((raw_on(c, a, b) && c != a && c != b)
            || (raw_on(d, a, b) && d != a && d != b)
            || (raw_on(a, c, d) && a != c && a != d)
            || (raw_on(b, c, d) && b != c && b != d)
            || (a == c && b == d)
            || (a == d && b == c))
}
