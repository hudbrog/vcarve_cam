use super::*;
use cam_core::project::ToolGeometry;

impl App {
    pub(super) fn plot_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(doc) = &self.document else { return };
        let Some(settings) = crate::plot::settings(&doc.job, &id) else {
            return;
        };
        let mut selected = settings.chains.clone();
        if self.operation_section_visible(1) {
            let mut pen_id = settings.tool_id.clone();
            let pens: Vec<_> = doc
                .job
                .tools
                .iter()
                .filter(|t| t.geometry == Some(ToolGeometry::Pen))
                .collect();
            let label = pens
                .iter()
                .find(|p| p.id == pen_id)
                .map_or("Select pen", |p| p.name.as_str());
            let picker = egui::ComboBox::from_label("Pen tool")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for pen in pens {
                        ui.selectable_value(&mut pen_id, pen.id.clone(), &pen.name);
                    }
                });
            observe_control("Pen tool", picker.response.rect);
            if settings.compression_mm == 0. {
                ui.label("Pen down at stock top");
            } else {
                ui.label(format!(
                    "Pen down {:.3} mm below stock top",
                    settings.compression_mm
                ));
            }
            if let Some(up) = doc.job.setup.clearance_above_stock_mm {
                ui.label(format!("Pen up: {up:.3} mm above stock"));
            }
            if pen_id != settings.tool_id {
                self.edit_job(ctx, &[2, 10, 126], |job| {
                    *job = cam_core::project::v5::resources::use_job_tool(
                        job,
                        &id,
                        cam_core::project::v5::resources::AssignmentRole::Pen,
                        &pen_id,
                    )
                    .map_err(|e| e.to_string())?
                    .job;
                    Ok(())
                });
            }
            self.operation_numbers(ui, ctx, &[2, 10, 126, 127]);
        }
        if !self.operation_section_visible(0) {
            return;
        }
        let previous = selected.clone();
        let chains = self.view.knife_chains();
        let paths = ui.heading("Paths to plot");
        observe_control("Paths to plot", paths.rect);
        if chains.is_empty() {
            ui.label("Add SVG or sketch paths to select centerlines.");
        } else {
            let idle = self.active.is_none();
            let all = ui.add_enabled(idle, egui::Button::new("Select all"));
            observe_control("Select all plot paths", all.rect);
            if all.clicked() {
                selected = chains.iter().map(|chain| chain.reference.clone()).collect();
            }
            if idle && let Some(chosen) = source_identity::select_chain_group(ui, &chains) {
                selected = chosen;
            }
        }
        egui::ScrollArea::vertical()
            .id_salt("plot-paths")
            .max_height(180.)
            .show(ui, |ui| {
                for chain in chains.iter() {
                    let reference = &chain.reference;
                    let mut on = selected.contains(reference);
                    let label = format!(
                        "{} / {}",
                        reference.artwork_item_id.0,
                        chain.label.as_ref().unwrap_or(&reference.local_geometry_id)
                    );
                    if ui.checkbox(&mut on, label).changed() {
                        if on {
                            selected.push(reference.clone());
                        } else {
                            selected.retain(|r| r != reference);
                        }
                    }
                }
            });
        if !selected.is_empty() {
            ui.strong("Drawing order");
        }
        let mut swap = None;
        let mut remove = None;
        for (i, r) in selected.iter().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{}. {} / {}",
                        i + 1,
                        r.artwork_item_id.0,
                        r.local_geometry_id
                    ));
                    if ui
                        .add_enabled(i > 0, egui::Button::new("↑"))
                        .on_hover_text("Draw earlier")
                        .clicked()
                    {
                        swap = Some((i, i - 1));
                    }
                    if ui
                        .add_enabled(i + 1 < selected.len(), egui::Button::new("↓"))
                        .on_hover_text("Draw later")
                        .clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                    if ui.small_button("×").on_hover_text("Remove path").clicked() {
                        remove = Some(i);
                    }
                })
            });
        }
        if let Some((a, b)) = swap {
            selected.swap(a, b);
        }
        if let Some(i) = remove {
            selected.remove(i);
        }
        if selected != previous {
            self.artwork_command(
                engine::ArtworkCommand::PlotSelection {
                    references: selected,
                },
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation_authoring::{self, Kind};

    #[test]
    fn plot_bulk_buttons_select_all_paths_or_replace_them_with_one_layer() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="20mm" height="20mm" viewBox="0 0 20 20" fill="none" stroke="black">
            <g inkscape:label="Ink" inkscape:groupmode="layer">
                <path id="a" d="M1 1h3"/><path id="b" d="M4 1v3"/>
            </g>
            <g inkscape:label="Guides" inkscape:groupmode="layer"><path id="c" d="M2 2h5"/></g>
            <path id="d" d="M8 8v2"/>
        </svg>"#;
        let artwork = crate::authoring::import_svg("layers.svg".into(), svg.into()).unwrap();
        let job = operation_authoring::apply(
            &artwork,
            operation_authoring::add(Kind::CenterlinePlot, &artwork),
        )
        .unwrap();
        let preview = engine::run(Command::Preview {
            job: job.to_json().unwrap(),
            have_artwork: None,
        })
        .unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            inspector_tab: 2,
            operation_tab: 0,
            ..Default::default()
        };
        app.view.load_scene(Ok(preview));
        app.port.use_inline_compute();
        let ctx = egui::Context::default();
        let frame = |app: &mut App, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 900.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.inspector(ctx),
            );
        };
        for (label, expected) in [
            (
                "Select all plot paths",
                vec!["a-chain-0", "b-chain-0", "c-chain-0", "d-chain-0"],
            ),
            ("Select all in Ink", vec!["a-chain-0", "b-chain-0"]),
            ("Select all in Guides", vec!["c-chain-0"]),
        ] {
            frame(&mut app, vec![]);
            frame(&mut app, vec![]);
            let [left, top, right, bottom] = control_rect(label).expect(label);
            let pos = egui::pos2((left + right) * 0.5, (top + bottom) * 0.5);
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            app.poll(&ctx);
            let doc = app.document.as_ref().unwrap();
            let selected = &crate::plot::settings(&doc.job, &doc.raw.operation)
                .unwrap()
                .chains;
            assert_eq!(
                selected
                    .iter()
                    .map(|r| r.local_geometry_id.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{label}"
            );
        }
    }
}
