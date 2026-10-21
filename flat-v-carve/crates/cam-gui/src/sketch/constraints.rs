use super::*;
use crate::app::observe_control;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operand {
    Entity(String),
    Point(PointRef),
}
impl Operand {
    pub fn reference(e: &Entity) -> Self {
        if e.id == ORIGIN_ID {
            Self::Point(origin_ref())
        } else {
            Self::Entity(e.id.clone())
        }
    }
    pub fn entity(&self) -> &str {
        match self {
            Self::Entity(id) => id,
            Self::Point(p) => &p.entity_id,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Choice {
    #[default]
    Horizontal,
    Vertical,
    Coincident,
    Parallel,
    Perpendicular,
    EqualLength,
    EqualRadius,
    Tangent,
    PointOnLine,
    PointOnCircle,
    Midpoint,
    Symmetry,
    Distance,
    HorizontalDistance,
    VerticalDistance,
    Length,
    Radius,
    Diameter,
    Angle,
    Fixed,
    FixedPoint,
}
impl Choice {
    pub(super) const ALL: [Self; 21] = [
        Self::Horizontal,
        Self::Vertical,
        Self::Coincident,
        Self::Parallel,
        Self::Perpendicular,
        Self::EqualLength,
        Self::EqualRadius,
        Self::Tangent,
        Self::PointOnLine,
        Self::PointOnCircle,
        Self::Midpoint,
        Self::Symmetry,
        Self::Distance,
        Self::HorizontalDistance,
        Self::VerticalDistance,
        Self::Length,
        Self::Radius,
        Self::Diameter,
        Self::Angle,
        Self::Fixed,
        Self::FixedPoint,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Coincident => "Coincident",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::EqualLength => "Equal length",
            Self::EqualRadius => "Equal radius",
            Self::Tangent => "Tangent",
            Self::PointOnLine => "Point on line",
            Self::PointOnCircle => "Point on circle",
            Self::Midpoint => "Midpoint",
            Self::Symmetry => "Symmetry",
            Self::Distance => "Distance",
            Self::HorizontalDistance => "Horizontal distance",
            Self::VerticalDistance => "Vertical distance",
            Self::Length => "Length",
            Self::Radius => "Radius",
            Self::Diameter => "Diameter",
            Self::Angle => "Angle",
            Self::Fixed => "Fix entity",
            Self::FixedPoint => "Fix point",
        }
    }
    pub(super) fn dimension(self) -> bool {
        matches!(
            self,
            Self::Distance
                | Self::HorizontalDistance
                | Self::VerticalDistance
                | Self::Length
                | Self::Radius
                | Self::Diameter
                | Self::Angle
        )
    }
    pub(super) fn hint(self) -> &'static str {
        match self {
            Self::Horizontal | Self::Vertical | Self::Length => "Select one line body.",
            Self::Parallel | Self::Perpendicular | Self::EqualLength => {
                "Select two line bodies (A, then B)."
            }
            Self::Angle => "Select two line bodies (A, then B). Signed angle runs from A to B.",
            Self::Radius | Self::Diameter => "Select a circle or arc body.",
            Self::EqualRadius => "Select two circle or arc bodies.",
            Self::Tangent => {
                "Select a line body, then a circle or arc body. Tangency is to the supporting circle; add endpoint coincidence separately."
            }
            Self::PointOnLine | Self::Midpoint => "Select a point, then a line body.",
            Self::PointOnCircle => "Select a point, then a circle or arc body.",
            Self::Symmetry => "Select two points, then the symmetry line body.",
            Self::Fixed => "Select one entity body. Locks its current geometry.",
            Self::FixedPoint => "Select one endpoint or centre. Locks its current position.",
            Self::HorizontalDistance | Self::VerticalDistance => {
                "Select a line, two points, or a point and the perpendicular axis. Line distance is end − start; point distance is A − B."
            }
            Self::Distance => {
                "Select a line for its length, two points, or a point and an axis. Axis offsets are signed (+X / +Y)."
            }
            _ => "Select two points (A, then B).",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Form {
    pub choice: Choice,
    pub operands: Vec<Operand>,
    pub value: String,
    pub reference: bool,
    pub editing: Option<String>,
    #[serde(default)]
    pub changed: bool,
}
impl Form {
    fn definition(&self, model: &SketchDocument) -> Result<ConstraintKind, String> {
        use ConstraintKind as K;
        let entity = |i: usize| match self.operands.get(i) {
            Some(Operand::Entity(id)) => Ok(id.clone()),
            _ => Err(self.choice.hint().to_owned()),
        };
        let point = |i: usize| match self.operands.get(i) {
            Some(Operand::Point(p)) => Ok(p.clone()),
            _ => Err(self.choice.hint().to_owned()),
        };
        let count = match self.choice {
            Choice::Horizontal
            | Choice::Vertical
            | Choice::Length
            | Choice::Radius
            | Choice::Diameter
            | Choice::Fixed
            | Choice::FixedPoint => 1,
            Choice::Symmetry => 3,
            _ => 2,
        };
        let distance = matches!(
            self.choice,
            Choice::Distance | Choice::HorizontalDistance | Choice::VerticalDistance
        );
        if if distance {
            !(1..=2).contains(&self.operands.len())
        } else {
            self.operands.len() != count
        } {
            return Err(self.choice.hint().into());
        }
        let value = if self.choice.dimension() && !self.reference && !self.value.trim().is_empty() {
            let v = self
                .value
                .trim()
                .parse::<f64>()
                .map_err(|_| "Enter a finite dimension value")?;
            if !v.is_finite() {
                return Err("Enter a finite dimension value".into());
            }
            v
        } else {
            1.
        };
        let mut k = match self.choice {
            Choice::Horizontal => K::Horizontal { line: entity(0)? },
            Choice::Vertical => K::Vertical { line: entity(0)? },
            Choice::Coincident => K::Coincident {
                a: point(0)?,
                b: point(1)?,
            },
            Choice::Parallel => K::Parallel {
                a: entity(0)?,
                b: entity(1)?,
            },
            Choice::Perpendicular => K::Perpendicular {
                a: entity(0)?,
                b: entity(1)?,
            },
            Choice::EqualLength => K::EqualLength {
                a: entity(0)?,
                b: entity(1)?,
            },
            Choice::EqualRadius => K::EqualRadius {
                a: entity(0)?,
                b: entity(1)?,
            },
            Choice::Tangent => K::Tangent {
                line: entity(0)?,
                curve: entity(1)?,
            },
            Choice::PointOnLine => K::PointOnLine {
                point: point(0)?,
                line: entity(1)?,
            },
            Choice::PointOnCircle => K::PointOnCircle {
                point: point(0)?,
                curve: entity(1)?,
            },
            Choice::Midpoint => K::Midpoint {
                point: point(0)?,
                line: entity(1)?,
            },
            Choice::Symmetry => K::Symmetry {
                a: point(0)?,
                b: point(1)?,
                line: entity(2)?,
            },
            Choice::Distance | Choice::HorizontalDistance | Choice::VerticalDistance => {
                self.distance_definition(model, value)?
            }
            Choice::Length => K::Length {
                line: entity(0)?,
                value_mm: value,
            },
            Choice::Radius => K::Radius {
                curve: entity(0)?,
                value_mm: value,
            },
            Choice::Diameter => K::Diameter {
                curve: entity(0)?,
                value_mm: value,
            },
            Choice::Angle => K::Angle {
                a: entity(0)?,
                b: entity(1)?,
                value_rad: value.to_radians(),
            },
            Choice::Fixed => {
                let id = entity(0)?;
                let target = model
                    .entity(&id)
                    .map_err(|e| e.to_string())?
                    .geometry
                    .clone();
                K::Fixed { entity: id, target }
            }
            Choice::FixedPoint => {
                let p = point(0)?;
                let position = model.point(&p).map_err(|e| e.to_string())?;
                K::FixedPoint { point: p, position }
            }
        };
        if self.choice.dimension() && (self.reference || self.value.trim().is_empty()) {
            let (_, measurement) = model.evaluate(&k).map_err(|e| e.to_string())?;
            set_value(&mut k, measurement.ok_or("Cannot measure these operands")?);
        }
        Ok(k)
    }
    // Resolve from the current selection at submission time, so the tools,
    // shortcuts and dropdown support both select-first and command-first use.
    fn distance_definition(
        &self,
        model: &SketchDocument,
        value_mm: f64,
    ) -> Result<ConstraintKind, String> {
        use ConstraintKind as K;
        let mut choice = self.choice;
        let (a, b) = match self.operands.as_slice() {
            [Operand::Entity(line)] if reference_entity(line).is_none() => {
                model
                    .line(line)
                    .map_err(|_| self.choice.hint().to_owned())?;
                if choice == Choice::Distance {
                    return Ok(K::Length {
                        line: line.clone(),
                        value_mm,
                    });
                }
                (
                    PointRef {
                        entity_id: line.clone(),
                        point: PointRole::End,
                    },
                    PointRef {
                        entity_id: line.clone(),
                        point: PointRole::Start,
                    },
                )
            }
            [Operand::Point(a), Operand::Point(b)] => (a.clone(), b.clone()),
            [Operand::Point(a)] if reference_entity(&a.entity_id).is_none() => {
                (a.clone(), origin_ref())
            }
            [Operand::Point(p), Operand::Entity(axis)]
            | [Operand::Entity(axis), Operand::Point(p)]
                if axis == X_AXIS_ID || axis == Y_AXIS_ID =>
            {
                let normal = if axis == X_AXIS_ID {
                    Choice::VerticalDistance
                } else {
                    Choice::HorizontalDistance
                };
                if choice != Choice::Distance && choice != normal {
                    return Err("Use vertical distance to the X axis, or horizontal distance to the Y axis.".into());
                }
                choice = normal;
                (
                    p.clone(),
                    PointRef {
                        entity_id: axis.clone(),
                        point: PointRole::Start,
                    },
                )
            }
            _ => return Err(self.choice.hint().into()),
        };
        Ok(match choice {
            Choice::HorizontalDistance => K::HorizontalDistance { a, b, value_mm },
            Choice::VerticalDistance => K::VerticalDistance { a, b, value_mm },
            _ => K::Distance { a, b, value_mm },
        })
    }
    fn from_constraint(c: &Constraint) -> Self {
        use ConstraintKind as K;
        let e = |s: &String| Operand::Entity(s.clone());
        let p = |s: &PointRef| Operand::Point(s.clone());
        let (choice, operands) = match &c.definition {
            K::Horizontal { line } => (Choice::Horizontal, vec![e(line)]),
            K::Vertical { line } => (Choice::Vertical, vec![e(line)]),
            K::Coincident { a, b } => (Choice::Coincident, vec![p(a), p(b)]),
            K::Parallel { a, b } => (Choice::Parallel, vec![e(a), e(b)]),
            K::Perpendicular { a, b } => (Choice::Perpendicular, vec![e(a), e(b)]),
            K::EqualLength { a, b } => (Choice::EqualLength, vec![e(a), e(b)]),
            K::EqualRadius { a, b } => (Choice::EqualRadius, vec![e(a), e(b)]),
            K::Tangent { line, curve } => (Choice::Tangent, vec![e(line), e(curve)]),
            K::PointOnLine { point, line } => (Choice::PointOnLine, vec![p(point), e(line)]),
            K::PointOnCircle { point, curve } => (Choice::PointOnCircle, vec![p(point), e(curve)]),
            K::Midpoint { point, line } => (Choice::Midpoint, vec![p(point), e(line)]),
            K::Symmetry { a, b, line } => (Choice::Symmetry, vec![p(a), p(b), e(line)]),
            K::Distance { a, b, .. } => (Choice::Distance, vec![p(a), p(b)]),
            K::HorizontalDistance { a, b, .. } => (
                Choice::HorizontalDistance,
                vec![
                    p(a),
                    if b.entity_id == Y_AXIS_ID && b.point == PointRole::Start {
                        e(&b.entity_id)
                    } else {
                        p(b)
                    },
                ],
            ),
            K::VerticalDistance { a, b, .. } => (
                Choice::VerticalDistance,
                vec![
                    p(a),
                    if b.entity_id == X_AXIS_ID && b.point == PointRole::Start {
                        e(&b.entity_id)
                    } else {
                        p(b)
                    },
                ],
            ),
            K::Length { line, .. } => (Choice::Length, vec![e(line)]),
            K::Radius { curve, .. } => (Choice::Radius, vec![e(curve)]),
            K::Diameter { curve, .. } => (Choice::Diameter, vec![e(curve)]),
            K::Angle { a, b, .. } => (Choice::Angle, vec![e(a), e(b)]),
            K::Fixed { entity, .. } => (Choice::Fixed, vec![e(entity)]),
            K::FixedPoint { point, .. } => (Choice::FixedPoint, vec![p(point)]),
        };
        Self {
            choice,
            operands,
            reference: c.reference,
            editing: Some(c.id.clone()),
            changed: false,
            value: dimension_value(&c.definition)
                .map(|v| edit_value(choice, v))
                .unwrap_or_default(),
        }
    }
}
fn dimension_value(k: &ConstraintKind) -> Option<f64> {
    match k {
        ConstraintKind::Distance { value_mm, .. }
        | ConstraintKind::HorizontalDistance { value_mm, .. }
        | ConstraintKind::VerticalDistance { value_mm, .. }
        | ConstraintKind::Length { value_mm, .. }
        | ConstraintKind::Radius { value_mm, .. }
        | ConstraintKind::Diameter { value_mm, .. } => Some(*value_mm),
        ConstraintKind::Angle { value_rad, .. } => Some(*value_rad),
        _ => None,
    }
}
fn set_value(k: &mut ConstraintKind, value: f64) {
    match k {
        ConstraintKind::Distance { value_mm, .. }
        | ConstraintKind::HorizontalDistance { value_mm, .. }
        | ConstraintKind::VerticalDistance { value_mm, .. }
        | ConstraintKind::Length { value_mm, .. }
        | ConstraintKind::Radius { value_mm, .. }
        | ConstraintKind::Diameter { value_mm, .. } => *value_mm = value,
        ConstraintKind::Angle { value_rad, .. } => *value_rad = value,
        _ => {}
    }
}
fn display_value(choice: Choice, value: f64) -> String {
    let v = if choice == Choice::Angle {
        value.to_degrees()
    } else {
        value
    };
    format!("{v:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
fn edit_value(choice: Choice, value: f64) -> String {
    if choice == Choice::Angle {
        value.to_degrees().to_string()
    } else {
        value.to_string()
    }
}
impl Editor {
    fn dimension_field() -> egui::Id {
        egui::Id::new("sketch-dimension-value")
    }
    /// Commit before changing the operands, never after: the value belongs to
    /// the old selection. Incomplete command-first forms can still pick operands.
    pub(super) fn finish_dimension_edit(&mut self) -> bool {
        let form = &self.saved.constraint_form;
        if !form.changed || !form.choice.dimension() {
            return true;
        }
        let mut measured = form.clone();
        measured.value.clear();
        if measured.definition(&self.saved.model).is_err() {
            return true;
        }
        if let Err(e) = self.store_constraint() {
            self.message = e;
            self.show_constraints = true;
            self.focus_dimension = true;
            return false;
        }
        self.focus_dimension = false;
        true
    }
    pub fn dimension_shortcut(&mut self) {
        let choice = match self.saved.constraint_form.operands.as_slice() {
            [Operand::Entity(id)] => match self.saved.model.entity(id).map(|e| &e.geometry) {
                Ok(SketchGeometry::Circle { .. }) => Choice::Diameter,
                Ok(SketchGeometry::Arc { .. }) => Choice::Radius,
                _ => Choice::Distance,
            },
            [Operand::Entity(a), Operand::Entity(b)]
                if self.saved.model.line(a).is_ok() && self.saved.model.line(b).is_ok() =>
            {
                Choice::Angle
            }
            _ => Choice::Distance,
        };
        self.choose_constraint(choice);
    }
    pub(super) fn prepare_dimension(&mut self) {
        self.focus_dimension = true;
        let Ok(mut definition) = self.saved.constraint_form.definition(&self.saved.model) else {
            return;
        };
        // Reopening the same dimension must not introduce a duplicate equation.
        set_value(&mut definition, 0.);
        if let Some(c) = self.saved.model.constraints.iter().find(|c| {
            let mut candidate = c.definition.clone();
            set_value(&mut candidate, 0.);
            candidate == definition
        }) {
            self.saved.constraint_form = Form::from_constraint(c);
            self.focus_dimension = !c.reference;
        } else if let Ok(k) = self.saved.constraint_form.definition(&self.saved.model)
            && let Some(value) = dimension_value(&k)
        {
            // Normalize contextual Distance to its actual kind and operands.
            self.saved.constraint_form = Form::from_constraint(&Constraint {
                id: String::new(),
                reference: self.saved.constraint_form.reference,
                definition: k,
            });
            self.saved.constraint_form.editing = None;
            self.saved.constraint_form.value = edit_value(self.saved.constraint_form.choice, value);
            self.saved.constraint_form.changed = true;
            self.focus_dimension = !self.saved.constraint_form.reference;
        }
    }
    pub fn dimension_keys(&mut self, ctx: &egui::Context) {
        let id = Self::dimension_field();
        // egui clears focus for Escape at the beginning of the frame.
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if !ctx.memory(|m| m.has_focus(id) || (escape && m.had_focus_last_frame(id))) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            let form = &self.saved.constraint_form;
            let stored = form
                .editing
                .as_ref()
                .and_then(|id| self.saved.model.constraints.iter().find(|c| &c.id == id));
            if let Some(c) = stored {
                self.saved.constraint_form = Form::from_constraint(c);
            } else {
                self.saved.constraint_form.value.clear();
                self.saved.constraint_form.changed = false;
            }
            self.message.clear();
            self.focus_dimension = false;
            ctx.memory_mut(|m| m.surrender_focus(id));
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            match self.store_constraint() {
                Ok(()) => {
                    self.focus_dimension = false;
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
                Err(e) => {
                    self.message = e;
                    self.focus_dimension = true;
                }
            }
        }
    }
    pub(super) fn pick_operand(&mut self, operand: Option<Operand>, additive: bool) -> bool {
        if (!additive || self.saved.constraint_form.editing.is_some())
            && !self.finish_dimension_edit()
        {
            return false;
        }
        let form = &mut self.saved.constraint_form;
        if form.editing.take().is_some() {
            form.value.clear();
            form.changed = false;
            form.reference = false;
        }
        if !additive {
            form.operands.clear();
        }
        if let Some(o) = operand {
            if let Some(i) = form.operands.iter().position(|v| v == &o) {
                form.operands.remove(i);
            } else if form.operands.len() < MAX_ENTITIES * 4 {
                form.operands.push(o);
            }
        }
        self.selected = form
            .operands
            .iter()
            .map(|o| o.entity().to_owned())
            .collect();
        true
    }
    pub fn store_constraint(&mut self) -> Result<(), String> {
        self.cancel_gesture();
        let form = &self.saved.constraint_form;
        if let Some(id) = &form.editing
            && !self.saved.model.constraints.iter().any(|c| &c.id == id)
        {
            return Err("This constraint was removed. Choose New constraint to add one.".into());
        }
        let definition = form.definition(&self.saved.model)?;
        let c = Constraint {
            id: match &form.editing {
                Some(id) => id.clone(),
                None => self.id("c")?,
            },
            reference: form.reference && definition.is_dimension(),
            definition,
        };
        let before = self.saved.model.clone();
        if let Some(old) = self
            .saved
            .model
            .constraints
            .iter_mut()
            .find(|old| old.id == c.id)
        {
            *old = c.clone();
        } else {
            self.saved.model.constraints.push(c.clone());
        }
        self.commit(before)?;
        self.saved.constraint_form = Form::from_constraint(&c);
        self.message.clear();
        Ok(())
    }
    pub fn edit_constraint(&mut self, id: &str) {
        if !self.finish_dimension_edit() {
            return;
        }
        self.cancel_gesture();
        if let Some(c) = self.saved.model.constraints.iter().find(|c| c.id == id) {
            self.saved.constraint_form = Form::from_constraint(c);
            self.selected = constraint_entities(&c.definition)
                .into_iter()
                .cloned()
                .collect();
            self.show_constraints = true;
            self.tool = Tool::Select;
            self.focus_dimension = c.definition.is_dimension() && !c.reference;
        }
    }
    pub fn remove_constraint(&mut self, id: &str) -> Result<(), String> {
        self.cancel_gesture();
        let before = self.saved.model.clone();
        self.saved.model.constraints.retain(|c| c.id != id);
        self.saved.constraint_form.editing = None;
        self.saved.constraint_form.changed = false;
        self.saved.constraint_form.value.clear();
        self.commit(before)
    }
    pub(super) fn refresh_form(&mut self) {
        if let Some(id) = self.saved.constraint_form.editing.clone() {
            if let Some(c) = self.saved.model.constraints.iter().find(|c| c.id == id) {
                self.saved.constraint_form = Form::from_constraint(c);
            } else {
                self.saved.constraint_form = Form::default();
            }
        }
        self.saved.constraint_form.operands.retain(|o| match o {
            Operand::Entity(id) => self.saved.model.entity(id).is_ok(),
            Operand::Point(p) => self.saved.model.point(p).is_ok(),
        });
        self.selected = self
            .saved
            .constraint_form
            .operands
            .iter()
            .map(|o| o.entity().to_owned())
            .collect();
    }
    pub(super) fn operand_label(&self, o: &Operand) -> String {
        if let Some(e) = reference_entity(o.entity()) {
            return e.label.clone().unwrap_or_else(|| e.id.clone());
        }
        let Some((i, e)) = self
            .saved
            .model
            .entities
            .iter()
            .enumerate()
            .find(|(_, e)| e.id == o.entity())
        else {
            return "Removed geometry".into();
        };
        let name = match e.geometry {
            SketchGeometry::Line { .. } => "Line",
            SketchGeometry::Circle { .. } => "Circle",
            SketchGeometry::Arc { .. } => "Arc",
            SketchGeometry::Point { .. } => "Point",
        };
        match o {
            Operand::Entity(_) => format!("{name} {}", i + 1),
            Operand::Point(p) => format!("{name} {} · {:?}", i + 1, p.point),
        }
    }
    pub fn constraint_panel(&mut self, ui: &mut egui::Ui) {
        observe_control("Constraint panel", ui.clip_rect());
        ui.heading("Constraints");
        ui.small("Click endpoints/centres or curve bodies. Shift-click adds operands in order.");
        let labels: Vec<_> = self
            .saved
            .constraint_form
            .operands
            .iter()
            .map(|o| self.operand_label(o))
            .collect();
        for (i, label) in labels.iter().take(3).enumerate() {
            ui.label(format!("{}: {label}", ['A', 'B', 'C'][i]));
        }
        if labels.len() > 3 {
            ui.small(format!("{} more selected", labels.len() - 3));
        }
        if labels.is_empty() {
            ui.weak("No operands selected");
        }
        if control(ui, "Clear operands", true) {
            self.saved.constraint_form.operands.clear();
            self.saved.constraint_form.editing = None;
            self.saved.constraint_form.value.clear();
            self.saved.constraint_form.changed = false;
            self.selected.clear();
        }
        let old = self.saved.constraint_form.choice;
        let r = egui::ComboBox::from_id_salt("constraint-kind")
            .height(460.)
            .selected_text(old.label())
            .show_ui(ui, |ui| {
                observe_control("Constraint choices", ui.clip_rect());
                for choice in Choice::ALL {
                    let r = ui.selectable_value(
                        &mut self.saved.constraint_form.choice,
                        choice,
                        choice.label(),
                    );
                    observe_control(&format!("Constraint kind {}", choice.label()), r.rect);
                }
            })
            .response;
        observe_control("Constraint kind", r.rect);
        if old != self.saved.constraint_form.choice {
            self.saved.constraint_form.value.clear();
            self.saved.constraint_form.reference = false;
            self.saved.constraint_form.changed = true;
        }
        let choice = self.saved.constraint_form.choice;
        ui.small(choice.hint());
        if choice.dimension() {
            let measurement = self
                .saved
                .constraint_form
                .definition(&self.saved.model)
                .ok()
                .and_then(|k| self.saved.model.evaluate(&k).ok())
                .and_then(|(_, v)| v);
            let r = ui.checkbox(
                &mut self.saved.constraint_form.reference,
                "Reference dimension",
            );
            observe_control("Reference dimension", r.rect);
            if r.changed() {
                self.saved.constraint_form.changed = true;
            }
            if r.changed()
                && let Some(v) = measurement
            {
                self.saved.constraint_form.value = edit_value(choice, v);
            }
            if self.saved.constraint_form.reference {
                ui.label(format!(
                    "Measured: {} {}",
                    measurement
                        .map(|v| display_value(choice, v))
                        .unwrap_or_else(|| "—".into()),
                    if choice == Choice::Angle { "°" } else { "mm" }
                ));
                ui.small("Reference dimensions measure geometry and do not constrain it.");
            } else {
                ui.horizontal(|ui| {
                    let mut output =
                        egui::TextEdit::singleline(&mut self.saved.constraint_form.value)
                            .id(Self::dimension_field())
                            .desired_width(110.)
                            .char_limit(128)
                            .hint_text("Current value")
                            .show(ui);
                    let r = &output.response;
                    observe_control("Constraint value", r.rect);
                    if self.focus_dimension {
                        r.request_focus();
                        r.scroll_to_me(Some(egui::Align::Center));
                        output
                            .state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(
                                    self.saved.constraint_form.value.chars().count(),
                                ),
                            )));
                        output.state.store(ui.ctx(), r.id);
                        self.focus_dimension = false;
                    }
                    if r.changed() {
                        self.saved.constraint_form.changed = true;
                    }
                    ui.label(if choice == Choice::Angle {
                        "degrees"
                    } else {
                        "mm"
                    });
                });
                ui.small("Enter or select next geometry to save · Esc to discard value");
            }
        }
        let editing = self.saved.constraint_form.editing.clone();
        if control(
            ui,
            if editing.is_some() {
                "Update constraint"
            } else {
                "Add constraint"
            },
            !self.pending(),
        ) && let Err(e) = self.store_constraint()
        {
            self.message = e;
        }
        if let Some(id) = editing {
            if control(ui, "Remove constraint", true)
                && let Err(e) = self.remove_constraint(&id)
            {
                self.message = e;
            }
            if control(ui, "New constraint", true) {
                self.saved.constraint_form.editing = None;
                self.saved.constraint_form.changed = false;
            }
        }
        ui.separator();
        ui.strong(self.solver_status());
        ui.horizontal_wrapped(|ui| {
            if control(ui, "Solve sketch", !self.pending()) {
                self.request_solve();
            }
            if control(ui, "Cancel solve", self.solving()) {
                self.cancel_solve();
            }
        });
        ui.small("Wire endpoints remain joined. DOF counts unconstrained variables; it does not prove that constraints are independent.");
        if let Some(out) = &self.solver.outcome {
            ui.collapsing("Solver details",|ui| {
                ui.small(format!("Arael: {} · {} iterations",out.status,out.iterations));
                if let Some(lag)=out.drag_error_mm {ui.small(format!("Cursor distance: {lag:.4} mm"));}
                ui.small("Unsatisfied constraints identify residuals, not a minimal conflicting set. Exact duplicates are flagged below.");
            });
        }
        let diagnostics = self
            .solver
            .outcome
            .as_ref()
            .map(|r| r.issues.clone())
            .unwrap_or_default();
        if !diagnostics.is_empty() {
            ui.strong("Rejected candidate");
        }
        for d in &diagnostics {
            let constraint = d.source_id.as_ref().and_then(|id| {
                self.saved
                    .model
                    .constraints
                    .iter()
                    .enumerate()
                    .find(|(_, c)| &c.id == id)
            });
            let summary = if let Some((i, c)) = constraint {
                let residual = self
                    .solver
                    .outcome
                    .as_ref()
                    .and_then(|out| out.constraints.iter().find(|v| v.constraint_id == c.id));
                format!(
                    "Constraint {} ({}): {}",
                    i + 1,
                    Form::from_constraint(c).choice.label(),
                    residual.map_or(d.message.clone(), |v| format!(
                        "residual {:.3e} mm",
                        v.residual_mm
                    ))
                )
            } else {
                d.message.clone()
            };
            let r = ui
                .selectable_label(
                    false,
                    egui::RichText::new(summary).color(crate::ui_theme::ERROR),
                )
                .on_hover_text(&d.message);
            if r.clicked()
                && let Some(id) = &d.source_id
            {
                self.edit_constraint(id);
            }
        }
        let evaluations = self
            .saved
            .model
            .validate_solution()
            .map(|r| r.constraints)
            .unwrap_or_default();
        let constraints = self.saved.model.constraints.clone();
        ui.separator();
        ui.strong(format!("{} constraints", constraints.len()));
        ui.horizontal(|ui| {
            for (filter, label) in [(0, "All"), (1, "Dimensions"), (2, "Problems")] {
                let r = ui.selectable_value(&mut self.dimension_filter, filter, label);
                observe_control(&format!("Constraint filter {label}"), r.rect);
            }
        });
        for (i, c) in constraints.iter().enumerate() {
            let form = Form::from_constraint(c);
            let eval = evaluations.iter().find(|v| v.constraint_id == c.id);
            let failed = eval.is_some_and(|v| !c.reference && v.residual_mm > 1e-6);
            if (self.dimension_filter == 1 && !c.definition.is_dimension())
                || (self.dimension_filter == 2 && !failed)
            {
                continue;
            }
            let value = if c.reference {
                eval.and_then(|e| e.measured)
            } else {
                dimension_value(&c.definition)
            };
            let text = format!(
                "{}: {}{}{}{}",
                i + 1,
                form.choice.label(),
                value
                    .map(|v| format!(
                        " {}{}",
                        display_value(form.choice, v),
                        if form.choice == Choice::Angle {
                            "°"
                        } else {
                            " mm"
                        }
                    ))
                    .unwrap_or_default(),
                if c.reference { " (ref)" } else { "" },
                if failed { " · unsatisfied" } else { "" }
            );
            let r = ui.selectable_label(
                self.saved.constraint_form.editing.as_ref() == Some(&c.id),
                text,
            );
            observe_control(&format!("Constraint row {}", i + 1), r.rect);
            if r.on_hover_text(format!(
                "{}\nResidual: {}",
                c.id,
                eval.map_or("unavailable".into(), |e| format!("{:.3e}", e.residual_mm))
            ))
            .clicked()
            {
                self.edit_constraint(&c.id);
            }
            if !c.reference
                && constraints[..i]
                    .iter()
                    .any(|p| !p.reference && p.definition == c.definition)
            {
                ui.colored_label(crate::ui_theme::WARNING, "Duplicate driving constraint");
            }
        }
    }
    pub(super) fn dimension_labels(&self) -> Vec<(String, Point, String, bool)> {
        self.saved
            .model
            .constraints
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if !c.definition.is_dimension() {
                    return None;
                }
                let f = Form::from_constraint(c);
                let points: Vec<_> = f
                    .operands
                    .iter()
                    .filter_map(|o| match o {
                        Operand::Point(p) => self.saved.model.point(p).ok(),
                        Operand::Entity(id) => self.saved.model.entity(id).ok().and_then(|e| {
                            e.geometry
                                .line()
                                .map(|(a, b)| Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.))
                                .or_else(|| {
                                    e.geometry.circle().map(|(p, r)| Point::new(p.x + r, p.y))
                                })
                        }),
                    })
                    .collect();
                if points.is_empty() {
                    return None;
                }
                let position = Point::new(
                    points.iter().map(|p| p.x).sum::<f64>() / points.len() as f64,
                    points.iter().map(|p| p.y).sum::<f64>() / points.len() as f64,
                );
                let value = if c.reference {
                    self.saved.model.evaluate(&c.definition).ok()?.1?
                } else {
                    dimension_value(&c.definition)?
                };
                Some((
                    c.id.clone(),
                    position,
                    format!(
                        "{}: {}{}{}",
                        i + 1,
                        display_value(f.choice, value),
                        if f.choice == Choice::Angle { "°" } else { "" },
                        if c.reference { " (ref)" } else { "" }
                    ),
                    !c.reference
                        && self
                            .saved
                            .model
                            .evaluate(&c.definition)
                            .is_ok_and(|(r, _)| r > RESIDUAL_TOLERANCE_MM),
                ))
            })
            .collect()
    }
}
fn control(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, egui::Button::new(label));
    observe_control(label, r.rect);
    r.clicked()
}
