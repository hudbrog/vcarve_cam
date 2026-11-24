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
