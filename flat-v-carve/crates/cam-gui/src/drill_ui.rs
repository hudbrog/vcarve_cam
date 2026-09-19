//! Drill operation editor: marker-point selection with per-diameter groups,
//! heights (top, bottom, retract) and the cycle (pecking, dwell, depth
//! reference, ordering). The planner reports what the settings actually cut.
use super::*;
use crate::drill;
use cam_core::project::v5;
use cam_core::project::v5::artwork::PointEntry;
use cam_core::project::{
    DrillDepthReference, DrillHoleOrder, DrillPeckMode, HeightReference, SpindleDirection,
};
use std::collections::BTreeMap;

impl App {
    pub(super) fn drill_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(doc) = &self.document else { return };
        let id = doc.raw.operation.clone();
        if crate::session::drill(&doc.job, &id).is_none() {
            return;
        }
        ui.small("Positional markers · holes cut by the drill bit");
        self.operation_tabs(ui, &["Holes", "Heights", "Cycle"]);
        let _ = ctx;
    }

    pub(super) fn drill_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        if id.is_empty() {
            return;
        }
        if self.operation_section_visible(0) {
            self.drill_points(ui, ctx);
        }
        if self.operation_section_visible(1) {
            self.drill_heights(ui, ctx);
        }
        if self.operation_section_visible(2) {
            self.drill_cycle(ui, ctx);
            self.drill_tool(ui, ctx);
        }
    }

    /// The catalogue's marker points, refreshed when the document revision
    /// moves. Points come from the same import the other readings do, so the
    /// panel never re-parses anything the reference inspection did not.
    fn refresh_points(&mut self) {
        let Some(doc) = &self.document else { return };
        if self.points_revision == self.revision {
            return;
        }
        self.points = v5::artwork::inspect_artwork(&doc.job)
            .map(|catalogue| {
                catalogue
                    .items
                    .iter()
                    .flat_map(|item| item.point_entries.iter().cloned())
                    .collect()
            })
            .unwrap_or_default();
        self.points_revision = self.revision;
    }

    fn drill_points(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.search.is_empty() {
            return;
        }
        self.refresh_points();
        let job = &self.document.as_ref().unwrap().job;
        let operation_id = self.document.as_ref().unwrap().raw.operation.clone();
        let selected = crate::session::drill(job, &operation_id)
            .map(|s| s.points.clone())
            .unwrap_or_default();
        let points = self.points.clone();
        let artwork = job
            .artwork
            .iter()
            .map(|item| (item.id.0.clone(), item.name.clone()))
            .collect::<Vec<_>>();
        let unresolved = selected
            .iter()
            .filter(|reference| !points.iter().any(|p| &p.reference == *reference))
            .cloned()
            .collect::<Vec<_>>();
        let idle = self.active.is_none();
        let mut next = selected.clone();
        self.operation_group(ui, "Holes to drill", true, |app, ui| {
            ui.small(format!(
                "{} selected · {} marker points",
                selected.len(),
                points.len()
            ))
            .on_hover_text("Circles mark hole centers; drawn dots mark their own centroid. The marker's diameter is the drawing, not the hole — the drill bit cuts the hole.");
            ui.horizontal_wrapped(|ui| {
                let all = ui.add_enabled(idle && !points.is_empty(), egui::Button::new("Select all"));
                observe_control("Select all drill points", all.rect);
                if all.clicked() {
                    next = points.iter().map(|p| p.reference.clone()).collect();
                }
                let clear = ui.add_enabled(idle && !selected.is_empty(), egui::Button::new("Clear"));
                observe_control("Clear point selection", clear.rect);
                if clear.clicked() {
                    next.clear();
                }
            });
            // The requested per-diameter autoselect: one button per distinct
            // marker diameter picks every circle that size.
            if !points.is_empty() {
                let mut by_diameter: BTreeMap<u64, Vec<&PointEntry>> = BTreeMap::new();
                for point in &points {
                    by_diameter
                        .entry((point.diameter_mm * 100.).round() as u64)
                        .or_default()
                        .push(point);
                }
                if by_diameter.len() > 1 || !by_diameter.is_empty() {
                    help::label(ui, "Select by marker size");
                    ui.horizontal_wrapped(|ui| {
                        for (_, group) in by_diameter {
                            let label = format!(
                                "Ø{:.2} ×{}",
                                group[0].diameter_mm,
                                group.len()
                            );
                            let r = ui.add_enabled(idle, egui::Button::new(label));
                            observe_control(&format!("Select all Ø{:.2}", group[0].diameter_mm), r.rect);
                            if r.clicked() {
                                next.retain(|kept| {
                                    !group.iter().any(|p| &p.reference == kept)
                                });
                                next.extend(group.iter().map(|p| p.reference.clone()));
                            }
                        }
                    });
                }
            }
            if points.is_empty() {
                ui.label("This artwork has no drill markers: draw circles where holes go, or filled dots — a dot's centroid becomes the hole position.");
            } else {
                let list = egui::ScrollArea::vertical()
                    .id_salt("drill-geometry-list")
                    .auto_shrink([false, true])
                    .max_height(180.)
                    .show(ui, |ui| {
                        for (item, name) in &artwork {
                            let local = points
                                .iter()
                                .filter(|p| &p.reference.artwork_item_id.0 == item)
                                .collect::<Vec<_>>();
                            if local.is_empty() {
                                continue;
                            }
                            ui.strong(name);
                            for point in local {
                                let mut on = next.contains(&point.reference);
                                let label = source_identity::source_name(
                                    &source_identity::SourceRow {
                                        value: point.reference.clone(),
                                        label: point.label.as_deref(),
                                        group: point.group.as_deref(),
                                        paint: point.paint,
                                        id: &point.reference.local_geometry_id,
                                    },
                                );
                                let response = ui
                                    .horizontal(|ui| {
                                        let response = ui.add_enabled(
                                            idle,
                                            egui::Checkbox::new(&mut on, label),
                                        );
                                        source_identity::paint_swatch(ui, point.paint);
                                        response
                                    })
                                    .inner;
                                observe_control(
                                    &format!(
                                        "Drill point {} / {}",
                                        point.reference.artwork_item_id.0,
                                        point.reference.local_geometry_id
                                    ),
                                    response.rect,
                                );
                                if response.changed() {
                                    next.retain(|r| r != &point.reference);
                                    if on {
                                        next.push(point.reference.clone());
                                    }
                                }
                                ui.small(format!(
                                    "({:.2}, {:.2}) · Ø{:.2}{}",
                                    point.center.x,
                                    point.center.y,
                                    point.diameter_mm,
                                    if point.exact { "" } else { " · centroid" },
                                ));
                            }
                        }
                    });
                observe_control("Drill geometry viewport", list.inner_rect);
            }
            if !unresolved.is_empty() {
                let response = ui.colored_label(
                    Color32::from_rgb(176, 42, 35),
                    format!(
                        "{} selected reference(s) are unresolved: their source changed. Replace or remove each one explicitly.",
                        unresolved.len()
                    ),
                );
                observe_control("Unresolved drill selections", response.rect);
                for (index, reference) in unresolved.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "{} / {} · revision {}",
                            reference.artwork_item_id.0,
                            reference.local_geometry_id,
                            &reference.source_revision.content_digest[..12]
                        ));
                        if button(
                            ui,
                            &format!("Remove unresolved reference {}", index + 1),
                            idle,
                        )
                        .clicked()
                        {
                            app.edit_job(ctx, &[], |job| {
                                drill::drill_mut(job, &operation_id)
                                    .ok_or("This operation is not a drill")?
                                    .points
                                    .retain(|r| r != reference);
                                Ok(())
                            });
                        }
                    });
                }
            }
            if next != selected {
                app.artwork_command(
                    engine::ArtworkCommand::DrillSelection { references: next },
                    ctx,
                );
            }
        });
    }

    fn drill_heights(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let operation_id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = crate::session::drill(&job, &operation_id) else {
            return;
        };
        let top = settings.top.clone();
        let bottom = settings.bottom.clone();
        let retract = settings.retract_height.clone();
        let depth_reference = settings.depth_reference;
        let faces = crate::face::published_faces(&job, &operation_id);
        self.operation_group(ui, "Heights", true, |app, ui| {
            app.drill_height_choice(ui, ctx, 0, &top, &faces);
            app.operation_numbers(ui, ctx, &[111]);
            ui.separator();
            app.drill_height_choice(ui, ctx, 1, &bottom, &faces);
            app.operation_numbers(ui, ctx, &[112]);
            ui.separator();
            help::label(ui, "Retract height (R plane)");
            app.drill_height_choice(ui, ctx, 2, &retract, &faces);
            app.operation_numbers(ui, ctx, &[113]);
            ui.small("Peck retracts return here and feeding starts here; travel between holes stays at the job clearance plane.");
        });
        self.operation_group(ui, "Depth reference", true, |app, ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Drill tip", DrillDepthReference::Tip),
                    ("Full diameter", DrillDepthReference::FullDiameter),
                ] {
                    let r = ui.selectable_label(depth_reference == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && depth_reference != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            drill::set_depth_reference(job, &id, value)
                        });
                    }
                }
            });
            ui.small(match depth_reference {
                DrillDepthReference::Tip => "The bottom is where the drill's point lands.",
                DrillDepthReference::FullDiameter => {
                    "The bottom is where the cutting lips arrive: the tip over-drills by its cone height so the hole breaks through at full width."
                }
            });
            app.operation_numbers(ui, ctx, &[114]);
        });
    }

    fn drill_height_choice(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        which: usize,
        current: &cam_core::project::HeightRef,
        faces: &[String],
    ) {
        let label = match which {
            0 => "Top",
            1 => "Bottom",
            _ => "Retract",
        };
        help::label(ui, label);
        ui.horizontal_wrapped(|ui| {
            let mut choices: Vec<(String, HeightReference)> = vec![
                (
                    match which {
                        0 => "Top: stock top".to_string(),
                        1 => "Bottom: stock top".to_string(),
                        _ => "Retract: stock top".to_string(),
                    },
                    HeightReference::StockTop,
                ),
                (
                    match which {
                        0 => "Top: stock bottom".to_string(),
                        1 => "Bottom: stock bottom (through)".to_string(),
                        _ => "Retract: stock bottom".to_string(),
                    },
                    HeightReference::StockBottom,
                ),
            ];
            for face in faces {
                choices.push((
                    format!("{label}: face result of '{face}'"),
                    HeightReference::FaceResult {
                        operation_id: face.clone(),
                    },
                ));
            }
            let selected = choices
                .iter()
                .find(|(_, reference)| *reference == current.reference)
                .map(|(text, _)| text.clone())
                .unwrap_or_else(|| "Custom reference".to_string());
            let mut chosen: Option<HeightReference> = None;
            let combo = egui::ComboBox::from_id_salt(("drill-height-choice", which))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (text, reference) in &choices {
                        let hit = ui.selectable_label(*reference == current.reference, text);
                        if hit.clicked() {
                            chosen = Some(reference.clone());
                        }
                    }
                });
            let _ = combo;
            if let Some(reference) = chosen {
                let id = self.operation_id();
                self.edit_job(ctx, &[], move |job| {
                    drill::set_height_reference(job, &id, which, reference)
                });
            }
        });
    }

    fn drill_cycle(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let operation_id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = crate::session::drill(&job, &operation_id) else {
            return;
        };
        let peck = settings.peck.clone();
        let order = settings.hole_order;
        let warn = settings.warn_drill_exceeds_marker;
        self.operation_group(ui, "Pecking", true, |app, ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, mode) in [
                    ("Single plunge", None),
                    ("Chip break", Some(DrillPeckMode::ChipBreak)),
                    ("Full retract", Some(DrillPeckMode::FullRetract)),
                ] {
                    let selected = match (&peck, mode) {
                        (None, None) => true,
                        (Some(current), Some(mode)) => current.mode == mode,
                        _ => false,
                    };
                    let r = ui.selectable_label(selected, label);
                    observe_control(label, r.rect);
                    if r.clicked() && !selected {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            drill::set_peck_mode(job, &id, mode)
                        });
                    }
                }
            });
            if let Some(peck) = &peck {
                ui.small(match peck.mode {
                    DrillPeckMode::ChipBreak => {
                        "Each peck retracts a short distance inside the hole to break the chip (G73-style)."
                    }
                    DrillPeckMode::FullRetract => {
                        "Each peck retracts fully to the R plane so the flutes clear (G83-style deep drilling)."
                    }
                });
                app.operation_numbers(ui, ctx, &[116, 117, 118]);
                if peck.mode == DrillPeckMode::ChipBreak {
                    app.operation_numbers(ui, ctx, &[119]);
                }
            } else {
                ui.small("The whole hole is drilled in one feeding plunge (G81-style).");
            }
        });
        self.operation_group(ui, "Bottom dwell", false, |app, ui| {
            app.operation_numbers(ui, ctx, &[115]);
            ui.small(
                "Seconds the spindle keeps cutting at full depth before retracting (G82-style).",
            );
        });
        self.operation_group(ui, "Hole order", false, |app, ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("X then Y", DrillHoleOrder::XThenY),
                    ("As selected", DrillHoleOrder::AsSelected),
                ] {
                    let r = ui.selectable_label(order == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && order != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            drill::set_hole_order(job, &id, value)
                        });
                    }
                }
            });
            ui.checkbox(&mut warn.clone(), "Warn when the drill is wider than a marker")
                .changed()
                .then(|| {
                    let id = app.operation_id();
                    app.edit_job(ctx, &[], move |job| {
                        drill::set_warn_drill_exceeds_marker(job, &id, warn)
                    });
                });
            ui.small("Markers are positional: a small dot drilled by a wider bit is a legitimate drawing style, so the advisory is opt-in.");
        });
    }

    fn drill_tool(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.operation_group(ui, "Tool & cutting profile", true, |app, ui| {
            let operation = app.operation_id();
            let cutter = super::tool_picker::Cutter::drill(&operation);
            app.tool_picker(ui, ctx, &cutter);
        });
        let missing_geometry = authoring::tool_in(
            &self.document.as_ref().unwrap().job,
            &self.operation_id(),
            false,
        )
        .is_none_or(|t| t.geometry.is_none());
        self.operation_group(
            ui,
            "Geometry & capabilities",
            missing_geometry,
            |app, ui| {
                app.operation_numbers(ui, ctx, &[12, 13]);
                ui.separator();
                app.operation_capabilities(ui, ctx, false);
            },
        );
        self.operation_group(ui, "Feeds & speed", true, |app, ui| {
            // A drill's cutting feed is its plunge feed; the shared ids bind
            // the one value the assignment carries.
            app.operation_numbers(ui, ctx, &[10, 11]);
            let direction =
                crate::session::drill(&app.document.as_ref().unwrap().job, &app.operation_id())
                    .and_then(|s| s.assignment.spindle_direction);
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Clockwise", Some(SpindleDirection::Clockwise)),
                    ("Counterclockwise", Some(SpindleDirection::Counterclockwise)),
                ] {
                    let r = ui.selectable_label(direction == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && direction != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            drill::set_spindle_direction(job, &id, value)
                        });
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace with circle artwork and one selected drill operation.
    fn drill_workspace() -> CamJobV5 {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><circle id="a" cx="10" cy="15" r="2.5" fill="#000"/><circle id="b" cx="30" cy="20" r="4" fill="#000"/></svg>"##;
        let artwork = crate::authoring::import_svg("holes.svg".into(), svg.into()).unwrap();
        crate::operation_authoring::apply(
            &artwork,
            crate::operation_authoring::add(crate::operation_authoring::Kind::Drill, &artwork),
        )
        .unwrap()
    }

    /// The gate every worker command passes the document through: a job that
    /// carries a Drill operation must survive it, or no selection, tool or
    /// generation command can ever run (the 2026-09-19 report).
    #[test]
    fn a_document_with_a_drill_operation_passes_the_workspace_gate() {
        let job = drill_workspace();
        let text = job.to_json().unwrap();
        assert!(
            crate::session::open(&text).is_ok(),
            "drill documents must pass the session document gate"
        );
    }

    /// The full worker path a clicked marker row drives: the document goes
    /// through the gate, the DrillSelection command binds the catalogue's
    /// point references, and the reply carries the updated selection.
    #[test]
    fn the_drill_selection_command_selects_marker_points() {
        let job = drill_workspace();
        let operation_id = job.operations[0].id.clone();
        let catalogue = v5::artwork::inspect_artwork(&job).unwrap();
        let references: Vec<v5::GeometryRef> = catalogue.items[0]
            .point_entries
            .iter()
            .map(|entry| entry.reference.clone())
            .collect();
        assert_eq!(references.len(), 2, "both circles publish points");
        let (meta, _scene_bytes) = crate::session::run(crate::session::Command::Artwork {
            job: job.to_json().unwrap(),
            operation_id,
            action: engine::ArtworkCommand::DrillSelection { references },
        })
        .expect("the selection command runs");
        let updated = CamJobV5::from_json(&meta.job).unwrap();
        let points = crate::session::drill(&updated, &updated.operations[0].id)
            .expect("drill operation survives the reply")
            .points
            .clone();
        assert_eq!(points.len(), 2, "both markers selected: {points:?}");
    }
}
