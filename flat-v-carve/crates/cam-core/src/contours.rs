//! Contour catalogue (plan section 7): stable closed-contour identity over
//! the imported artwork, explicit per-contour roles and sides, and
//! source-parameterized anchors. Flat V-carve resolves component selections
//! into filled unions; profile and knife resolve contour IDs through this
//! catalogue and never union selected boundaries first.
//!
//! Open centerline chains (knife import, plan section 7.3) are kept exactly
//! as drawn: open subpaths stay open, vertex order and direction are the
//! source's, and their identity is placement-independent like everything
//! else here.

use crate::geometry::{Diagnostic, Point, Region, Result};
use crate::project::{CamJob, ContourAnchor, ContourSide};
use crate::svg::Placement;

/// Quantization of the page-space fingerprint grid, in mm. Coarse enough to
/// absorb placement round-trip snapping and float noise, fine enough that
/// any meaningful artwork edit changes it.
const FINGERPRINT_STEP_MM: f64 = 0.01;

/// Interior vertices may be merged only within this fraction of the import
/// tolerance, and never when they carry a real corner turn (plan 7.3: no
/// deleting corners needed for knife orientation). Import flattening already
/// bounds curve error by a quarter tolerance; this spends at most another
/// quarter, leaving half the budget for planning.
const CHAIN_SIMPLIFY_BUDGET_FRACTION: f64 = 4.;
/// Turns sharper than this are corners and always survive simplification.
const CHAIN_CORNER_TURN_DEG: f64 = 30.;

/// Contour IDs use the document's identifier alphabet. The importer's
/// component IDs (`source::index`) are normalized into it deterministically
/// (runs of foreign characters collapse to one dash); a build-time collision
/// check keeps identity unambiguous.
fn contour_id(component_id: &str, suffix: &str) -> String {
    format!("{}-{suffix}", normalize_id(component_id))
}
fn normalize_id(component_id: &str) -> String {
    let mut normalized = String::with_capacity(component_id.len());
    let mut dash = false;
    for c in component_id.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            normalized.push(c);
            dash = false;
        } else if !dash {
            normalized.push('-');
            dash = true;
        }
    }
    normalized
}

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("contours")
}

/// A contour's role within its filled component. `Open` marks imported
/// centerline chains (knife import): `closed` on the chain then records
/// whether the source subpath closed itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContourRole {
    Outer,
    Hole,
    Open,
}

/// One closed boundary chain with stable identity and canonical geometry.
/// Open-chain entries keep the source vertex order and direction instead of
/// the canonical ring form (plan section 7.3).
#[derive(Clone, Debug)]
pub struct Contour {
    pub id: String,
    pub source_id: String,
    pub component_id: String,
    pub closed: bool,
    pub role: ContourRole,
    /// The enclosing outer contour of a hole, when applicable.
    pub parent_contour_id: Option<String>,
    /// Canonical setup-space vertices for closed contours (counter-clockwise,
    /// started at the lexicographically smallest vertex); source-order setup
    /// vertices for open chains. Traversal direction for cutting is chosen by
    /// the planner and never inferred from this winding.
    pub vertices: Vec<Point>,
    pub perimeter_mm: f64,
    /// The same ring in placement-independent page space, rotated to its
    /// deterministic canonical start (closed contours) or in source order
    /// (open chains). Anchor fractions address this geometry, so the same
    /// fraction always means the same source location whatever the artwork
    /// placement.
    pub(crate) page_vertices: Vec<Point>,
    /// Placement-independent fingerprint of the source geometry (page-space
    /// quantized at `FINGERPRINT_STEP_MM`).
    pub source_fingerprint: String,
    /// The owning artwork placement this entry's setup vertices were produced
    /// with. Single-source catalogues carry the document placement on every
    /// entry; an assembled collection catalogue carries each owner item's
    /// placement, so anchor resolution never crosses items (plan section 22.4).
    pub(crate) placement: Placement,
}

impl Contour {
    /// The polygon enclosed by this contour, independent of its role: the
    /// substrate every explicit side is compensated against. `Inside` erodes
    /// it, `Outside` dilates it, `On` follows it. Open chains have no enclosed
    /// region and no compensation side; the knife follows them directly.
    pub fn enclosed_region(&self, grid: crate::geometry::Grid) -> Result<Region> {
        Region::from_rings(grid, std::slice::from_ref(&self.vertices))
    }

