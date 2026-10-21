//! FreeCAD command keys share one catalogue with the in-app reference.
use super::*;
use constraints::{Choice, Operand};
use egui::{Key, Modifiers};

#[path = "freecad_shortcuts.rs"]
mod catalogue;
use catalogue::BINDINGS;

#[cfg(test)]
#[path = "shortcut_tests.rs"]
mod tests;

struct Binding {
    id: &'static str,
    label: &'static str,
    text: &'static str,
    keys: &'static [Key],
    modifiers: Modifiers,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Draw(Tool),
    Constraint(Choice),
    Dimension,
    HorVer,
    Equal,
    Coincident,
    PointOnObject,
    Tangent,
    Symmetric,
    Radiam,
    Lock,
    Construction,
    Reference,
    SelectReference(&'static str),
    AssociatedGeometry,
    RemoveAlignment,
    View,
}
impl Binding {
    fn action(&self) -> Option<Action> {
        use Action::*;
        Some(match self.id {
            "Sketcher_CreateLine" => Draw(Tool::Line),
            "Sketcher_CreatePolyline" => Draw(Tool::Polyline),
            "Sketcher_CreateRectangle" => Draw(Tool::Rectangle),
            "Sketcher_CreateCircle" => Draw(Tool::Circle),
            "Sketcher_Create3PointArc" => Draw(Tool::Arc),
            "Sketcher_CreatePoint" => Draw(Tool::Point),
            "Sketcher_Dimension" => Dimension,
            "Sketcher_ConstrainHorVer" => HorVer,
            "Sketcher_ConstrainHorizontal" => Constraint(Choice::Horizontal),
            "Sketcher_ConstrainVertical" => Constraint(Choice::Vertical),
            "Sketcher_ConstrainCoincidentUnified" => Coincident,
            "Sketcher_ConstrainCoincident" => Constraint(Choice::Coincident),
            "Sketcher_ConstrainPointOnObject" => PointOnObject,
            "Sketcher_ConstrainParallel" => Constraint(Choice::Parallel),
            "Sketcher_ConstrainPerpendicular" => Constraint(Choice::Perpendicular),
            "Sketcher_ConstrainTangent" => Tangent,
            "Sketcher_ConstrainEqual" => Equal,
            "Sketcher_ConstrainSymmetric" => Symmetric,
            "Sketcher_ConstrainDistance" => Constraint(Choice::Distance),
            "Sketcher_ConstrainDistanceX" => Constraint(Choice::HorizontalDistance),
            "Sketcher_ConstrainDistanceY" => Constraint(Choice::VerticalDistance),
            "Sketcher_ConstrainRadius" => Constraint(Choice::Radius),
            "Sketcher_ConstrainDiameter" => Constraint(Choice::Diameter),
            "Sketcher_ConstrainRadiam" | "Sketcher_CompConstrainRadDia" => Radiam,
            "Sketcher_ConstrainAngle" => Constraint(Choice::Angle),
            "Sketcher_ConstrainLock" => Lock,
            "Sketcher_ConstrainBlock" => Constraint(Choice::Fixed),
            "Sketcher_ToggleConstruction" => Construction,
            "Sketcher_ToggleDrivingConstraint" => Reference,
            "Sketcher_SelectOrigin" => SelectReference(ORIGIN_ID),
            "Sketcher_SelectVerticalAxis" => SelectReference(Y_AXIS_ID),
            "Sketcher_SelectHorizontalAxis" => SelectReference(X_AXIS_ID),
            "Sketcher_SelectElementsAssociatedWithConstraints" => AssociatedGeometry,
            "Sketcher_RemoveAxesAlignment" => RemoveAlignment,
            "Sketcher_ViewSketch" => View,
            _ => return None,
        })
    }
}
fn exact(keys: &[Key]) -> Option<&'static Binding> {
    BINDINGS
        .iter()
        .find(|b| b.modifiers == Modifiers::NONE && b.keys == keys)
}
fn sequence_text(keys: &[Key]) -> String {
    keys.iter()
        .map(|key| key.name())
        .collect::<Vec<_>>()
        .join(", ")
}
impl Tool {
    pub(super) fn shortcut(self) -> &'static str {
        BINDINGS
            .iter()
            .find(|b| b.action() == Some(Action::Draw(self)))
            .map_or("Esc", |b| b.text)
    }
}
impl Choice {
    pub(super) fn shortcut(self) -> &'static str {
        match self {
            Self::Coincident => "C / C,C",
            Self::EqualLength | Self::EqualRadius => "E",
            Self::PointOnLine | Self::PointOnCircle => "O / C",
            Self::Tangent => "T",
            Self::Symmetry | Self::Midpoint => "S",
            Self::Length => "K,D / D",
            // Block on a selected point uses the same fixed-point primitive.
            Self::FixedPoint => "K,B",
            _ => BINDINGS
                .iter()
                .find(|b| b.action() == Some(Action::Constraint(self)))
                .map_or("", |b| b.text),
        }
    }
}
impl Editor {
    pub fn shortcut_prefix_clear(&mut self) {
        self.shortcut_prefix = None;
    }
    fn run_binding(&mut self, binding: &Binding) {
        let Some(action) = binding.action() else {
            self.message = format!(
                "{} ({}) is not available in this sketch editor yet. See Shortcuts for supported commands.",
                binding.label, binding.text
            );
            return;
        };
        let result = match action {
            Action::Draw(tool) => {
                self.set_tool(tool);
                Ok(())
            }
            Action::Constraint(choice) => {
                self.constraint_shortcut(choice);
                Ok(())
            }
            Action::Dimension => {
                self.dimension_shortcut();
                Ok(())
            }
            Action::HorVer => self.auto_alignment(),
            Action::Equal => {
                self.equal_shortcut();
                Ok(())
            }
            Action::Coincident => {
                self.coincident_shortcut();
                Ok(())
            }
            Action::PointOnObject => self.point_on_object_shortcut(),
            Action::Tangent => self.tangent_shortcut(),
            Action::Symmetric => {
                self.symmetry_shortcut();
                Ok(())
            }
            Action::Radiam => self.radiam_shortcut(),
            Action::Lock => self.lock_position_shortcut(),
            Action::Construction => {
                if self.tool == Tool::Select && !self.selected.is_empty() {
                    self.toggle_construction()
                } else {
                    self.construction = !self.construction;
                    Ok(())
                }
            }
            Action::Reference => self.toggle_reference_shortcut(),
            Action::SelectReference(id) => {
                if !self.finish_dimension_edit() {
                    return;
                }
                self.set_tool(Tool::Select);
                self.pick_operand(
                    Some(Operand::reference(reference_entity(id).unwrap())),
                    true,
                );
                Ok(())
            }
            Action::AssociatedGeometry => self.associated_geometry_shortcut(),
            Action::RemoveAlignment => self.remove_alignment_shortcut(),
            Action::View => {
                self.message = "The sketch is already displayed in its local XY plane.".into();
                Ok(())
            }
        };
        if let Err(e) = result {
            self.message = e;
        }
    }
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        // Text entry, IME (guarded by the workspace), and modal help own their keys.
        if ctx.wants_keyboard_input() || self.show_shortcuts {
            self.shortcut_prefix_clear();
            return;
        }
        let now = ctx.input(|i| i.time);
        if let Some((keys, deadline)) = &self.shortcut_prefix
            && now >= *deadline
        {
            let binding = exact(keys);
            self.shortcut_prefix_clear();
            if let Some(binding) = binding {
                self.run_binding(binding);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            if self.finish_dimension_edit() {
                self.cancel_gesture();
                self.saved.constraint_form = Default::default();
                self.saved.constraint_form.operands = self
                    .saved
                    .model
                    .entities
                    .iter()
                    .map(|e| Operand::Entity(e.id.clone()))
                    .collect();
                self.selected = self
                    .saved
                    .model
                    .entities
                    .iter()
                    .map(|e| e.id.clone())
                    .collect();
                self.tool = Tool::Select;
            }
            self.shortcut_prefix_clear();
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let clipboard = match &event {
                egui::Event::Copy => Some(Key::C),
                egui::Event::Cut => Some(Key::X),
                egui::Event::Paste(_) => Some(Key::V),
                _ => None,
            };
            if let Some(key) = clipboard {
                self.shortcut_prefix_clear();
                if let Some(b) = BINDINGS
                    .iter()
                    .find(|b| b.modifiers == Modifiers::COMMAND && b.keys == [key])
                {
                    self.run_binding(b);
                }
                continue;
            }
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if modifiers != Modifiers::NONE {
                self.shortcut_prefix_clear();
                if let Some(b) = BINDINGS.iter().find(|b| {
                    b.modifiers != Modifiers::NONE
                        && modifiers.matches_logically(b.modifiers)
                        && b.keys == [key]
                }) {
                    ctx.input_mut(|i| i.consume_key(modifiers, key));
                    self.run_binding(b);
                }
                continue;
            }
            let mut keys = self.shortcut_prefix.take().map_or_else(Vec::new, |p| p.0);
            let pending = exact(&keys);
            keys.push(key);
            if !BINDINGS
                .iter()
                .any(|b| b.modifiers == Modifiers::NONE && b.keys.starts_with(&keys))
            {
                if let Some(binding) = pending {
                    self.run_binding(binding);
                    keys = vec![key];
                } else if keys.len() > 1 {
                    // A mistyped prefix must never fall through to a bare constraint.
                    self.message = format!("No Sketcher shortcut for {}.", sequence_text(&keys));
                    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
                    continue;
                }
            }
            let matches: Vec<_> = BINDINGS
                .iter()
                .filter(|b| b.modifiers == Modifiers::NONE && b.keys.starts_with(&keys))
                .collect();
            if matches.is_empty() {
                continue;
            }
            ctx.input_mut(|i| {
                i.consume_key(Modifiers::NONE, key);
                // Do not insert the last key of a shortcut into a newly focused value field.
                i.events.retain(
                    |e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case(key.name())),
                );
            });
            if matches.iter().any(|b| b.keys.len() > keys.len()) {
                // C and C,C (also G,S / G,S,2 and Z,P / Z,P,*) share prefixes.
                let timeout = if exact(&keys).is_some() { 0.7 } else { 2. };
                self.shortcut_prefix = Some((keys, now + timeout));
            } else if let Some(binding) = exact(&keys) {
                self.run_binding(binding);
            }
        }
        if self.shortcut_prefix.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    pub fn prefix_hint(&self) -> String {
        let Some((keys, _)) = &self.shortcut_prefix else {
            return String::new();
        };
        let choices = BINDINGS
            .iter()
            .filter(|b| {
                b.modifiers == Modifiers::NONE && b.keys.starts_with(keys) && b.action().is_some()
            })
            .map(|b| format!("{} {}", b.text, b.label))
            .collect::<Vec<_>>();
        if choices.is_empty() {
            format!(
                "{} … · Esc cancels · Shortcuts lists availability",
                sequence_text(keys)
            )
        } else {
            choices.join(" · ")
        }
    }
    pub fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        egui::Window::new("Sketcher shortcuts · FreeCAD 1.1.4")
            .open(&mut open).default_width(620.).default_height(440.)
            .vscroll(true).show(ctx, |ui| {
                ui.label("Press comma-separated keys in order. Commands are inactive while typing. Esc cancels a sequence.");
                ui.small("C waits briefly for C,C. Other sequences expire after two seconds. Missing tools report their availability without changing the sketch.");
                ui.label("Ctrl+A: select geometry · Ctrl+Z: undo · Ctrl+Y / Ctrl+Shift+Z: redo · Delete: delete selection");
                ui.label("Dimension field: Enter saves · Esc discards · selecting the next geometry saves");
                egui::Grid::new("sketch-shortcuts-reference").striped(true).show(ui, |ui| {
                    for b in BINDINGS {
                        ui.monospace(b.text); ui.label(b.label);
                        ui.label(if b.action().is_some() { "Available" } else { "Not available yet" }); ui.end_row();
                    }
                });
                ui.hyperlink_to("FreeCAD 1.1.4 source", "https://github.com/FreeCAD/FreeCAD/tree/1.1.4/src/Mod/Sketcher/Gui");
            });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            open = false;
        }
        self.show_shortcuts = open;
    }
}
