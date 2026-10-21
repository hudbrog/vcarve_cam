//! Selection-sensitive commands expressed with the existing sketch primitives.
use super::*;
use constraints::{Choice, Operand};

impl Editor {
    fn point_operands(&mut self) {
        for o in &mut self.saved.constraint_form.operands {
            if let Operand::Entity(id) = o
                && self
                    .saved
                    .model
                    .entity(id)
                    .is_ok_and(|e| matches!(e.geometry, SketchGeometry::Point { .. }))
            {
                *o = Operand::Point(PointRef {
                    entity_id: id.clone(),
                    point: PointRole::Position,
                });
            }
        }
    }
    pub(super) fn constraint_shortcut(&mut self, choice: Choice) {
        if !self.finish_dimension_edit() {
            return;
        }
        self.point_operands();
        if matches!(choice, Choice::Horizontal | Choice::Vertical) {
            if let Err(e) = self.alignment_shortcut(choice) {
                self.message = e;
            }
            return;
        }
        if choice == Choice::Coincident {
            let ops = &mut self.saved.constraint_form.operands;
            if ops.len() == 2 && ops.iter().all(|o| matches!(o, Operand::Entity(id) if self.saved.model.entity(id).is_ok_and(|e| e.geometry.circle().is_some()))) {
                for o in ops {
                    *o = Operand::Point(PointRef {entity_id:o.entity().into(), point:PointRole::Center});
                }
            }
        }
        let choice = if choice == Choice::Fixed
            && matches!(
                self.saved.constraint_form.operands.as_slice(),
                [Operand::Point(_)]
            ) {
            Choice::FixedPoint
        } else {
            choice
        };
        if choice == Choice::Angle
            && let [Operand::Entity(id)] = self.saved.constraint_form.operands.as_slice()
            && reference_entity(id).is_none()
            && self.saved.model.line(id).is_ok()
        {
            // Angle is signed from A to B, so the X axis is A for a single line.
            self.saved
                .constraint_form
                .operands
                .insert(0, Operand::Entity(X_AXIS_ID.into()));
        }
        self.choose_constraint(choice);
    }
    fn store_shortcut_constraints(
        &mut self,
        kinds: Vec<ConstraintKind>,
        reference: bool,
    ) -> Result<(), String> {
        if kinds.is_empty() {
            return Err("Select geometry for this constraint.".into());
        }
        self.cancel_gesture();
        let mut constraints = Vec::new();
        for definition in kinds {
            let reference = reference && definition.is_dimension();
            if !self
                .saved
                .model
                .constraints
                .iter()
                .any(|c| c.definition == definition && c.reference == reference)
            {
                constraints.push(Constraint {
                    id: self.id("c")?,
                    definition,
                    reference,
                });
            }
        }
        let before = self.saved.model.clone();
        self.saved.model.constraints.extend(constraints);
        self.commit(before)?;
        self.saved.constraint_form.changed = false;
        self.saved.constraint_form.editing = None;
        self.saved.constraint_form.value.clear();
        self.focus_dimension = false;
        self.tool = Tool::Select;
        self.show_constraints = true;
        self.message.clear();
        Ok(())
    }
    fn alignment_shortcut(&mut self, choice: Choice) -> Result<(), String> {
        let ops = &self.saved.constraint_form.operands;
        let kinds = if let [Operand::Point(a), Operand::Point(b)] = ops.as_slice() {
            vec![if choice == Choice::Horizontal {
                ConstraintKind::VerticalDistance { a:a.clone(), b:b.clone(), value_mm:0. }
            } else { ConstraintKind::HorizontalDistance { a:a.clone(), b:b.clone(), value_mm:0. } }]
        } else if !ops.is_empty() && ops.iter().all(|o| matches!(o, Operand::Entity(id) if reference_entity(id).is_none() && self.saved.model.line(id).is_ok())) {
            ops.iter().map(|o| if choice == Choice::Horizontal { ConstraintKind::Horizontal {line:o.entity().into()} } else { ConstraintKind::Vertical {line:o.entity().into()} }).collect()
        } else {
            self.choose_constraint(choice);
            return Ok(());
        };
        self.store_shortcut_constraints(kinds, false)
    }
    pub(super) fn auto_alignment(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        self.point_operands();
        let (a, b) =
            match self.saved.constraint_form.operands.as_slice() {
                [Operand::Entity(id)] => self.saved.model.line(id).map_err(
                    |_| "Select a line or two points for horizontal/vertical alignment.",
                )?,
                [Operand::Point(a), Operand::Point(b)] => (
                    self.saved.model.point(a).map_err(|e| e.to_string())?,
                    self.saved.model.point(b).map_err(|e| e.to_string())?,
                ),
                _ => {
                    return Err(
                        "Select a line or two points for horizontal/vertical alignment.".into(),
                    );
                }
            };
        self.alignment_shortcut(if (b.x - a.x).abs() >= (b.y - a.y).abs() {
            Choice::Horizontal
        } else {
            Choice::Vertical
        })
    }
    pub(super) fn equal_shortcut(&mut self) {
        let curves = self.saved.constraint_form.operands.iter().all(|o| matches!(o, Operand::Entity(id) if self.saved.model.entity(id).is_ok_and(|e| e.geometry.circle().is_some())));
        self.choose_constraint(if curves {
            Choice::EqualRadius
        } else {
            Choice::EqualLength
        });
    }
    pub(super) fn coincident_shortcut(&mut self) {
        if !self.finish_dimension_edit() {
            return;
        }
        self.point_operands();
        if matches!(
            self.saved.constraint_form.operands.as_slice(),
            [Operand::Point(_), Operand::Entity(_)] | [Operand::Entity(_), Operand::Point(_)]
        ) {
            if let Err(e) = self.point_on_object_shortcut() {
                self.message = e;
            }
        } else {
            self.constraint_shortcut(Choice::Coincident);
        }
    }
    pub(super) fn point_on_object_shortcut(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        self.point_operands();
        let ops = &mut self.saved.constraint_form.operands;
        if matches!(ops.as_slice(), [Operand::Entity(_), Operand::Point(_)]) {
            ops.swap(0, 1);
        }
        let choice = match ops.as_slice() {
            [Operand::Point(_), Operand::Entity(id)]
                if self
                    .saved
                    .model
                    .entity(id)
                    .is_ok_and(|e| e.geometry.circle().is_some()) =>
            {
                Choice::PointOnCircle
            }
            _ => Choice::PointOnLine,
        };
        self.choose_constraint(choice);
        Ok(())
    }
    pub(super) fn tangent_shortcut(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        let ops = &mut self.saved.constraint_form.operands;
        if let [Operand::Entity(a), Operand::Entity(b)] = ops.as_slice()
            && self.saved.model.line(a).is_ok()
            && self.saved.model.line(b).is_ok()
        {
            let kinds = [PointRole::Start, PointRole::End]
                .into_iter()
                .map(|point| ConstraintKind::PointOnLine {
                    point: PointRef {
                        entity_id: a.clone(),
                        point,
                    },
                    line: b.clone(),
                })
                .collect();
            return self.store_shortcut_constraints(kinds, false);
        }
        if let [Operand::Entity(a), Operand::Entity(b)] = ops.as_slice()
            && self
                .saved
                .model
                .entity(a)
                .is_ok_and(|e| e.geometry.circle().is_some())
            && self.saved.model.line(b).is_ok()
        {
            ops.swap(0, 1);
        }
        self.choose_constraint(Choice::Tangent);
        Ok(())
    }
    pub(super) fn symmetry_shortcut(&mut self) {
        if !self.finish_dimension_edit() {
            return;
        }
        self.point_operands();
        let ops = &mut self.saved.constraint_form.operands;
        if matches!(ops.as_slice(), [Operand::Entity(_), Operand::Point(_)]) {
            ops.swap(0, 1);
        }
        let choice = if matches!(ops.as_slice(), [Operand::Point(_), Operand::Entity(_)]) {
            Choice::Midpoint
        } else {
            Choice::Symmetry
        };
        if choice == Choice::Symmetry
            && ops.len() == 3
            && let Some(i) = ops.iter().position(|o| matches!(o, Operand::Entity(_)))
        {
            let line = ops.remove(i);
            ops.push(line);
        }
        self.choose_constraint(choice);
    }
    pub(super) fn radiam_shortcut(&mut self) -> Result<(), String> {
        let choice = match self.saved.constraint_form.operands.as_slice() {
            [Operand::Entity(id)] => match self.saved.model.entity(id).map(|e| &e.geometry) {
                Ok(SketchGeometry::Circle { .. }) => Choice::Diameter,
                Ok(SketchGeometry::Arc { .. }) => Choice::Radius,
                _ => return Err("Select a circle or arc for radius/diameter.".into()),
            },
            _ => return Err("Select a circle or arc for radius/diameter.".into()),
        };
        self.choose_constraint(choice);
        Ok(())
    }
    pub(super) fn lock_position_shortcut(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        self.point_operands();
        let mut points: Vec<_> = self
            .saved
            .constraint_form
            .operands
            .iter()
            .filter_map(|o| {
                if let Operand::Point(p) = o {
                    Some(p.clone())
                } else {
                    None
                }
            })
            .collect();
        if points.is_empty() || points.len() != self.saved.constraint_form.operands.len() {
            return Err("Select points to lock their X and Y distances; the last point is the reference, or the origin for one point.".into());
        }
        let origin = if points.len() == 1 {
            origin_ref()
        } else {
            points.pop().unwrap()
        };
        let b = self.saved.model.point(&origin).map_err(|e| e.to_string())?;
        let mut kinds = Vec::new();
        for p in points {
            let a = self.saved.model.point(&p).map_err(|e| e.to_string())?;
            kinds.push(ConstraintKind::HorizontalDistance {
                a: p.clone(),
                b: origin.clone(),
                value_mm: a.x - b.x,
            });
            kinds.push(ConstraintKind::VerticalDistance {
                a: p,
                b: origin.clone(),
                value_mm: a.y - b.y,
            });
        }
        self.store_shortcut_constraints(kinds, self.new_dimensions_reference)
    }
    pub(super) fn toggle_reference_shortcut(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        if self.saved.constraint_form.editing.is_some() {
            if !self.saved.constraint_form.choice.dimension() {
                return Err("Only dimensions can be driving or reference.".into());
            }
            self.saved.constraint_form.reference = !self.saved.constraint_form.reference;
            self.saved.constraint_form.value.clear(); // Conversion starts at the measured geometry.
            self.store_constraint()?;
            self.show_constraints = true;
        } else {
            self.new_dimensions_reference = !self.new_dimensions_reference;
            self.saved.constraint_form.reference = self.new_dimensions_reference;
            self.message = if self.new_dimensions_reference {
                "New dimensions: reference."
            } else {
                "New dimensions: driving."
            }
            .into();
        }
        Ok(())
    }
    pub(super) fn associated_geometry_shortcut(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        let Some(id) = &self.saved.constraint_form.editing else {
            return Err("Select a constraint in the list first.".into());
        };
        let c = self
            .saved
            .model
            .constraints
            .iter()
            .find(|c| &c.id == id)
            .ok_or("Constraint no longer exists")?;
        self.selected = constraint_entities(&c.definition)
            .into_iter()
            .cloned()
            .collect();
        self.saved.constraint_form.operands =
            self.selected.iter().cloned().map(Operand::Entity).collect();
        self.saved.constraint_form.editing = None;
        self.saved.constraint_form.value.clear();
        self.show_geometry = true;
        Ok(())
    }
    pub(super) fn remove_alignment_shortcut(&mut self) -> Result<(), String> {
        if !self.finish_dimension_edit() {
            return Ok(());
        }
        let before = self.saved.model.clone();
        self.saved.model.constraints.retain(|c| !matches!(&c.definition,ConstraintKind::Horizontal {line} | ConstraintKind::Vertical {line} if self.selected.contains(line)));
        self.commit(before)?;
        self.refresh_form();
        Ok(())
    }
}
