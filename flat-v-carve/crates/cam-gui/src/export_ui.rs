use super::*;

/// Feed moves this short are inside the machine's noise floor: it cannot reach
/// the programmed feed and stop again within one. The report calls them out by
/// name, and the charts colour them.
const MICRO_MOVE_MM: f64 = 0.05;

/// One bar of a motion-length histogram, in the form the painter needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HistogramBar {
    /// Upper bound of the bucket in mm; `None` is the open-ended last bucket.
    pub upper_mm: Option<f64>,
    pub motions: usize,
    /// Fraction of the plot height, 0..=1. Square-root scaled: the tail of
    /// tiny moves is the point of these charts, and a linear plot of a few
    /// dominant buckets would hide exactly that.
    pub height: f32,
}

/// Read one published histogram into drawable bars.
pub(super) fn histogram_bars(histogram: &Value) -> Vec<HistogramBar> {
    let buckets: Vec<(Option<f64>, usize)> = histogram
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    (
                        row["upperMm"].as_f64(),
                        row["motions"].as_u64().unwrap_or(0) as usize,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let peak = buckets
        .iter()
        .map(|(_, motions)| *motions)
        .max()
        .unwrap_or(0);
    buckets
        .into_iter()
        .map(|(upper_mm, motions)| HistogramBar {
            upper_mm,
            motions,
            height: if peak == 0 {
                0.
            } else {
                (motions as f32 / peak as f32).sqrt()
            },
        })
        .collect()
}

fn bucket_label(upper_mm: Option<f64>) -> String {
    match upper_mm {
        Some(value) => format!("{value}"),
        None => ">".into(),
    }
}

/// The motion profile the export actually measured, drawn per stage with the
/// whole program last: how many moves there are, how long they are, and how
/// many the machine cannot execute at feed.
fn motion_histograms(ui: &mut egui::Ui, report: &Value) {
    let Some(profile) = report.get("motionProfile") else {
        return;
    };
    ui.add_space(8.);
    ui.strong("Motion profile");
    let requested = profile["decimalPlacesRequested"].as_u64().unwrap_or(0);
    let written = profile["decimalPlacesWritten"].as_u64().unwrap_or(0);
    if written > requested {
        ui.colored_label(
            Color32::from_rgb(164, 55, 38),
            format!(
                "Written at {written} decimal places: the machine asked for {requested}. Moves finer than its own grid forced it."
            ),
        );
    }
    let stages = profile["stages"].as_array().cloned().unwrap_or_default();
    for stage in &stages {
        draw_motion_histogram(
            ui,
            stage["stageId"].as_str().unwrap_or("stage"),
            &stage["profile"],
        );
    }
    if let Some(total) = profile.get("total") {
        draw_motion_histogram(ui, "whole program", total);
    }
}

/// One caption plus its bar chart.
fn draw_motion_histogram(ui: &mut egui::Ui, label: &str, profile: &Value) {
    let bars = histogram_bars(&profile["histogram"]);
    if bars.is_empty() {
        return;
    }
    let moves = profile["motions"].as_u64().unwrap_or(0);
    let arcs = profile["arcFeedMotions"].as_u64().unwrap_or(0);
    let median = profile["medianFeedMm"].as_f64().unwrap_or(0.);
    // Counted from the bars the chart draws, so the caption and the picture
    // can never disagree.
    let total: usize = bars.iter().map(|bar| bar.motions).sum();
    let micro: usize = bars
        .iter()
        .filter(|bar| bar.upper_mm.is_some_and(|upper| upper <= MICRO_MOVE_MM))
        .map(|bar| bar.motions)
        .sum();
    let share = if total == 0 {
        0.
    } else {
        micro as f64 / total as f64
    };
    ui.add_space(4.);
    ui.small(format!(
        "{label} · {moves} moves ({arcs} arcs) · median {median:.4} mm · {:.0}% under {MICRO_MOVE_MM} mm",
        share * 100.
    ));
    let width = ui.available_width().min(460.);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 74.), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(246, 248, 250));
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4., rect.top() + 4.),
        egui::pos2(rect.right() - 4., rect.bottom() - 16.),
    );
    let slot = plot.width() / bars.len() as f32;
    let bar_width = (slot * 0.66).max(2.);
    // Buckets at or below the micro-move limit are the ones the machine
    // struggles with; the rest are healthy.
    let micro = Color32::from_rgb(196, 122, 46);
    let healthy = Color32::from_rgb(49, 190, 195);
    for (index, bar) in bars.iter().enumerate() {
        let centre = plot.left() + slot * (index as f32 + 0.5);
        let height = bar.height * plot.height();
        let top = plot.bottom() - height;
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(centre - bar_width / 2., top),
            egui::pos2(centre + bar_width / 2., plot.bottom()),
        );
        let small = bar.upper_mm.is_some_and(|upper| upper <= MICRO_MOVE_MM);
        painter.rect_filled(bar_rect, 1.0, if small { micro } else { healthy });
        if bar.motions > 0 {
            painter.text(
                egui::pos2(centre, top - 1.),
                egui::Align2::CENTER_BOTTOM,
                bar.motions.to_string(),
                egui::FontId::proportional(9.),
                Color32::from_rgb(70, 76, 84),
            );
        }
        painter.text(
            egui::pos2(centre, plot.bottom() + 2.),
            egui::Align2::CENTER_TOP,
            bucket_label(bar.upper_mm),
            egui::FontId::proportional(9.),
            Color32::from_rgb(110, 116, 124),
        );
    }
    // Where the noise floor sits, so the coloured tail is readable as a
    // threshold rather than as a decoration.
    if let Some(index) = bars
        .iter()
        .rposition(|bar| bar.upper_mm.is_some_and(|upper| upper <= MICRO_MOVE_MM))
    {
        let x = plot.left() + slot * (index as f32 + 1.);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0, micro),
        );
    }
    painter.text(
        egui::pos2(plot.right(), plot.top() + 1.),
        egui::Align2::RIGHT_TOP,
        format!("{MICRO_MOVE_MM} mm limit"),
        egui::FontId::proportional(9.),
        Color32::from_rgb(150, 156, 164),
    );
}

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
                            motion_histograms(ui, report);
                            ui.add_space(8.);
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
                self.plan_scope = None;
                self.prepared = None;
                self.view.cancel_stock_requests();
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
    fn the_histogram_chart_paints_bars_for_a_published_profile() {
        let buckets = json!([
            {"upperMm": 0.002, "motions": 0},
            {"upperMm": 0.005, "motions": 4},
            {"upperMm": 0.01, "motions": 30},
            {"upperMm": 0.02, "motions": 120},
            {"upperMm": 0.05, "motions": 210},
            {"upperMm": 0.1, "motions": 380},
            {"upperMm": 0.2, "motions": 620},
            {"upperMm": 0.5, "motions": 900},
            {"upperMm": 1., "motions": 480},
            {"upperMm": 2., "motions": 120},
            {"upperMm": 5., "motions": 25},
            {"upperMm": null, "motions": 2}
        ]);
        let profile = json!({
            "motions": 2893,
            "arcFeedMotions": 1626,
            "medianFeedMm": 0.8466,
            "feedLengthMm": 2544.,
            "histogram": buckets,
        });
        let report = json!({
            "motionProfile": {
                "decimalPlacesRequested": 3,
                "decimalPlacesWritten": 4,
                "stages": [{"stageId": "carving-1-vcarve-finish", "profile": profile}],
                "total": profile,
            }
        });
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700., 500.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| motion_histograms(ui, &report));
            },
        );
        // The stage chart and the whole-program chart, twelve buckets each.
        let bars: Vec<&egui::epaint::RectShape> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.rect.width() < 40. && rect.rect.width() > 1. => {
                    Some(rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 24, "one bar per bucket per chart");
        // The micro-move buckets carry the warning colour: five per chart.
        let micro = Color32::from_rgb(196, 122, 46);
        let warm = bars.iter().filter(|bar| bar.fill == micro).count();
        assert_eq!(warm, 10, "every bucket at or under the limit is flagged");
        // The tallest bar is the bucket with the most motions, and the empty
        // bucket draws nothing.
        let tallest = bars
            .iter()
            .map(|bar| bar.rect.height())
            .fold(0.0_f32, f32::max);
        assert!(
            tallest > 20.,
            "the busiest bucket fills the plot: {tallest}"
        );
        assert_eq!(
            bars.iter().filter(|bar| bar.rect.height() == 0.).count(),
            2,
            "the empty bucket draws no bar in either chart"
        );
        // Captions and bucket labels are painted as text.
        let texts = output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::Text(_)))
            .count();
        assert!(texts >= 24, "bucket labels and captions: {texts}");
    }

    #[test]
    fn histogram_bars_scale_the_tail_into_visibility() {
        // A realistic export profile: one dominant bucket and a long tail of
        // micro-moves, which is what the chart has to keep visible.
        let histogram = json!([
            {"upperMm": 0.002, "motions": 5},
            {"upperMm": 0.005, "motions": 40},
            {"upperMm": 0.01, "motions": 900},
            {"upperMm": 0.02, "motions": 1200},
            {"upperMm": 0.05, "motions": 300},
            {"upperMm": 0.1, "motions": 0},
            {"upperMm": 0.2, "motions": 0},
            {"upperMm": 0.5, "motions": 0},
            {"upperMm": 1., "motions": 12},
            {"upperMm": 2., "motions": 3},
            {"upperMm": 5., "motions": 1},
            {"upperMm": null, "motions": 0}
        ]);
        let bars = histogram_bars(&histogram);
        assert_eq!(bars.len(), 12);
        assert_eq!(bars[3].upper_mm, Some(0.02));
        assert_eq!(bars[3].motions, 1200);
        // The tallest bucket fills the plot, the smallest non-zero one is
        // still a visible fraction of it (linear scaling would draw it at 0.4%
        // of the height), and an empty bucket draws nothing.
        assert!((bars[3].height - 1.).abs() < 1e-6);
        assert!(bars[0].height > 0.05, "{:?}", bars[0]);
        assert_eq!(bars[5].height, 0.);
        assert_eq!(bars[11].upper_mm, None);
        assert_eq!(bucket_label(None), ">");
        // An empty or missing profile draws no bars at all.
        assert!(histogram_bars(&json!([])).is_empty());
        assert!(histogram_bars(&Value::Null).is_empty());
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
