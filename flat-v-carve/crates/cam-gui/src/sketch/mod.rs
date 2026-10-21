//! Local sketch transactions with Arael solving. Only Apply crosses into the job.
mod canvas;
mod constraints;
mod shortcut_actions;
mod shortcuts;
mod solve;
#[cfg(test)]
mod tests;
mod tools_ui;
pub mod worker;

use cam_core::{geometry::Point, project::v5::ArtworkItemId, sketch::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedDraft {
    pub target: Option<ArtworkItemId>,
    pub source_revision: Option<String>,
    pub name: String,
    pub model: SketchDocument,
    /// A recovered first sketch can still be cancelled back to the welcome screen.
    pub had_document: bool,
    #[serde(default)]
    pub constraint_form: constraints::Form,
}
impl SavedDraft {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.len() > 1000
            || self.constraint_form.value.len() > 128
            || self.constraint_form.operands.len() > MAX_ENTITIES * 4
            || self.constraint_form.operands.iter().any(|o| {
                reference_entity(o.entity()).is_none()
                    && (o.entity().len() > 80 || !cam_core::preview::valid_id(o.entity()))
            })
            || self
                .constraint_form
                .editing
                .as_ref()
                .is_some_and(|id| id.len() > 80 || !cam_core::preview::valid_id(id))
            || (self.target.is_some() && !self.had_document)
            || self
                .target
                .as_ref()
                .is_some_and(|id| !cam_core::preview::valid_id(&id.0))
            || self.target.is_some() != self.source_revision.is_some()
            || self
                .source_revision
                .as_ref()
                .is_some_and(|r| r.len() != 64 || !r.bytes().all(|c| c.is_ascii_hexdigit()))
            || serde_json::to_vec(&self.model)
                .map_err(|e| e.to_string())?
                .len()
                > MAX_SKETCH_BYTES
        {
            return Err("Invalid saved sketch draft".into());
        }
        self.model.validate_structure().map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Line,
    Polyline,
    Rectangle,
    Circle,
    Arc,
    Point,
}
impl Tool {
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::Select, "Select"),
        (Self::Line, "Line"),
        (Self::Polyline, "Polyline"),
        (Self::Rectangle, "Rectangle"),
        (Self::Circle, "Circle"),
        (Self::Arc, "Arc"),
        (Self::Point, "Point"),
    ];
    pub fn hint(self) -> &'static str {
        match self {
            Self::Select => {
                "Click an edge or point · Shift-click adds · Double-click selects wire · Drag empty space to box-select"
            }
            Self::Line => "Click start and end · Snapped open endpoints join wires",
            Self::Polyline => {
                "Click successive vertices · Click the first vertex to close · Enter finishes an open wire"
            }
            Self::Rectangle => "Click two opposite corners",
            Self::Circle => "Click centre, then a point on the circle",
            Self::Arc => "Click start, a point along the arc, then end",
            Self::Point => "Click to place a reference point (no machining geometry)",
        }
    }
}

