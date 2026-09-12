//! Library browsing stays separate from the scrolling editor and job actions.
use super::*;

fn action(ui: &mut egui::Ui, title: &str, probe: &str, enabled: bool) -> egui::Response {
    let r = button(ui, title, enabled);
    observe_control(probe, r.rect);
    r
}

impl App {
    pub(super) fn library_window(&mut self, ctx: &egui::Context) {
        if !self.resources.ready && !self.resources.busy && self.resources.error.is_none() {
            self.request_resources(ResourceIntent::Load, ctx);
        }
        self.library_selection();
        let screen = ctx.content_rect();
        let size = egui::vec2(1000., 640.).min(screen.size() - egui::vec2(72., 72.));
        egui::Window::new("Library")
            .id(egui::Id::new("library-browser"))
            .title_bar(false)
            .collapsible(false)
            .default_pos(screen.center() - size * 0.5)
            .default_size(size)
            .min_size(egui::vec2(680., 400.).min(size))
            .max_size(screen.size() - egui::vec2(72., 72.))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(16.))
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Library").size(25.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if action(ui, "×", "Close library", true)
                            .on_hover_text(
                                "Close library; unsaved edits stay here until you save or reload.",
                            )
                            .clicked()
                        {
                            self.resources.open = false;
                        }
                        let menu = ui.menu_button("Library actions  ···", |ui| {
                            self.library_actions(ui, ctx)
                        });
                        observe_control("Library actions", menu.response.rect);
                    });
                });
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    for (title, machines) in [("Tools & profiles", false), ("Machines", true)] {
                        let selected = self.resources.machines_view == machines;
                        let r = ui.add_sized(
                            [190., 38.],
                            egui::Button::new(RichText::new(title).size(17.).strong())
                                .selected(selected),
                        );
                        observe_control(title, r.rect);
                        if selected {
                            ui.painter().hline(
                                r.rect.x_range(),
                                r.rect.bottom(),
                                egui::Stroke::new(3., Color32::from_rgb(21, 148, 157)),
                            );
                        }
                        if r.clicked() {
                            self.resources.machines_view = machines;
                        }
                    }
                });
                ui.separator();
                let body_height = (ui
                    .available_height()
                    .min(screen.bottom() - ui.cursor().min.y - 28.)
                    - 96.)
                    .max(160.);
                ui.add_enabled_ui(self.resources.ready && !self.resources.busy, |ui| {
                    ui.horizontal_top(|ui| {
                        let width = (ui.available_width() * 0.27).clamp(200., 270.);
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(width);
                                self.library_list(ui, body_height);
                            },
                        );
                        let (divider, _) = ui
                            .allocate_exact_size(egui::vec2(1., body_height), egui::Sense::hover());
                        ui.painter().vline(
                            divider.center().x,
                            divider.y_range(),
                            egui::Stroke::new(1., Color32::from_rgb(209, 219, 225)),
                        );
                        let width = ui.available_width();
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(width);
                                let selected = if self.resources.machines_view {
                                    &self.resources.machine
                                } else {
                                    &self.resources.tool
                                };
                                let scroll = egui::ScrollArea::vertical()
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                    )
                                    .id_salt((
                                        "library-details",
                                        self.resources.machines_view,
                                        selected,
                                    ))
                                    .auto_shrink([false, false])
                                    .max_height(body_height)
                                    .show(ui, |ui| {
                                        self.library_details_header(ui, ctx);
                                        if self.resources.machines_view {
                                            self.library_machines(ui, ctx);
                                        } else {
                                            self.library_tools(ui, ctx);
                                        }
                                    });
                                observe_control("Resource viewport", scroll.inner_rect);
                            },
                        );
                    });
                });
                ui.separator();
                let footer = ui.scope(|ui| self.library_footer(ui, ctx));
                observe_control("Library footer", footer.response.rect);
                observe_control("Library window", ui.max_rect());
            });
    }

    fn library_selection(&mut self) {
        if !self
            .resources
            .draft
            .library
            .tools
            .iter()
            .any(|t| t.id == self.resources.tool)
        {
            if let Some(t) = self.resources.draft.library.tools.first() {
                self.resources.tool = t.id.clone();
                self.resources.preset = t
                    .cutting_presets
                    .first()
                    .map(|p| p.id.clone())
                    .unwrap_or_default();
            } else {
                self.resources.tool.clear();
                self.resources.preset.clear();
            }
        }
        if !self
            .resources
            .draft
            .machines
            .iter()
            .any(|m| m.id == self.resources.machine)
        {
            self.resources.machine = self
                .resources
                .draft
                .machines
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_default();
        }
    }

    fn library_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let idle = !self.resources.busy && self.io.is_none();
        if action(ui, "Import library…", "Import library", idle && self.resources.ready && !self.resources.dirty)
            .on_hover_text("Replace the library edit buffer from a file. Save pending edits first; imported data is saved only when you choose Save changes.").clicked() {
            self.open(IoKind::LibraryImport, ctx); ui.close();
        }
        if action(
            ui,
            "Import machine profile…",
            "Import machine configuration",
            idle && self.resources.ready,
        )
        .clicked()
        {
            self.open(IoKind::MachineImport, ctx);
            ui.close();
        }
        if action(
            ui,
            "Export library…",
            "Export library",
            idle && self.resources.ready && self.resources.invalid.is_empty(),
        )
        .clicked()
        {
            match self.resources.draft.validate() {
                Ok(()) => self.save(
                    "cam-library.json".into(),
                    serde_json::to_vec_pretty(&self.resources.draft).unwrap(),
                    None,
                    ctx,
                ),
                Err(e) => {
                    self.resources.status = e.clone();
                    self.resources.error = Some(e);
                }
            }
            ui.close();
        }
        ui.separator();
        if action(ui, "Reload saved library", "Load library", idle && !self.resources.dirty)
            .on_hover_text("Read this computer's saved library again. This does not import a file. Save pending edits before reloading.").clicked() {
            self.request_resources(ResourceIntent::Load, ctx); ui.close();
        }
        if button(ui, "Compare stored revision", idle && self.resources.ready).clicked() {
            self.request_resources(ResourceIntent::Compare, ctx);
            ui.close();
        }
        if let Some(other) = self.resources.conflict.clone() {
            ui.separator();
            ui.label(format!("Stored revision {}", other.revision));
            egui::CollapsingHeader::new("Compare saved content").show(ui, |ui| {
                egui::ScrollArea::both().max_height(200.).show(ui, |ui| {
                    ui.strong("Stored content");
                    ui.monospace(serde_json::to_string_pretty(&other.snapshot).unwrap());
                    ui.strong("Your edit buffer");
                    ui.monospace(serde_json::to_string_pretty(&self.resources.draft).unwrap());
                });
            });
            if button(ui, "Reload stored library", idle)
                .on_hover_text("Replace your edit buffer with this compared saved revision.")
                .clicked()
            {
                self.resources.draft = other.snapshot.clone();
                self.resources.base = Some(other.clone());
                self.resources.dirty = false;
                self.resources.raw.clear();
                self.resources.invalid.clear();
                self.resources.conflict = None;
                self.resources.error = None;
                ui.close();
            }
            if button(ui, "Overwrite reviewed revision", idle).clicked() {
                self.resources.base = Some(other);
                self.request_resources(ResourceIntent::Save, ctx);
                ui.close();
            }
        }
    }

    fn library_list(&mut self, ui: &mut egui::Ui, height: f32) {
        let machines = self.resources.machines_view;
        let count = if machines {
            self.resources.draft.machines.len()
        } else {
            self.resources.draft.library.tools.len()
        };
        ui.heading(format!(
            "{}  ·  {count}",
            if machines { "Machines" } else { "Tools" }
        ));
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.resources.search[usize::from(machines)])
                .hint_text(if machines {
                    "Search machines"
                } else {
                    "Search tools"
                })
                .desired_width(f32::INFINITY),
        );
        observe_control("Library search", r.rect);
        let menu = egui::containers::menu::MenuButton::from_button(
            egui::Button::new(if machines {
                "+ New machine"
            } else {
                "+ New tool"
            })
            .min_size(egui::vec2(ui.available_width(), 32.)),
        )
        .ui(ui, |ui| {
            ui.set_width(300.);
            if machines {
                text(ui, "New machine ID", &mut self.resources.new_machine_id);
                let valid = cam_core::preview::valid_id(&self.resources.new_machine_id);
                if !valid {
                    ui.small("Use letters, numbers, hyphens or underscores.");
                }
                if action(
                    ui,
                    "Create machine",
                    "New machine profile",
                    valid && count < 100,
                )
                .clicked()
                {
                    let requested = self.resources.new_machine_id.clone();
                    let id = if self
                        .resources
                        .draft
                        .machines
                        .iter()
                        .any(|m| m.id == requested)
                    {
                        unique(
                            &requested,
                            self.resources.draft.machines.iter().map(|m| m.id.clone()),
                        )
                    } else {
                        requested
                    };
                    self.resources.machine = id.clone();
                    self.resources
                        .draft
                        .machines
                        .push(crate::resources::new_machine(id));
                    self.resources.dirty = true;
                    self.resources.search[1].clear();
                    ui.close();
                }
                ui.separator();
                if button(
                    ui,
                    "Capture applied machine",
                    self.document
                        .as_ref()
                        .is_some_and(|d| d.job.machine_configuration.is_some()),
                )
                .clicked()
                {
                    self.library_capture_machine();
                    ui.close();
                }
            } else {
                for (label, role) in [
                    ("New endmill", Role::Endmill),
                    ("New V-bit", Role::Vbit),
                    ("New drag knife", Role::Knife),
                ] {
                    if button(ui, label, count < 1000).clicked() {
                        self.library_new_tool(label, role);
                        ui.close();
                    }
                }
                if button(ui, "Capture job geometry", self.document.is_some()).clicked() {
                    self.library_capture_tool();
                    ui.close();
                }
            }
        });
        observe_control(
            if machines { "New machine" } else { "New tool" },
            menu.0.rect,
        );
        ui.add_space(4.);
        let query = self.resources.search[usize::from(machines)].to_lowercase();
        let rows: Vec<_> = if machines {
            self.resources
                .draft
                .machines
                .iter()
                .filter(|m| m.id.to_lowercase().contains(&query))
                .map(|m| {
                    let summary = if m.validate_shape().is_ok() {
                        format!("LinuxCNC · {}", m.work_offset)
                    } else {
                        "Needs setup".into()
                    };
                    (m.id.clone(), m.id.clone(), summary)
                })
                .collect()
        } else {
            self.resources
                .draft
                .library
                .tools
                .iter()
                .filter(|t| {
                    format!("{} {}", t.name, t.id)
                        .to_lowercase()
                        .contains(&query)
                })
                .map(|t| {
                    let summary = if t.validate().is_err() {
                        "Needs setup".into()
                    } else {
                        match &t.geometry {
                            LibraryGeometry::Endmill(g) => format!(
                                "Endmill · {} mm · {} profiles",
                                g.diameter_mm,
                                t.cutting_presets.len()
                            ),
                            LibraryGeometry::Vbit(g) => format!(
                                "V-bit · {}° · {} profiles",
                                g.included_angle_deg,
                                t.cutting_presets.len()
                            ),
                            LibraryGeometry::DragKnife(_) => "Drag knife".into(),
                        }
                    };
                    (t.id.clone(), t.name.clone(), summary)
                })
                .collect()
        };
        let list = egui::ScrollArea::vertical()
            .id_salt(("library-list", machines))
            .auto_shrink([false, false])
            .max_height((height - 115.).max(60.))
            .show(ui, |ui| {
                if rows.is_empty() {
                    ui.label(if count == 0 {
                        "Create an item or import a library to begin."
                    } else {
                        "No matching items."
                    });
                }
                for (id, name, summary) in rows {
                    let selected = if machines {
                        self.resources.machine == id
                    } else {
                        self.resources.tool == id
                    };
                    let r = ui.add_sized(
                        [ui.available_width(), 60.],
                        egui::Button::new(format!("{name}\n{summary}"))
                            .selected(selected)
                            .wrap(),
                    );
                    observe_control(
                        &format!("Library {} {id}", if machines { "machine" } else { "tool" }),
                        r.rect,
                    );
                    if r.clicked() {
                        if machines {
                            self.resources.machine = id;
                        } else {
                            self.resources.tool = id.clone();
                            self.resources.preset = self
                                .resources
                                .draft
                                .library
                                .tools
                                .iter()
                                .find(|t| t.id == id)
                                .and_then(|t| t.cutting_presets.first())
                                .map(|p| p.id.clone())
                                .unwrap_or_default();
                        }
                    }
                }
            });
        observe_control("Resource list viewport", list.inner_rect);
    }

    fn library_details_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let machines = self.resources.machines_view;
        let name = if machines {
            self.resources
                .draft
                .machines
                .iter()
                .find(|m| m.id == self.resources.machine)
                .map(|m| m.id.clone())
        } else {
            self.resources
                .draft
                .library
                .tools
                .iter()
                .find(|t| t.id == self.resources.tool)
                .map(|t| t.name.clone())
        };
        let Some(name) = name else {
            ui.heading("Select an item");
            return;
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(name);
                ui.small(if machines {
                    "Machine profile"
                } else {
                    "Tool geometry & cutting profiles"
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                let menu = ui.menu_button("···", |ui| {
                    if machines {
                        if button(ui, "Duplicate machine configuration", true).clicked() {
                            if let Some(mut m) = self
                                .resources
                                .draft
                                .machines
                                .iter()
                                .find(|m| m.id == self.resources.machine)
                                .cloned()
                            {
                                m.id = unique(
                                    &m.id,
                                    self.resources.draft.machines.iter().map(|m| m.id.clone()),
                                );
                                self.resources.machine = m.id.clone();
                                self.resources.draft.machines.push(m);
                                self.resources.dirty = true;
                                self.resources.search[1].clear();
                            }
                            ui.close();
                        }
                    } else {
                        if button(ui, "Duplicate library tool", true).clicked() {
                            if let Some(mut t) = self
                                .resources
                                .draft
                                .library
                                .tools
                                .iter()
                                .find(|t| t.id == self.resources.tool)
                                .cloned()
                            {
                                t.id = unique(
                                    "tool",
                                    self.resources
                                        .draft
                                        .library
                                        .tools
                                        .iter()
                                        .map(|t| t.id.clone()),
                                );
                                t.name.push_str(" copy");
                                self.resources.tool = t.id.clone();
                                self.resources.draft.library.tools.push(t);
                                self.resources.dirty = true;
                                self.resources.search[0].clear();
                            }
                            ui.close();
                        }
                        let ready = self.library_can_use();
                        if button(ui, "Add geometry to job", ready).clicked() {
                            self.resource_command(
                                R::AddTool {
                                    catalog: self.resources.base.as_ref().unwrap().snapshot.clone(),
                                    tool: self.resources.tool.clone(),
                                },
                                ctx,
                            );
                            ui.close();
                        }
                    }
                });
                observe_control("Library item actions", menu.response.rect);
            });
        });
        ui.add_space(8.);
    }

    fn library_can_use(&self) -> bool {
        self.resources.ready
            && self.resources.base.is_some()
            && !self.resources.dirty
            && self.resources.invalid.is_empty()
            && !self.resources.busy
            && self.document.is_some()
            && self.active.is_none()
            && self.io.is_none()
    }

    fn library_footer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(error) = &self.resources.error {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::DARK_RED, "Library needs attention")
                    .on_hover_text(error);
                let menu = ui.menu_button("Show details", |ui| {
                    ui.set_max_width(500.);
                    ui.label(error);
                });
                observe_control("Library error details", menu.response.rect);
            });
        } else if self.resources.conflict.is_some() {
            ui.small("Stored revision available to compare in Library actions.");
        } else if self.resources.machines_view {
            ui.small("Using a profile copies its settings into this job. Set tool numbers in the job's Machine panel.");
        } else {
            self.resource_role(ui);
        }
        ui.horizontal(|ui| {
            let status = if self.resources.busy {
                "Working…"
            } else if self.resources.dirty {
                "Unsaved changes"
            } else if self.resources.ready {
                "Saved locally"
            } else {
                "Library unavailable"
            };
            ui.label(status).on_hover_text(&self.resources.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let machines = self.resources.machines_view;
                let tool = self
                    .resources
                    .draft
                    .library
                    .tools
                    .iter()
                    .find(|t| t.id == self.resources.tool);
                let has_preset = tool.is_some_and(|t| {
                    t.cutting_presets
                        .iter()
                        .any(|p| p.id == self.resources.preset)
                        || t.knife_cutting_presets
                            .iter()
                            .any(|p| p.id == self.resources.preset)
                });
                let fits = tool.is_some_and(|t| {
                    matches!(
                        (&t.geometry, self.resources.role),
                        (LibraryGeometry::Endmill(_), Role::Endmill)
                            | (LibraryGeometry::Vbit(_), Role::Vbit)
                            | (LibraryGeometry::DragKnife(_), Role::Knife)
                    )
                });
                let selected = if machines {
                    self.resources
                        .draft
                        .machines
                        .iter()
                        .any(|m| m.id == self.resources.machine)
                } else {
                    fits
                };
                let title = if machines {
                    "Use machine"
                } else if has_preset {
                    "Use tool & profile"
                } else {
                    "Use tool"
                };
                let r = ui.add_enabled(
                    self.library_can_use() && selected,
                    egui::Button::new(RichText::new(title).color(Color32::WHITE))
                        .fill(Color32::from_rgb(18, 133, 144))
                        .min_size(egui::vec2(140., 34.)),
                );
                observe_control(title, r.rect);
                if machines {
                    observe_control("Apply reviewed machine", r.rect);
                }
                let r = r.on_disabled_hover_text(if self.resources.dirty {
                    "Save changes before using this item."
                } else if !selected {
                    "Select an item that fits the target assignment."
                } else {
                    "Open a job and finish the current action first."
                });
                if r.clicked() {
                    let catalog = self.resources.base.as_ref().unwrap().snapshot.clone();
                    if machines {
                        if let Some(profile) = catalog
                            .machines
                            .iter()
                            .find(|m| m.id == self.resources.machine)
                            .cloned()
                        {
                            self.resource_command(R::Machine { profile }, ctx);
                        }
                    } else {
                        let operation =
                            self.document.as_ref().unwrap().job.operations[0].id.clone();
                        let role = self.resources.role;
                        let command = if has_preset {
                            R::ApplyToolProfile {
                                catalog,
                                tool: self.resources.tool.clone(),
                                preset: self.resources.preset.clone(),
                                operation,
                                role,
                            }
                        } else {
                            R::SelectLibraryTool {
                                catalog,
                                tool: self.resources.tool.clone(),
                                operation,
                                role,
                            }
                        };
                        self.resource_command(command, ctx);
                    }
                }
                if action(
                    ui,
                    "Save changes",
                    "Save library",
                    self.resources.ready && !self.resources.busy && self.resources.dirty,
                )
                .clicked()
                {
                    self.request_resources(ResourceIntent::Save, ctx);
                }
            });
        });
    }

    fn library_new_tool(&mut self, label: &str, role: Role) {
        let id = unique(
            "tool",
            self.resources
                .draft
                .library
                .tools
                .iter()
                .map(|t| t.id.clone()),
        );
        let geometry = if role == Role::Knife {
            LibraryGeometry::DragKnife(cam_core::project::DragKnifeSpec {
                blade_offset_mm: 0.,
                max_cut_depth_mm: 0.,
            })
        } else if role == Role::Vbit {
            LibraryGeometry::Vbit(cam_core::model::VBitSpec {
                included_angle_deg: 0.,
                tip_diameter_mm: 0.,
                max_cutting_diameter_mm: 0.,
                cutting_height_mm: 0.,
            })
        } else {
            LibraryGeometry::Endmill(cam_core::model::EndmillSpec {
                diameter_mm: 0.,
                cutting_length_mm: 0.,
                plunge_capable: false,
            })
        };
        self.resources.draft.library.tools.push(LibraryTool {
            spindle_direction: (role != Role::Knife)
                .then_some(cam_core::project::SpindleDirection::Clockwise),
            id: id.clone(),
            name: label.into(),
            geometry,
            ramp_capable: None,
            plunge_capable: None,
            cutting_presets: vec![],
            knife_cutting_presets: vec![],
        });
        self.resources.tool = id;
        self.resources.preset.clear();
        self.resources.search[0].clear();
        self.resources.dirty = true;
    }

    fn library_capture_tool(&mut self) {
        let Some(job) = self.document.as_ref().map(|d| &d.job) else {
            return;
        };
        let Some(source) = crate::authoring::tool(job, self.resources.role == Role::Vbit) else {
            return;
        };
        let id = unique(
            "tool",
            self.resources
                .draft
                .library
                .tools
                .iter()
                .map(|t| t.id.clone()),
        );
        match crate::resources::capture_tool(source, id.clone(), source.name.clone()) {
            Ok(tool) => {
                self.resources.draft.library.tools.push(tool);
                self.resources.tool = id;
                self.resources.preset.clear();
                self.resources.dirty = true;
                self.resources.search[0].clear();
            }
            Err(e) => {
                self.resources.status = e.clone();
                self.resources.error = Some(e);
            }
        }
    }

    fn library_capture_machine(&mut self) {
        let Some(job) = self.document.as_ref().map(|d| &d.job) else {
            return;
        };
        match cam_core::project::v5::machine::resolve_sequence_profile(
            job,
            &cam_core::project::v5::ReadinessScope::AllEnabled,
        ) {
            Ok(mut m) => {
                m.id = unique(
                    "machine",
                    self.resources.draft.machines.iter().map(|m| m.id.clone()),
                );
                self.resources.machine = m.id.clone();
                self.resources.draft.machines.push(m);
                self.resources.dirty = true;
                self.resources.search[1].clear();
            }
            Err(e) => {
                self.resources.status = e.to_string();
                self.resources.error = Some(e.to_string());
            }
        }
    }
}
