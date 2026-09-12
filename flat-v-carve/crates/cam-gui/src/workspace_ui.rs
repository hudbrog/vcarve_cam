use super::*;

impl App {
    pub(super) fn theme(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = Color32::from_rgb(237, 242, 246);
        style.visuals.window_fill = Color32::from_rgb(247, 249, 251);
        style.visuals.extreme_bg_color = Color32::WHITE;
        style.visuals.override_text_color = Some(Color32::from_rgb(34, 49, 63));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(11.));
        style.visuals.selection.bg_fill = Color32::from_rgb(183, 230, 233);
        style.visuals.selection.stroke = egui::Stroke::new(1., Color32::from_rgb(13, 66, 73));
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(205, 218, 227);
        style.visuals.slider_trailing_fill = true;
        style.visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1., Color32::from_rgb(191, 204, 214));
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.corner_radius = egui::CornerRadius::same(3);
        }
        style.spacing.item_spacing = egui::vec2(8., 8.);
        style.spacing.button_padding = egui::vec2(10., 6.);
        style.spacing.interact_size.y = 28.;
        ctx.set_style(style);
    }
    pub(super) fn navigate(&mut self, index: usize) {
        if self.inspector_tab != index {
            self.inspector_tab = index;
            self.search.clear();
            self.edit_group = None;
        }
    }
    pub(super) fn commands(&mut self, ctx: &egui::Context) {
        let idle = self.active.is_none() && self.io.is_none();
        egui::TopBottomPanel::top("gui2-header")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(44, 55, 65))
                    .inner_margin(12.),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("2.5D CAM")
                            .size(22.)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(
                            self.document
                                .as_ref()
                                .map(|d| d.job.name.as_str())
                                .unwrap_or("New job"),
                        )
                        .color(Color32::WHITE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let prefix = matches!(
                            self.plan_scope(),
                            Some(crate::session::GenerateScope::ThroughOperation { .. })
                        );
                        let export = button(
                            ui,
                            if prefix {
                                "Export prefix…"
                            } else {
                                "Export…"
                            },
                            idle && self.current() && self.view.export_ready(),
                        );
                        observe_control("Prepare checked output", export.rect);
                        observe_control("Export…", export.rect);
                        if export.clicked() {
                            self.submit(
                                Command::Prepare {
                                    job: self.document.as_ref().unwrap().job.to_json().unwrap(),
                                    handle: self.plan.as_ref().unwrap().0.clone(),
                                },
                                ctx,
                            );
                        }
                        let ready = !self.operation_ramp_draft
                            && self
                                .document
                                .as_ref()
                                .is_some_and(|d| !d.pending() && !d.job.operations.is_empty());
                        let generate = ui.add_enabled(
                            idle && ready,
                            egui::Button::new("Generate").fill(Color32::from_rgb(49, 190, 195)),
                        );
                        observe_control("Generate", generate.rect);
                        if generate.clicked() {
                            self.generate(crate::session::GenerateScope::AllEnabled, ctx);
                        }
                        for (label, simulate) in [("Simulate", true), ("Prepare", false)] {
                            let fill = if self.simulate == simulate {
                                Color32::from_rgb(255, 166, 76)
                            } else {
                                Color32::from_rgb(226, 232, 237)
                            };
                            let response = ui.add(egui::Button::new(label).fill(fill));
                            observe_control(label, response.rect);
                            if response.clicked() {
                                self.simulate = simulate;
                                if !simulate && !self.current() {
                                    self.preview_dirty = true;
                                }
                                if simulate {
                                    self.navigate(6);
                                }
                            }
                        }
                    });
                });
            });
        egui::TopBottomPanel::top("gui2-files").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let menu = ui.menu_button("File", |ui| {
                    if button(ui, "New drag knife from SVG", idle).clicked() {
                        self.open(IoKind::KnifeSvg, ctx);
                        ui.close();
                    }
                    if button(ui, "New face job", idle).clicked() {
                        // Source-free facing: stock, tool and cutting values
                        // only. Nothing is invented; Setup and the Face panel
                        // report every value that is still missing.
                        self.operation_command(
                            crate::operation_authoring::add(
                                crate::operation_authoring::Kind::Face,
                                &crate::operation_authoring::empty_job(),
                            ),
                            ctx,
                        );
                        ui.close();
                    }
                    if button(ui, "New from SVG", idle).clicked() {
                        self.open(IoKind::Svg, ctx);
                        ui.close();
                    }
                    if button(ui, "Open job", idle).clicked() {
                        self.open(IoKind::Open, ctx);
                        ui.close();
                    }
                    if button(ui, "Import older job", idle).clicked() {
                        self.open(IoKind::Migrate, ctx);
                        ui.close();
                    }
                    ui.separator();
                    if button(ui, "Flower fixture", idle).clicked() {
                        self.submit(
                            Command::Open {
                                json: engine::FLOWER.into(),
                            },
                            ctx,
                        );
                        ui.close();
                    }
                });
                observe_control("File", menu.response.rect);
                if button(ui, "Save job", self.document.is_some() && self.io.is_none()).clicked() {
                    self.save_job(ctx);
                }
                if button(ui, "Undo", !self.undo.is_empty()).clicked() {
                    self.undo(ctx);
                }
                if button(ui, "Redo", !self.redo.is_empty()).clicked() {
                    self.redo(ctx);
                }
                ui.separator();
                if self.document.is_some() {
                    ui.small(if self.saved_revision == Some(self.revision) {
                        "Job saved"
                    } else {
                        "Unsaved changes"
                    });
                }
                if self.active.is_some() {
                    ui.spinner();
                    if button(ui, "Cancel", true).clicked() {
                        self.cancelled_id = self.active.map(|a| a.0);
                        self.port.cancel();
                        self.active = None;
                        self.plan = None;
                        self.plan_scope = None;
                        self.prepared = None;
                        self.status = "Compute cancelled. Draft retained.".into();
                    }
                }
                if self.view.motion_count() > 0 && !self.current() {
                    ui.colored_label(
                        Color32::from_rgb(164, 83, 12),
                        "Simulation is stale · Generate to update",
                    );
                }
            });
        });
        egui::TopBottomPanel::bottom("gui2-status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.small(&self.status);
                ui.separator();
                ui.small("mm");
                if let Some(doc) = &self.document {
                    ui.small(format!(
                        "Stock bottom Z {}",
                        doc.job
                            .setup
                            .stock
                            .thickness_mm
                            .map(|n| format!("{:.2}", -n))
                            .unwrap_or("unset".into())
                    ));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.small(&self.recovery.status);
                if self.recovery.offered.is_some() {
                    if button(ui, "Restore draft", true).clicked() {
                        let snapshot = self.recovery.offered.take().unwrap().snapshot;
                        self.restore(snapshot, ctx);
                    }
                    if button(ui, "Keep current", true).clicked() {
                        self.recovery.offered = None;
                        self.recovery.changed(ctx.input(|i| i.time));
                    }
                }
                if self.recovery.failed && button(ui, "Reload recovery", true).clicked() {
                    self.recovery.failed = false;
                    self.port.load_recovery(ctx.clone());
                }
                if self.retry
                    && self.io.is_none()
                    && button(
                        ui,
                        "Retry previous save",
                        self.retained_save_revision == self.revision
                            || self
                                .retained_save
                                .as_ref()
                                .is_some_and(|(_, _, r)| r.is_some()),
                    )
                    .clicked()
                    && let Some((name, bytes, revision)) = self.retained_save.clone()
                {
                    self.save(name, bytes, revision, ctx);
                }
            });
        });
    }
    fn nav_item(&mut self, ui: &mut egui::Ui, label: &str, probe: &str, index: usize) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.), egui::Sense::click());
        if self.inspector_tab == index || response.hovered() {
            ui.painter().rect_filled(
                rect,
                3.,
                if self.inspector_tab == index {
                    Color32::from_rgb(183, 230, 233)
                } else {
                    Color32::from_rgb(221, 232, 238)
                },
            );
        }
        if self.inspector_tab == index {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(3., rect.height())),
                0.,
                Color32::from_rgb(26, 169, 177),
            );
        }
        let icon = rect.left_center() + egui::vec2(18., 0.);
        let stroke = egui::Stroke::new(1.5, Color32::from_rgb(54, 74, 88));
        match index {
            0 | 1 | 3 => {
                ui.painter().rect_stroke(
                    egui::Rect::from_center_size(icon, egui::vec2(12., 14.)),
                    1.,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            2 => {
                ui.painter().line_segment(
                    [icon + egui::vec2(-7., -5.), icon + egui::vec2(0., 6.)],
                    stroke,
                );
                ui.painter().line_segment(
                    [icon + egui::vec2(0., 6.), icon + egui::vec2(7., -5.)],
                    stroke,
                );
            }
            _ => {
                ui.painter().line_segment(
                    [icon + egui::vec2(0., -7.), icon + egui::vec2(0., 7.)],
                    egui::Stroke::new(4., stroke.color),
                );
            }
        }
        ui.painter().with_clip_rect(rect).text(
            rect.left_center() + egui::vec2(36., 0.),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.),
            Color32::from_rgb(34, 49, 63),
        );
        observe_control(probe, response.rect);
        if response.clicked() {
            self.navigate(index);
        }
    }
    pub(super) fn navigator(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("gui2-navigator")
            .exact_width(205.)
            .resizable(false)
            .show(ctx, |ui| {
                let area = egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.);
                    ui.strong("SETUP");
                    self.nav_item(ui, "Stock & work zero", "Setup", 1);
                    self.nav_item(ui, "Machine", "Machine", 3);
                    self.nav_item(ui, "Job settings", "Job settings", 7);
                    if button(ui, "Tool library", true).clicked() {
                        self.resources.open = true;
                        if !self.resources.ready {
                            self.request_resources(ResourceIntent::Load, ctx);
                        }
                    }
                    ui.add_space(8.);
                    ui.separator();
                    ui.strong("ARTWORK");
                    let items: Vec<_> = self
                        .document
                        .as_ref()
                        .map(|d| {
                            d.job
                                .artwork
                                .iter()
                                .map(|i| (i.id.0.clone(), i.name.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if items.is_empty() {
                        self.nav_item(ui, "Artwork collection", "Artwork", 0);
                    }
                    for (id, name) in items {
                        let selected = self
                            .document
                            .as_ref()
                            .is_some_and(|d| d.raw.artwork_item == id);
                        let r = ui.add_sized(
                            [ui.available_width(), 34.],
                            egui::Button::selectable(selected && self.inspector_tab == 0, name)
                                .truncate(),
                        );
                        observe_control(&format!("Artwork {id}"), r.rect);
                        if selected {
                            observe_control("Artwork", r.rect);
                        }
                        if r.clicked() {
                            self.select_artwork(&id, ctx);
                        }
                    }
                    if button(
                        ui,
                        "+ Import artwork",
                        self.active.is_none() && self.io.is_none(),
                    )
                    .clicked()
                    {
                        self.open(
                            if self.document.is_some() {
                                IoKind::AddSvg
                            } else {
                                IoKind::Svg
                            },
                            ctx,
                        );
                    }
                    ui.add_space(8.);
                    ui.separator();
                    ui.strong("OPERATIONS");
                    let selected = self
                        .document
                        .as_ref()
                        .and_then(|d| d.active_operation())
                        .map(|op| (op.id.clone(), op.name.clone()));
                    let has_operation = selected.is_some();
                    let kind = self.operation_kind();
                    let knife = kind == Some(crate::session::OperationKind::DragKnife);
                    if has_operation {
                        let index =
                            self.document
                                .as_ref()
                                .and_then(|d| {
                                    d.job.operations.iter().position(|op| {
                                        Some(&op.id) == selected.as_ref().map(|s| &s.0)
                                    })
                                })
                                .unwrap_or(0);
                        let label = format!(
                            "{:02}  {}",
                            index + 1,
                            selected.as_ref().map(|s| s.1.clone()).unwrap_or_default()
                        );
                        self.nav_item(ui, &label, "Cutting", 2);
                        self.nav_item(ui, "Inspect result", "Inspect result", 6);
                    } else {
                        ui.label("No operations");
                    }
                    self.operation_actions(ui, ctx);
                    if let Some(doc) = &self.document {
                        ui.small(format!("{} ordered operation(s)", doc.job.operations.len()));
                        let id = doc.raw.operation.clone();
                        if let Some(s) = crate::knife::settings_in(&doc.job, &id) {
                            ui.small(format!("{} knife chains", s.chains.len()));
                        } else if let Some(s) = engine::settings_in(&doc.job, &id) {
                            ui.small(format!("{} filled components", s.components.len()));
                        }
                    }
                    ui.add_space(16.);
                    ui.separator();
                    ui.strong("ASSIGNED TOOLS");
                    if button(ui, "Job tools", has_operation).clicked() {
                        self.resources.jobs_open = true;
                    }
                    if knife {
                        self.nav_item(ui, "Drag knife", "Knife tool", 4);
                    } else if has_operation {
                        self.nav_item(ui, "Endmill", "Endmill tool", 4);
                        if kind == Some(crate::session::OperationKind::FlatVcarve) {
                            self.nav_item(ui, "V-bit", "V-bit tool", 5);
                        }
                    }
                    ui.small("Geometry belongs to this job.");
                    if ui.link("Controller mapping in Machine").clicked() {
                        self.navigate(3);
                    }
                });
                observe_control("Navigator viewport", area.inner_rect);
            });
    }
}
