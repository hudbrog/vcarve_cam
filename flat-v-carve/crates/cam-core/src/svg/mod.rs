//! Supported-subset SVG importer. XML is data only; no filesystem or network access.
mod path;
mod style;
use crate::geometry::spatial::{Aabb, SpatialIndex};
use crate::geometry::{BooleanOp, Diagnostic, Grid, Point, Region, Result, WindingRule};
use path::{ChainPoints, Flattener, MAX_VERTICES, Matrix};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use style::{Style, Stylesheet};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const INKSCAPE_NS: &str = "http://www.inkscape.org/namespaces/inkscape";
const SODIPODI_NS: &str = "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd";
pub const MAX_SVG_BYTES: usize = 32_000_000;

pub(super) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("svg")
}

/// Every element-scoped diagnostic names the element the user has to find in
/// the file: its `id` and, when the editor recorded one, its `inkscape:label`.
fn element_scope(mut diagnostic: Diagnostic, node: Node<'_, '_>, id: &str) -> Diagnostic {
    diagnostic.source_id = Some(id.to_owned());
    let label = node
        .attribute((INKSCAPE_NS, "label"))
        .map(str::trim)
        .filter(|label| !label.is_empty());
    let name = match label {
        Some(label) => format!("element id '{id}' (label \"{label}\")"),
        None => format!("element id '{id}'"),
    };
    diagnostic.message = format!("{name}: {}", diagnostic.message);
    diagnostic
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Workpiece coordinate = scale * rotate(artwork XY - origin_mm), where
    /// artwork XY is the page in millimetres with Y up (see
    /// [`page_to_artwork`]).
    pub origin_mm: Point,
    pub scale: f64,
    pub rotation_deg: f64,
}

/// The one conversion from SVG (document) coordinates to artwork (model)
/// coordinates.
///
/// The SVG page is measured in millimetres with Y down and the origin at the
/// visual top-left; the artwork is the same page with Y up and the origin at
/// the page's bottom-left. Y is therefore flipped exactly once, about the
/// physical page height, so the page's top edge becomes the artwork's maximum
/// Y. Every later stage — placement, stock, planning, preview and post —
/// consumes artwork/setup millimetres and never flips or rescales again.
pub fn page_to_artwork(page: Point, page_height_mm: f64) -> Point {
    Point::new(page.x, page_height_mm - page.y)
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
    /// Convert a page point through the same placement used by SVG import.
    pub fn to_setup(&self, page: Point) -> Result<Point> {
        if !page.finite() {
            return Err(error("SVG_PLACEMENT", "point must be finite"));
        }
        Ok(self.matrix()?.apply(page))
    }
    /// Inverse of `scale * rotate(page - origin_mm)` for editor gestures.
    pub fn to_page(&self, setup: Point) -> Result<Point> {
        self.matrix()?;
        if !setup.finite() {
            return Err(error("SVG_PLACEMENT", "point must be finite"));
        }
        let (s, c) = self.rotation_deg.to_radians().sin_cos();
        let x = setup.x / self.scale;
        let y = setup.y / self.scale;
        Ok(Point::new(
            c * x + s * y + self.origin_mm.x,
            -s * x + c * y + self.origin_mm.y,
        ))
    }
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

/// What the importer needs to turn an SVG file into artwork. There is no
/// interpretation mode: the importer publishes every reading the drawing
/// supports — each subpath as a centreline, each filled subpath as a region —
/// and the operation that consumes the artwork decides which reading it uses.
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

/// A solid paint taken from the drawing, kept for display only.
///
/// Geometry is decided by whether a paint is *present*, never by which colour
/// it is: two shapes that differ only in colour describe the same cutting
/// geometry. The colour still matters to the person picking that geometry, so
/// it is carried through as identity — the artwork panel and the viewport can
/// show the carving and the part outline apart at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePaint {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl SourcePaint {
    /// Parse a paint value, resolving `currentColor` against the inherited
    /// colour. Anything that is not a solid colour (a paint server) has no
    /// single colour to show and returns `None`.
    fn parse(value: &str, current_color: &str) -> Option<Self> {
        let value = if value == "currentColor" {
            current_color
        } else {
            value
        };
        value.parse::<svgtypes::Color>().ok().map(|color| Self {
            red: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha,
        })
    }
    /// The colour in the 0..1 RGBA the viewport draws with.
    pub fn rgba_unit(self) -> [f32; 4] {
        [
            f32::from(self.red) / 255.,
            f32::from(self.green) / 255.,
            f32::from(self.blue) / 255.,
            f32::from(self.alpha) / 255.,
        ]
    }
    /// How the colour is written in the panel (`#rrggbb`).
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
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
    /// The element's own `inkscape:label`, when the editor recorded one.
    pub label: Option<String>,
    /// The nearest enclosing named layer or group.
    pub group: Option<String>,
    /// The colour the element is drawn in, for identity only.
    pub paint: Option<SourcePaint>,
    pub geometry: Region,
}
/// One subpath in page coordinates, kept exactly as drawn: open subpaths stay
/// open (`closed` is false) and vertex order is source order. A stroked
/// subpath is a centreline — the stroke width is never offset into two
/// parallel cuts (plan section 7.3) — and a filled subpath is also a region.
#[derive(Clone, Debug, Serialize)]
pub struct SourceChain {
    pub id: String,
    pub source_id: String,
    pub label: Option<String>,
    pub group: Option<String>,
    pub paint: Option<SourcePaint>,
    pub closed: bool,
    pub points: Vec<Point>,
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
    /// Every subpath as drawn, one per subpath, in document and subpath order.
    /// A knife cuts these; a profile may cut the closed ones as contours.
    pub chains: Vec<SourceChain>,
    pub selected_region_ids: Vec<String>,
    pub selected: Region,
    pub components: Vec<MappedComponent>,
    pub bounds: Option<Bounds>,
    pub diagnostics: Vec<Diagnostic>,
}

