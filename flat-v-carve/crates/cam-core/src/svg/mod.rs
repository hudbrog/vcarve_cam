//! Supported-subset SVG importer. XML is data only; no filesystem or network access.
mod path;
mod style;
use crate::geometry::spatial::{Aabb, SpatialIndex};
use crate::geometry::{BooleanOp, Diagnostic, Grid, Point, Region, Result, WindingRule};
use path::{Flattener, MAX_VERTICES, Matrix};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use style::Style;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
pub const MAX_SVG_BYTES: usize = 32_000_000;

pub(super) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("svg")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Workpiece coordinate = scale * rotate(page XY - origin_mm).
    pub origin_mm: Point,
    pub scale: f64,
    pub rotation_deg: f64,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            origin_mm: Point::new(0., 0.),
            scale: 1.,
            rotation_deg: 0.,
        }
    }
}
impl Placement {
    fn matrix(&self) -> Result<Matrix> {
        if !self.origin_mm.finite()
            || !self.scale.is_finite()
            || self.scale <= 0.
            || !self.rotation_deg.is_finite()
        {
            return Err(error(
                "SVG_PLACEMENT",
                "finite origin/rotation and positive scale required",
            ));
        }
        let (s, c) = self.rotation_deg.to_radians().sin_cos();
        let k = self.scale;
        let [x, y] = [self.origin_mm.x, self.origin_mm.y];
        let m = Matrix([
            k * c,
            k * s,
            -k * s,
            k * c,
            k * (-c * x + s * y),
            k * (-s * x - c * y),
        ]);
        m.validate()?;
        Ok(m)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImportOptions {
    pub geometry_tolerance_mm: f64,
    pub ticks_per_mm: Option<f64>,
    pub placement: Placement,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            placement: Placement::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}
impl Bounds {
    pub fn of(region: &Region) -> Option<Self> {
        let points = region.rings_mm();
        let mut all = points.iter().flatten();
        let first = *all.next()?;
        Some(all.fold(
            Self {
                min: first,
                max: first,
            },
            |mut b, p| {
                b.min.x = b.min.x.min(p.x);
                b.min.y = b.min.y.min(p.y);
                b.max.x = b.max.x.max(p.x);
                b.max.y = b.max.y.max(p.y);
                b
            },
        ))
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceComponent {
    pub id: String,
    pub source_id: String,
    pub label: Option<String>,
    pub geometry: Region,
}
#[derive(Clone, Debug, Serialize)]
pub struct MappedComponent {
    pub geometry: Region,
    pub selected_region_ids: Vec<String>,
    pub source_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct NormalizedGeometry {
    pub page_width_mm: f64,
    pub page_height_mm: f64,
    pub flattening_bound_mm: f64,
    pub source_grid: Grid,
    pub source_snap_bound_mm: f64,
    pub grid: Grid,
    pub sources: Vec<SourceComponent>,
    pub selected_region_ids: Vec<String>,
    pub selected: Region,
    pub components: Vec<MappedComponent>,
    pub bounds: Option<Bounds>,
    pub diagnostics: Vec<Diagnostic>,
}

struct RawShape {
    id: String,
    label: Option<String>,
    rings: Vec<Vec<Point>>,
    rule: WindingRule,
}
struct Reader {
    shapes: Vec<RawShape>,
    diagnostics: Vec<Diagnostic>,
    vertices: usize,
    serial: usize,
    tolerance: f64,
    ids: BTreeSet<String>,
    page_matrix: Matrix,
    width: f64,
    height: f64,
    namespaced: bool,
}

pub fn import_svg(
    source: &str,
    options: &ImportOptions,
    selection: Option<&[String]>,
) -> Result<NormalizedGeometry> {
    if source.len() > MAX_SVG_BYTES {
        return Err(error(
            "SVG_RESOURCE_LIMIT",
            "SVG exceeds the 32 MB input limit",
        ));
    }
    Grid::new(options.geometry_tolerance_mm, 0.)?;
    let placement = options.placement.matrix()?;
    let doc = Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 200_000,
            ..Default::default()
        },
    )
    .map_err(|e| error("SVG_XML", format!("{e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg"
        || !matches!(root.tag_name().namespace(), None | Some(SVG_NS))
    {
        return Err(error("SVG_ROOT", "expected an SVG root element"));
    }
    if doc
        .descendants()
        .any(|n| n.pi().is_some_and(|p| p.target == "xml-stylesheet"))
    {
        return Err(error(
            "SVG_STYLESHEET",
            "external stylesheets are unsupported; save explicit presentation styles",
        ));
    }
    let mut ids = BTreeSet::new();
    for node in doc.descendants().filter(Node::is_element) {
        if matches!(
            node.tag_name().name(),
            "script" | "animate" | "animateMotion" | "animateTransform" | "set" | "discard"
        ) && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
        {
            return Err(error(
                "SVG_DYNAMIC_CONTENT",
                "scripts and animation must be removed before importing static artwork",
            ));
        }
        if node.attributes().any(|a| {
            a.namespace().is_none()
                && (a.name().starts_with("on")
                    || matches!(
                        a.name(),
                        "requiredFeatures" | "requiredExtensions" | "systemLanguage"
                    ))
        }) {
            return Err(error(
                "SVG_DYNAMIC_CONTENT",
                "event handlers and conditional rendering attributes are unsupported",
            ));
        }
        if let Some(id) = node.attribute("id")
            && (id.is_empty() || !ids.insert(id.to_owned()))
        {
            return Err(
                error("SVG_DUPLICATE_ID", "source IDs must be nonempty and unique").source(id),
            );
        }
        if node.tag_name().name() == "style"
            && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
        {
            return Err(error(
                "SVG_STYLESHEET",
                "CSS stylesheets are unsupported; use presentation attributes or inline styles",
            ));
        }
    }
    let width = physical(root.attribute("width").ok_or_else(|| {
        error(
            "SVG_PAGE_SIZE",
            "explicit root width and height are required",
        )
    })?)?;
    let height = physical(root.attribute("height").ok_or_else(|| {
        error(
            "SVG_PAGE_SIZE",
            "explicit root width and height are required",
        )
    })?)?;
    if width <= 0. || height <= 0. {
        return Err(error(
            "SVG_PAGE_SIZE",
            "page width and height must be positive",
        ));
    }
    let viewport = viewport(root, width, height)?;
    let page_matrix = Matrix([1., 0., 0., -1., 0., height]).then(viewport);
    let mut reader = Reader {
        shapes: vec![],
        diagnostics: vec![],
        vertices: 0,
        serial: 0,
        tolerance: options.geometry_tolerance_mm / (4. * options.placement.scale),
        ids,
        page_matrix,
        width,
        height,
        namespaced: root.tag_name().namespace() == Some(SVG_NS),
    };
    reader.walk(root, Matrix::ID, &Style::default(), 0, true)?;
    let extent = reader
        .shapes
        .iter()
        .flat_map(|s| s.rings.iter().flatten())
        .map(|&p| {
            let p = placement.apply(p);
            p.x.abs().max(p.y.abs())
        })
        .fold(0., f64::max);
    let grid = match options.ticks_per_mm {
        Some(s) => Grid::with_scale(options.geometry_tolerance_mm, extent, s)?,
        None => Grid::new(options.geometry_tolerance_mm, extent)?,
    };
    let mut sources = vec![];
    let page_paths: Vec<_> = reader
        .shapes
        .iter()
        .flat_map(|s| s.rings.iter().cloned())
        .collect();
    let page_extent = page_paths
        .iter()
        .flatten()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(0., f64::max);
    let automatic_source_grid = Grid::new(
        options.geometry_tolerance_mm / (2. * options.placement.scale),
        page_extent,
    )?;
    let source_grid = Grid::with_scale(
        options.geometry_tolerance_mm / (2. * options.placement.scale),
        page_extent,
        automatic_source_grid
            .scale()
            .max(grid.scale() * options.placement.scale * 2.),
    )?;
    Region::check_snapping(source_grid, &page_paths)?;
    let mut placed_paths = vec![];
    for shape in reader.shapes {
        let region = Region::from_filled_paths(source_grid, &shape.rings, shape.rule)
            .map_err(|d| d.source(&shape.id))?;
        for d in region.diagnostics() {
            reader.diagnostics.push(d.clone().source(&shape.id));
        }
        if region.rings().is_empty() {
            return Err(error("SVG_EMPTY_FILL","fill resolution produced no area; remove intentionally cancelling contours or use a finer precision").source(&shape.id));
        }
        // IDs are assigned in page coordinates, before rotating/moving the workpiece.
        for (index, page_component) in region.components().into_iter().enumerate() {
            let rings: Vec<Vec<_>> = page_component
                .rings_mm()
                .iter()
                .map(|ring| ring.iter().map(|&p| placement.apply(p)).collect())
                .collect();
            let geometry = Region::from_filled_paths(grid, &rings, WindingRule::Nonzero)
                .map_err(|d| d.source(&shape.id))?;
            if geometry.component_count() != 1
                || geometry.hole_count() != page_component.hole_count()
            {
                return Err(error(
                    "SVG_PLACEMENT_TOPOLOGY",
                    "placement snapping changed component or hole topology",
                )
                .source(&shape.id));
            }
            placed_paths.extend(rings);
            sources.push(SourceComponent {
                id: format!("{}::{index}", shape.id),
                source_id: shape.id.clone(),
                label: shape.label.clone(),
                geometry,
            });
        }
    }
    Region::check_snapping(grid, &placed_paths)?;
    if sources.is_empty() {
        return Err(error(
            "SVG_NO_REGIONS",
            "SVG has no supported visible filled regions",
        ));
    }
    let selection = selection.map_or_else(
        || sources.iter().map(|s| s.id.clone()).collect(),
        |s| s.to_vec(),
    );
    let unique: BTreeSet<_> = selection.iter().collect();
    let known: BTreeSet<_> = sources.iter().map(|s| &s.id).collect();
    if unique.len() != selection.len() || !unique.is_subset(&known) {
        return Err(error(
            "SVG_SELECTION",
            "selection contains duplicate or unknown region IDs; inspect the source components",
        ));
    }
    let selected_sources: Vec<_> = sources.iter().filter(|s| unique.contains(&s.id)).collect();
    let selected = Region::union_all(
        grid,
        &selected_sources
            .iter()
            .map(|s| &s.geometry)
            .collect::<Vec<_>>(),
    )?;
    let source_index = SpatialIndex::new(
        selected_sources
            .iter()
            .map(|s| {
                let b = Bounds::of(&s.geometry).unwrap();
                Aabb::new(b.min, b.max)
            })
            .collect(),
    );
    let mut components = vec![];
    for geometry in selected.components() {
        let mut selected_region_ids = vec![];
        let mut source_ids = BTreeSet::new();
        let bounds = Bounds::of(&geometry).unwrap();
        let mut candidates = vec![];
        source_index.visit(Aabb::new(bounds.min, bounds.max), |i| {
            candidates.push(i);
            Ok(())
        })?;
        candidates.sort_unstable();
        for i in candidates {
            let s = selected_sources[i];
            if geometry
                .boolean(BooleanOp::Intersection, &s.geometry)?
                .area_mm2()
                > 0.
            {
                selected_region_ids.push(s.id.clone());
                source_ids.insert(s.source_id.clone());
            }
        }
        components.push(MappedComponent {
            geometry,
            selected_region_ids,
            source_ids: source_ids.into_iter().collect(),
        });
    }
    let bounds = Bounds::of(&selected);
    Ok(NormalizedGeometry {
        page_width_mm: width,
        page_height_mm: height,
        flattening_bound_mm: options.geometry_tolerance_mm / 4.,
        source_grid,
        source_snap_bound_mm: source_grid.snap_bound_mm() * options.placement.scale,
        grid,
        sources,
        selected_region_ids: selection,
        selected,
        components,
        bounds,
        diagnostics: reader.diagnostics,
    })
}

pub(super) fn number(s: &str) -> Result<f64> {
    let n: svgtypes::Number = s
        .trim()
        .parse()
        .map_err(|e| error("SVG_NUMBER", format!("{e}")))?;
    if !n.0.is_finite() || n.0.abs() > 1e12 {
        return Err(error(
            "SVG_NUMBER",
            "number exceeds the finite supported range",
        ));
    }
    Ok(n.0)
}
pub(super) fn user_length(s: &str) -> Result<f64> {
    let n: svgtypes::Length = s
        .trim()
        .parse()
        .map_err(|e| error("SVG_LENGTH", format!("{e}")))?;
    use svgtypes::LengthUnit::*;
    let factor = match n.unit {
        None | Px => 1.,
        Mm => 96. / 25.4,
        Cm => 96. / 2.54,
        In => 96.,
        Pt => 96. / 72.,
        Pc => 16.,
        _ => {
            return Err(error(
                "SVG_LENGTH_UNIT",
                "percent and font-relative lengths are unsupported; use explicit physical/user units",
            ));
        }
    };
    let v = n.number * factor;
    if !v.is_finite() || v.abs() > 1e12 {
        return Err(error("SVG_LENGTH", "length exceeds the supported range"));
    }
    Ok(v)
}
fn physical(s: &str) -> Result<f64> {
    Ok(user_length(s)? * 25.4 / 96.)
}
fn viewport(root: Node<'_, '_>, w: f64, h: f64) -> Result<Matrix> {
    let Some(raw) = root.attribute("viewBox") else {
        return Ok(Matrix([25.4 / 96., 0., 0., 25.4 / 96., 0., 0.]));
    };
    let vb: svgtypes::ViewBox = raw
        .parse()
        .map_err(|e| error("SVG_VIEWBOX", format!("{e}")))?;
    if [vb.x, vb.y, vb.w, vb.h].iter().any(|v| !v.is_finite()) || vb.w <= 0. || vb.h <= 0. {
        return Err(error(
            "SVG_VIEWBOX",
            "viewBox needs finite coordinates and positive size",
        ));
    }
    let aspect = root
        .attribute("preserveAspectRatio")
        .unwrap_or("xMidYMid meet");
    let tokens: Vec<_> = aspect.split_whitespace().collect();
    if tokens == ["none"] {
        return Ok(Matrix([
            w / vb.w,
            0.,
            0.,
            h / vb.h,
            -vb.x * w / vb.w,
            -vb.y * h / vb.h,
        ]));
    }
    if tokens.is_empty() || tokens.len() > 2 || (tokens.len() == 2 && tokens[1] != "meet") {
        return Err(error(
            "SVG_ASPECT_RATIO",
            "support is limited to none or aligned meet; slice clipping is unsupported",
        ));
    }
    let align = tokens[0];
    let (ax, ay) = match align {
        "xMinYMin" => (0., 0.),
        "xMidYMin" => (0.5, 0.),
        "xMaxYMin" => (1., 0.),
        "xMinYMid" => (0., 0.5),
        "xMidYMid" => (0.5, 0.5),
        "xMaxYMid" => (1., 0.5),
        "xMinYMax" => (0., 1.),
        "xMidYMax" => (0.5, 1.),
        "xMaxYMax" => (1., 1.),
        _ => {
            return Err(error(
                "SVG_ASPECT_RATIO",
                "unsupported preserveAspectRatio alignment",
            ));
        }
    };
    let k = (w / vb.w).min(h / vb.h);
    Ok(Matrix([
        k,
        0.,
        0.,
        k,
        -vb.x * k + ax * (w - vb.w * k),
        -vb.y * k + ay * (h - vb.h * k),
    ]))
}

impl Reader {
    fn walk(
        &mut self,
        node: Node<'_, '_>,
        parent: Matrix,
        inherited: &Style,
        depth: usize,
        root: bool,
    ) -> Result<()> {
        if depth > 64 {
            return Err(error(
                "SVG_RESOURCE_LIMIT",
                "element nesting exceeds 64 levels",
            ));
        }
        if !node.is_element() {
            return Ok(());
        }
        let tag = node.tag_name().name();
        let ns = node.tag_name().namespace();
        if matches!(
            ns,
            Some(
                "http://www.inkscape.org/namespaces/inkscape"
                    | "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
            )
        ) {
            return Ok(());
        }
        if ns != if self.namespaced { Some(SVG_NS) } else { None } {
            return Err(error(
                "SVG_NAMESPACE",
                format!("unsupported element namespace on {tag}"),
            ));
        }
        if matches!(tag, "defs" | "metadata" | "title" | "desc") {
            return Ok(());
        }
        self.serial += 1;
        let id = match node.attribute("id") {
            Some(s) => s.to_owned(),
            None => {
                let mut id = format!("svg-node-{}", self.serial);
                while self.ids.contains(&id) {
                    id.push('_');
                }
                self.ids.insert(id.clone());
                id
            }
        };
        let style = Style::resolve(node, inherited).map_err(|d| d.source(&id))?;
        if style.suppressed {
            self.diagnostics.push(
                error(
                    "SVG_HIDDEN",
                    "display:none or zero opacity excludes this element and its descendants",
                )
                .source(&id)
                .warning(),
            );
            return Ok(());
        }
        let local = node
            .attribute("transform")
            .map(Matrix::parse)
            .transpose()?
            .unwrap_or(Matrix::ID);
        let matrix = parent.then(local);
        matrix.validate()?;
        if tag == "g" || (root && tag == "svg") {
            for child in node.children() {
                self.walk(child, matrix, &style, depth + 1, false)?;
            }
            return Ok(());
        }
        if !style.visible {
            self.diagnostics.push(
                error("SVG_HIDDEN", "visibility excludes this element")
                    .source(&id)
                    .warning(),
            );
            return Ok(());
        }
        if !matches!(tag, "path" | "rect" | "circle" | "ellipse" | "polygon") {
            let code = match tag {
                "text" | "tspan" | "flowRoot" => "SVG_TEXT",
                "line" | "polyline" => "SVG_OPEN_PATH",
                _ => "SVG_UNSUPPORTED_ELEMENT",
            };
            return Err(error(
                code,
                format!(
                    "unsupported <{tag}>; convert visible text/strokes to closed paths in Inkscape"
                ),
            )
            .source(&id));
        }
        if style.stroke != "none" && style.stroke_width > 0. && style.stroke_opacity > 0. {
            return Err(error(
                "SVG_STROKE",
                "visible strokes must be converted with Inkscape Stroke to Path",
            )
            .source(&id));
        }
        if style.fill == "none" || style.fill_opacity == 0. {
            self.diagnostics.push(
                error("SVG_NO_FILL", "element has no visible fill")
                    .source(&id)
                    .warning(),
            );
            return Ok(());
        }
        let alpha = style.paint_alpha().map_err(|d| d.source(&id))?;
        if alpha == 0 {
            self.diagnostics.push(
                error("SVG_NO_FILL", "transparent fill excludes this element")
                    .source(&id)
                    .warning(),
            );
            return Ok(());
        }
        if style.opacity < 1. || style.fill_opacity < 1. || alpha < 255 {
            self.diagnostics.push(error("SVG_OPACITY","positive fill opacity selects the same geometric region; color intensity is not a carve depth").source(&id).warning());
        }
        if node.children().any(|c| {
            c.is_element() && !matches!(c.tag_name().name(), "title" | "desc" | "metadata")
        }) {
            return Err(error(
                "SVG_UNSUPPORTED_CHILD",
                "geometry elements cannot contain nested rendering elements",
            )
            .source(&id));
        }
        let transform = self.page_matrix.then(matrix);
        transform.validate()?;
        let rings = self.shape(node, transform).map_err(|d| d.source(&id))?;
        self.vertices += rings.iter().map(Vec::len).sum::<usize>();
        if self.vertices > MAX_VERTICES {
            return Err(error(
                "SVG_RESOURCE_LIMIT",
                "total flattened input exceeds two million vertices",
            ));
        }
        // We do not emulate viewport clipping. Refuse artwork that could be cropped.
        if rings.iter().flatten().any(|p| {
            p.x < -self.tolerance
                || p.y < -self.tolerance
                || p.x > self.width + self.tolerance
                || p.y > self.height + self.tolerance
        }) {
            return Err(error(
                "SVG_VIEWPORT_CLIPPING",
                "artwork extends outside the page; resize the page to the drawing before import",
            )
            .source(&id));
        }
        let label = node
            .attribute(("http://www.inkscape.org/namespaces/inkscape", "label"))
            .map(str::to_owned);
        self.shapes.push(RawShape {
            id,
            label,
            rings,
            rule: style.rule,
        });
        Ok(())
    }

    fn shape(&self, node: Node<'_, '_>, matrix: Matrix) -> Result<Vec<Vec<Point>>> {
        let length = |name: &str, default: f64| {
            node.attribute(name)
                .map(user_length)
                .transpose()
                .map(|x| x.unwrap_or(default))
        };
        let positive = |v: f64| {
            if v > 0. {
                Ok(v)
            } else {
                Err(error(
                    "SVG_DIMENSION",
                    "visible basic shapes require positive dimensions",
                ))
            }
        };
        let flattener = Flattener::new(matrix, self.tolerance);
        match node.tag_name().name() {
            "path" => flattener.path(node.attribute("d").unwrap_or("")),
            "polygon" => {
                let nums = svgtypes::NumberListParser::from(node.attribute("points").unwrap_or(""))
                    .map(|n| n.map_err(|e| error("SVG_POINTS", format!("{e}"))))
                    .collect::<Result<Vec<_>>>()?;
                if nums.len() < 6 || nums.len() % 2 != 0 {
                    return Err(error(
                        "SVG_POINTS",
                        "polygon requires complete XY pairs for at least three points",
                    ));
                }
                Ok(vec![
                    nums.chunks_exact(2)
                        .map(|xy| matrix.apply(Point::new(xy[0], xy[1])))
                        .collect(),
                ])
            }
            "circle" | "ellipse" => {
                let c = Point::new(length("cx", 0.)?, length("cy", 0.)?);
                let rx = positive(length(
                    if node.tag_name().name() == "circle" {
                        "r"
                    } else {
                        "rx"
                    },
                    0.,
                )?)?;
                let ry = if node.tag_name().name() == "circle" {
                    rx
                } else {
                    positive(length("ry", 0.)?)?
                };
                let mut f = flattener;
                let first = matrix.apply(Point::new(c.x + rx, c.y));
                f.push(first)?;
                f.ellipse(
                    c,
                    [Point::new(rx, 0.), Point::new(0., ry)],
                    0.,
                    std::f64::consts::TAU,
                )?;
                if let Some(p) = f.points.last_mut() {
                    *p = first;
                }
                f.points.pop();
                Ok(vec![f.points])
            }
            "rect" => {
                let x = length("x", 0.)?;
                let y = length("y", 0.)?;
                let w = positive(length("width", 0.)?)?;
                let h = positive(length("height", 0.)?)?;
                let rx = length("rx", length("ry", 0.)?)?;
                let ry = length("ry", rx)?;
                if rx < 0. || ry < 0. {
                    return Err(error("SVG_DIMENSION", "corner radii must be nonnegative"));
                }
                let rx = rx.min(w / 2.);
                let ry = ry.min(h / 2.);
                if rx == 0. || ry == 0. {
                    return Ok(vec![
                        [
                            Point::new(x, y),
                            Point::new(x + w, y),
                            Point::new(x + w, y + h),
                            Point::new(x, y + h),
                        ]
                        .map(|p| matrix.apply(p))
                        .to_vec(),
                    ]);
                }
                let d = format!(
                    "M {} {} H {} A {rx} {ry} 0 0 1 {} {} V {} A {rx} {ry} 0 0 1 {} {} H {} A {rx} {ry} 0 0 1 {} {} V {} A {rx} {ry} 0 0 1 {} {} Z",
                    x + rx,
                    y,
                    x + w - rx,
                    x + w,
                    y + ry,
                    y + h - ry,
                    x + w - rx,
                    y + h,
                    x + rx,
                    x,
                    y + h - ry,
                    y + ry,
                    x + rx,
                    y
                );
                flattener.path(&d)
            }
            _ => unreachable!(),
        }
    }
}
