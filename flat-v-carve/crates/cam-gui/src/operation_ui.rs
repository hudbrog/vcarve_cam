use super::*;

pub(super) fn tab_for_control(label: &str) -> usize {
    if let Some(field) = FIELDS.iter().position(|f| *f == label) {
        return match field {
            0 | 1 | 4 | 47 => 0,
            3 | 5 | 16..=22 | 46 | 52..=60 => 2,
            _ => 1,
        };
    }
    // The operation's own geometry selection lives with the operation shape.
    if label.starts_with("Carving component")
        || label.starts_with("Replace reference")
        || label.starts_with("Remove unresolved reference")
        || matches!(
            label,
            "Select all filled components"
                | "Clear component selection"
                | "Unresolved selections"
                | "Operation Geometry to carve"
        )
    {
        return 0;
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
    /// The selected operation's Flat V-carve settings. Only called after the
    /// panel has established that the operation is a Flat V-carve.
    pub(super) fn flat_vcarve(&self) -> &cam_core::project::v5::FlatVcarveSettingsV5 {
        let doc = self.document.as_ref().expect("document present");
        engine::settings_in(&doc.job, &doc.raw.operation).expect("operation is a Flat V-carve")
    }

    pub(super) fn operation_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.document.as_ref().unwrap().job.operations.is_empty() {
            ui.heading("No operations");
            return;
        }
        match self.operation_kind() {
            Some(crate::session::OperationKind::Face) => return self.face_header(ui, ctx),
            Some(crate::session::OperationKind::Profile) => {
                return self.profile_header(ui, ctx);
            }
            Some(crate::session::OperationKind::DragKnife) => {
                let name = self
                    .document
                    .as_ref()
                    .and_then(|d| d.active_operation())
                    .map(|op| op.name.clone())
                    .unwrap_or_else(|| "Drag knife".into());
                ui.heading(name);
                ui.small("Passive XYZ · spindle and coolant off");
                ui.separator();
                return;
            }
            _ => {}
        }
        if let Some(label) = &self.issue_focus {
            self.operation_tab = tab_for_control(label);
        }
        let name = self
            .document
            .as_ref()
            .and_then(|d| d.active_operation())
            .map(|op| op.name.clone())
            .unwrap_or_else(|| "Flat V-carve".into());
        ui.heading(name);
        ui.horizontal(|ui| {
            for (label, mode) in [
                ("Endmill only", FlatVcarveMode::EndmillOnly),
                ("Combined", FlatVcarveMode::Combined),
            ] {
                let response = ui.selectable_label(self.flat_vcarve().mode == mode, label);
                observe_control(label, response.rect);
                if response.clicked() {
                    let id = self.operation_id();
                    self.edit_job(ctx, &[], move |job| {
                        authoring::set_mode_in(job, &id, mode);
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

    pub(super) fn operation_group(
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

    pub(super) fn operation_numbers(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        fields: &[usize],
    ) {
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
        match self.operation_kind() {
            Some(crate::session::OperationKind::Face) => return self.face_panel(ui, ctx),
            Some(crate::session::OperationKind::DragKnife) => return self.knife_panel(ui, ctx),
            _ => {}
        }
        match self.operation_tab {
            0 => {
                self.carving_geometry(ui, ctx);
                self.operation_group(ui, "Height & depth", true, |app, ui| {
                    help::label(ui, "Operation top");
                    ui.horizontal_wrapped(|ui| {
                        let mut choices: Vec<(String, cam_core::project::HeightReference)> = vec![
                            (
                                "Top: stock top".to_string(),
                                cam_core::project::HeightReference::StockTop,
                            ),
                            (
                                "Top: stock bottom".to_string(),
                                cam_core::project::HeightReference::StockBottom,
                            ),
                        ];
                        // A preceding Face operation publishes a plane this
                        // carve may start from; reordering or disabling it
                        // leaves the reference unresolved with a located issue.
                        for face_id in crate::face::published_faces(
                            &app.document.as_ref().unwrap().job,
                            &app.operation_id(),
                        ) {
                            choices.push((
                                format!("Top: {face_id} face result"),
                                cam_core::project::HeightReference::FaceResult {
                                    operation_id: face_id,
                                },
                            ));
                        }
                        for (label, reference) in choices {
                            let response = ui.selectable_label(
                                app.flat_vcarve().top.reference == reference,
                                &label,
                            );
                            observe_control(&label, response.rect);
                            if response.clicked() {
                                let id = app.operation_id();
                                let reference = reference.clone();
                                app.edit_job(ctx, &[], move |job| {
                                    crate::authoring::settings_mut_in(job, &id)
                                        .ok_or("This operation is not a Flat V-carve")?
                                        .top
                                        .reference = reference;
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
                let target_only = finish && self.flat_vcarve().mode == FlatVcarveMode::EndmillOnly;
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
                let missing_geometry = authoring::tool_in(
                    &self.document.as_ref().unwrap().job,
                    &self.operation_id(),
                    finish,
                )
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
                if !finish && self.flat_vcarve().rough.is_some() {
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

    /// The operation's own filled-component selection. Geometry belongs to
    /// each operation, so nothing here reads or writes an artwork-level
    /// assignment; the viewport assigns through the same command.
    fn carving_geometry(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let job = &self.document.as_ref().unwrap().job;
        let operation_id = self.document.as_ref().unwrap().raw.operation.clone();
        let selected = engine::settings_in(job, &operation_id)
            .map(|settings| settings.components.clone())
            .unwrap_or_default();
        let components = self.components.clone();
        let artwork = job
            .artwork
            .iter()
            .map(|item| (item.id.0.clone(), item.name.clone()))
            .collect::<Vec<_>>();
        let unresolved = selected
            .iter()
            .filter(|reference| !components.iter().any(|c| &c.reference == *reference))
            .cloned()
            .collect::<Vec<_>>();
        let idle = self.active.is_none();
        let mut next = selected.clone();
        self.operation_group(ui, "Geometry to carve", true, |app, ui| {
            ui.small(format!(
                "{} of {} filled components selected. Cyan = selected; gray = excluded.",
                selected.len(),
                components.len()
            ));
            ui.small("Click a filled region in the viewport to assign it to this operation; Shift-click adds or removes one.");
            ui.horizontal_wrapped(|ui| {
                if button(ui, "Select all filled components", idle && !components.is_empty())
                    .clicked()
                {
                    next = components.iter().map(|c| c.reference.clone()).collect();
                }
                if button(ui, "Clear component selection", idle && !selected.is_empty()).clicked() {
                    next.clear();
                }
            });
            if components.is_empty() {
                ui.label("This artwork has no filled components to carve. Stroked paths are knife geometry, not carving regions.");
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("carving-geometry-list")
                    .auto_shrink([false, true])
                    .max_height(180.)
                    .show(ui, |ui| {
                        for (item, name) in &artwork {
                            let local = components
                                .iter()
                                .filter(|c| &c.reference.artwork_item_id.0 == item)
                                .collect::<Vec<_>>();
                            if local.is_empty() {
                                continue;
                            }
                            ui.strong(name);
                            for component in local {
                                let mut on = next.contains(&component.reference);
                                let response = ui.add_enabled(
                                    idle,
                                    egui::Checkbox::new(
                                        &mut on,
                                        component.reference.local_geometry_id.clone(),
                                    ),
                                );
                                observe_control(
                                    &format!(
                                        "Carving component {} / {}",
                                        component.reference.artwork_item_id.0,
                                        component.reference.local_geometry_id
                                    ),
                                    response.rect,
                                );
                                if response.changed() {
                                    next.retain(|r| r != &component.reference);
                                    if on {
                                        next.push(component.reference.clone());
                                    }
                                }
                                ui.small(format!(
                                    "[{:.2}, {:.2}] – [{:.2}, {:.2}] mm",
                                    component.bounds[0],
                                    component.bounds[1],
                                    component.bounds[2],
                                    component.bounds[3]
                                ));
                            }
                        }
                    });
            }
            if !unresolved.is_empty() {
                let response = ui.colored_label(
                    Color32::from_rgb(176, 42, 35),
                    format!(
                        "{} selected reference(s) are unresolved: their source changed. Replace or remove each one explicitly.",
                        unresolved.len()
                    ),
                );
                observe_control("Unresolved selections", response.rect);
                ui.small("Reused local IDs never repair a changed source by themselves.");
                for (index, reference) in unresolved.iter().enumerate() {
                    ui.label(format!(
                        "{} / {} · revision {}",
                        reference.artwork_item_id.0,
                        reference.local_geometry_id,
                        &reference.source_revision.content_digest[..12]
                    ));
                    let same_item = components
                        .iter()
                        .filter(|c| c.reference.artwork_item_id == reference.artwork_item_id)
                        .collect::<Vec<_>>();
                    let candidates = if same_item.is_empty() {
                        components.iter().collect::<Vec<_>>()
                    } else {
                        same_item
                    };
                    ui.horizontal_wrapped(|ui| {
                        let menu = ui.menu_button(
                            format!("Replace reference {} with…", index + 1),
                            |ui| {
                                if candidates.is_empty() {
                                    ui.label(
                                        "This artwork has no current filled component to replace it with.",
                                    );
                                }
                                for candidate in &candidates {
                                    let label = format!(
                                        "{} / {}",
                                        candidate.reference.artwork_item_id.0,
                                        candidate.reference.local_geometry_id
                                    );
                                    if button(ui, &label, idle).clicked() {
                                        app.artwork_command(
                                            engine::ArtworkCommand::Repair {
                                                expected: reference.clone(),
                                                replacement: candidate.reference.clone(),
                                            },
                                            ctx,
                                        );
                                        ui.close();
                                    }
                                }
                            },
                        );
                        observe_control(
                            &format!("Replace reference {}", index + 1),
                            menu.response.rect,
                        );
                        if button(
                            ui,
                            &format!("Remove unresolved reference {}", index + 1),
                            idle,
                        )
                        .clicked()
                        {
                            app.edit_job(ctx, &[], |job| {
                                crate::authoring::settings_mut_in(job, &operation_id)
                                    .ok_or("This operation is not a Flat V-carve")?
                                    .components
                                    .retain(|r| r != reference);
                                Ok(())
                            });
                        }
                    });
                }
            }
            if next != selected {
                app.artwork_command(
                    engine::ArtworkCommand::CarveSelection { references: next },
                    ctx,
                );
            }
        });
    }

    fn operation_tool(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        finish: bool,
        _cutting: bool,
    ) {
        // Every operation renders the same cutter picker; a carving stage's
        // two assignments differ only in their role and their words.
        let operation = self.operation_id();
        let cutter = if finish {
            super::tool_picker::Cutter::vbit(&operation)
        } else {
            super::tool_picker::Cutter::endmill(&operation)
        };
        self.tool_picker(ui, ctx, &cutter);
    }

    pub(super) fn operation_capabilities(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        finish: bool,
    ) {
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
            let tool = authoring::tool_in(
                &self.document.as_ref().unwrap().job,
                &self.operation_id(),
                finish,
            );
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
                let id = self.operation_id();
                self.edit_job(ctx, &[], move |job| {
                    let tool = authoring::tool_mut_in(job, &id, finish)?;
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
        let selected = self.flat_vcarve().rough.as_ref().map(|r| r.strategy);
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
                        let id = self.operation_id();
                        self.edit_job(ctx, &[], move |job| {
                            crate::authoring::settings_mut_in(job, &id)
                                .ok_or("This operation is not a Flat V-carve")?
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
        let artwork = crate::authoring::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        let job = crate::operation_authoring::apply(
            &artwork,
            crate::operation_authoring::add(crate::operation_authoring::Kind::FlatVcarve, &artwork),
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
    fn geometry_issue_focus_reveals_the_shape_tab() {
        for label in [
            "Select all filled components",
            "Clear component selection",
            "Unresolved selections",
            "Operation Geometry to carve",
            "Carving component artwork-1 / letter-l::0",
            "Replace reference 1",
            "Remove unresolved reference 1",
        ] {
            assert_eq!(tab_for_control(label), 0, "{label}");
        }
        assert_eq!(tab_for_control("Roughing feed"), 1);
        assert_eq!(tab_for_control("Finishing feed"), 2);
    }

    #[test]
    fn operation_owns_the_component_selection_and_artwork_does_not() {
        let mut app = app();
        let components = crate::authoring::catalogue_components(
            &cam_core::project::v5::artwork::inspect_artwork(&app.document.as_ref().unwrap().job)
                .unwrap(),
        );
        assert!(!components.is_empty());
        app.components = components;
        app.operation_tab = 0;
        let ctx = egui::Context::default();
        let controls = render(&mut app, &ctx);
        assert!(controls.contains_key("Operation Geometry to carve"));
        assert!(controls.contains_key("Select all filled components"));
        assert!(controls.contains_key("Clear component selection"));
        assert!(
            controls.keys().any(|k| k.starts_with("Carving component ")),
            "the operation lists its own selectable components"
        );
        assert!(!controls.contains_key("Use picked"));
        app.inspector_tab = 0;
        let artwork = render(&mut app, &ctx);
        assert!(artwork.contains_key("Open operation geometry"));
        assert!(!artwork.contains_key("Select all filled components"));
        assert!(!artwork.contains_key("Clear component selection"));
        assert!(!artwork.keys().any(|k| k.starts_with("Carving component ")));
        app.inspector_tab = 2;
        app.operation_tab = 1;
        let endmill = render(&mut app, &ctx);
        assert!(
            !endmill.keys().any(|k| k.starts_with("Carving component ")),
            "the tool tabs keep their own fields"
        );
    }

    #[test]
    fn incomplete_ramp_choice_cannot_generate_export_or_save_as_plunge() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.operation_ramp_draft = true;
        let job = app.document.as_ref().unwrap().job.to_json().unwrap();
        app.submit(Command::generate(job.clone()), &ctx);
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
