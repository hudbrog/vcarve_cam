use super::*;
use crate::app::observe_control;
use constraints::{Choice, Operand};
use egui::{Key, Modifiers};

impl Choice {
    fn shortcut(self) -> &'static str {
        match self {
            Self::Horizontal => "H",
            Self::Vertical => "V",
            Self::Coincident => "C",
            Self::Parallel => "P",
            Self::Perpendicular => "N",
            Self::EqualLength => "E",
            Self::Tangent => "T",
            Self::Symmetry => "S",
            Self::PointOnLine => "O",
            Self::Distance => "K, D",
            Self::HorizontalDistance => "L",
            Self::VerticalDistance => "I",
            Self::Radius => "K, R",
            Self::Diameter => "K, O",
            Self::Angle => "K, A",
            Self::Fixed => "K, B",
            _ => "",
        }
    }
}

impl Editor {
    pub fn shortcut_prefix_clear(&mut self) {
        self.shortcut_prefix = None;
    }
    pub fn choose_constraint(&mut self, mut choice: Choice) {
        self.cancel_gesture();
        self.tool = Tool::Select;
        self.shortcut_prefix = None;
        let form = &mut self.saved.constraint_form;
        // FreeCAD's Distance command also dimensions a single line.
        if choice == Choice::Distance
            && matches!(form.operands.as_slice(), [Operand::Entity(id)] if self.saved.model.entity(id).is_ok_and(|e| e.geometry.line().is_some()))
        {
            choice = Choice::Length;
        }
        form.choice = choice;
        form.editing = None;
        form.value.clear();
        form.reference = false;
        form.changed = false;
        self.show_constraints = true;
        self.message = choice.hint().into();
        if !choice.dimension() {
            if let Err(e) = self.store_constraint() {
                self.message = e;
            }
        } else {
            self.focus_dimension = true;
        }
    }

    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.shortcut_prefix.is_some_and(|(_, t)| now - t > 2.) {
            self.shortcut_prefix = None;
        }
        if ctx.wants_keyboard_input() {
            self.shortcut_prefix = None;
            return;
        }
        // Consume key events, not text: typing into a dimension never activates a tool.
        let keys = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if *modifiers == Modifiers::NONE => Some(*key),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        for key in keys {
            let prefix = self.shortcut_prefix.take().map(|p| p.0);
            if let Some(prefix) = prefix {
                ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
                match (prefix, key) {
                    (Key::G, Key::L) => self.set_tool(Tool::Line),
                    (Key::G, Key::M) => self.set_tool(Tool::Polyline),
                    (Key::G, Key::R) => self.set_tool(Tool::Rectangle),
                    (Key::G, Key::C) => self.set_tool(Tool::Circle),
                    (Key::G, Key::Y) => self.set_tool(Tool::Point),
                    (Key::G, Key::Num3) => self.shortcut_prefix = Some((Key::Num3, now)),
                    (Key::Num3, Key::A) => self.set_tool(Tool::Arc),
                    (Key::G, Key::N) => {
                        if self.tool == Tool::Select && !self.selected.is_empty() {
                            if let Err(e) = self.toggle_construction() {
                                self.message = e;
                            }
                        } else {
                            self.construction = !self.construction;
                        }
                    }
                    (Key::K, Key::D) => self.choose_constraint(Choice::Distance),
                    (Key::K, Key::R) => self.choose_constraint(Choice::Radius),
                    (Key::K, Key::O) => self.choose_constraint(Choice::Diameter),
                    (Key::K, Key::A) => self.choose_constraint(Choice::Angle),
                    (Key::K, Key::B) => self.choose_constraint(Choice::Fixed),
                    (Key::K, Key::X) if self.saved.constraint_form.choice.dimension() => {
                        self.show_constraints = true;
                        self.saved.constraint_form.reference =
                            !self.saved.constraint_form.reference;
                        self.saved.constraint_form.changed = true;
                    }
                    _ => {}
                }
                continue;
            }
            if matches!(key, Key::G | Key::K) {
                self.shortcut_prefix = Some((key, now));
                ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
                continue;
            }
            let choice = match key {
                Key::H => Choice::Horizontal,
                Key::V => Choice::Vertical,
                Key::C => Choice::Coincident,
                Key::P => Choice::Parallel,
                Key::N => Choice::Perpendicular,
                Key::E => Choice::EqualLength,
                Key::T => Choice::Tangent,
                Key::S => Choice::Symmetry,
                Key::O => Choice::PointOnLine,
                Key::L => Choice::HorizontalDistance,
                Key::I => Choice::VerticalDistance,
                _ => continue,
            };
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            self.choose_constraint(choice);
        }
        if self.shortcut_prefix.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub fn tools_panel(&mut self, ui: &mut egui::Ui) {
        observe_control("Sketch tools panel", ui.clip_rect());
        ui.heading("Tools");
        ui.strong("Draw");
        egui::Grid::new("sketch-draw-tools")
            .num_columns(2)
            .show(ui, |ui| {
                for (i, (tool, label)) in Tool::ALL.into_iter().enumerate() {
                    let key = match tool {
                        Tool::Select => "Esc",
                        Tool::Line => "G, L",
                        Tool::Polyline => "G, M",
                        Tool::Rectangle => "G, R",
                        Tool::Circle => "G, C",
                        Tool::Arc => "G, 3, A",
                        Tool::Point => "G, Y",
                    };
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
                            self.choose_constraint(choice);
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
                        self.pick_operand(None, false);
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
    pub fn prefix_hint(&self) -> &'static str {
        match self.shortcut_prefix.map(|p| p.0) {
            Some(Key::G) => {
                "G → L Line · M Polyline · R Rectangle · C Circle · 3,A Arc · Y Point · N Construction"
            }
            Some(Key::Num3) => "G, 3 → A Three-point arc",
            Some(Key::K) => {
                "K → D Distance/length · R Radius · O Diameter · A Angle · B Fix · X Driving/reference"
            }
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
