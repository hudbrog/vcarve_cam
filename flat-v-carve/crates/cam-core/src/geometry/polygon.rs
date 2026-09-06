use super::precision::{MAX_EDGES, cross, inside, intersects, on_segment, twice_area};
use super::spatial::{Aabb, SpatialIndex};
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
    /// Reserve extra integer precision for constructed offsets/sweeps while
    /// preserving every normalized boundary coordinate exactly. Power-of-two
    /// scaling preserves both integer topology and the resulting f64 points.
    pub(crate) fn refine_construction_grid(mut self) -> Self {
        const FACTOR: i64 = 16;
        let max_tick = self
            .rings
            .iter()
            .flat_map(|r| &r.points)
            .map(|p| p.x.abs().max(p.y.abs()))
            .max()
            .unwrap_or(0);
        if max_tick > super::precision::MAX_GRID_COORD / FACTOR
            || self.grid.scale() > 1e12 / FACTOR as f64
        {
            return self;
        }
        let Ok(grid) = Grid::with_scale(
            self.grid.tolerance_mm(),
            max_tick as f64 / self.grid.scale(),
            self.grid.scale() * FACTOR as f64,
        ) else {
            return self;
        };
        for p in self.rings.iter_mut().flat_map(|r| &mut r.points) {
            p.x *= FACTOR;
            p.y *= FACTOR;
        }
        self.grid = grid;
        self
    }
    /// Resolve filled SVG contours before passing nonintersecting boundaries downstream.
    /// Intentional contour crossings are allowed; snapping must preserve their relations.
    pub fn from_filled_paths(grid: Grid, input: &[Vec<Point>], rule: WindingRule) -> Result<Self> {
        if input.iter().map(Vec::len).sum::<usize>() > MAX_EDGES {
            return Err(Diagnostic::new(
                "GEOMETRY_LIMIT",
                "at most two million flattened vertices are supported",
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
            let mut snapped = points
                .iter()
                .map(|&p| grid.quantize(p))
                .collect::<Result<Vec<_>>>()?;
            snapped.dedup();
            if snapped.len() > 1 && snapped.first() == snapped.last() {
                snapped.pop();
            }
            if snapped.len() < 3 {
                return Err(Diagnostic::new(
                    "QUANTIZATION_COLLAPSE",
                    "snapping would erase a closed boundary; refine the grid",
                ));
            }
            if snapped.len() < points.len() {
                diagnostics.push(Diagnostic::new("SNAPPED_VERTEX_COALESCED", format!("coalesced {} consecutive vertices within the reported grid-snap bound; local topology checked", points.len() - snapped.len())));
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

    /// Check topology with a spatial broad phase, permitting only local grid contractions.
    pub(crate) fn check_snapping(grid: Grid, input: &[Vec<Point>]) -> Result<()> {
        super::snapping::check(grid, input)
    }
    /// Split filled components with their immediate hole rings, in geometric order.
    pub fn components(&self) -> Vec<Self> {
        let mut holes = vec![vec![]; self.rings.len()];
        for (index, ring) in self.rings.iter().enumerate() {
            if ring.is_hole
                && let Some(parent) = ring.parent
            {
                holes[parent].push(index);
            }
        }
        let mut result: Vec<_> = self
            .rings
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.is_hole)
            .map(|(i, r)| {
                let mut outer = r.clone();
                outer.parent = None;
                let mut rings = vec![outer];
                for &index in &holes[i] {
                    let mut h = self.rings[index].clone();
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
                "at most two million boundary vertices are supported",
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
        validate_boundaries(grid, &rings, Some(&originals))?;
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

    /// Union all selected regions in one backend operation, without repeatedly
    /// rebuilding an ever larger prefix of the selection.
    pub fn union_all(grid: Grid, regions: &[&Self]) -> Result<Self> {
        let mut engine = Clipper64::new();
        let mut count = 0;
        for region in regions {
            if region.grid != grid {
                return Err(Diagnostic::new(
                    "GRID_MISMATCH",
                    "union operands must share a grid",
                ));
            }
            count += region.rings.iter().map(|r| r.points.len()).sum::<usize>();
            if count > MAX_EDGES {
                return Err(Diagnostic::new(
                    "GEOMETRY_LIMIT",
                    "union input exceeds two million vertices",
                ));
            }
            engine.add_subject(&region.paths());
        }
        if count == 0 {
            return Self::from_rings(grid, &[]);
        }
        let mut tree = PolyTree64::new();
        if !engine.execute_tree(ClipType::Union, FillRule::NonZero, &mut tree, &mut vec![])
            || engine.error_code() != 0
        {
            return Err(Diagnostic::new("POLYGON_BACKEND", "selection union failed"));
        }
        Self::from_tree(grid, tree)
    }

    /// Clip a constructed open/closed polyline to filled area (including
    /// holes). Endpoints are snapped to the region grid; this is a planning
    /// operation, not a stock-clearance proof. No connector is added across a
    /// clipped-out interval. Output ordering and direction are canonical.
    pub fn clip_polyline(&self, points: &[Point]) -> Result<Vec<Vec<Point>>> {
        self.clip_polylines(&[points])
    }

    /// Clip independent contours together, preparing the shared clipping
    /// region once. The backend keeps open subjects disconnected; canonicalize
    /// all returned paths just as for a single subject.
    pub(crate) fn clip_polylines(&self, polylines: &[&[Point]]) -> Result<Vec<Vec<Point>>> {
        let mut inputs = vec![];
        for points in polylines {
            if points.len() > 65_536 {
                return Err(Diagnostic::new(
                    "GEOMETRY_LIMIT",
                    "polyline exceeds 65536 vertices",
                ));
            }
            let mut input = points
                .iter()
                .map(|&p| self.grid.quantize(p))
                .collect::<Result<Vec<_>>>()?;
            input.dedup();
            if input.len() >= 2 {
                inputs.push(input.iter().map(|p| Point64::new(p.x, p.y)).collect());
            }
        }
        if inputs.is_empty() || self.rings.is_empty() {
            return Ok(vec![]);
        }
        let mut engine = Clipper64::new();
        engine.add_open_subject(&inputs);
        engine.add_clip(&self.paths());
        let mut open = vec![];
        if !engine.execute(
            ClipType::Intersection,
            FillRule::NonZero,
            &mut vec![],
            Some(&mut open),
        ) || engine.error_code() != 0
        {
            return Err(Diagnostic::new(
                "POLYLINE_BACKEND",
                "open path clipping failed",
            ));
        }
        let mut paths = vec![];
        for raw in open {
            let mut path: Vec<_> = raw
                .into_iter()
                .map(|p| GridPoint { x: p.x, y: p.y })
                .collect();
            path.dedup();
            if path.len() < 2 {
                continue;
            }
            let closed = path.first() == path.last();
            if closed {
                path.pop();
                let start = path.iter().enumerate().min_by_key(|(_, p)| **p).unwrap().0;
                path.rotate_left(start);
                path.push(path[0]);
            }
            let reverse: Vec<_> = path.iter().rev().copied().collect();
            if reverse < path {
                path = reverse;
            }
            paths.push(path);
        }
        paths.sort();
        paths.dedup();
        Ok(paths
            .into_iter()
            .map(|p| p.into_iter().map(|q| self.grid.point(q)).collect())
            .collect())
    }

    /// Disk erosion: round joins are essential around holes and reflex corners.
    /// Positive-area output only; Target::center_set also retains exact-fit lines/points.
    pub fn erode(&self, radius_mm: f64) -> Result<Self> {
        // A positive-radius disk cannot cross the empty gap between disjoint
        // filled components. Erode each component (with its holes) independently
        // before collecting the result, avoiding one dense offset arrangement.
        if radius_mm > 0.
            && self.component_count() > 1
            && self.rings.iter().map(|r| r.points.len()).sum::<usize>() > 4096
        {
            let components = self.components();
            let workers = std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(4)
                .min(components.len());
            let batches = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..workers)
                    .map(|worker| {
                        let components = &components;
                        scope.spawn(move || {
                            components
                                .iter()
                                .enumerate()
                                .skip(worker)
                                .step_by(workers)
                                .map(|(i, c)| (i, c.offset(radius_mm, false)))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().map_err(|_| {
                            Diagnostic::new("OFFSET_WORKER_PANIC", "component offset worker failed")
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })?;
            let mut results: Vec<_> = batches.into_iter().flatten().collect();
            results.sort_by_key(|(i, _)| *i);
            let results = results
                .into_iter()
                .map(|(_, r)| r)
                .collect::<Result<Vec<_>>>()?;
            let mut output = Self::union_all(self.grid, &results.iter().collect::<Vec<_>>())?;
            if output.rings.is_empty() {
                output.diagnostics.push(Diagnostic::new("OFFSET_EMPTY_AREA", "erosion has no positive-area output; this does not rule out exact-fit centerlines or points"));
            }
            return Ok(output);
        }
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
        Self::from_tree_checked(grid, tree, 0)
    }

    fn from_tree_checked(grid: Grid, tree: PolyTree64, normalization_pass: usize) -> Result<Self> {
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
        if let Err(error) = validate_boundaries(grid, &rings, None) {
            if error.code != "OUTPUT_INTERSECTION" {
                return Err(error);
            }
            // Rounding a constructed intersection to integer coordinates can
            // make neighboring output edges cross again. Resolve the same
            // nonzero filled paths and revalidate; never accept crossed rings.
            // This applies only to backend output, not invalid source rings.
            let moved_intersection = split_rounded_crossings(grid, &mut rings)?;
            if !moved_intersection && validate_boundaries(grid, &rings, None).is_ok() {
                // Inserting a vertex on an existing edge changes neither its
                // geometry nor the tree hierarchy. Keep exact point contacts.
                return Ok(Self {
                    grid,
                    rings,
                    diagnostics: vec![
                        Diagnostic::new(
                            "OUTPUT_RENORMALIZED",
                            "split exact backend point contacts and revalidated polygon boundaries",
                        )
                        .warning(),
                    ],
                });
            }
            if normalization_pass == 2 {
                return Err(error);
            }
            let mut engine = Clipper64::new();
            let paths: Paths64 = rings
                .iter()
                .map(|r| r.points.iter().map(|p| Point64::new(p.x, p.y)).collect())
                .collect();
            engine.add_subject(&paths);
            let mut normalized = PolyTree64::new();
            if !engine.execute_tree(
                ClipType::Union,
                FillRule::NonZero,
                &mut normalized,
                &mut vec![],
            ) || engine.error_code() != 0
            {
                return Err(error);
            }
            let mut result = Self::from_tree_checked(grid, normalized, normalization_pass + 1)?;
            result.diagnostics.push(Diagnostic::new("OUTPUT_RENORMALIZED", "resolved rounded backend intersections with an additional checked polygon union").warning());
            return Ok(result);
        }
        Ok(Self {
            grid,
            rings,
            diagnostics: vec![],
        })
    }
}

/// Integer output edges can cross between grid points even after Clipper's
/// cleanup. Give both edges the same exactly rounded rational intersection
/// before re-running the fill operation. Each inserted vertex is within the
/// grid's existing snap bound of its intersection; no input paths are edited.
fn split_rounded_crossings(grid: Grid, rings: &mut [Ring]) -> Result<bool> {
    let edges: Vec<_> = rings
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
    let index = SpatialIndex::new(
        edges
            .iter()
            .map(|&(_, _, a, b)| Aabb::new(grid.point(a), grid.point(b)))
            .collect(),
    );
    let mut splits = vec![Vec::new(); edges.len()];
    let mut count = edges.len();
    let mut moved_intersection = false;
    index.pairs(|left, right| {
        let (_, _, a, b) = edges[left];
        let (_, _, c, d) = edges[right];
        let mut add = |edge: usize, p| {
            count += 1;
            if count > MAX_EDGES {
                return Err(Diagnostic::new(
                    "GEOMETRY_LIMIT",
                    "rounded output intersections exceed the boundary vertex limit",
                ));
            }
            splits[edge].push(p);
            Ok(())
        };
        for p in [a, b] {
            if p != c && p != d && on_segment(p, c, d) {
                add(right, p)?;
            }
        }
        for p in [c, d] {
            if p != a && p != b && on_segment(p, a, b) {
                add(left, p)?;
            }
        }
        let opposite = |x: i128, y: i128| x != 0 && y != 0 && (x < 0) != (y < 0);
        if !opposite(cross(a, b, c), cross(a, b, d)) || !opposite(cross(c, d, a), cross(c, d, b)) {
            return Ok(());
        }
        let (rx, ry) = ((b.x - a.x) as i128, (b.y - a.y) as i128);
        let (sx, sy) = ((d.x - c.x) as i128, (d.y - c.y) as i128);
        let den = rx * sy - ry * sx;
        let t = (c.x - a.x) as i128 * sy - (c.y - a.y) as i128 * sx;
        let round = |n: i128| {
            let (n, den) = if den < 0 { (-n, -den) } else { (n, den) };
            (n / den
                + if (n % den).abs() * 2 >= den {
                    n.signum()
                } else {
                    0
                }) as i64
        };
        let p = GridPoint {
            x: round(a.x as i128 * den + rx * t),
            y: round(a.y as i128 * den + ry * t),
        };
        moved_intersection = true;
        add(left, p)?;
        add(right, p)?;
        Ok(())
    })?;
    let mut output = vec![vec![]; rings.len()];
    for (edge, (r, _, a, b)) in edges.into_iter().enumerate() {
        output[r].push(a);
        let points = &mut splits[edge];
        points.sort_by_key(|p| {
            (p.x - a.x) as i128 * (b.x - a.x) as i128 + (p.y - a.y) as i128 * (b.y - a.y) as i128
        });
        points.dedup();
        output[r].extend(points.iter().copied().filter(|&p| p != a && p != b));
    }
    for (ring, points) in rings.iter_mut().zip(output) {
        ring.points = points;
    }
    Ok(moved_intersection)
}

/// Exact i128 predicates on snapped coordinates, robust predicates on source f64s.
/// Adjacent input edges may share endpoints. Input rings must be simple and mutually
/// disjoint at their boundaries so nesting is unambiguous. Clipper output has an
/// explicit hierarchy and may retain shared endpoints between components.
fn validate_boundaries(grid: Grid, rings: &[Ring], originals: Option<&[Vec<Point>]>) -> Result<()> {
    let edges: Vec<_> = rings
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
            "at most two million boundary edges are supported",
        ));
    }
    let index = SpatialIndex::new(
        edges
            .iter()
            .map(|&(r, i, a, b)| {
                let bounds = Aabb::new(grid.point(a), grid.point(b));
                originals.map_or(bounds, |raw| {
                    bounds.union(Aabb::new(raw[r][i], raw[r][(i + 1) % raw[r].len()]))
                })
            })
            .collect(),
    );
    index.pairs(|left, right| {
        let (r, i, a, b) = edges[left];
        let (s, j, c, d) = edges[right];
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
                    format!("edges {r}:{i} ({a:?} to {b:?}) and {s}:{j} ({c:?} to {d:?}) intersect or overlap"),
                ));
            }
        Ok(())
    })?;
    Ok(())
}

pub(super) fn orient(a: Point, b: Point, c: Point) -> f64 {
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
pub(super) fn raw_intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
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

#[cfg(test)]
mod output_normalization_tests {
    use super::*;

    #[test]
    fn partitioned_erosion_preserves_boundaries_and_holes_within_grid_rounding() {
        let grid = Grid::new(0.001, 300.).unwrap();
        let circle = |x: f64, radius: f64, count: usize| {
            (0..count)
                .map(|i| {
                    let angle = std::f64::consts::TAU * i as f64 / count as f64;
                    Point::new(x + radius * angle.cos(), radius * angle.sin())
                })
                .collect::<Vec<_>>()
        };
        let mut rings: Vec<_> = (0..8).map(|i| circle(i as f64 * 30., 10., 720)).collect();
        rings.push(circle(0., 3., 120));
        let region = Region::from_rings(grid, &rings).unwrap();
        let partitioned = region.erode(1.5).unwrap();
        let whole = region.offset(1.5, false).unwrap();
        // Clipper's final union order may round intersections differently.
        // Compare boundary geometry in both directions, including the hole,
        // with the reserve for two independently rounded constructions.
        let reserve = 2. * grid.snap_bound_mm();
        for (a, b) in [(&partitioned, &whole), (&whole, &partitioned)] {
            let query = crate::geometry::BoundaryQuery::new(b);
            for segment in a.segments() {
                for p in [segment.start, segment.start.lerp(segment.end, 0.5)] {
                    let sample = query.sample(p).unwrap();
                    assert!(
                        sample.distance_mm <= reserve + sample.numerical_reserve_mm,
                        "boundary moved {} mm",
                        sample.distance_mm
                    );
                }
            }
        }
        let perimeter: f64 = whole
            .segments()
            .iter()
            .map(|s| s.start.distance(s.end))
            .sum();
        assert!(
            partitioned
                .boolean(BooleanOp::Xor, &whole)
                .unwrap()
                .area_mm2()
                <= perimeter * reserve
        );
        assert_eq!(partitioned.component_count(), 8);
        assert_eq!(partitioned.hole_count(), 1);
        assert_eq!(
            serde_json::to_value(region.erode(20.).unwrap()).unwrap(),
            serde_json::to_value(region.offset(20., false).unwrap()).unwrap()
        );
    }

    #[test]
    fn flower_point_contact_splits_the_other_edge_without_moving_geometry() {
        let grid = Grid::with_scale(0.001, 200., 10000.).unwrap();
        let a = vec![
            Point64::new(193819, 412610),
            Point64::new(193442, 412545),
            Point64::new(193600, 412500),
        ];
        let b = vec![
            Point64::new(193442, 412544),
            Point64::new(193442, 412546),
            Point64::new(193440, 412545),
        ];
        let expected = [&a, &b]
            .iter()
            .map(|path| {
                twice_area(
                    &path
                        .iter()
                        .map(|p| GridPoint { x: p.x, y: p.y })
                        .collect::<Vec<_>>(),
                )
                .abs() as f64
                    / 2.
                    / grid.scale().powi(2)
            })
            .sum::<f64>();
        let mut tree = PolyTree64::new();
        tree.add_child(0, a);
        tree.add_child(0, b);
        let result = Region::from_tree(grid, tree).unwrap();
        assert!((result.area_mm2() - expected).abs() <= f64::EPSILON * expected);
        assert_eq!(result.component_count(), 2);
        assert!(result.rings().iter().all(|r| r.points.contains(&GridPoint {
            x: 193442,
            y: 412545
        })));
        validate_boundaries(grid, result.rings(), None).unwrap();
    }

    #[test]
    fn flower_rounding_crossing_gets_one_shared_exactly_rounded_vertex() {
        let grid = Grid::with_scale(0.001, 200., 10000.).unwrap();
        let point = |x, y| GridPoint { x, y };
        let a = point(1497736, 505359);
        let b = point(1497851, 505467);
        let c = point(1497926, 505558);
        let d = point(1497826, 505437);
        let mut rings = vec![
            Ring {
                points: vec![a, b, point(1497000, 505600)],
                parent: None,
                is_hole: false,
            },
            Ring {
                points: vec![c, d, point(1499000, 505000)],
                parent: None,
                is_hole: false,
            },
        ];
        assert!(validate_boundaries(grid, &rings, None).is_err());
        split_rounded_crossings(grid, &mut rings).unwrap();
        let shared = point(1497850, 505466);
        assert!(rings.iter().all(|r| r.points.contains(&shared)));
        for (a, b) in [(a, b), (c, d)] {
            let edge = Segment {
                start: grid.point(a),
                end: grid.point(b),
            };
            assert!(edge.distance(grid.point(shared)) <= grid.snap_bound_mm());
        }
        let mut tree = PolyTree64::new();
        for ring in rings {
            tree.add_child(
                0,
                ring.points.iter().map(|p| Point64::new(p.x, p.y)).collect(),
            );
        }
        let result = Region::from_tree(grid, tree).unwrap();
        validate_boundaries(grid, result.rings(), None).unwrap();
    }

    #[test]
    fn crossed_backend_paths_are_revalidated_with_holes_and_area_preserved() {
        let grid = Grid::with_scale(0.001, 20., 10000.).unwrap();
        let rectangle = |x0, y0, x1, y1| {
            vec![
                Point64::new(x0, y0),
                Point64::new(x1, y0),
                Point64::new(x1, y1),
                Point64::new(x0, y1),
            ]
        };
        let tree = || {
            let mut tree = PolyTree64::new();
            let left = tree.add_child(0, rectangle(0, 0, 100000, 100000));
            tree.add_child(left, rectangle(20000, 20000, 30000, 30000));
            tree.add_child(0, rectangle(50000, -10000, 150000, 90000));
            tree
        };
        // 100 + 100 - 45 overlapping mm² - 1 mm² hole.
        let normalized = Region::from_tree(grid, tree()).unwrap();
        assert_eq!(normalized.area_mm2(), 154.);
        assert_eq!(normalized.component_count(), 1);
        assert_eq!(normalized.hole_count(), 1);
        assert!(
            normalized
                .diagnostics()
                .iter()
                .any(|d| d.code == "OUTPUT_RENORMALIZED")
        );
        validate_boundaries(grid, normalized.rings(), None).unwrap();
        // Exhausting normalization must still reject crossed boundaries.
        assert_eq!(
            Region::from_tree_checked(grid, tree(), 2).unwrap_err().code,
            "OUTPUT_INTERSECTION"
        );
        // The source-ring constructor still rejects intersecting input.
        assert!(
            Region::from_rings(
                grid,
                &[
                    vec![
                        Point::new(0., 0.),
                        Point::new(10., 0.),
                        Point::new(10., 10.),
                        Point::new(0., 10.)
                    ],
                    vec![
                        Point::new(5., -1.),
                        Point::new(15., -1.),
                        Point::new(15., 9.),
                        Point::new(5., 9.)
                    ],
                ]
            )
            .is_err()
        );
    }
}
