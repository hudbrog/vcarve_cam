use super::*;
use crate::app::observe_control;
use constraints::{Choice, Operand};
impl Editor {
    pub fn choose_constraint(&mut self, mut choice: Choice) {
        if !self.finish_dimension_edit() {
            return;
        }
        self.cancel_gesture();
        self.tool = Tool::Select;
        self.shortcut_prefix = None;
        let form = &mut self.saved.constraint_form;
        // FreeCAD's Distance command also dimensions a single line.
        if choice == Choice::Distance
            && matches!(form.operands.as_slice(), [Operand::Entity(id)] if reference_entity(id).is_none() && self.saved.model.entity(id).is_ok_and(|e| e.geometry.line().is_some()))
        {
            choice = Choice::Length;
        }
        form.choice = choice;
        form.editing = None;
        form.value.clear();
        form.reference = choice.dimension() && self.new_dimensions_reference;
        form.changed = false;
        self.show_constraints = true;
        // The form already shows selection guidance. Reserve the error banner
        // for an actual failed action, not normal dimension entry.
        self.message.clear();
        if !choice.dimension() {
            if let Err(e) = self.store_constraint() {
                self.message = e;
            }
        } else {
            self.prepare_dimension();
        }
    }

    pub fn tools_panel(&mut self, ui: &mut egui::Ui) {
        observe_control("Sketch tools panel", ui.clip_rect());
        ui.heading("Tools");
        let r = ui.button("Shortcuts");
        observe_control("Sketch shortcuts", r.rect);
        if r.clicked() {
            self.show_shortcuts = !self.show_shortcuts;
            self.shortcut_prefix_clear();
        }
        let r = ui.button("Dimension · D").on_hover_text("Dimension the selection: line length, circle diameter, arc radius, angle between lines, or point distance.");
        observe_control("Sketch dimension", r.rect);
        if r.clicked() {
            self.dimension_shortcut();
        }
        ui.strong("Draw");
        egui::Grid::new("sketch-draw-tools")
            .num_columns(2)
            .show(ui, |ui| {
                for (i, (tool, label)) in Tool::ALL.into_iter().enumerate() {
                    let key = tool.shortcut();
                    let r = ui
                        .selectable_label(self.tool == tool, label)
                        .on_hover_text(format!("{label} · {key}\n{}", tool.hint()));
                    observe_control(&format!("Sketch {label}"), r.rect);
                    if r.clicked() {
                        self.set_tool(tool);
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        let r = ui
            .checkbox(&mut self.construction, "Construction")
            .on_hover_text("New geometry is construction · G, N");
        observe_control("Sketch construction", r.rect);
        if r.changed() {
            self.cancel_gesture();
        }
        for dimensions in [false, true] {
            ui.separator();
            ui.strong(if dimensions {
                "Dimensions"
            } else {
                "Constraints"
            });
            if !dimensions {
                ui.horizontal(|ui| {
                    if ui.button("Auto H/V · A").clicked()
                        && let Err(e) = self.auto_alignment()
                    {
                        self.message = e;
                    }
                    if ui
                        .button("Lock · K,L")
                        .on_hover_text("Lock selected points with X and Y dimensions")
                        .clicked()
                        && let Err(e) = self.lock_position_shortcut()
                    {
                        self.message = e;
                    }
                });
            } else {
                let r = ui.checkbox(
                    &mut self.new_dimensions_reference,
                    "New dimensions are reference",
                );
                observe_control("Sketch reference mode", r.rect);
                r.on_hover_text("K, X toggles the selected dimension, or this creation mode when no constraint is selected.");
            }
            let width = ((ui.available_width() - 12.) / 2.).max(65.);
            egui::Grid::new(("sketch-constraint-tools", dimensions))
                .num_columns(2)
                .show(ui, |ui| {
                    for (i, choice) in Choice::ALL
                        .into_iter()
                        .filter(|c| c.dimension() == dimensions)
                        .enumerate()
                    {
                        let r = ui
                            .add_sized([width, 30.], egui::Button::new(choice.label()).wrap())
                            .on_hover_text(format!(
                                "{} · {}\n{}",
                                choice.label(),
                                choice.shortcut(),
                                choice.hint()
                            ));
                        observe_control(&format!("Sketch constrain {}", choice.label()), r.rect);
                        if r.clicked() {
                            self.constraint_shortcut(choice);
                        }
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
        }
        ui.separator();
        for label in ["Select wire", "Toggle construction", "Delete selection"] {
            let r = ui.add_enabled(!self.selected.is_empty(), egui::Button::new(label));
            observe_control(label, r.rect);
            if r.clicked() {
                let result = match label {
                    "Select wire" => {
                        let ids: Vec<_> = self.selected.iter().cloned().collect();
                        if !self.pick_operand(None, false) {
                            return;
                        }
                        for id in ids {
                            if !self.selected.contains(&id) {
                                self.select_wire(Some(&id), true);
                            }
                        }
                        Ok(())
                    }
                    "Toggle construction" => self.toggle_construction(),
                    _ => self.delete(),
                };
                if let Err(e) = result {
                    self.message = e;
                }
            }
        }
    }
    pub fn geometry_panel(&mut self, ui: &mut egui::Ui) {
        observe_control("Geometry panel", ui.clip_rect());
        ui.heading("Geometry");
        let r = ui.button("Locate selection");
        observe_control("Locate selection", r.rect);
        if r.clicked() {
            self.locate_selected();
        }
        ui.small("Fixed construction references");
        for e in reference_entities() {
            let o = Operand::reference(e);
            let r = ui
                .selectable_label(
                    self.saved.constraint_form.operands.contains(&o),
                    self.operand_label(&o),
                )
                .on_hover_text(
                    "Fixed local reference. Shift-click adds it to the current selection.",
                );
            observe_control(&format!("Geometry row {}", e.id), r.rect);
            if r.clicked() {
                self.set_tool(Tool::Select);
                self.pick_operand(Some(o), ui.input(|i| i.modifiers.shift));
            }
        }
        ui.separator();
        for e in self.saved.model.entities.clone() {
            let o = Operand::Entity(e.id.clone());
            let label = format!(
                "{}{}",
                self.operand_label(&o),
                if e.construction {
                    " · construction"
                } else {
                    ""
                }
            );
            let r = ui.selectable_label(self.saved.constraint_form.operands.contains(&o), label);
            observe_control(&format!("Geometry row {}", e.id), r.rect);
            if r.clicked() {
                self.set_tool(Tool::Select);
                self.pick_operand(Some(o), ui.input(|i| i.modifiers.shift));
            }
            if r.double_clicked() {
                self.select_wire(Some(&e.id), false);
                self.locate_selected();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};
    #[test]
    fn shortcut_sequences_respect_text_focus_and_do_not_confuse_prefixes_with_constraints() {
        let mut e = Editor::new(SavedDraft {
            target: None,
            source_revision: None,
            name: "Test".into(),
            model: SketchDocument::default(),
            had_document: true,
            constraint_form: Default::default(),
        });
        let ctx = egui::Context::default();
        let mut time = 0.;
        let mut text = String::new();
        let mut frame = |e: &mut Editor, key, focus| {
            time += 0.1;
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    events: vec![
                        egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        },
                        egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed: false,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let r = ui.text_edit_singleline(&mut text);
                        if focus {
                            r.request_focus();
                        }
                        e.shortcuts(ctx);
                    });
                },
            );
        };
        frame(&mut e, Key::G, false);
        assert!(!e.prefix_hint().is_empty());
        frame(&mut e, Key::C, false);
        assert_eq!(e.tool, Tool::Circle);
        assert!(!e.show_constraints);
        frame(&mut e, Key::G, false);
        frame(&mut e, Key::Num3, false);
        frame(&mut e, Key::A, false);
        assert_eq!(e.tool, Tool::Arc);
        frame(&mut e, Key::G, true);
        frame(&mut e, Key::L, true);
        assert_eq!(e.tool, Tool::Arc);
        assert!(e.shortcut_prefix.is_none());
    }
}