pub struct Editor {
    pub saved: SavedDraft,
    pub tool: Tool,
    pub construction: bool,
    pub snapping: bool,
    pub grid_snap: bool,
    pub grid_mm: f64,
    pub selected: BTreeSet<String>,
    pub message: String,
    pub show_constraints: bool,
    pub show_geometry: bool,
    pub dimension_filter: u8,
    pub(super) shortcut_prefix: Option<(Vec<egui::Key>, f64)>,
    pub show_shortcuts: bool,
    pub(super) new_dimensions_reference: bool,
    pub(super) box_start: Option<egui::Pos2>,
    pub(super) focus_dimension: bool,
    solver: solve::Solver,
    pub(crate) undo: Vec<SketchDocument>,
    pub(crate) redo: Vec<SketchDocument>,
    pub(crate) points: Vec<Point>,
    pub(crate) view: canvas::View,
    drag: Option<solve::Drag>,
}
impl Editor {
    pub fn new(saved: SavedDraft) -> Self {
        let selected = saved
            .constraint_form
            .operands
            .iter()
            .map(|o| o.entity().to_owned())
            .collect();
        Self {
            saved,
            tool: Tool::Select,
            construction: false,
            snapping: true,
            grid_snap: true,
            grid_mm: 1.,
            selected,
            message: String::new(),
            show_constraints: false,
            show_geometry: false,
            dimension_filter: 0,
            shortcut_prefix: None,
            show_shortcuts: false,
            new_dimensions_reference: false,
            box_start: None,
            focus_dimension: false,
            solver: Default::default(),
            undo: vec![],
            redo: vec![],
            points: vec![],
            view: Default::default(),
            drag: None,
        }
    }
    pub fn history_counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn pending(&self) -> bool {
        !self.points.is_empty() || self.drag.is_some()
    }
    pub fn cancel_gesture(&mut self) {
        self.points.clear();
        self.box_start = None;
        if let Some(drag) = self.drag.take() {
            self.saved.model = drag.before;
            self.invalidate_solve();
        }
    }
    pub fn set_tool(&mut self, tool: Tool) {
        if tool != Tool::Select && !self.finish_dimension_edit() {
            return;
        }
        self.cancel_gesture();
        self.tool = tool;
        self.shortcut_prefix = None;
        self.message.clear();
    }
    pub fn fit(&mut self) {
        self.view.scale = 0.;
        self.view.auto_fit = true;
    }
    fn id(&self, prefix: &str) -> Result<String, String> {
        loop {
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes)
                .map_err(|e| format!("Cannot allocate a sketch identity: {e}"))?;
            let id = format!("{prefix}{:032x}", u128::from_le_bytes(bytes));
            if !self.saved.model.entities.iter().any(|e| e.id == id)
                && !self.saved.model.wires.iter().any(|w| w.id == id)
                && !self.saved.model.constraints.iter().any(|c| c.id == id)
            {
                return Ok(id);
            }
        }
    }
    fn trim(history: &mut Vec<SketchDocument>) {
        while history.len() > 64
            || history
                .iter()
                .map(|m| serde_json::to_vec(m).map_or(MAX_SKETCH_BYTES, |b| b.len()))
                .sum::<usize>()
                > 4_000_000
        {
            history.remove(0);
        }
    }
    fn commit(&mut self, before: SketchDocument) -> Result<(), String> {
        if let Err(e) = self.saved.model.validate_structure() {
            self.saved.model = before;
            return Err(e.to_string());
        }
        if before != self.saved.model {
            self.record_history(before);
            self.invalidate_solve();
        }
        Ok(())
    }
    pub fn undo(&mut self) {
        self.cancel_gesture();
        if let Some(model) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.saved.model, model));
            self.selected.clear();
            self.message.clear();
            self.invalidate_solve();
            self.refresh_form();
        }
    }
    pub fn redo(&mut self) {
        self.cancel_gesture();
        if let Some(model) = self.redo.pop() {
            self.undo
                .push(std::mem::replace(&mut self.saved.model, model));
            self.selected.clear();
            self.message.clear();
            self.invalidate_solve();
            self.refresh_form();
        }
    }
    pub fn select_wire(&mut self, id: Option<&str>, additive: bool) {
        if !self.finish_dimension_edit() {
            return;
        }
        if let Some(id) = id
            && let Some(reference) = reference_entity(id)
        {
            self.pick_operand(Some(constraints::Operand::reference(reference)), additive);
            return;
        }
        let group = id.map(|id| self.group(id)).unwrap_or_default();
        let remove = additive && group.iter().all(|id| self.selected.contains(id));
        if !additive {
            self.selected.clear();
        }
        for id in group {
            if remove {
                self.selected.remove(&id);
            } else {
                self.selected.insert(id);
            }
        }
        self.saved.constraint_form.operands = self
            .selected
            .iter()
            .cloned()
            .map(constraints::Operand::Entity)
            .collect();
        self.saved.constraint_form.editing = None;
        self.saved.constraint_form.value.clear();
        self.saved.constraint_form.changed = false;
    }
    fn group(&self, id: &str) -> BTreeSet<String> {
        self.saved
            .model
            .wires
            .iter()
            .find(|w| w.edges.iter().any(|e| e.entity_id == id))
            .map(|w| w.edges.iter().map(|e| e.entity_id.clone()).collect())
            .unwrap_or_else(|| BTreeSet::from([id.into()]))
    }
    fn guard_selection(&self) -> Result<(), String> {
        if self.saved.model.constraints.iter().any(|c| {
            constraint_entities(&c.definition)
                .iter()
                .any(|id| self.selected.contains(*id))
        }) {
            Err(
                "This wire has saved constraints. Remove its constraints before deleting it."
                    .into(),
            )
        } else {
            Ok(())
        }
    }
    pub fn delete(&mut self) -> Result<(), String> {
        self.cancel_gesture();
        self.require_wire_selection()?;
        self.guard_selection()?;
        let before = self.saved.model.clone();
        self.saved
            .model
            .entities
            .retain(|e| !self.selected.contains(&e.id));
        self.saved
            .model
            .wires
            .retain(|w| !w.edges.iter().any(|e| self.selected.contains(&e.entity_id)));
        self.selected.clear();
        self.saved.constraint_form = Default::default();
        self.commit(before)
    }
    pub fn toggle_construction(&mut self) -> Result<(), String> {
        self.cancel_gesture();
        self.require_wire_selection()?;
        let before = self.saved.model.clone();
        let construction = !self
            .saved
            .model
            .entities
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .all(|e| e.construction);
        for e in &mut self.saved.model.entities {
            if self.selected.contains(&e.id) {
                e.construction = construction;
            }
        }
        self.commit(before)
    }
    fn require_wire_selection(&self) -> Result<(), String> {
        if self
            .selected
            .iter()
            .any(|id| reference_entity(id).is_some())
        {
            return Err("Origin and axes are fixed construction geometry; they cannot be deleted or converted.".into());
        }
        if self
            .saved
            .constraint_form
            .operands
            .iter()
            .any(|o| matches!(o, constraints::Operand::Point(p) if p.point != PointRole::Position))
        {
            return Err(
                "Select geometry bodies for this action. Double-click an edge to select its wire."
                    .into(),
            );
        }
        if self
            .selected
            .iter()
            .any(|id| !self.group(id).is_subset(&self.selected))
        {
            return Err("Select the complete wire for this action: double-click an edge, or use Select wire. No geometry changed.".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn translate(&mut self, origin: &SketchDocument, delta: Point) -> Result<(), String> {
        self.require_wire_selection()?;
        let mut next = origin.clone();
        for e in &mut next.entities {
            if !self.selected.contains(&e.id) {
                continue;
            }
            let move_point = |p: &mut Point| {
                p.x += delta.x;
                p.y += delta.y;
            };
            match &mut e.geometry {
                SketchGeometry::Line { start, end } => {
                    move_point(start);
                    move_point(end);
                }
                SketchGeometry::Circle { center, .. } | SketchGeometry::Arc { center, .. } => {
                    move_point(center)
                }
                SketchGeometry::Point { position } => move_point(position),
            }
        }
        next.require_valid_solution().map_err(|e| e.to_string())?;
        self.saved.model = next;
        Ok(())
    }
    pub fn click(&mut self, point: Point) -> Result<(), String> {
        self.message.clear();
        if self.points.last().is_some_and(|p| p.distance(point) < 1e-8) {
            return Err("Choose a distinct point".into());
        }
        let n = self.points.len();
        match self.tool {
            Tool::Select => {}
            Tool::Point => self.add_path(vec![SketchGeometry::Point { position: point }], false)?,
            Tool::Line | Tool::Polyline if n > 0 => {
                let start = self.points[n - 1];
                self.add_path(vec![SketchGeometry::Line { start, end: point }], false)?;
                if self.tool == Tool::Line || self.points[0].distance(point) < 1e-8 {
                    self.points.clear();
                } else {
                    self.points.push(point);
                }
            }
            Tool::Rectangle if n == 1 => {
                let a = self.points[0];
                if (a.x - point.x).abs() < 1e-8 || (a.y - point.y).abs() < 1e-8 {
                    return Err("Rectangle needs width and height".into());
                }
                let corners = [
                    a,
                    Point::new(point.x, a.y),
                    point,
                    Point::new(a.x, point.y),
                    a,
                ];
                self.add_path(
                    corners
                        .windows(2)
                        .map(|p| SketchGeometry::Line {
                            start: p[0],
                            end: p[1],
                        })
                        .collect(),
                    true,
                )?;
                self.points.clear();
            }
            Tool::Circle if n == 1 => {
                let center = self.points[0];
                self.add_path(
                    vec![SketchGeometry::Circle {
                        center,
                        radius_mm: center.distance(point),
                    }],
                    true,
                )?;
                self.points.clear();
            }
            Tool::Arc if n == 2 => {
                let arc = arc_through(self.points[0], self.points[1], point)?;
                self.add_path(vec![arc], false)?;
                self.points.clear();
            }
            _ => self.points.push(point),
        }
        Ok(())
    }
    fn add_path(&mut self, geometries: Vec<SketchGeometry>, closed: bool) -> Result<(), String> {
        let before = self.saved.model.clone();
        let mut edges = vec![];
        let mut entities = vec![];
        let mut constraints = vec![];
        let rectangle = self.tool == Tool::Rectangle && closed && geometries.len() == 4;
        let wire_id = self.id("w")?;
        for geometry in geometries {
            let id = self.id("e")?;
            if rectangle {
                let (start, end) = geometry.line().ok_or("Rectangle needs line edges")?;
                constraints.push(Constraint {
                    id: self.id("c")?,
                    reference: false,
                    definition: if start.y == end.y {
                        ConstraintKind::Horizontal { line: id.clone() }
                    } else {
                        ConstraintKind::Vertical { line: id.clone() }
                    },
                });
            }
            if !matches!(geometry, SketchGeometry::Point { .. }) {
                edges.push(WireEdge {
                    entity_id: id.clone(),
                    reversed: false,
                });
            }
            entities.push(Entity {
                id,
                label: None,
                construction: self.construction,
                geometry,
            });
        }
        self.saved.model.entities.extend(entities);
        // Corner coincidence is already enforced by the closed wire. H/V leaves
        // width, height and position free while preserving the rectangle shape.
        self.saved.model.constraints.extend(constraints);
        if !edges.is_empty() {
            self.saved.model.wires.push(Wire {
                id: wire_id.clone(),
                label: None,
                edges,
                closed,
            });
            join_wires(&mut self.saved.model, wire_id);
        }
        self.commit(before)
    }
}

fn constraint_entities(c: &ConstraintKind) -> Vec<&String> {
    use ConstraintKind::*;
    match c {
        Coincident { a, b }
        | Distance { a, b, .. }
        | HorizontalDistance { a, b, .. }
        | VerticalDistance { a, b, .. } => vec![&a.entity_id, &b.entity_id],
        Horizontal { line } | Vertical { line } | Length { line, .. } => vec![line],
        Parallel { a, b }
        | Perpendicular { a, b }
        | EqualLength { a, b }
        | EqualRadius { a, b }
        | Angle { a, b, .. } => vec![a, b],
        Tangent { line, curve } => vec![line, curve],
        PointOnLine { point, line } | Midpoint { point, line } => vec![&point.entity_id, line],
        PointOnCircle { point, curve } => vec![&point.entity_id, curve],
        Symmetry { a, b, line } => vec![&a.entity_id, &b.entity_id, line],
        Radius { curve, .. } | Diameter { curve, .. } => vec![curve],
        Fixed { entity, .. } => vec![entity],
        FixedPoint { point, .. } => vec![&point.entity_id],
    }
}

fn ends(model: &SketchDocument, wire: &Wire) -> Option<(Point, Point)> {
    let a = wire.edges.first()?;
    let b = wire.edges.last()?;
    Some((
        model
            .entity(&a.entity_id)
            .ok()?
            .geometry
            .point(if a.reversed {
                PointRole::End
            } else {
                PointRole::Start
            })?,
        model
            .entity(&b.entity_id)
            .ok()?
            .geometry
            .point(if b.reversed {
                PointRole::Start
            } else {
                PointRole::End
            })?,
    ))
}
fn reverse(wire: &mut Wire) {
    wire.edges.reverse();
    for e in &mut wire.edges {
        e.reversed = !e.reversed;
    }
}
/// Join only analytic endpoints, never a screen-space tolerance. Existing wire
/// IDs survive extension; ambiguous branch junctions are left separate.
fn join_wires(model: &mut SketchDocument, mut active_id: String) {
    loop {
        let active = model
            .wires
            .iter()
            .position(|w| w.id == active_id)
            .expect("new or extended wire");
        if !model.wires[active].closed
            && ends(model, &model.wires[active]).is_some_and(|(a, b)| a.distance(b) < 1e-8)
        {
            model.wires[active].closed = true;
        }
        let mut pair = None;
        'outer: for i in 0..model.wires.len() {
            if model.wires[i].closed {
                continue;
            }
            let Some((a, b)) = ends(model, &model.wires[i]) else {
                continue;
            };
            for j in i + 1..model.wires.len() {
                if i != active && j != active {
                    continue;
                }
                if model.wires[j].closed {
                    continue;
                }
                let Some((c, d)) = ends(model, &model.wires[j]) else {
                    continue;
                };
                let ci = model
                    .entity(&model.wires[i].edges[0].entity_id)
                    .unwrap()
                    .construction;
                let cj = model
                    .entity(&model.wires[j].edges[0].entity_id)
                    .unwrap()
                    .construction;
                if ci != cj {
                    continue;
                }
                for (p, q, ri, rj) in [
                    (b, c, false, false),
                    (b, d, false, true),
                    (a, c, true, false),
                    (a, d, true, true),
                ] {
                    if p.distance(q) >= 1e-8 {
                        continue;
                    }
                    let count = model
                        .wires
                        .iter()
                        .filter(|w| !w.closed)
                        .filter_map(|w| ends(model, w))
                        .flat_map(|(a, b)| [a, b])
                        .filter(|e| e.distance(p) < 1e-8)
                        .count();
                    if count == 2 {
                        pair = Some((i, j, ri, rj));
                        break 'outer;
                    }
                }
            }
        }
        let Some((i, j, ri, rj)) = pair else { break };
        let mut other = model.wires.remove(j);
        if ri {
            reverse(&mut model.wires[i]);
        }
        if rj {
            reverse(&mut other);
        }
        model.wires[i].edges.extend(other.edges);
        active_id = model.wires[i].id.clone();
    }
}

pub(crate) fn arc_through(a: Point, b: Point, c: Point) -> Result<SketchGeometry, String> {
    let u = Point::new(b.x - a.x, b.y - a.y);
    let v = Point::new(c.x - a.x, c.y - a.y);
    let cross = u.x * v.y - u.y * v.x;
    if cross.abs() < 1e-9 || a.distance(c) < 1e-8 {
        return Err("Arc needs three distinct, non-collinear points".into());
    }
    let uu = u.x * u.x + u.y * u.y;
    let vv = v.x * v.x + v.y * v.y;
    let center = Point::new(
        a.x + (uu * v.y - vv * u.y) / (2. * cross),
        a.y + (u.x * vv - v.x * uu) / (2. * cross),
    );
    let start_angle_rad = (a.y - center.y).atan2(a.x - center.x);
    let end = (c.y - center.y).atan2(c.x - center.x);
    let mut sweep_angle_rad = (end - start_angle_rad).rem_euclid(std::f64::consts::TAU);
    if cross < 0. {
        sweep_angle_rad -= std::f64::consts::TAU;
    }
    Ok(SketchGeometry::Arc {
        center,
        radius_mm: center.distance(a),
        start_angle_rad,
        sweep_angle_rad,
    })
}