    /// The side that keeps the cutter in the void when cutting this contour
    /// free: outside for outer boundaries, inside for holes (plan 7.1). The
    /// stored side stays explicit per contour; this is only a suggestion.
    /// Chains suggest `On`: the knife rides the centerline itself.
    pub fn suggested_side(&self) -> ContourSide {
        match self.role {
            ContourRole::Hole => ContourSide::Inside,
            ContourRole::Outer => ContourSide::Outside,
            ContourRole::Open => ContourSide::On,
        }
    }
}

/// The resolved position of an anchor in setup coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedAnchor {
    pub contour_id: String,
    pub point: Point,
    /// Unit tangent along the canonical (counter-clockwise) direction.
    pub tangent: Point,
}

#[derive(Clone, Debug)]
pub struct ContourCatalogue {
    pub contours: Vec<Contour>,
    /// Imported centerline chains (centerline import mode only): open or
    /// closed per the source subpath, in document and subpath order. Knife
    /// operations select these by ID; they never union.
    pub open_chains: Vec<Contour>,
}

impl ContourCatalogue {
    /// Build the catalogue from a job's SVG source. Jobs without a source
    /// have no contours (face-only jobs); that is an explicit condition.
    pub fn build(job: &CamJob) -> Result<Self> {
        let Some(source) = &job.source else {
            return Err(error(
                "PROJECT_SOURCE_REQUIRED",
                "the contour catalogue requires an SVG source; face-only jobs have no contours",
            ));
        };
        let geometry = crate::svg::import_svg(&source.svg, &job.import, None)?;
        let placement = job.import.placement.clone();
        let mut contours = vec![];
        for component in &geometry.sources {
            let region = &component.geometry;
            let rings = region.rings();
            let vertices = region.rings_mm();
            let mut outers = vec![];
            let mut holes = vec![];
            for (ring, vertices) in rings.iter().zip(&vertices) {
                if ring.is_hole() {
                    holes.push(canonical_ring(vertices));
                } else {
                    outers.push(canonical_ring(vertices));
                }
            }
            // A source component is single-outer by construction; the hole
            // order is derived from page-space geometry so contour IDs do not
            // follow any importer ring ordering.
            if outers.len() != 1 {
                return Err(error(
                    "CONTOUR_COMPONENT_TOPOLOGY",
                    format!(
                        "component '{}' has {} outer boundaries; expected one",
                        component.id,
                        outers.len()
                    ),
                ));
            }
            let outer_setup = outers.pop().expect("checked");
            let outer_page = page_ring(&outer_setup, &placement);
            let outer_id = contour_id(&component.id, "outer");
            let mut hole_entries: Vec<_> = holes
                .into_iter()
                .map(|setup| {
                    let page = page_ring(&setup, &placement);
                    (setup, page)
                })
                .collect();
            hole_entries
                .sort_by(|a, b| sort_key(&a.1).partial_cmp(&sort_key(&b.1)).expect("finite"));
            contours.push(Contour {
                id: outer_id.clone(),
                source_id: component.source_id.clone(),
                component_id: component.id.clone(),
                closed: true,
                role: ContourRole::Outer,
                parent_contour_id: None,
                perimeter_mm: perimeter(&outer_setup),
                source_fingerprint: fingerprint(&outer_page),
                vertices: outer_setup,
                page_vertices: outer_page,
                placement: placement.clone(),
            });
            for (index, (setup, page_ring)) in hole_entries.into_iter().enumerate() {
                contours.push(Contour {
                    id: contour_id(&component.id, &format!("hole-{index}")),
                    source_id: component.source_id.clone(),
                    component_id: component.id.clone(),
                    closed: true,
                    role: ContourRole::Hole,
                    parent_contour_id: Some(outer_id.clone()),
                    perimeter_mm: perimeter(&setup),
                    source_fingerprint: fingerprint(&page_ring),
                    vertices: setup,
                    page_vertices: page_ring,
                    placement: placement.clone(),
                });
            }
        }
        let unique: std::collections::BTreeSet<&str> =
            contours.iter().map(|c| c.id.as_str()).collect();
        if unique.len() != contours.len() {
            return Err(error(
                "CONTOUR_ID_COLLISION",
                "source component IDs normalize to colliding contour IDs; rename them in the SVG",
            ));
        }
        // Centerline chains: place each chain with the artwork transform,
        // then merge only within the declared near-zero budget. Source order,
        // direction and endpoints survive exactly as drawn.
        let budget = job.import.geometry_tolerance_mm / CHAIN_SIMPLIFY_BUDGET_FRACTION;
        let mut open_chains = vec![];
        for chain in &geometry.chains {
            let forward = forward_space(&placement);
            let placed: Vec<Point> = chain.points.iter().map(|p| forward(*p)).collect();
            let vertices = simplify_chain(&placed, budget, chain.closed);
            if vertices.len() < if chain.closed { 3 } else { 2 } {
                return Err(error(
                    "CONTOUR_COMPONENT_TOPOLOGY",
                    format!("chain '{}' degenerates after placement", chain.id),
                ));
            }
            let page = page_space(&placement);
            let page_vertices: Vec<Point> = vertices.iter().map(|p| page(*p)).collect();
            open_chains.push(Contour {
                id: normalize_id(&chain.id),
                source_id: chain.source_id.clone(),
                component_id: chain.source_id.clone(),
                closed: chain.closed,
                role: ContourRole::Open,
                parent_contour_id: None,
                perimeter_mm: perimeter_of(&vertices, chain.closed),
                source_fingerprint: fingerprint(&page_vertices),
                vertices,
                page_vertices,
                placement: placement.clone(),
            });
        }
        let chain_ids: std::collections::BTreeSet<&str> =
            open_chains.iter().map(|c| c.id.as_str()).collect();
        if chain_ids.len() != open_chains.len() || chain_ids.intersection(&unique).next().is_some()
        {
            return Err(error(
                "CONTOUR_ID_COLLISION",
                "source chain IDs normalize to colliding contour IDs; rename them in the SVG",
            ));
        }
        Ok(Self {
            contours,
            open_chains,
        })
    }

