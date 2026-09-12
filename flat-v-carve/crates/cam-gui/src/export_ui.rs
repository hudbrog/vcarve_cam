use super::*;

#[derive(Default)]
pub(super) struct ExportDialog {
    pub request: Option<u64>,
    pub error: Option<String>,
    pub saving: bool,
}

impl App {
    pub(super) fn export_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.export_dialog else {
            return;
        };
        let preparing = dialog.request.is_some();
        let saving = dialog.saving;
        let prepared = self.prepared.as_ref().filter(|(_, r)| *r == self.revision);
        let mut close = false;
        let mut save = false;
        let response = egui::Modal::new(egui::Id::new("export-gcode")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(240., 560.));
            ui.heading("Export G-code");
            ui.add_space(8.);
            egui::ScrollArea::vertical()
                .id_salt("export-results")
                .max_height((ctx.content_rect().height() - 210.).max(120.))
                .show(ui, |ui| {
                    if preparing {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.strong("Preparing and validating your program…");
                        });
                        ui.label("Checking the toolpath and machine settings, generating G-code, and reading it back to verify the output.");
                        ui.small("When validation finishes, choose Save as… to select a destination.");
                    } else if let Some((prepared, _)) = prepared {
                        ui.colored_label(Color32::from_rgb(27, 112, 77), RichText::new("Ready to save").size(20.).strong());
                        ui.label(format!(
                            "{} · {} bytes",
                            prepared["file"]["filename"].as_str().unwrap_or("G-code program"),
                            prepared["file"]["byteLength"],
                        ));
                        ui.add_space(8.);
                        ui.strong("Validation results");
                        for label in ["Toolpath checks passed", "Machine and tool settings accepted", "Generated G-code readback passed"] {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(14., 18.), egui::Sense::hover());
                                let origin = rect.center();
                                ui.painter().add(egui::Shape::line(
                                    vec![origin + egui::vec2(-5., 0.), origin + egui::vec2(-1., 4.), origin + egui::vec2(6., -4.)],
                                    egui::Stroke::new(1.8, Color32::from_rgb(27, 112, 77)),
                                ));
                                ui.label(label);
                            });
                        }
                        let report = &prepared["bundle"]["report"];
                        if let Some(findings) = report["basicChecks"]["findings"].as_array() {
                            for finding in findings {
                                if let Some(message) = finding["message"].as_str() {
                                    ui.label(message);
                                }
                            }
                        }
                        if let Some(notes) = report["diagnostics"].as_array().filter(|n| !n.is_empty()) {
                            ui.add_space(8.);
                            ui.strong("Export notes");
                            for note in notes.iter().filter_map(Value::as_str) {
                                ui.label(note);
                            }
                        }
                        ui.add_space(8.);
                        ui.collapsing("Technical details", |ui| {
                            ui.small(format!("SHA256 {}", prepared["file"]["sha256"].as_str().unwrap_or("")));
                            ui.add(egui::Label::new(RichText::new(serde_json::to_string_pretty(report).unwrap()).monospace()).wrap());
                        });
                    } else {
                        ui.colored_label(Color32::from_rgb(164, 55, 38), RichText::new("Export needs attention").size(20.).strong());
                        ui.label("No file was saved. Resolve the issue below, then try Export again.");
                    }
                    if let Some(error) = &dialog.error {
                        ui.add_space(8.);
                        ui.colored_label(Color32::from_rgb(164, 55, 38), error);
                        if prepared.is_some() {
                            ui.label("Your validated program is still ready. Choose Save as… to try another destination.");
                        }
                    } else if !preparing && prepared.is_none() {
                        ui.label("The job changed or no validated output is available. Generate the current job before exporting.");
                    }
                    if saving {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Choose a destination in the file picker. Saving your program…");
                        });
                    }
                });
            ui.add_space(8.);
            ui.separator();
            ui.horizontal(|ui| {
                let dismiss = button(ui, if preparing { "Cancel export" } else { "Close" }, !saving);
                observe_control("Close export", dismiss.rect);
                close = dismiss.clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let response = ui.add_enabled(
                        !preparing && !saving && prepared.is_some() && self.io.is_none() && self.active.is_none(),
                        egui::Button::new("Save as…").fill(Color32::from_rgb(49, 190, 195)),
                    );
                    observe_control("Save as…", response.rect);
                    save = response.clicked();
                });
            });
        });
        observe_control("Export dialog", response.response.rect);
        if !saving && (close || response.should_close()) {
            if preparing {
                // Cancelling terminates the worker, so its retained plan expires too.
                self.cancelled_id = self.active.map(|a| a.0);
                self.port.cancel();
                self.active = None;
                self.plan = None;
                self.prepared = None;
                self.status = "Export cancelled. Generate again before exporting.".into();
            }
            self.export_dialog = None;
        } else if save {
            let dialog = self.export_dialog.as_mut().unwrap();
            dialog.error = None;
            dialog.saving = true;
            self.save_output(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exporting() -> App {
        App {
            active: Some((12, 0)),
            export_dialog: Some(ExportDialog {
                request: Some(12),
                ..Default::default()
            }),
            inspector_tab: 2,
            ..Default::default()
        }
    }

    #[test]
    fn export_errors_stay_in_the_dialog_and_late_completions_are_ignored() {
        let mut app = exporting();
        let ctx = egui::Context::default();
        app.accept(11, Err("old failure".into()), &ctx);
        assert_eq!(app.export_dialog.as_ref().unwrap().request, Some(12));
        app.accept(
            12,
            Err("POST_M6_CONTRACT: review the tool change".into()),
            &ctx,
        );
        let dialog = app.export_dialog.as_ref().unwrap();
        assert!(dialog.request.is_none());
        assert!(dialog.error.as_ref().unwrap().contains("POST_M6_CONTRACT"));
        assert!(app.prepared.is_none() && app.io.is_none());
        assert_eq!(app.inspector_tab, 2);
    }

    #[test]
    fn editing_during_export_blocks_stale_bytes_and_explains_why() {
        let mut app = exporting();
        let ctx = egui::Context::default();
        app.changed(&ctx);
        app.accept(12, Err("outdated validation".into()), &ctx);
        assert!(
            app.export_dialog
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("older edit")
        );
        assert!(app.prepared.is_none());
        app.save_output(&ctx);
        assert!(app.io.is_none());
    }

    #[test]
    fn failed_save_keeps_validated_bytes_and_success_closes_the_dialog() {
        let mut app = App {
            prepared: Some((
                json!({"file": {"filename": "sequence.ngc", "gcode": "G21\nM2\n"}}),
                0,
            )),
            export_dialog: Some(ExportDialog {
                saving: true,
                ..Default::default()
            }),
            io: Some((9, IoKind::Save(None))),
            ..Default::default()
        };
        let checked = app.prepared.clone();
        let ctx = egui::Context::default();
        app.event(
            Event::Io {
                id: 9,
                result: Err("Save cancelled; bytes retained".into()),
            },
            &ctx,
        );
        assert_eq!(app.prepared, checked);
        assert!(!app.export_dialog.as_ref().unwrap().saving);
        assert!(app.export_dialog.as_ref().unwrap().error.is_some());
        app.export_dialog.as_mut().unwrap().saving = true;
        app.io = Some((10, IoKind::Save(None)));
        app.event(
            Event::Io {
                id: 10,
                result: Ok(IoValue::Saved("Saved program".into())),
            },
            &ctx,
        );
        assert!(app.export_dialog.is_none());
        assert_eq!(app.prepared, checked);
    }
}