struct RawShape {
    id: String,
    label: Option<String>,
    group: Option<String>,
    paint: Option<SourcePaint>,
    rings: Vec<Vec<Point>>,
    rule: WindingRule,
}
struct RawChain {
    id: String,
    label: Option<String>,
    group: Option<String>,
    paint: Option<SourcePaint>,
    closed: bool,
    points: Vec<Point>,
}
struct Reader {
    shapes: Vec<RawShape>,
    chains: Vec<RawChain>,
    diagnostics: Vec<Diagnostic>,
    vertices: usize,
    serial: usize,
    tolerance: f64,
    ids: BTreeSet<String>,
    page_matrix: Matrix,
    width: f64,
    height: f64,
    namespaced: bool,
    stylesheet: Stylesheet,
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
    let mut style_texts: Vec<String> = vec![];
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
            // `<style>` is data, parsed below and applied through the same
            // cascade as presentation attributes and inline styles.
            style_texts.push(
                node.children()
                    .filter(|child| child.is_text())
                    .filter_map(|child| child.text())
                    .collect(),
            );
        }
    }
    let (stylesheet, stylesheet_warnings) = Stylesheet::parse(&style_texts)?;
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
        chains: vec![],
        diagnostics: vec![],
        vertices: 0,
        serial: 0,
        tolerance: options.geometry_tolerance_mm / (4. * options.placement.scale),
        ids,
        page_matrix,
        width,
        height,
        namespaced: root.tag_name().namespace() == Some(SVG_NS),
        stylesheet,
    };
    reader.diagnostics.extend(stylesheet_warnings);
    reader.walk(root, Matrix::ID, &Style::default(), 0, true, None)?;
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
                group: shape.group.clone(),
                paint: shape.paint,
                geometry,
            });
        }
    }
    Region::check_snapping(grid, &placed_paths)?;
    // Centerline chains: page-space, in document and subpath order, one ID
    // per subpath. They never pass through region resolution.
    let mut chains = vec![];
    let mut element = String::new();
    let mut subpath = 0usize;
    for chain in reader.chains {
        if chain.id != element {
            element = chain.id.clone();
            subpath = 0;
        } else {
            subpath += 1;
        }
        chains.push(SourceChain {
            id: format!("{}::chain-{subpath}", chain.id),
            source_id: chain.id,
            label: chain.label,
            group: chain.group,
            paint: chain.paint,
            closed: chain.closed,
            points: chain.points,
        });
    }
    if sources.is_empty() && chains.is_empty() {
        return Err(error(
            "SVG_NO_REGIONS",
            "SVG has no supported visible geometry: nothing is filled and nothing is stroked",
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
        chains,
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
        group: Option<&str>,
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
        if matches!(ns, Some(INKSCAPE_NS | SODIPODI_NS)) {
            return Ok(());
        }
        if ns != if self.namespaced { Some(SVG_NS) } else { None } {
            return Err(error(
                "SVG_NAMESPACE",
                format!("unsupported element namespace on {tag}"),
            ));
        }
        // Editor metadata and the parsed stylesheet carry no geometry.
        if matches!(tag, "defs" | "metadata" | "title" | "desc" | "style") {
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
        let (style, style_warnings) = Style::resolve(node, inherited, &self.stylesheet)
            .map_err(|d| element_scope(d, node, &id))?;
        self.diagnostics.extend(
            style_warnings
                .into_iter()
                .map(|d| element_scope(d, node, &id)),
        );
        if style.suppressed {
            self.diagnostics.push(element_scope(
                error(
                    "SVG_HIDDEN",
                    "display:none or zero opacity excludes this element and its descendants",
                )
                .warning(),
                node,
                &id,
            ));
            return Ok(());
        }
        let local = node
            .attribute("transform")
            .map(Matrix::parse)
            .transpose()?
            .unwrap_or(Matrix::ID);
        let matrix = parent.then(local);
        matrix.validate()?;
        let transform = self.page_matrix.then(matrix);
        transform.validate()?;
        if tag == "g" || (root && tag == "svg") {
            // A named layer or group is how the drawing is organised; it is
            // part of the geometry's identity, so it travels to every element
            // inside it. The nearest name wins.
            let group = node
                .attribute((INKSCAPE_NS, "label"))
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .or(group);
            for child in node.children() {
                self.walk(child, matrix, &style, depth + 1, false, group)?;
            }
            return Ok(());
        }
        if !style.visible {
            self.diagnostics.push(element_scope(
                error("SVG_HIDDEN", "visibility excludes this element").warning(),
                node,
                &id,
            ));
            return Ok(());
        }
        if !matches!(
            tag,
            "path" | "rect" | "circle" | "ellipse" | "polygon" | "line" | "polyline"
        ) {
            let code = match tag {
                "text" | "tspan" | "flowRoot" => "SVG_TEXT",
                _ => "SVG_UNSUPPORTED_ELEMENT",
            };
            return Err(element_scope(
                error(
                    code,
                    format!(
                        "unsupported <{tag}>; convert visible text/strokes to closed paths in Inkscape"
                    ),
                ),
                node,
                &id,
            ));
        }
        let has_stroke =
            style.stroke != "none" && style.stroke_width > 0. && style.stroke_opacity > 0.;
        let painted_fill = style.fill != "none" && style.fill_opacity != 0.;
        let (alpha, assumed_opaque) = if painted_fill {
            style
                .paint_alpha()
                .map_err(|d| element_scope(d, node, &id))?
        } else {
            (255, false)
        };
        if assumed_opaque {
            self.diagnostics.push(element_scope(
                error(
                    "SVG_PAINT_OPACITY",
                    "gradient or pattern fill is imported as an opaque region; its opacity is not evaluated, so the whole drawn area is carved",
                )
                .warning(),
                node,
                &id,
            ));
        }
        // A <line> has no interior, so its fill never paints anything; the
        // other shapes read their fill as a region.
        let has_fill = painted_fill && alpha != 0 && tag != "line";
        // The colour the element is drawn in, carried as identity only: a
        // paint server has no single colour, and a fully transparent fill
        // shows nothing, in which case the stroke's colour stands in.
        let paint = SourcePaint::parse(&style.fill, &style.color)
            .filter(|paint| paint.alpha > 0)
            .or_else(|| {
                has_stroke
                    .then(|| SourcePaint::parse(&style.stroke, &style.color))
                    .flatten()
                    .filter(|paint| paint.alpha > 0)
            });
        if !has_stroke && !has_fill {
            // Nothing is drawn: no stroke to follow and no solid fill to
            // carve. Say so instead of inventing geometry.
            self.diagnostics.push(element_scope(
                error(
                    "SVG_NO_PAINT",
                    "element has no visible fill or stroke, so it contributes no geometry",
                )
                .warning(),
                node,
                &id,
            ));
            return Ok(());
        }
        if style.opacity < 1. || (painted_fill && (style.fill_opacity < 1. || alpha < 255)) {
            self.diagnostics.push(element_scope(error("SVG_OPACITY","positive fill opacity selects the same geometric region; color intensity is not a carve depth").warning(), node, &id));
        }
        if node.children().any(|c| {
            c.is_element() && !matches!(c.tag_name().name(), "title" | "desc" | "metadata")
        }) {
            return Err(element_scope(
                error(
                    "SVG_UNSUPPORTED_CHILD",
                    "geometry elements cannot contain nested rendering elements",
                ),
                node,
                &id,
            ));
        }
        // One flattening pass: every subpath as drawn. Each reading below is
        // derived from it, so an element can be an area and a line at once.
        let chains = self
            .flatten(node, transform)
            .map_err(|d| element_scope(d, node, &id))?;
        self.vertices += chains.iter().map(|c| c.points.len()).sum::<usize>();
        if self.vertices > MAX_VERTICES {
            return Err(error(
                "SVG_RESOURCE_LIMIT",
                "total flattened input exceeds two million vertices",
            ));
        }
        // We do not emulate viewport clipping. Refuse artwork that could be cropped.
        if chains.iter().flat_map(|c| c.points.iter()).any(|p| {
            p.x < -self.tolerance
                || p.y < -self.tolerance
                || p.x > self.width + self.tolerance
                || p.y > self.height + self.tolerance
        }) {
            return Err(element_scope(
                error(
                    "SVG_VIEWPORT_CLIPPING",
                    "artwork extends outside the page; resize the page to the drawing before import",
                ),
                node,
                &id,
            ));
        }
        let label = node.attribute((INKSCAPE_NS, "label")).map(str::to_owned);
        // The centreline reading: every subpath as drawn, open or closed. It
        // is what a drag knife follows and what a stroke means; a filled
        // element reads the same way, because its path is the boundary the
        // user drew.
        if has_stroke {
            self.diagnostics.push(element_scope(
                error(
                    "SVG_STROKE_CENTERLINE",
                    format!(
                        "stroke read as a centreline: the {} mm stroke width is ignored, so the line is cut where it is drawn (use Inkscape's Stroke to Path if you want the outline instead)",
                        style.stroke_width
                    ),
                )
                .warning(),
                node,
                &id,
            ));
        }
        let mut dropped = 0usize;
        for chain in &chains {
            if chain.points.len() < 2 {
                dropped += 1;
                continue;
            }
            self.chains.push(RawChain {
                id: id.clone(),
                label: label.clone(),
                group: group.map(str::to_owned),
                paint,
                closed: chain.closed,
                points: chain.points.clone(),
            });
        }
        // The fill reading: the same subpaths taken as rings. A renderer closes
        // an open subpath implicitly when it fills it, so CAM does too — out
        // loud, because that closing edge is not in the file.
        let mut implicitly_closed = 0usize;
        let mut rings: Vec<Vec<Point>> = vec![];
        if has_fill {
            for chain in &chains {
                if chain.points.len() < 3 {
                    dropped += 1;
                    continue;
                }
                if !chain.closed {
                    implicitly_closed += 1;
                }
                rings.push(chain.points.clone());
            }
            if rings.is_empty() {
                self.diagnostics.push(element_scope(
                    error(
                        "SVG_NO_FILL",
                        "filled element has no subpath with enough vertices to enclose an area",
                    )
                    .warning(),
                    node,
                    &id,
                ));
            }
        }
        if implicitly_closed > 0 {
            self.diagnostics.push(element_scope(
                error(
                    "SVG_OPEN_PATH",
                    format!(
                        "{implicitly_closed} open subpath(s) carry a fill: every renderer closes them for filling, so that closing edge is added. Close the path in Inkscape, or draw it with a stroke if you meant a line."
                    ),
                )
                .warning(),
                node,
                &id,
            ));
        }
        if dropped > 0 {
            self.diagnostics.push(element_scope(
                error(
                    "SVG_DEGENERATE_PATH",
                    format!("{dropped} subpath(s) have too few distinct vertices to use"),
                )
                .warning(),
                node,
                &id,
            ));
        }
        if has_fill && !rings.is_empty() {
            self.shapes.push(RawShape {
                id,
                label,
                group: group.map(str::to_owned),
                paint,
                rings,
                rule: style.rule,
            });
        }
        Ok(())
    }

    /// Flatten one element into its subpaths as drawn. Subpath order is the
    /// source order; open subpaths stay open and nothing is closed or opened
    /// implicitly (plan section 7.3). This is the single geometry extraction
    /// both readings share: a subpath is a centreline as drawn, and a ring
    /// when the element carries a fill.
    fn flatten(&self, node: Node<'_, '_>, transform: Matrix) -> Result<Vec<ChainPoints>> {
        let points = |list: &str, code: &str| -> Result<Vec<Point>> {
            let numbers = svgtypes::NumberListParser::from(list)
                .map(|n| n.map_err(|e| error(code, format!("{e}"))))
                .collect::<Result<Vec<_>>>()?;
            Ok(numbers
                .chunks_exact(2)
                .map(|xy| transform.apply(Point::new(xy[0], xy[1])))
                .collect())
        };
        Ok(match node.tag_name().name() {
            "path" => {
                Flattener::new(transform, self.tolerance).path(node.attribute("d").unwrap_or(""))?
            }
            "line" => {
                let at = |name: &str| {
                    node.attribute(name)
                        .map(user_length)
                        .transpose()
                        .map(|v| v.unwrap_or(0.))
                };
                vec![ChainPoints {
                    closed: false,
                    points: vec![
                        transform.apply(Point::new(at("x1")?, at("y1")?)),
                        transform.apply(Point::new(at("x2")?, at("y2")?)),
                    ],
                }]
            }
            "polyline" => {
                let list = points(node.attribute("points").unwrap_or(""), "SVG_POINTS")?;
                if list.len() < 2 {
                    return Err(error(
                        "SVG_POINTS",
                        "polyline requires complete XY pairs for at least two points",
                    ));
                }
                vec![ChainPoints {
                    closed: false,
                    points: list,
                }]
            }
            "polygon" => {
                let mut list = points(node.attribute("points").unwrap_or(""), "SVG_POINTS")?;
                if list.len() < 3 {
                    return Err(error(
                        "SVG_POINTS",
                        "polygon requires complete XY pairs for at least three points",
                    ));
                }
                // A polygon is closed by definition; do not duplicate the
                // closing vertex.
                if list.first() == list.last() {
                    list.pop();
                }
                vec![ChainPoints {
                    closed: true,
                    points: list,
                }]
            }
            // Closed basic shapes: one closed subpath each.
            "rect" | "circle" | "ellipse" => self.shape(node, transform)?,
            _ => unreachable!("dispatch checked the tag"),
        })
    }

    /// The closed outline of a basic shape, as one chain.
    fn shape(&self, node: Node<'_, '_>, matrix: Matrix) -> Result<Vec<ChainPoints>> {
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
                Ok(vec![ChainPoints {
                    closed: true,
                    points: f.points,
                }])
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
                    return Ok(vec![ChainPoints {
                        closed: true,
                        points: [
                            Point::new(x, y),
                            Point::new(x + w, y),
                            Point::new(x + w, y + h),
                            Point::new(x, y + h),
                        ]
                        .map(|p| matrix.apply(p))
                        .to_vec(),
                    }]);
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