    pub fn contour(&self, id: &str) -> Option<&Contour> {
        self.contours.iter().find(|c| c.id == id)
    }

    pub fn chain(&self, id: &str) -> Option<&Contour> {
        self.open_chains.iter().find(|c| c.id == id)
    }

    /// Resolve explicit contour references. Exactly the requested contours
    /// are returned: selecting one hole never selects the enclosing boundary
    /// or a sibling hole.
    pub fn select(&self, ids: &[String]) -> Result<Vec<&Contour>> {
        let mut selected = vec![];
        for id in ids {
            let contour = self.contour(id).ok_or_else(|| {
                error(
                    "CONTOUR_REFERENCE",
                    format!("unknown contour '{id}'; inspect the contour catalogue"),
                )
                .source(id)
            })?;
            selected.push(contour);
        }
        Ok(selected)
    }

    /// Resolve explicit open-chain references (knife selections) in document
    /// and subpath order. Exactly the requested chains are returned.
    pub fn select_chains(&self, ids: &[String]) -> Result<Vec<&Contour>> {
        let mut selected = vec![];
        for id in ids {
            let chain = self.chain(id).ok_or_else(|| {
                error(
                    "CONTOUR_REFERENCE",
                    format!("unknown chain '{id}'; inspect the contour catalogue"),
                )
                .source(id)
            })?;
            selected.push(chain);
        }
        Ok(selected)
    }

    /// Resolve an anchor against the current catalogue. Placement changes
    /// move the resolved point; a changed source fingerprint leaves the
    /// anchor unresolved — there is no nearest-contour guessing. Anchors may
    /// address closed contours or imported open chains.
    pub fn resolve_anchor(&self, anchor: &ContourAnchor) -> Result<ResolvedAnchor> {
        let contour = self
            .contour(&anchor.contour_id)
            .or_else(|| self.chain(&anchor.contour_id))
            .ok_or_else(|| {
                error(
                    "CONTOUR_REFERENCE",
                    format!("anchor references unknown contour '{}'", anchor.contour_id),
                )
            })?;
        if contour.source_fingerprint != anchor.source_geometry_fingerprint {
            return Err(error(
                "CONTOUR_ANCHOR_UNRESOLVED",
                format!(
                    "contour '{}' no longer matches the anchor's source geometry; reattach the anchor",
                    anchor.contour_id
                ),
            ));
        }
        let (page_point, page_tangent) = point_at(
            &contour.page_vertices,
            anchor.fraction_along_source_contour,
            contour.closed,
        )?;
        // Forward placement transform (plan section 6.1): rotate about the
        // origin, scale, translate. Tangents rotate without translation. The
        // placement is the owning item's, never a document-wide one, so an
        // assembled collection catalogue resolves each anchor through its
        // own source space (plan section 22.4).
        let placement = &contour.placement;
        let (s, c) = placement.rotation_deg.to_radians().sin_cos();
        let k = placement.scale;
        let origin = placement.origin_mm;
        let dx = page_point.x - origin.x;
        let dy = page_point.y - origin.y;
        let point = Point::new(k * (c * dx - s * dy), k * (s * dx + c * dy));
        let tangent = Point::new(
            c * page_tangent.x - s * page_tangent.y,
            s * page_tangent.x + c * page_tangent.y,
        );
        Ok(ResolvedAnchor {
            contour_id: contour.id.clone(),
            point,
            tangent,
        })
    }
}

