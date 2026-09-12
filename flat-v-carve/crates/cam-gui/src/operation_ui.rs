use super::*;

pub(super) fn tab_for_control(label: &str) -> usize {
    if let Some(field) = FIELDS.iter().position(|f| *f == label) {
        return match field {
            0 | 1 | 4 | 47 => 0,
            3 | 5 | 16..=22 | 46 | 52..=60 => 2,
            _ => 1,
        };
    }
    if label.contains("V-bit") || label.starts_with("Finish") {
        2
    } else if label == "Combined" || label == "Endmill only" {
        0
    } else {
        1
    }
}

impl App {
    pub(super) fn operation_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(label) = &self.issue_focus {
            self.operation_tab = tab_for_control(label);
        }
        ui.heading("Flat V-carve");
        ui.horizontal(|ui| {
            for (label, mode) in [
                ("Endmill only", FlatVcarveMode::EndmillOnly),
                ("Combined", FlatVcarveMode::Combined),
            ] {
                let response = ui.selectable_label(
                    engine::settings(&self.document.as_ref().unwrap().job).mode == mode,
                    label,
                );
                observe_control(label, response.rect);
                if response.clicked() {
                    self.edit_job(ctx, &[], |job| {
                        authoring::set_mode(job, mode);
                        Ok(())
                    });
                }
            }
            help::icon(ui, "Carving mode");
        });
        ui.add_space(4.);
        ui.columns(3, |columns| {
            for (index, label) in ["Shape & depth", "Endmill", "V-bit"].iter().enumerate() {
                let selected = self.operation_tab == index;
                let response = columns[index].add_sized(
                    [columns[index].available_width(), 42.],
                    egui::Button::new(*label).selected(selected),
                );
                observe_control(&format!("Operation {label}"), response.rect);
                if selected {
                    columns[index].painter().line_segment(
                        [response.rect.left_bottom(), response.rect.right_bottom()],
                        egui::Stroke::new(2., Color32::from_rgb(26, 169, 177)),
                    );
                }
                if response.clicked() {
                    self.operation_tab = index;
                    self.search.clear();
                    self.edit_group = None;
                }
            }
        });
        ui.separator();
    }

    fn operation_group(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        default_open: bool,
        contents: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let forced = self.issue_focus.is_some().then_some(true);
        egui::Frame::new()
            .fill(Color32::from_rgb(249, 251, 252))
            .stroke(egui::Stroke::new(1., Color32::from_rgb(209, 219, 225)))
            .corner_radius(4)
            .inner_margin(10.)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if !self.search.is_empty() {
                    let title_response = ui.strong(title);
                    observe_control(&format!("Operation {title}"), title_response.rect);
                    contents(self, ui);
                    return;
                }
                let response = egui::CollapsingHeader::new(RichText::new(title).strong().size(15.))
                    .id_salt(("operation-group", self.operation_tab, title))
                    .default_open(default_open)
                    .open(forced)
                    .show(ui, |ui| contents(self, ui));
                observe_control(&format!("Operation {title}"), response.header_response.rect);
            });
        ui.add_space(4.);
    }

    fn operation_numbers(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, fields: &[usize]) {
        if ui.available_width() >= 370. {
            for row in fields.chunks(2) {
                ui.columns(2, |columns| {
                    for (index, field) in row.iter().enumerate() {
                        self.numbers(&mut columns[index], ctx, &[*field]);
                    }
                });
            }
        } else {
            self.numbers(ui, ctx, fields);
        }
    }

    pub(super) fn cutting_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.operation_tab {
            0 => {
                self.operation_group(ui, "Height & depth", true, |app, ui| {
                    help::label(ui, "Operation top");
                    ui.horizontal_wrapped(|ui| {
                        for (label, reference) in [
                            (
                                "Top: stock top",
                                cam_core::project::HeightReference::StockTop,
                            ),
                            (
                                "Top: stock bottom",
                                cam_core::project::HeightReference::StockBottom,
                            ),
                        ] {
                            let response = ui.selectable_label(
                                engine::settings(&app.document.as_ref().unwrap().job)
                                    .top
                                    .reference
                                    == reference,
                                label,
                            );
                            observe_control(label, response.rect);
                            if response.clicked() {
                                app.edit_job(ctx, &[], |job| {
                                    settings_mut(job).top.reference = reference;
                                    Ok(())
                                });
                            }
                        }
                    });
                    app.operation_numbers(ui, ctx, &[47, 0]);
                    ui.small("Depth is measured below the operation top.");
                });
                self.operation_group(ui, "Surface quality", true, |app, ui| {
                    app.operation_numbers(ui, ctx, &[1, 4])
                });
            }
            index => {
                let finish = index == 2;
                let target_only = finish
                    && engine::settings(&self.document.as_ref().unwrap().job).mode
                        == FlatVcarveMode::EndmillOnly;
                if target_only {
                    ui.colored_label(
                        Color32::from_rgb(31, 105, 116),
                        "Defines the target shape; this tool will not cut.",
                    );
                    ui.add_space(6.);
                }
                self.operation_group(
                    ui,
                    if target_only {
                        "Target tool"
                    } else {
                        "Tool & cutting profile"
                    },
                    true,
                    |app, ui| {
                        app.operation_tool(ui, ctx, finish, !target_only);
                    },
                );
                let missing_geometry =
                    authoring::tool(&self.document.as_ref().unwrap().job, finish)
                        .is_none_or(|t| t.geometry.is_none());
                self.operation_group(
                    ui,
                    "Geometry & capabilities",
                    missing_geometry,
                    |app, ui| {
                        app.operation_numbers(
                            ui,
                            ctx,
                            if finish { &[16, 17, 18, 19] } else { &[12, 13] },
                        );
                        if !target_only {
                            app.operation_capabilities(ui, ctx, finish);
                        }
                    },
                );
                if target_only {
                    return;
                }
                self.operation_group(ui, "Feeds & speeds", true, |app, ui| {
                    app.operation_numbers(
                        ui,
                        ctx,
                        if finish { &[3, 21, 22] } else { &[2, 10, 11] },
                    );
                    app.direction(ui, ctx, finish);
                });
                self.operation_group(ui, "Cutting passes", true, |app, ui| {
                    app.operation_numbers(ui, ctx, if finish { &[20, 46, 5] } else { &[8, 9] });
                    if !finish {
                        app.operation_strategy(ui, ctx);
                    }
                });
                if !finish
                    && engine::settings(&self.document.as_ref().unwrap().job)
                        .rough
                        .is_some()
                {
                    self.operation_group(ui, "Entry", true, |app, ui| app.entry_panel(ui, ctx));
                }
                self.operation_group(ui, "Advanced", false, |app, ui| {
                    ui.small(if finish {
                        "Planner limits & inspection sampling"
                    } else {
                        "Planner limits"
                    });
                    app.operation_numbers(
                        ui,
                        ctx,
                        if finish {
                            &[52, 53, 54, 55, 56, 57, 58, 59, 60]
                        } else {
                            &[48, 49, 50]
                        },
                    );
                });
            }
        }
    }

    fn operation_tool(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        finish: bool,
        cutting: bool,
    ) {
        let role = if finish {
            cam_core::project::v5::resources::AssignmentRole::Vbit
        } else {
            cam_core::project::v5::resources::AssignmentRole::Endmill
        };
        if let Some(tool) = authoring::tool(&self.document.as_ref().unwrap().job, finish) {
            ui.strong(&tool.name);
        }
        let index = usize::from(finish);
        let changing = self.operation_picker == Some(index);
        let change = ui.button(if changing {
            "Done choosing tool"
        } else {
            "Change tool…"
        });
        observe_control(
            if finish {
                "Change V-bit tool"
            } else {
                "Change endmill tool"
            },
            change.rect,
        );
        if change.clicked() {
            self.operation_picker = if changing { None } else { Some(index) };
        }
        if self.operation_picker == Some(index) {
            ui.scope(|ui| {
                ui.set_max_width(ui.available_width().min(350.));
                self.assignment_library_picker(ui, ctx, role);
                if button(ui, "Browse library…", self.active.is_none()).clicked() {
                    self.resources.role = role;
                    self.resources.machines_view = false;
                    self.resources.open = true;
                    if !self.resources.ready && !self.resources.busy {
                        self.request_resources(ResourceIntent::Load, ctx);
                    }
                    self.operation_picker = None;
                }
                ui.separator();
                self.assignment_tool(ui, ctx, finish);
            });
        }
        if cutting {
            self.assignment_profiles(ui, ctx, finish);
        }
    }

    fn operation_capabilities(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, finish: bool) {
        for ramp in [false, true] {
            if finish && ramp {
                continue;
            }
            let label = if ramp {
                "Endmill can ramp"
            } else if finish {
                "V-bit can plunge"
            } else {
                "Endmill can plunge"
            };
            let tool = authoring::tool(&self.document.as_ref().unwrap().job, finish);
            let mut value = tool.and_then(|t| {
                if ramp {
                    t.capabilities.ramp_capable
                } else {
                    t.capabilities.plunge_capable
                }
            });
            let before = value;
            help::label(ui, label);
            let prefix = if ramp {
                "Ramp"
            } else if finish {
                "V-bit plunge"
            } else {
                "Plunge"
            };
            ui.horizontal_wrapped(|ui| {
                for (name, v) in [("unset", None), ("yes", Some(true)), ("no", Some(false))] {
                    let response = ui.selectable_value(&mut value, v, name);
                    observe_control(&format!("{prefix} {name}"), response.rect);
                }
            });
            if before != value {
                self.edit_job(ctx, &[], |job| {
                    let tool = authoring::tool_mut(job, finish)?;
                    if ramp {
                        tool.capabilities.ramp_capable = value;
                    } else {
                        tool.capabilities.plunge_capable = value;
                    }
                    Ok(())
                });
            }
        }
    }

    fn operation_strategy(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        help::label(ui, "Clearing strategy");
        let selected = engine::settings(&self.document.as_ref().unwrap().job)
            .rough
            .as_ref()
            .map(|r| r.strategy);
        let response = egui::ComboBox::from_id_salt("operation-strategy")
            .width(ui.available_width())
            .selected_text(match selected {
                Some(cam_core::pocket::ClearingStrategy::DeepestRegion) => {
                    "Deepest-region clearing"
                }
                Some(_) => "Depth-dependent clearing",
                None => "Choose strategy…",
            })
            .show_ui(ui, |ui| {
                for (name, strategy) in [
                    (
                        "Depth-dependent clearing",
                        cam_core::pocket::ClearingStrategy::DepthDependent,
                    ),
                    (
                        "Deepest-region clearing",
                        cam_core::pocket::ClearingStrategy::DeepestRegion,
                    ),
                ] {
                    let response = ui.selectable_label(selected == Some(strategy), name);
                    observe_control(name, response.rect);
                    if response.clicked() {
                        self.edit_job(ctx, &[], |job| {
                            settings_mut(job)
                                .rough
                                .get_or_insert_with(Default::default)
                                .strategy = strategy;
                            Ok(())
                        });
                        ui.close();
                    }
                }
            });
        observe_control("Operation clearing strategy", response.response.rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let job = crate::authoring::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        App {
            document: Some(Document::new(job)),
            inspector_tab: 2,
            operation_tab: 2,
            ..Default::default()
        }
    }

    fn render(app: &mut App, ctx: &egui::Context) -> std::collections::BTreeMap<String, [f32; 4]> {
        for _ in 0..3 {
            CONTROLS.with(|c| c.borrow_mut().clear());
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 900.),
                    )),
                    ..Default::default()
                },
                |ctx| app.inspector(ctx),
            );
        }
        CONTROLS.with(|c| c.borrow().clone())
    }

    #[test]
    fn endmill_only_keeps_vbit_geometry_but_hides_finishing_fields() {
        let mut app = app();
        let ctx = egui::Context::default();
        let controls = render(&mut app, &ctx);
        assert!(controls.contains_key("V-bit angle"));
        assert!(controls.contains_key("Tip diameter"));
        assert!(!controls.contains_key("Finishing feed"));
        assert!(!controls.contains_key("Finish stepdown"));
        authoring::set_mode(
            &mut app.document.as_mut().unwrap().job,
            FlatVcarveMode::Combined,
        );
        let controls = render(&mut app, &ctx);
        assert!(controls.contains_key("Finishing feed"));
        app.operation_tab = 0;
        let controls = render(&mut app, &ctx);
        assert!(controls.contains_key("Maximum depth"));
        assert!(!controls.contains_key("V-bit angle"));
    }

    #[test]
    fn incomplete_ramp_choice_cannot_generate_export_or_save_as_plunge() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.operation_ramp_draft = true;
        let job = app.document.as_ref().unwrap().job.to_json().unwrap();
        app.submit(Command::Generate { job: job.clone() }, &ctx);
        assert!(app.active.is_none());
        app.submit(
            Command::Prepare {
                job,
                handle: "retained".into(),
            },
            &ctx,
        );
        assert!(app.export_dialog.is_none());
        app.save_job(&ctx);
        assert!(app.io.is_none());
        assert!(!app.current());
    }

    #[test]
    fn completing_ramp_fields_commits_one_undoable_entry_change() {
        let mut app = app();
        app.operation_tab = 1;
        app.operation_ramp_draft = true;
        let doc = app.document.as_mut().unwrap();
        settings_mut(&mut doc.job)
            .rough
            .get_or_insert_with(Default::default);
        doc.edit(14, "5".into()).unwrap();
        doc.edit(51, "250".into()).unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx);
        assert!(!app.operation_ramp_draft);
        assert_eq!(app.undo.len(), 1);
        assert!(matches!(
            engine::settings(&app.document.as_ref().unwrap().job)
                .rough
                .as_ref()
                .unwrap()
                .entry,
            cam_core::pocket::EntryStrategy::Ramp {
                max_angle_deg: 5.,
                feed_mm_min: 250.
            }
        ));
        app.undo(&ctx);
        assert!(matches!(
            engine::settings(&app.document.as_ref().unwrap().job)
                .rough
                .as_ref()
                .unwrap()
                .entry,
            cam_core::pocket::EntryStrategy::Plunge
        ));
    }
}
