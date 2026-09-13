use super::*;
use cam_core::project::v5::StartSelectionV5;

impl App {
    pub(super) fn knife_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.inspector_tab {
            0 => {
                ui.heading("Knife artwork");
                ui.small("Use stroked paths with fill=none. Open and closed chains are preserved; filled regions are not knife selections.");
                self.numbers(ui, ctx, &[26, 27, 28, 29]);
                ui.separator();
                ui.label("Geometry selection");
                ui.small("Which chains get cut belongs to the operation, not to the artwork. Select them under Cutting → Geometry to cut, or click them in the viewport. This panel keeps placement and source management only.");
                if button(ui, "Open operation geometry", self.document.is_some()).clicked() {
                    self.operation_tab = 0;
                    self.navigate(2);
                }
                ui.separator();
                if button(
                    ui,
                    "Replace knife SVG",
                    self.active.is_none() && self.io.is_none(),
                )
                .clicked()
                {
                    self.open(IoKind::ReplaceSvg, ctx);
                }
                if button(ui, "Delete knife artwork", self.active.is_none()).clicked() {
                    let active = self.document.as_ref().unwrap().raw.artwork_item.clone();
                    self.artwork_command(
                        engine::ArtworkCommand::Delete {
                            item: cam_core::project::v5::ArtworkItemId(active),
                        },
                        ctx,
                    );
                }
            }
            4 | 5 => {
                ui.heading("Knife geometry");
                self.numbers(ui, ctx, &[61, 62]);
                ui.small("Blade offset is pivot-to-tip distance. Heading points from pivot toward tip, counterclockwise from +X.");
                self.knife_resources(ui, ctx);
            }
            6 => self.view.knife_inspection_controls(ui),
            _ => {
                self.knife_selection(ui, ctx);
                ui.separator();
                ui.heading("Tool & cutting profile");
                self.knife_resources(ui, ctx);
                self.numbers(ui, ctx, &[61, 62, 63, 64, 65, 66]);
                self.knife_suggestions(ui, ctx);
                ui.separator();
                ui.heading("Depth & passes");
                for bottom in [false, true] {
                    let s = crate::knife::settings(&self.document.as_ref().unwrap().job).unwrap();
                    let reference = if bottom {
                        s.bottom.reference.clone()
                    } else {
                        s.top.reference.clone()
                    };
                    ui.label(if bottom {
                        "Bottom reference"
                    } else {
                        "Top reference"
                    });
                    for (name, r) in [
                        ("Stock top", cam_core::project::HeightReference::StockTop),
                        (
                            "Stock bottom",
                            cam_core::project::HeightReference::StockBottom,
                        ),
                        (
                            "Operation top",
                            cam_core::project::HeightReference::OperationTop,
                        ),
                    ] {
                        if !bottom && matches!(r, cam_core::project::HeightReference::OperationTop)
                        {
                            continue;
                        }
                        let response = ui.selectable_label(reference == r, name);
                        observe_control(
                            &format!("Knife {} {name}", if bottom { "bottom" } else { "top" }),
                            response.rect,
                        );
                        if response.clicked() {
                            self.edit_job(ctx, &[], |job| {
                                let OperationSettingsV5::DragKnife(s) =
                                    &mut job.operations[0].settings
                                else {
                                    unreachable!()
                                };
                                if bottom {
                                    s.bottom.reference = r;
                                } else {
                                    s.top.reference = r;
                                }
                                Ok(())
                            });
                        }
                    }
                }
                self.numbers(ui, ctx, &[73, 74, 67, 68, 69, 70, 71, 72]);
                let settings =
                    crate::knife::settings(&self.document.as_ref().unwrap().job).unwrap();
                ui.label(match &settings.start {
                    StartSelectionV5::Automatic => "Start: automatic source seam / endpoint".into(),
                    StartSelectionV5::Anchor(a) => format!(
                        "Start: {} / {} at {:.1}% of source length",
                        a.geometry.artwork_item_id.0,
                        a.geometry.local_geometry_id,
                        a.fraction_along_source_contour * 100.
                    ),
                });
                let selected = settings.chains.clone();
                let chains = self.view.knife_chains();
                let menu = ui.menu_button("Knife start", |ui| {
                    if button(ui, "Automatic knife start", self.active.is_none()).clicked() {
                        self.artwork_command(
                            engine::ArtworkCommand::KnifeStart {
                                reference: None,
                                fraction: 0.,
                            },
                            ctx,
                        );
                        ui.close();
                    }
                    for chain in chains.iter().filter(|c| selected.contains(&c.reference)) {
                        for fraction in if chain.closed {
                            &[0., 0.25, 0.5, 0.75][..]
                        } else {
                            &[0.][..]
                        } {
                            let label = format!(
                                "{} / {} · {}%",
                                chain.reference.artwork_item_id.0,
                                chain.reference.local_geometry_id,
                                fraction * 100.
                            );
                            if button(ui, &label, self.active.is_none()).clicked() {
                                self.artwork_command(
                                    engine::ArtworkCommand::KnifeStart {
                                        reference: Some(chain.reference.clone()),
                                        fraction: *fraction,
                                    },
                                    ctx,
                                );
                                ui.close();
                            }
                        }
                    }
                });
                observe_control("Knife start", menu.response.rect);
                ui.small("Offsets are signed: negative is downward. Initial heading must match the physically aligned blade. Blank optional allowance/overlap values retain core semantics.");
                ui.small("Near reversals and unsupported passive alignment remain rejected by the planner.");
            }
        }
    }

    fn knife_suggestions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let doc = self.document.as_ref().unwrap();
        if [61, 62, 66, 67]
            .iter()
            .any(|&field| Draft::parse(&doc.text(field)).is_err())
        {
            ui.small("Complete the knife dimensions and stepdown fields to calculate suggestions.");
            return;
        }
        let suggestions = crate::knife_defaults::suggestions(&doc.job)
            .into_iter()
            .filter(|v| doc.text(v.field).trim().is_empty())
            .collect::<Vec<_>>();
        if suggestions.is_empty() {
            return;
        }
        ui.heading("Suggested operation values");
        for suggestion in &suggestions {
            ui.label(format!(
                "{}: {:.3} {}",
                FIELDS[suggestion.field],
                suggestion.value,
                if suggestion.field == 69 { "°" } else { "mm" }
            ));
            ui.small(suggestion.reason);
        }
        ui.small("Fills blank fields only. Feeds come from a cutting profile; cut depth and physical initial heading remain your choices.");
        if button(ui, "Use suggested operation values", self.active.is_none()).clicked() {
            let fields = suggestions.iter().map(|s| s.field).collect::<Vec<_>>();
            self.edit_job(ctx, &fields, |job| {
                for suggestion in suggestions {
                    crate::knife::set(job, suggestion.field, Some(suggestion.value))?;
                }
                Ok(())
            });
        }
    }

    fn knife_selection(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Geometry to cut");
        let chains = self.view.knife_chains();
        let job = &self.document.as_ref().unwrap().job;
        let mut selected = crate::knife::settings(job).unwrap().chains.clone();
        let old = selected.clone();
        ui.label(format!(
            "{} selected · {} available chains",
            selected.len(),
            chains.len()
        ));
        ui.small("Check the open paths or closed outlines this operation should cut.");
        let sources = job
            .artwork
            .iter()
            .map(|i| (i.id.clone(), i.name.clone()))
            .collect::<Vec<_>>();
        if chains.is_empty() {
            ui.label("No knife paths available. Stroked SVG paths can be selected here. Filled artwork needs an explicit outline conversion.");
        } else {
            if button(ui, "Select all knife chains", self.active.is_none()).clicked() {
                selected = chains.iter().map(|c| c.reference.clone()).collect();
            }
            if button(ui, "Clear knife selection", self.active.is_none()).clicked() {
                selected.clear();
            }
            egui::ScrollArea::vertical()
                .id_salt("knife-geometry-list")
                .auto_shrink([false, true])
                .max_height(180.)
                .show(ui, |ui| {
                    for item in &job.artwork {
                        let local = chains
                            .iter()
                            .filter(|c| c.reference.artwork_item_id == item.id)
                            .collect::<Vec<_>>();
                        if local.is_empty() {
                            continue;
                        }
                        ui.strong(&item.name);
                        for chain in local {
                            let mut on = selected.contains(&chain.reference);
                            let response = ui.add_enabled(
                                self.active.is_none(),
                                egui::Checkbox::new(
                                    &mut on,
                                    format!(
                                        "{} · {}",
                                        chain.reference.local_geometry_id,
                                        if chain.closed { "closed" } else { "open" }
                                    ),
                                ),
                            );
                            observe_control(
                                &format!(
                                    "Knife chain {} / {}",
                                    item.id.0, chain.reference.local_geometry_id
                                ),
                                response.rect,
                            );
                            if response.changed() {
                                if on {
                                    selected.push(chain.reference.clone());
                                } else {
                                    selected.retain(|r| r != &chain.reference);
                                }
                            }
                        }
                    }
                });
        }
        if selected
            .iter()
            .any(|r| !chains.iter().any(|c| &c.reference == r))
        {
            let response = ui.colored_label(Color32::DARK_RED,"Some selected paths are unresolved. Clear the selection and choose their replacements.");
            observe_control("Unresolved knife selections", response.rect);
        }
        if selected != old {
            self.artwork_command(
                engine::ArtworkCommand::KnifeSelection {
                    references: selected,
                },
                ctx,
            );
        }
        if !sources.is_empty() {
            let menu=ui.menu_button("Create knife outlines",|ui| {
                ui.label("Create a stroked copy of the imported outer and hole boundaries. Original artwork stays intact; curves use its import tolerance. Choose paths after creating the copy.");
                for (id,name) in sources {
                    if button(ui,&format!("Outlines of {name}"),self.active.is_none()).clicked() {
                        self.artwork_command(engine::ArtworkCommand::KnifeOutlines {item:id},ctx);
                        ui.close();
                    }
                }
            });
            observe_control("Create knife outlines", menu.response.rect);
        }
        if button(
            ui,
            "Import knife geometry",
            self.active.is_none() && self.io.is_none(),
        )
        .clicked()
        {
            self.open(IoKind::AddSvg, ctx);
        }
    }

    fn knife_resources(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // The same cutter picker every operation uses, with the knife's own
        // words and its typed knife cutting presets.
        let operation = self.operation_id();
        let cutter = super::tool_picker::Cutter::knife(&operation);
        self.tool_picker(ui, ctx, &cutter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render(app: &mut App, ctx: &egui::Context) -> std::collections::BTreeMap<String, [f32; 4]> {
        for _ in 0..3 {
            CONTROLS.with(|c| c.borrow_mut().clear());
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 800.),
                    )),
                    ..Default::default()
                },
                |ctx| app.inspector(ctx),
            );
        }
        CONTROLS.with(|c| c.borrow().clone())
    }

    #[test]
    fn knife_geometry_is_selected_in_the_operation_not_in_the_artwork_panel() {
        let job = crate::knife::import_svg(
            "chains.svg".into(),
            include_str!("../../../fixtures/gui6/chains.svg").into(),
        )
        .unwrap();
        let mut app = App {
            document: Some(Document::new(job.clone())),
            ..Default::default()
        };
        let scene = engine::run(Command::Preview {
            job: job.to_json().unwrap(),
        })
        .unwrap();
        app.view.load_scene(Ok(scene));
        let ctx = egui::Context::default();
        app.inspector_tab = 0;
        let artwork = render(&mut app, &ctx);
        assert!(artwork.contains_key("Open operation geometry"));
        assert!(!artwork.keys().any(|k| k.starts_with("Knife chain ")));
        app.inspector_tab = 2;
        let cutting = render(&mut app, &ctx);
        assert!(cutting.contains_key("Select all knife chains"));
        assert!(cutting.keys().any(|k| k.starts_with("Knife chain ")));
    }

    #[test]
    fn knife_editor_renders_every_panel_without_milling_assignments() {
        let job = crate::knife::import_svg(
            "chains.svg".into(),
            include_str!("../../../fixtures/gui6/chains.svg").into(),
        )
        .unwrap();
        let mut app = App {
            document: Some(Document::new(job.clone())),
            ..Default::default()
        };
        app.resources.ready = true;
        let scene = engine::run(Command::Preview {
            job: job.to_json().unwrap(),
        })
        .unwrap();
        app.view.load_scene(Ok(scene));
        let ctx = egui::Context::default();
        for tab in 0..8 {
            app.inspector_tab = tab;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 800.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.navigator(ctx);
                    app.inspector(ctx);
                },
            );
        }
        assert!(crate::knife::settings(&app.document.unwrap().job).is_some());
    }
}