/// Canonicalize a closed ring: drop the closing duplicate, orient
/// counter-clockwise, and start at the lexicographically smallest vertex.
/// Source winding and start choice leave no trace.
fn canonical_ring(vertices: &[Point]) -> Vec<Point> {
    let mut points = vertices.to_vec();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return points;
    }
    let twice = twice_signed_area(&points);
    if twice < 0. {
        points.reverse();
    }
    let start = (0..points.len())
        .min_by(|&a, &b| {
            (points[a].x, points[a].y)
                .partial_cmp(&(points[b].x, points[b].y))
                .expect("finite canonicalized vertices")
        })
        .expect("nonempty");
    points.rotate_left(start);
    points
}

/// Closed-ring edge pairs, including the wrap-around edge.
fn segments(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
}

fn twice_signed_area(points: &[Point]) -> f64 {
    segments(points).map(|(a, b)| a.x * b.y - b.x * a.y).sum()
}

fn perimeter(points: &[Point]) -> f64 {
    segments(points).map(|(a, b)| a.distance(b)).sum()
}

/// Edge pairs of a chain: the drawn segments only — an open chain never
/// gains a closing edge.
fn chain_segments(points: &[Point], closed: bool) -> Box<dyn Iterator<Item = (Point, Point)> + '_> {
    if closed {
        Box::new(segments(points))
    } else if points.len() < 2 {
        Box::new(std::iter::empty())
    } else {
        Box::new(points.windows(2).map(|w| (w[0], w[1])))
    }
}

fn perimeter_of(points: &[Point], closed: bool) -> f64 {
    chain_segments(points, closed)
        .map(|(a, b)| a.distance(b))
        .sum()
}

/// Position and unit tangent at a fraction of the perimeter, walked along the
/// stored direction (canonical for closed contours, source direction for
/// chains). The parameterization is proportional to length, so the same
/// fraction always addresses the same source location regardless of artwork
/// scale.
fn point_at(vertices: &[Point], fraction: f64, closed: bool) -> Result<(Point, Point)> {
    if vertices.len() < 2 || !fraction.is_finite() || !(0. ..1.).contains(&fraction) {
        return Err(error(
            "CONTOUR_ANCHOR_UNRESOLVED",
            format!("anchor fraction {fraction} is not a position along the contour"),
        ));
    }
    let total = perimeter_of(vertices, closed);
    if total <= 0. {
        return Err(error(
            "CONTOUR_ANCHOR_UNRESOLVED",
            "contour has no positive perimeter",
        ));
    }
    let target = fraction * total;
    let mut walked = 0.;
    for (a, b) in chain_segments(vertices, closed) {
        let length = a.distance(b);
        if length == 0. {
            continue;
        }
        if walked + length >= target {
            let t = (target - walked) / length;
            let point = a.lerp(b, t);
            let tangent = Point::new((b.x - a.x) / length, (b.y - a.y) / length);
            return Ok((point, tangent));
        }
        walked += length;
    }
    // Floating-point accumulation ended just short of the target: clamp to
    // the stored start with its forward tangent.
    let last = vertices[0];
    let toward = vertices[1.min(vertices.len() - 1)];
    let length = last.distance(toward);
    Ok((
        last,
        Point::new((toward.x - last.x) / length, (toward.y - last.y) / length),
    ))
}

/// Quantization bucket of a page-space coordinate: the noise-proof identity
/// for vertex comparison across placements.
fn bucket(p: Point) -> (i64, i64) {
    (
        (p.x / FINGERPRINT_STEP_MM).round() as i64,
        (p.y / FINGERPRINT_STEP_MM).round() as i64,
    )
}

/// Deterministic page-space sort key of a ring: its quantized canonical
/// start vertex, tie-broken by exact coordinates.
fn sort_key(page_ring: &[Point]) -> ((i64, i64), f64, f64) {
    let first = page_ring[0];
    let b = bucket(first);
    (b, first.x, first.y)
}

