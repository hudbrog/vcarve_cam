use super::*;
use crate::{
    ui_icons::{self, Icon},
    ui_theme as theme, ui_widgets,
};

impl App {
    pub(super) fn theme(ctx: &egui::Context) {
        theme::apply(ctx);
    }
    pub(super) fn navigate(&mut self, index: usize) {
        self.inspector_collapsed = false;
        if self.inspector_tab != index {
            self.inspector_tab = index;
            self.search.clear();
            self.edit_group = None;
        }
    }
    fn document_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, icons: bool) {
        for (label, icon, enabled) in [
            (
                "Save job",
                Icon::Save,
                self.document.is_some() && self.io.is_none(),
            ),
            ("Undo", Icon::Undo, !self.undo.is_empty()),
            ("Redo", Icon::Redo, !self.redo.is_empty()),
        ] {
            let response = ui
                .add_enabled_ui(enabled, |ui| {
                    if icons {
                        ui_icons::button(ui, icon, label, false)
                    } else {
                        ui.button(label)
                    }
                })
                .inner;
            observe_control(label, response.rect);
            if response.clicked() {
                match label {
                    "Save job" => self.save_job(ctx),
                    "Undo" => self.undo(ctx),
                    _ => self.redo(ctx),
                }
                if !icons {
                    ui.close();
                }
            }
        }
    }
    fn file_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, compact: bool) {
        let idle = self.active.is_none() && self.io.is_none();
        let menu = ui.menu_button("File", |ui| {
            if button(ui, "New job", idle).clicked() {
                self.submit(
                    Command::Open {
                        json: crate::operation_authoring::empty_job().to_json().unwrap(),
                    },
                    ctx,
                );
                ui.close();
            }
            if button(ui, "Open job", idle).clicked() {
                self.open(IoKind::Open, ctx);
                ui.close();
            }
            if compact {
                ui.separator();
                self.document_actions(ui, ctx, false);
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
    }
    fn generation_ready(&self) -> bool {
        self.active.is_none()
            && self.io.is_none()
            && !self.operation_ramp_draft
            && self
                .document
                .as_ref()
                .is_some_and(|d| !d.pending() && !d.job.operations.is_empty())
    }
    fn generation_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ready = self.generation_ready();
        let scope = ui
            .add_enabled_ui(ready, |ui| {
                ui_icons::button(ui, Icon::More, "Generation scope", false)
            })
            .inner;
        observe_control("Generation scope", scope.rect);
        egui::Popup::menu(&scope).show(|ui| {
            ui.strong("Generate cutting paths");
            if button(ui, "All enabled operations", ready).clicked() {
                self.generate(GenerateScope::AllEnabled, ctx);
                ui.close();
            }
            let id = self.operation_id();
            if button(ui, "Through selected operation", ready && !id.is_empty()).clicked() {
                self.generate(GenerateScope::ThroughOperation { operation_id: id }, ctx);
                ui.close();
            }
            ui.small("Export uses the exact scope of the generated result.");
        });
        let generate = ui.add_enabled(
            ready,
            egui::Button::new("Generate all").fill(theme::PRIMARY),
        );
        observe_control("Generate", generate.rect);
        if generate.clicked() {
            self.generate(GenerateScope::AllEnabled, ctx);
        }
    }
    pub(super) fn commands(&mut self, ctx: &egui::Context) {
        let narrow = ctx.content_rect().width() < 960.;
        if narrow && !self.navigator_collapsed && !self.inspector_collapsed {
            self.navigator_collapsed = true;
        }
        let compact = ctx.content_rect().width() < 1000.;
        let idle = self.active.is_none() && self.io.is_none();
        egui::TopBottomPanel::top("gui2-header")
            .frame(
                egui::Frame::new()
                    .fill(theme::HEADER)
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(32.);
                    ui.label(
                        RichText::new(if compact { "CAM" } else { "2.5D CAM" })
                            .size(20.)
                            .color(Color32::WHITE),
                    );
                    self.file_menu(ui, ctx, compact);
                    if !compact {
                        self.document_actions(ui, ctx, true);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let prefix = matches!(
                            self.plan_scope(),
                            Some(GenerateScope::ThroughOperation { .. })
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
                        self.generation_controls(ui, ctx);
                        ui.add_space(8.);
                        for (label, simulate) in [("Simulate", true), ("Prepare", false)] {
                            let response = ui.add(egui::Button::new(label).fill(
                                if self.simulate == simulate {
                                    theme::MODE
                                } else {
                                    theme::SURFACE
                                },
                            ));
                            observe_control(label, response.rect);
                            if simulate {
                                observe_control("Inspect result", response.rect);
                            }
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
                        let name = self
                            .document
                            .as_ref()
                            .map(|d| d.job.name.as_str())
                            .unwrap_or("New job");
                        let state = if self.document.as_ref().is_some_and(Document::pending) {
                            "Partial input"
                        } else if self.saved_revision == Some(self.revision) {
                            "Saved"
                        } else {
                            "Unsaved"
                        };
                        let status = ui.label(RichText::new(state).size(12.).color(Color32::WHITE));
                        observe_control("Document state", status.rect);
                        let title = ui
                            .add_sized(
                                [ui.available_width().max(1.), 28.],
                                egui::Label::new(RichText::new(name).color(Color32::WHITE))
                                    .truncate(),
                            )
                            .on_hover_text(name);
                        observe_control("Document title", title.rect);
                    });
                });
            });
        egui::TopBottomPanel::top("workspace-plan")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let nav = ui.selectable_label(!self.navigator_collapsed, "Navigator");
                    observe_control("Toggle navigator", nav.rect);
                    if nav.clicked() {
                        self.navigator_collapsed = !self.navigator_collapsed;
                        if narrow && !self.navigator_collapsed {
                            self.inspector_collapsed = true;
                        }
                    }
                    let inspector = ui.selectable_label(!self.inspector_collapsed, "Inspector");
                    observe_control("Toggle inspector", inspector.rect);
                    if inspector.clicked() {
                        self.inspector_collapsed = !self.inspector_collapsed;
                        if narrow && !self.inspector_collapsed {
                            self.navigator_collapsed = true;
                        }
                    }
                    ui.separator();
                    if self.active.is_some() {
                        ui.spinner();
                        if button(ui, "Cancel", true).clicked() {
                            self.cancel_compute();
                        }
                    }
                    let state = if self.active.is_some() {
                        "Working"
                    } else if self.current() {
                        "Current"
                    } else if self.view.motion_count() > 0 {
                        "Out of date"
                    } else {
                        "Not generated"
                    };
                    let shown_scope = self
                        .active
                        .as_ref()
                        .and(self.active_scope.as_ref())
                        .or(self.plan_scope());
                    let scope = match shown_scope {
                        Some(GenerateScope::ThroughOperation { operation_id }) => {
                            self.scope_name(operation_id)
                        }
                        _ => "All enabled operations".into(),
                    };
                    let text = format!("Plan: {state} · {scope}");
                    let label = ui
                        .add(
                            egui::Label::new(RichText::new(&text).color(
                                if state == "Out of date" {
                                    theme::WARNING
                                } else {
                                    theme::MUTED
                                },
                            ))
                            .truncate(),
                        )
                        .on_hover_text(&text);
                    observe_control("Plan scope", label.rect);
                });
            });
        self.workspace_status(ctx);
    }
    pub(super) fn scope_name(&self, id: &str) -> String {
        self.document
            .as_ref()
            .and_then(|d| {
                d.job
                    .operations
                    .iter()
                    .enumerate()
                    .find(|(_, o)| o.id == id)
            })
            .map(|(i, o)| format!("Through {:02} · {}", i + 1, o.name))
            .unwrap_or_else(|| format!("Through {id}"))
    }
    fn workspace_status(&mut self, ctx: &egui::Context) {
        // Exceptional recovery/save actions expand independently of normal status.
        egui::TopBottomPanel::bottom("gui2-status")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ctx, |ui| {
                let row_height = ui.spacing().interact_size.y;
                ui.spacing_mut().interact_size.y = 18.;
                ui.horizontal(|ui| {
                    ui.set_min_height(18.);
                    ui.small("mm");
                    ui.separator();
                    let status = ui
                        .add(egui::Label::new(RichText::new(&self.status).size(12.)).truncate())
                        .on_hover_text(format!("{}\n{}", self.status, self.recovery.status));
                    observe_control("Workspace status", status.rect);
                });
                ui.spacing_mut().interact_size.y = row_height;
                if self.recovery.offered.is_some() || self.recovery.failed || self.retry {
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
                        if self.recovery.failed {
                            if button(ui, "Reload recovery", true).clicked() {
                                self.recovery.failed = false;
                                self.port.load_recovery(ctx.clone());
                            }
                            if button(ui, "Clear stored recovery", true).clicked() {
                                self.port.clear_recovery(ctx.clone());
                            }
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
                }
            });
    }
    fn nav_item(&mut self, ui: &mut egui::Ui, label: &str, probe: &str, index: usize) {
        let icon = match index {
            0 => Icon::Artwork,
            1 => Icon::Stock,
            3 => Icon::Machine,
            4 => Icon::Endmill,
            5 => Icon::Vbit,
            _ => Icon::Settings,
        };
        let response = ui_widgets::navigation_row(
            ui,
            label,
            None,
            icon,
            self.inspector_tab == index && !self.resources.open && !self.resources.jobs_open,
        );
        observe_control(probe, response.rect);
        if response.clicked() {
            self.navigate(index);
        }
    }
    fn navigator_artwork(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
            ui.push_id(&id, |ui| {
                ui.horizontal(|ui| {
                    let row = ui
                        .allocate_ui_with_layout(
                            egui::vec2((ui.available_width() - 72.).max(20.), 32.),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui_widgets::navigation_row(
                                    ui,
                                    &name,
                                    None,
                                    Icon::Artwork,
                                    selected && self.inspector_tab == 0,
                                )
                            },
                        )
                        .inner;
                    observe_control(&format!("Artwork {id}"), row.rect);
                    if selected {
                        observe_control("Artwork", row.rect);
                    }
                    if row.clicked() {
                        self.select_artwork(&id, ctx);
                    }
                    let hidden = self.view.artwork.hidden.contains(&id);
                    let eye = ui_icons::button(
                        ui,
                        if hidden { Icon::Hidden } else { Icon::Eye },
                        "Hide artwork — display only; assigned geometry still cuts",
                        hidden,
                    );
                    observe_control(&format!("Artwork eye {id}"), eye.rect);
                    if eye.clicked() {
                        if hidden {
                            self.view.artwork.hidden.remove(&id);
                        } else {
                            self.view.artwork.hidden.insert(id.clone());
                        }
                    }
                    let locked = self.view.artwork.locked.contains(&id);
                    let lock = ui_icons::button(
                        ui,
                        if locked { Icon::Lock } else { Icon::Unlock },
                        "Lock viewport picking and gestures; numeric edits remain available",
                        locked,
                    );
                    observe_control(&format!("Artwork lock {id}"), lock.rect);
                    if lock.clicked() {
                        if locked {
                            self.view.artwork.locked.remove(&id);
                        } else {
                            self.view.artwork.locked.insert(id.clone());
                        }
                    }
                })
            });
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
    }
    fn navigator_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, compact: bool) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("JOB TOOLS");
            let manage = button(ui, "Manage", self.document.is_some());
            observe_control("Job tools", manage.rect);
            if manage.clicked() {
                self.resources.jobs_open = true;
            }
        });
        let tools: Vec<_> = self
            .document
            .as_ref()
            .map(|d| {
                d.job
                    .tools
                    .iter()
                    .map(|t| {
                        let icon = match t.geometry {
                            Some(cam_core::project::ToolGeometry::Vbit(_)) => Icon::Vbit,
                            Some(cam_core::project::ToolGeometry::DragKnife(_)) => Icon::Knife,
                            _ => Icon::Endmill,
                        };
                        (t.id.clone(), t.name.clone(), t.geometry.is_some(), icon)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if tools.is_empty() {
            ui.small("No cutters in this job");
        }
        for (id, name, chosen, icon) in tools.iter().take(if compact { 0 } else { 2 }) {
            let label = if *chosen {
                name.clone()
            } else {
                format!("{name} · unset")
            };
            let row = ui_widgets::navigation_row(ui, &label, None, *icon, false);
            observe_control(&format!("Navigator job tool {id}"), row.rect);
            if row.clicked() {
                self.resources.job_tool = id.clone();
                self.resources.jobs_open = true;
            }
        }
        if compact && !tools.is_empty() {
            ui.small(format!("{} cutters in this job", tools.len()));
        } else if tools.len() > 2 {
            ui.small(format!("+{} more in Manage", tools.len() - 2));
        }
        let menu = ui.menu_button("Selected operation tools", |ui| {
            let kind = self.operation_kind();
            for finish in [false, true] {
                if let Some(tab) = crate::resources::tool_tab(kind, finish) {
                    let probe = if finish {
                        "V-bit tool"
                    } else if kind == Some(crate::session::OperationKind::DragKnife) {
                        "Knife tool"
                    } else {
                        "Endmill tool"
                    };
                    let r = ui.button(tab.noun);
                    observe_control(probe, r.rect);
                    if r.clicked() {
                        self.navigate(if finish { 5 } else { 4 });
                        ui.close();
                    }
                }
            }
        });
        observe_control("Tool geometry", menu.response.rect);
        ui.horizontal(|ui| {
            if button(ui, "Tool library", true).clicked() {
                self.resources.open = true;
                if !self.resources.ready {
                    self.request_resources(ResourceIntent::Load, ctx);
                }
            }
            ui.small("Global");
        });
    }
    pub(super) fn navigator(&mut self, ctx: &egui::Context) {
        if self.navigator_collapsed {
            return;
        }
        let panel = egui::SidePanel::left("gui2-navigator")
            .default_width(self.navigator_width)
            .width_range(224.0..=300.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.add_space(4.);
                let compact = ctx.content_rect().height() < 650.;
                if ctx.content_rect().height() < 500. {
                    let setup = ui.menu_button("Setup pages", |ui| {
                        for (label, probe, index) in [
                            ("Stock & work zero", "Setup", 1),
                            ("Machine", "Machine", 3),
                            ("Job settings", "Job settings", 7),
                        ] {
                            let r = ui.button(label);
                            observe_control(probe, r.rect);
                            if r.clicked() {
                                self.navigate(index);
                                ui.close();
                            }
                        }
                    });
                    observe_control("Setup pages", setup.response.rect);
                } else {
                    ui.strong("SETUP");
                    self.nav_item(ui, "Stock & work zero", "Setup", 1);
                    self.nav_item(ui, "Machine", "Machine", 3);
                    let machine = self
                        .document
                        .as_ref()
                        .and_then(|d| d.job.machine_configuration.as_ref())
                        .map(|m| m.origin.name.as_str())
                        .unwrap_or("No machine applied");
                    ui.add(
                        egui::Label::new(RichText::new(machine).size(12.).color(theme::MUTED))
                            .truncate(),
                    )
                    .on_hover_text(machine);
                    self.nav_item(ui, "Job settings", "Job settings", 7);
                }
                ui.separator();
                let tools_height = if compact {
                    142.
                } else if self
                    .document
                    .as_ref()
                    .is_some_and(|d| d.job.tools.len() > 2)
                {
                    202.
                } else {
                    182.
                };
                let area = egui::ScrollArea::vertical()
                    .id_salt("navigator-collections")
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - tools_height - 8.).max(60.))
                    .show(ui, |ui| {
                        self.navigator_artwork(ui, ctx);
                        ui.add_space(8.);
                        ui.separator();
                        ui.strong("OPERATIONS");
                        self.operation_actions(ui, ctx);
                    });
                observe_control("Navigator viewport", area.inner_rect);
                self.navigator_tools(ui, ctx, compact);
            });
        self.navigator_width = panel.response.rect.width();
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    fn review_app() -> App {
        let mut job =
            engine::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
        job.name =
            "A very long document name with repeated words for checking the compact header.svg"
                .repeat(2);
        let first = job.operations[0].clone();
        job.operations = (1..=12)
            .map(|i| {
                let mut op = first.clone();
                op.id = format!("row-{i}");
                op.name = "Duplicate operation name".into();
                op
            })
            .collect();
        App {
            document: Some(Document::new(job)),
            ..Default::default()
        }
    }

    #[test]
    fn shell_controls_fit_long_names_and_short_windows() {
        for (width, height) in [
            (640., 400.),
            (640., 480.),
            (853., 533.),
            (1280., 800.),
            (1440., 900.),
            (1920., 1080.),
        ] {
            let mut app = review_app();
            app.inspector_collapsed = width < 960.;
            let ctx = egui::Context::default();
            App::theme(&ctx);
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        app.commands(ctx);
                        app.navigator(ctx);
                    },
                );
            }
            CONTROLS.with(|c| {
                let c = c.borrow();
                let header: Vec<_> = [
                    "File",
                    "Document title",
                    "Document state",
                    "Prepare",
                    "Simulate",
                    "Generate",
                    "Generation scope",
                    "Export…",
                ]
                .into_iter()
                .map(|label| (label, c[label]))
                .collect();
                for (label, rect) in &header {
                    assert!(
                        rect[0] >= 0. && rect[2] <= width && rect[3] <= 52.,
                        "{width}×{height} {label}: {rect:?}"
                    );
                }
                for (i, (label, a)) in header.iter().enumerate() {
                    for (other, b) in &header[i + 1..] {
                        assert!(
                            a[2] <= b[0] + 0.5 || b[2] <= a[0] + 0.5,
                            "{width}: {label} overlaps {other}: {a:?} {b:?}"
                        );
                    }
                }
                let clip = c["Navigator viewport"];
                for fixed in [
                    "Setup",
                    "Machine",
                    "Job settings",
                    "Job tools",
                    "Tool geometry",
                    "Tool library",
                ] {
                    let r = c.get(fixed).copied().unwrap_or_else(|| c["Setup pages"]);
                    assert!(
                        r[1] >= 52. && r[3] <= height - 24.,
                        "{width}×{height} anchored {fixed}: {r:?}"
                    );
                }
                assert!(c["Job tools"][1] > clip[3]);
                assert!(clip[3] - clip[1] >= 60.);
                assert_eq!(
                    c.keys().filter(|k| k.starts_with("Operation row ")).count(),
                    12
                );
                for index in 1..=12 {
                    let row = c[&format!("Operation row row-{index}")];
                    let toggle = c[&format!("Operation enabled row-{index}")];
                    let menu = c[&format!("Operation actions row-{index}")];
                    assert!(
                        toggle[2] < row[0] && row[2] < menu[0] && menu[2] <= app.navigator_width
                    );
                }
            });
        }
    }

    #[test]
    fn returning_to_selected_operation_preserves_its_ramp_draft_and_scroll() {
        let mut app = review_app();
        app.operation_ramp_draft = true;
        app.operation_scroll = [12., 42., 91.];
        app.inspector_tab = 1;
        app.inspector_collapsed = true;
        app.select_operation("row-1", &egui::Context::default());
        assert_eq!(app.inspector_tab, 2);
        assert!(!app.inspector_collapsed);
        assert!(app.operation_ramp_draft);
        assert_eq!(app.operation_scroll, [12., 42., 91.]);
    }

    #[test]
    fn operation_footer_stays_below_the_scroll_and_above_status() {
        for (width, height) in [(640., 400.), (1280., 800.), (1440., 900.)] {
            let mut app = review_app();
            let ctx = egui::Context::default();
            App::theme(&ctx);
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        app.commands(ctx);
                        app.navigator(ctx);
                        app.inspector(ctx);
                    },
                );
            }
            CONTROLS.with(|c| {
                let c = c.borrow();
                let scroll = c["Inspector viewport"];
                for label in ["Generate operation", "Generate through here"] {
                    let r = c[label];
                    assert!(
                        r[1] > scroll[3] && r[3] < height - 24.,
                        "{width}×{height} {label}: {r:?}; viewport {scroll:?}"
                    );
                }
            });
        }
    }

    #[test]
    fn resource_windows_open_with_retained_tools_and_no_operations() {
        let mut app = review_app();
        app.document.as_mut().unwrap().job.operations.clear();
        app.document.as_mut().unwrap().raw.operation.clear();
        app.resources.ready = true;
        app.resources.draft =
            crate::resources::Catalog::decode(include_str!("../../../fixtures/gui5/library.json"))
                .unwrap();
        app.resources.job_tool = "endmill".into();
        let ctx = egui::Context::default();
        App::theme(&ctx);
        for (tab, control) in [(0, "Replace SVG"), (3, "Create or choose machine profile")] {
            app.inspector_tab = tab;
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440., 900.),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.inspector(ctx),
                );
            }
            assert!(control_rect(control).is_some(), "empty job lost {control}");
        }
        for library in [false, true] {
            app.resources.open = library;
            app.resources.jobs_open = !library;
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440., 900.),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.resource_windows(ctx),
                );
            }
            assert!(
                control_rect(if library {
                    "Close library"
                } else {
                    "Close job tools"
                })
                .is_some()
            );
            assert!(app.document.as_ref().unwrap().job.operations.is_empty());
        }
    }
}
