//! Pocket selectors use document edits; all numeric input uses shared drafts.
use super::*;
use cam_core::project::{CutDirection, HeightReference, LeadSpec, v5::PocketEntry};

fn height(
    ui: &mut egui::Ui,
    label: &str,
    reference: &mut HeightReference,
    bottom: bool,
    faces: &[String],
) {
    ui.push_id(label, |ui| {
        ui.label(label);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(reference, HeightReference::StockTop, "Stock top");
            ui.selectable_value(reference, HeightReference::StockBottom, "Stock bottom");
            if bottom {
                ui.selectable_value(reference, HeightReference::OperationTop, "Operation top");
            }
            for id in faces {
                ui.selectable_value(
                    reference,
                    HeightReference::FaceResult {
                        operation_id: id.clone(),
                    },
                    format!("Face: {id}"),
                );
            }
        });
    });
}
fn lead_selector(ui: &mut egui::Ui, label: &str, lead: &mut LeadSpec) {
    ui.push_id(label, |ui| {
        ui.label(label);
        ui.horizontal(|ui| {
            let kind = match lead {
                LeadSpec::None => 0,
                LeadSpec::TangentLine { .. } => 1,
                LeadSpec::TangentArc { .. } => 2,
            };
            for (i, name) in ["None", "Tangent line", "Tangent arc"].iter().enumerate() {
                let r = ui.selectable_label(kind == i, *name);
                observe_control(&format!("Pocket {label} {name}"), r.rect);
                if r.clicked() && i != kind {
                    *lead = match i {
                        0 => LeadSpec::None,
                        1 => LeadSpec::TangentLine {
                            length_mm: None,
                            feed_mm_min: None,
                        },
                        _ => LeadSpec::TangentArc {
                            radius_mm: None,
                            sweep_deg: None,
                            feed_mm_min: None,
                        },
                    };
                }
            }
        });
    });
}
impl App {
    pub(super) fn pocket_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(original) = crate::pocket::settings(&job, &id) else {
            return;
        };
        let mut settings = original.clone();
        if self.operation_section_visible(0) {
            self.carving_geometry(ui, ctx);
            self.operation_group(ui, "Pocket heights", true, |app, ui| {
                ui.small("Selected pockets share these heights. Use separate operations for different depths. Negative offsets cut below the reference.");
                let faces = crate::face::published_faces(&job, &id);
                height(ui, "Top", &mut settings.top.reference, false, &faces);
                app.operation_numbers(ui, ctx, &[123]);
                height(ui, "Bottom", &mut settings.bottom.reference, true, &faces);
                app.operation_numbers(ui, ctx, &[124]);
            });
        }
        if self.operation_section_visible(1) {
            self.operation_group(ui, "Pocket tool", true, |app, ui| {
                app.tool_picker(ui, ctx, &super::tool_picker::Cutter::milling(&id, "Pocket"));
                app.operation_numbers(ui, ctx, &[12, 13]);
                app.operation_capabilities(ui, ctx, false);
            });
            self.operation_group(ui, "Pocket cutting values", true, |app, ui| {
                app.operation_numbers(ui, ctx, &[2, 11, 8, 9, 1]);
                ui.horizontal(|ui| {
                    ui.label("Spindle");
                    ui.selectable_value(&mut settings.assignment.spindle_direction, Some(SpindleDirection::Clockwise), "CW");
                    ui.selectable_value(&mut settings.assignment.spindle_direction, Some(SpindleDirection::Counterclockwise), "CCW");
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut settings.direction, Some(CutDirection::Climb), "Climb");
                    ui.selectable_value(&mut settings.direction, Some(CutDirection::Conventional), "Conventional");
                });
                ui.checkbox(&mut settings.finish_walls, "Finish walls with the same tool");
                if settings.finish_walls {
                    app.operation_numbers(ui, ctx, &[92]);
                    ui.small("Unset finish feed uses cutting feed. Finishing uses the same depth layers.");
                }
            });
            self.operation_group(ui, "Planning limits", false, |app, ui| {
                app.operation_numbers(ui, ctx, &[48, 49, 50]);
            });
        }
        if self.operation_section_visible(2) {
            self.operation_group(ui, "Pocket entry & leads", true, |app, ui| {
                ui.horizontal(|ui| {
                    let kind = match settings.entry { PocketEntry::Plunge => 0, PocketEntry::Ramp{..} => 1, PocketEntry::Helix{..} => 2 };
                    for (i, name) in ["Plunge", "Linear ramp", "Helix"].iter().enumerate() {
                        let r = ui.selectable_label(kind == i, *name);
                        observe_control(&format!("Pocket entry {name}"), r.rect);
                        if r.clicked() && i != kind {
                            settings.entry = match i {
                                0 => PocketEntry::Plunge,
                                1 => PocketEntry::Ramp{max_angle_deg: None, feed_mm_min: None},
                                _ => PocketEntry::Helix{radius_mm: None, max_angle_deg: None, feed_mm_min: None},
                            };
                        }
                    }
                });
                // A changed selector commits below. Render its numbers on the next frame.
                if settings.entry == original.entry {
                    match settings.entry {
                        PocketEntry::Plunge => app.operation_numbers(ui, ctx, &[10]),
                        PocketEntry::Ramp{..} => app.operation_numbers(ui, ctx, &[98,99]),
                        PocketEntry::Helix{..} => {
                            app.operation_numbers(ui, ctx, &[125,98,99]);
                            ui.small("Radius is the cutter-center path radius and must be smaller than the cutter radius. Entry placement is automatic.");
                        }
                    }
                }
                if let PocketEntry::Helix{radius_mm: Some(r), ..} = settings.entry
                    && let Some(cam_core::project::ToolGeometry::Endmill(tool)) = authoring::tool_in(&job, &id, false).and_then(|t| t.geometry.as_ref()) {
                    ui.small(format!("Helix swept diameter: {:.3} mm", 2. * r + tool.diameter_mm));
                }
                for (out, label) in [(false, "Lead-in"), (true, "Lead-out")] {
                    ui.separator();
                    let lead = if out { &mut settings.lead_out } else { &mut settings.lead_in };
                    lead_selector(ui, label, lead);
                    let old = if out { &original.lead_out } else { &original.lead_in };
                    if lead == old {
                        match lead {
                            LeadSpec::None => {},
                            LeadSpec::TangentLine{..} => app.operation_numbers(ui, ctx, if out { &[104,105] } else { &[100,101] }),
                            LeadSpec::TangentArc{..} => app.operation_numbers(ui, ctx, if out { &[106,107,105] } else { &[102,103,101] }),
                        }
                    }
                }
            });
        }
        if settings != *original {
            let mut clear = Vec::new();
            if settings.entry != original.entry {
                clear.extend([98, 99, 125]);
            }
            if settings.lead_in != original.lead_in {
                clear.extend(100..=103);
            }
            if settings.lead_out != original.lead_out {
                clear.extend(104..=107);
            }
            self.edit_job(ctx, &clear, |job| {
                // Merge only selector changes: shared numeric/tool edits may already
                // have committed this frame and must never be overwritten.
                let s = crate::pocket::settings_mut(job, &id)?;
                s.top.reference = settings.top.reference;
                s.bottom.reference = settings.bottom.reference;
                if settings.assignment.spindle_direction != original.assignment.spindle_direction {
                    s.assignment.spindle_direction = settings.assignment.spindle_direction;
                }
                s.direction = settings.direction;
                s.finish_walls = settings.finish_walls;
                if settings.entry != original.entry {
                    s.entry = settings.entry;
                }
                if settings.lead_in != original.lead_in {
                    s.lead_in = settings.lead_in;
                }
                if settings.lead_out != original.lead_out {
                    s.lead_out = settings.lead_out;
                }
                Ok(())
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let job = crate::authoring::import_svg("pocket.svg".into(),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="30mm" height="30mm" viewBox="0 0 30 30"><rect id="p" x="3" y="3" width="20" height="20"/></svg>"#.into()).unwrap();
        let job = crate::operation_authoring::apply(
            &job,
            crate::operation_authoring::add(crate::operation_authoring::Kind::Pocket, &job),
        )
        .unwrap();
        let catalogue = cam_core::project::v5::inspect_artwork(&job).unwrap();
        App {
            document: Some(Document::new(job)),
            components: crate::authoring::catalogue_components(&catalogue),
            inspector_tab: 2,
            ..Default::default()
        }
    }

    #[test]
    fn pocket_editor_renders_geometry_cutting_and_entry_controls() {
        let mut app = app();
        let ctx = egui::Context::default();
        for (tab, expected) in [
            (0, "Select all filled components"),
            (1, "Stepover"),
            (2, "Pocket entry Helix"),
        ] {
            app.operation_tab = tab;
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 1200.),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.inspector(ctx),
                );
            }
            assert!(
                CONTROLS.with(|c| c.borrow().contains_key(expected)),
                "missing {expected}"
            );
        }
    }

    #[test]
    fn pocket_numeric_drafts_undo_and_recovery_preserve_unfinished_input() {
        let mut app = app();
        let ctx = egui::Context::default();
        let id = app.operation_id();
        app.edit_job(&ctx, &[98, 99, 125], |job| {
            crate::pocket::settings_mut(job, &id)?.entry = PocketEntry::Helix {
                radius_mm: None,
                max_angle_deg: None,
                feed_mm_min: None,
            };
            Ok(())
        });
        app.remember();
        let doc = app.document.as_mut().unwrap();
        doc.edit(125, "1.25".into()).unwrap();
        doc.edit(98, "5".into()).unwrap();
        doc.edit(99, "150".into()).unwrap();
        doc.edit(124, "-2.5".into()).unwrap();
        assert_eq!(value(&doc.job, &id, 125), Some(1.25));
        assert!(doc.edit(124, "-".into()).is_err());
        assert_eq!(value(&doc.job, &id, 124), Some(-2.5));
        let snapshot = doc.snapshot();
        snapshot.validate().unwrap();
        let recovered: crate::recovery::Snapshot =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(
            recovered.draft.raw.get(&recovered.draft.key(124)).unwrap(),
            "-"
        );
        app.undo(&ctx);
        assert_eq!(value(&app.document.as_ref().unwrap().job, &id, 125), None);
        app.redo(&ctx);
        assert_eq!(app.document.as_ref().unwrap().text(124), "-");
        assert_eq!(
            value(&app.document.as_ref().unwrap().job, &id, 125),
            Some(1.25)
        );
    }

    #[test]
    fn pocket_selection_flows_through_session_and_save_reopen() {
        let app = app();
        let doc = app.document.unwrap();
        let references = app.components.iter().map(|c| c.reference.clone()).collect();
        let mut retained = cam_service::retained::Retained::new();
        let (reply, _) = engine::execute(
            &mut retained,
            Command::Artwork {
                job: doc.job.to_json().unwrap(),
                operation_id: doc.raw.operation.clone(),
                action: engine::ArtworkCommand::CarveSelection { references },
            },
        )
        .unwrap();
        let reopened = engine::open(&reply.job).unwrap();
        assert_eq!(
            super::super::super::operation_selection(&reopened, &doc.raw.operation).len(),
            1
        );
        assert_eq!(
            engine::kind(&reopened, &doc.raw.operation),
            Some(OperationKind::Pocket)
        );
    }
}