/// The placement-independent counterpart of a canonical setup ring: invert
/// the placement transform, then rotate the ring to the deterministic
/// canonical start chosen by quantized-then-exact comparison (float noise
/// from the inverse must not move that choice).
fn page_ring(setup_ring: &[Point], placement: &Placement) -> Vec<Point> {
    let page = page_space(placement);
    let mut points: Vec<Point> = setup_ring.iter().map(|p| page(*p)).collect();
    if points.len() < 3 {
        return points;
    }
    if twice_signed_area(&points) < 0. {
        points.reverse();
    }
    let start = (0..points.len())
        .min_by(|&a, &b| {
            let (ka, kb) = (bucket(points[a]), bucket(points[b]));
            ka.cmp(&kb).then_with(|| {
                (points[a].x, points[a].y)
                    .partial_cmp(&(points[b].x, points[b].y))
                    .expect("finite page-space vertices")
            })
        })
        .expect("nonempty");
    points.rotate_left(start);
    points
}

/// The page-space counterpart of a setup point: inverse of
/// `setup = scale * rotate(page - origin)` (plan section 6.1).
fn page_space(placement: &Placement) -> impl Fn(Point) -> Point + '_ {
    let (s, c) = placement.rotation_deg.to_radians().sin_cos();
    let k = placement.scale;
    let origin = placement.origin_mm;
    move |p: Point| {
        let x = p.x / k;
        let y = p.y / k;
        Point::new(c * x + s * y + origin.x, -s * x + c * y + origin.y)
    }
}

/// The setup-space counterpart of a page point:
/// `setup = scale * rotate(page - origin)` (plan section 6.1).
fn forward_space(placement: &Placement) -> impl Fn(Point) -> Point + '_ {
    let (s, c) = placement.rotation_deg.to_radians().sin_cos();
    let k = placement.scale;
    let origin = placement.origin_mm;
    move |p: Point| {
        let dx = p.x - origin.x;
        let dy = p.y - origin.y;
        Point::new(k * (c * dx - s * dy), k * (s * dx + c * dy))
    }
}

/// Merge interior chain vertices only within the declared near-zero budget
/// (plan section 7.3): a vertex disappears only when it lies within
/// `budget` of the segment joining its neighbours AND the turn it carries is
/// below the corner threshold, so corners needed for knife orientation —
/// including near-reversals with tiny deviation — always survive. Endpoints
/// are never removed, and a chain never simplifies below its minimum vertex
/// count.
fn simplify_chain(points: &[Point], budget_mm: f64, closed: bool) -> Vec<Point> {
    let minimum = if closed { 3 } else { 2 };
    if budget_mm <= 0. || points.len() <= minimum {
        return points.to_vec();
    }
    let turn_deg = |a: Point, p: Point, b: Point| -> f64 {
        let u = Point::new(p.x - a.x, p.y - a.y);
        let v = Point::new(b.x - p.x, b.y - p.y);
        let (un, vn) = (u.x.hypot(u.y), v.x.hypot(v.y));
        if un < 1e-12 || vn < 1e-12 {
            return 0.;
        }
        let cosine = ((u.x * v.x + u.y * v.y) / (un * vn)).clamp(-1., 1.);
        cosine.acos().to_degrees()
    };
    let deviation = |p: Point, a: Point, b: Point| -> f64 {
        let d = Point::new(b.x - a.x, b.y - a.y);
        let length = d.x.hypot(d.y);
        if length < 1e-12 {
            return p.distance(a);
        }
        ((d.x * (p.y - a.y) - d.y * (p.x - a.x)) / length).abs()
    };
    let mut kept: Vec<Point> = vec![points[0]];
    let last = points.len() - 1;
    for index in 1..points.len() {
        let is_endpoint = !closed && index == last;
        let candidate = points[index];
        let previous = kept.last().expect("start kept");
        let next = if index == last {
            points[0]
        } else {
            points[index + 1]
        };
        let mergeable = !is_endpoint
            && deviation(candidate, *previous, next) <= budget_mm
            && turn_deg(*previous, candidate, next) < CHAIN_CORNER_TURN_DEG;
        if !mergeable {
            kept.push(candidate);
        }
    }
    if kept.len() < minimum {
        return points.to_vec();
    }
    kept
}

/// Quantized page-space fingerprint of a canonical ring.
fn fingerprint(page_ring: &[Point]) -> String {
    let ticks: Vec<[i64; 2]> = page_ring.iter().map(|p| bucket(*p).into()).collect();
    crate::plan_hash::hash(&ticks).expect("integers serialize")
}
