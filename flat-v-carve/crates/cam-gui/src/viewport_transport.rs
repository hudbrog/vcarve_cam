//! Presentation of the retained simulation clock; no timing or removal model.
use super::*;
use crate::{
    app::observe_control,
    ui_icons::{self, Icon},
    ui_theme as theme,
};

impl Viewport {
    pub(super) fn transport(&mut self, ui: &mut egui::Ui) {
        self.timeline_rows = 0;
        let width = ui.available_width();
        ui.horizontal(|ui| {
            let label = if self.playing { "Pause" } else { "Play" };
            let play = ui_icons::button(
                ui,
                if self.playing {
                    Icon::Pause
                } else {
                    Icon::Play
                },
                label,
                self.playing,
            );
            observe_control(label, play.rect);
            if play.clicked() {
                self.playing = !self.playing;
                if self.playing && self.stock_prefix == self.motion_count() {
                    self.stock_seek(0);
                }
            }
            let start = ui_icons::button(ui, Icon::Start, "Start", false);
            observe_control("Start", start.rect);
            if start.clicked() {
                self.playing = false;
                self.stock_seek(0);
            }
            let speed = egui::ComboBox::from_id_salt("playback-speed")
                .width(76.)
                .selected_text(if self.playback_fit {
                    "Fit playback".into()
                } else {
                    format!("{}×", self.playback_speed)
                })
                .show_ui(ui, |ui| {
                    egui::Grid::new("playback-rate-grid")
                        .num_columns(3)
                        .show(ui, |ui| {
                            for (index, (label, speed)) in [
                                ("0.5x", 0.5),
                                ("1x", 1.),
                                ("2x", 2.),
                                ("4x", 4.),
                                ("10x", 10.),
                                ("15x", 15.),
                                ("60x", 60.),
                                ("300x", 300.),
                                ("1000x", 1000.),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                let response = ui.selectable_label(
                                    !self.playback_fit
                                        && (self.playback_speed - speed).abs() < 1e-9,
                                    label,
                                );
                                observe_control(&format!("Playback {label}"), response.rect);
                                if response.clicked() {
                                    self.playback_speed = speed;
                                    self.playback_fit = false;
                                }
                                if index % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                    let fit = ui.selectable_label(self.playback_fit, "Fit playback");
                    observe_control("Playback Fit", fit.rect);
                    if fit.clicked() {
                        self.playback_fit = true;
                    }
                    ui.small(
                        "Rates scale modeled motion time. Fit plays the program in 20 seconds.",
                    );
                });
            observe_control("Playback speed", speed.response.rect);
            if width >= 360. {
                let stages = controls::settings_menu(ui, "Stages", |ui| self.stage_controls(ui));
                observe_control("Stage jumps", stages.rect);
            }
            let more =
                controls::settings_menu(ui, if width >= 360. { "Options" } else { "…" }, |ui| {
                    ui.set_width(270.);
                    if width < 360. {
                        self.stage_controls(ui);
                        ui.separator();
                    }
                    self.transport_options(ui);
                    let response = ui.checkbox(&mut self.transport_details, "Motion & diagnostics");
                    observe_control("Transport details", response.rect);
                });
            observe_control("Playback options", more.rect);
            if width >= 580. {
                if let Some((seconds, total)) = self.program_time() {
                    ui.label(format!(
                        "{} / {} modeled",
                        format_program_time(seconds),
                        format_program_time(total)
                    ));
                } else {
                    ui.label(format!(
                        "Motion {} / {}",
                        self.stock_prefix,
                        self.motion_count()
                    ));
                }
            }
        });
        self.time_track(ui);
        ui.horizontal_wrapped(|ui| {
            if width < 580. {
                if let Some((seconds, total)) = self.program_time() {
                    ui.small(format!(
                        "{} / {} modeled",
                        format_program_time(seconds),
                        format_program_time(total)
                    ));
                } else {
                    ui.small("Motion-based playback · time unavailable");
                }
            }
            if let Some(stock) = &self.stock {
                ui.small(format!(
                    "Display {:.4} mm · {} warning(s)",
                    stock.meta.cell_mm,
                    self.warnings.len()
                ));
                if let Some(clock) = stock.clock.as_ref() {
                    ui.small(format!(
                        "Rapid {}{:.0} mm/min",
                        if clock.time().assumes_rapid_rate() {
                            "assumed "
                        } else {
                            ""
                        },
                        clock.time().rapid_rate_mm_min()
                    ));
                }
            }
            if let Some(target) = self.requested_prefix {
                ui.small(format!(
                    "Seeking motion {target} · showing {}",
                    self.stock_prefix
                ));
            }
        });
        if let Ok(stats) = self.render_stats.lock()
            && stats.budget_omitted > 0
        {
            let response=ui.colored_label(theme::WARNING,format!("{} path page(s) are not drawn: display memory limit. Stock and seeking remain available.",stats.budget_omitted));
            observe_control("Path pages omitted", response.rect);
        }
        if self.transport_details {
            egui::ScrollArea::vertical()
                .id_salt("playback-details")
                .max_height(200.)
                .show(ui, |ui| {
                    self.motion_slider(ui);
                    self.transport_diagnostics(ui);
                });
        }
    }

    fn stage_controls(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.);
        let groups = self.groups.clone();
        let (first, last) = self.timeline_window(groups.len());
        if first > 0 {
            let earlier = ui.button(format!("{first} earlier stages"));
            observe_control("Earlier stages", earlier.rect);
            if earlier.clicked() {
                self.stage_window = first.saturating_sub(TIMELINE_WINDOW);
            }
        }
        for group in &groups[first..last] {
            let response = ui.button(&group.jump);
            observe_control(&group.jump, response.rect);
            self.timeline_rows += 1;
            if response.clicked() {
                self.playing = false;
                self.stock_seek(group.end);
                ui.close();
            }
        }
        if last < groups.len() {
            let later = ui.button(format!("{} later stages", groups.len() - last));
            observe_control("Later stages", later.rect);
            if later.clicked() {
                self.stage_window = last.min(groups.len().saturating_sub(1));
            }
        }
        if groups.is_empty() {
            ui.label("Generate enabled operations to see their stages.");
        }
    }

    fn transport_options(&mut self, ui: &mut egui::Ui) {
        ui.small("Displayed paths · cumulative stock is retained");
        let groups = self.groups.clone();
        let selected = groups
            .get(self.stage.wrapping_sub(1))
            .map(|g| g.label.as_str())
            .unwrap_or("All paths");
        let paths = egui::CollapsingHeader::new(format!("Paths: {selected}"))
            .id_salt("visible-path-stage")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("path-stage-list")
                    .max_height(140.)
                    .show(ui, |ui| {
                        let response = ui.selectable_value(&mut self.stage, 0, "All paths");
                        observe_control("All paths", response.rect);
                        for (index, group) in groups.iter().enumerate() {
                            let response =
                                ui.selectable_value(&mut self.stage, index + 1, &group.label);
                            observe_control(&format!("Paths {}", group.label), response.rect);
                        }
                    });
            });
        observe_control("Path stage", paths.header_response.rect);
        let current = self.display_preset();
        let combo = egui::CollapsingHeader::new(format!("Display: {}", current.label()))
            .id_salt("display-resolution")
            .show(ui, |ui| {
                for preset in crate::stock_preview::DisplayPreset::ALL {
                    let response = ui.selectable_label(
                        preset == current,
                        format!("{} · {}", preset.label(), preset.summary()),
                    );
                    observe_control(
                        &format!("Display resolution {}", preset.label()),
                        response.rect,
                    );
                    if response.clicked() {
                        self.set_display_preset(preset);
                        ui.close();
                    }
                }
            });
        observe_control("Display resolution", combo.header_response.rect);
    }

    fn motion_slider(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().slider_width = (ui.available_width() - 160.).max(40.);
        let mut prefix = self.stock_prefix;
        let response =
            ui.add(egui::Slider::new(&mut prefix, 0..=self.motion_count()).text("Stock motion"));
        observe_control("Stock motion", response.rect);
        if response.changed() {
            self.playing = false;
            self.stock_seek(prefix);
        }
    }

    fn time_track(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let timing = self.program_time();
        let (current, total) =
            timing.unwrap_or((self.stock_prefix as f64, self.motion_count() as f64));
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 24.),
            egui::Sense::click_and_drag(),
        );
        let track = rect.shrink2(egui::vec2(6., 5.));
        let x = |value: f64| {
            track.left() + (value / total.max(1e-9)).clamp(0., 1.) as f32 * track.width()
        };
        let at_prefix = |prefix: usize| {
            self.stock
                .as_ref()
                .and_then(|s| s.clock.as_ref())
                .map_or(prefix as f64, |c| c.time().seconds_at_prefix(prefix))
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(track, 2., theme::DIVIDER);
        for (index, group) in self.groups.iter().enumerate() {
            let band = egui::Rect::from_min_max(
                egui::pos2(x(at_prefix(group.start)), track.top()),
                egui::pos2(x(at_prefix(group.end)), track.bottom()),
            );
            painter.rect_filled(
                band,
                0.,
                if index % 2 == 0 {
                    theme::ACCENT
                } else {
                    theme::PRIMARY
                },
            );
            painter.vline(
                band.left(),
                track.y_range(),
                egui::Stroke::new(1., theme::SURFACE),
            );
        }
        let hovered_stage = response
            .hover_pos()
            .and_then(|point| {
                let position = ((point.x - track.left()) / track.width().max(1.)) as f64 * total;
                self.groups
                    .iter()
                    .find(|g| position >= at_prefix(g.start) && position <= at_prefix(g.end))
                    .map(|g| {
                        if timing.is_some() {
                            format!(
                                "{} · {}–{}",
                                g.label,
                                format_program_time(at_prefix(g.start)),
                                format_program_time(at_prefix(g.end))
                            )
                        } else {
                            format!("{} · motions {}–{}", g.label, g.start, g.end)
                        }
                    })
            })
            .unwrap_or_default();
        let mut marks: Vec<_> = self
            .warnings
            .iter()
            .enumerate()
            .map(|(i, w)| (i, x(at_prefix(w.motion))))
            .collect();
        marks.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut seek_warning = None;
        // Hit targets can overlap for coincident warnings; the tooltip lists
        // the cluster and the Warnings inspector exposes each exact occurrence.
        let mut previous_x = f32::NEG_INFINITY;
        for &(index, position) in &marks {
            painter.line_segment(
                [
                    egui::pos2(position, rect.top() + 1.),
                    egui::pos2(position, track.bottom()),
                ],
                egui::Stroke::new(2., theme::WARNING),
            );
            if (position - previous_x).abs() < 12. {
                continue;
            }
            previous_x = position;
            let hit = egui::Rect::from_center_size(
                egui::pos2(position, rect.center().y),
                egui::vec2(12., 24.),
            )
            .intersect(rect);
            let marker = ui.interact(
                hit,
                response.id.with(("warning", index)),
                egui::Sense::click(),
            );
            observe_control(&format!("Warning marker {index}"), marker.rect);
            let nearby = marks
                .iter()
                .filter(|(_, p)| (*p - position).abs() < 12.)
                .map(|(i, _)| {
                    let w = &self.warnings[*i];
                    format!(
                        "{} · motion {} · worst {:.2} mm",
                        w.kind.label(),
                        w.motion,
                        w.max_depth_mm
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            marker.clone().on_hover_text(format!("{nearby}\nClick to seek the first occurrence here; Warnings lists every occurrence."));
            if marker.clicked() {
                seek_warning = Some(self.warnings[index].motion);
            }
        }
        painter.vline(
            x(current),
            rect.y_range(),
            egui::Stroke::new(2., theme::TEXT),
        );
        let name = if timing.is_some() {
            "Program time"
        } else {
            "Stock motion"
        };
        observe_control(name, response.rect);
        observe_control("Simulation timeline", response.rect);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), name)
        });
        let mut target = None;
        if response.clicked()
            || (response.dragged() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO))
        {
            response.request_focus();
            if let Some(pointer) = response.interact_pointer_pos() {
                target = Some(
                    ((pointer.x - track.left()) / track.width().max(1.)).clamp(0., 1.) as f64
                        * total,
                );
            }
        }
        if response.has_focus() {
            ui.input(|input| {
                let step = if timing.is_some() { total / 100. } else { 1. };
                if input.key_pressed(egui::Key::ArrowLeft) {
                    target = Some((current - step).max(0.));
                }
                if input.key_pressed(egui::Key::ArrowRight) {
                    target = Some((current + step).min(total));
                }
                if input.key_pressed(egui::Key::Home) {
                    target = Some(0.);
                }
                if input.key_pressed(egui::Key::End) {
                    target = Some(total);
                }
            });
        }
        if let Some(motion) = seek_warning {
            self.playing = false;
            self.seek_motion(motion);
        } else if let Some(target) = target {
            self.playing = false;
            if timing.is_some() {
                self.seek_seconds(target);
            } else {
                self.stock_seek(target.round() as usize);
            }
        }
        response.on_hover_text(format!("{hovered_stage}\nModeled stage durations · drag to seek · arrows step, Home/End jump. Very short stages remain available in Stages."));
        rect
    }

    pub(super) fn warning_controls(&mut self, ui: &mut egui::Ui) {
        if self.stock.is_none() {
            ui.label("Generate enabled operations to inspect display warnings.");
            return;
        }
        ui.small(format!(
            "Display estimates · {:.2} mm check raster",
            self.warning_cell_mm
        ));
        ui.small("These checks do not authorize output. Unstated assemblies are not modeled.");
        if self.warnings.is_empty() {
            ui.label("No display warnings were reported.");
            return;
        }
        let warnings = self.warnings.clone();
        for (index, warning) in warnings.iter().enumerate() {
            ui.separator();
            let label = ui.colored_label(theme::WARNING, warning.kind.label());
            observe_control("Machine warning", label.rect);
            let when = self
                .stock
                .as_ref()
                .and_then(|s| s.clock.as_ref())
                .map(|c| format_program_time(c.time().seconds_at_prefix(warning.motion)));
            ui.label(format!(
                "Tool {} · first at motion {}{}",
                self.tool_ids
                    .get(warning.tool)
                    .map(String::as_str)
                    .unwrap_or("unspecified"),
                warning.motion,
                when.map(|t| format!(" ({t})")).unwrap_or_default()
            ));
            ui.small(format!(
                "{:.2} mm at first occurrence · worst {:.2} mm",
                warning.depth_mm, warning.max_depth_mm
            ));
            let show = ui.button("Show");
            observe_control(&format!("Show warning {index}"), show.rect);
            if index == 0 {
                observe_control("Show warning", show.rect);
            }
            if show.clicked() {
                self.playing = false;
                self.seek_motion(warning.motion);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Viewport {
        let (meta, payload) = crate::session::run(crate::session::Command::generate(include_str!(
            "../../../fixtures/gui4/lettering.job.json"
        )))
        .unwrap();
        let mut view = Viewport::default();
        view.set_scene(Scene {
            meta,
            payload: Arc::new(payload),
        });
        view
    }

    fn frame(view: &mut Viewport, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 200.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = view.time_track(ui);
                });
            },
        );
        rect
    }

    fn click(view: &mut Viewport, ctx: &egui::Context, point: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                view,
                ctx,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    fn settle(view: &mut Viewport) {
        if let Some(prefix) = view.take_stock_request() {
            let handle = view.scene.as_ref().unwrap().meta.report["gui2"]["handle"]
                .as_str()
                .unwrap()
                .to_owned();
            let (meta, payload) =
                crate::session::run(crate::session::Command::Seek { handle, prefix }).unwrap();
            view.accept_stock(meta, payload).unwrap();
            view.finish_stock_request();
        }
    }

    #[test]
    fn timeline_pointer_keyboard_and_warning_seek_the_existing_clock() {
        let ctx = egui::Context::default();
        crate::ui_theme::apply(&ctx);
        let mut view = view();
        let rect = frame(&mut view, &ctx, vec![]).shrink2(egui::vec2(6., 5.));
        frame(&mut view, &ctx, vec![]);
        let total = view.program_time().unwrap().1;
        let point = |fraction| egui::pos2(rect.left() + rect.width() * fraction, rect.center().y);
        click(&mut view, &ctx, point(0.73));
        settle(&mut view);
        assert!((view.program_time().unwrap().0 - total * 0.73).abs() < 0.001);
        click(&mut view, &ctx, point(0.21));
        settle(&mut view);
        assert!((view.program_time().unwrap().0 - total * 0.21).abs() < 0.001);
        frame(
            &mut view,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::End,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        settle(&mut view);
        assert!((view.program_time().unwrap().0 - total).abs() < 0.001);
        let motion = view.motion_count() / 3;
        let when = view
            .stock
            .as_ref()
            .unwrap()
            .clock
            .as_ref()
            .unwrap()
            .time()
            .seconds_at_prefix(motion);
        view.warnings = Arc::new(vec![crate::sim_checks::Warning {
            kind: crate::sim_checks::WarningKind::RapidThroughMaterial,
            motion,
            depth_mm: 1.,
            max_depth_mm: 2.,
            tool: 0,
        }]);
        frame(&mut view, &ctx, vec![]);
        click(&mut view, &ctx, point((when / total) as f32));
        settle(&mut view);
        assert!((view.program_time().unwrap().0 - when).abs() < 0.001);
        assert!(!view.playing);

        view.stock.as_mut().unwrap().clock = None;
        view.warnings = Arc::new(vec![]);
        frame(&mut view, &ctx, vec![]);
        click(&mut view, &ctx, point(0.6));
        assert_eq!(
            view.requested_prefix,
            Some((view.motion_count() as f64 * 0.6).round() as usize)
        );
    }

    #[test]
    fn a_seek_reply_preserves_the_latest_queued_fraction() {
        let mut view = view();
        let total = view.program_time().unwrap().1;
        view.seek_seconds(total * 0.8);
        settle(&mut view);
        view.seek_seconds(total * 0.2);
        let prefix = view
            .take_stock_request()
            .expect("backward seek needs restore");
        let handle = view.scene.as_ref().unwrap().meta.report["gui2"]["handle"]
            .as_str()
            .unwrap()
            .to_owned();
        view.seek_seconds(total * 0.6);
        view.seek_seconds(total * 0.35);
        let latest = view.requested_stock;
        let (meta, payload) =
            crate::session::run(crate::session::Command::Seek { handle, prefix }).unwrap();
        view.accept_stock(meta, payload).unwrap();
        view.finish_stock_request();
        assert!((view.program_time().unwrap().0 - total * 0.2).abs() < 0.001);
        assert_eq!(view.requested_prefix, latest);
        assert!(view.stock_request_pending());
        settle(&mut view);
        assert!((view.program_time().unwrap().0 - total * 0.35).abs() < 0.001);
        assert_eq!(view.requested_prefix, None);

        // A local target can replace a backward seek before it is submitted.
        view.seek_seconds(total * 0.1);
        assert!(view.stock_request_pending());
        view.seek_seconds(total * 0.36);
        settle(&mut view);
        assert!((view.program_time().unwrap().0 - total * 0.36).abs() < 0.001);
    }

    #[test]
    fn collapsed_transport_leaves_room_for_the_canvas() {
        let mut view = view();
        for width in [296., 688., 1000.] {
            let ctx = egui::Context::default();
            crate::ui_theme::apply(&ctx);
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 800.),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        let panel = egui::TopBottomPanel::bottom("transport-test")
                            .show(ctx, |ui| view.transport(ui));
                        assert!(panel.response.rect.width() <= width + 1.);
                        assert!(
                            panel.response.rect.height() <= if width < 400. { 135. } else { 100. },
                            "transport at width {width}: {:?}",
                            panel.response.rect
                        );
                    },
                );
            }
        }
    }
}

impl Viewport {
    fn transport_diagnostics(&mut self, ui: &mut egui::Ui) {
        if let Some(stock) = &self.stock {
            ui.label(format!("Display simulation · {:.4} mm cells (reference {:.4}) · {} / {} motions · {:.2} mm³ removed",stock.meta.cell_mm,stock.meta.reference_cell_mm,stock.prefix,self.motion_count(),stock.stats.removed_volume_mm3));
            // Machine time of the displayed position. The ratio between a
            // long pass and a short plunge is the plan's own ratio of length
            // to feed; a machine that states no rapid rate says so instead
            // of presenting the display's fallback as machine truth.
            if let Some(clock) = stock.clock.as_ref() {
                let (prefix, fraction) = clock.position();
                let in_move = if fraction > 0. {
                    format!(" · {:.0}% into motion {prefix}", fraction * 100.)
                } else {
                    String::new()
                };
                let feed = clock
                    .tip()
                    .and_then(|(_, motion)| motion.feed_mm_min)
                    .map(|feed| format!(" · feed {feed:.0} mm/min"))
                    .unwrap_or_default();
                let assumed = if clock.time().assumes_rapid_rate() {
                    format!(
                        " · rapids timed at {:.0} mm/min (the machine states no rapid rate)",
                        clock.time().rapid_rate_mm_min()
                    )
                } else {
                    String::new()
                };
                // Which tool and stage the clock is in, so the animation is
                // read against the operation it belongs to rather than the
                // job as a whole.
                let current = clock
                    .motion()
                    .and_then(|motion| self.stages.get(motion.stage as usize))
                    .map(|stage| {
                        let role = if stage.role.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", stage.role)
                        };
                        format!(" · tool {}{role}", stage.tool_id)
                    })
                    .unwrap_or_default();
                ui.label(format!(
                    "Program time · {} / {} modeled motion{in_move}{feed}{current}{assumed}",
                    format_program_time(clock.seconds()),
                    format_program_time(clock.total_seconds())
                ));
            } else if self.motion_count() > 0 {
                ui.label(
                        "Program time · unavailable (a feed motion carries no feed rate); playback steps whole motions.",
                    );
            }
            if let Some(transport) = self.scene.as_ref().map(|scene| &scene.meta.transport) {
                let (bytes, replayed) = self.last_stock_transfer();
                let checkpoints = stock.meta.ladder_frames.max(transport.stock_checkpoints);
                ui.label(format!("Display transfer · scene {:.1} MiB · {} motion pages · {} checkpoints ({:.1} MiB retained) · last stock update {:.0} KiB after replaying {} motions",transport.payload_bytes as f64/1048576.,transport.motion_pages,checkpoints,stock.meta.retained_bytes as f64/1048576.,bytes as f64/1024.,replayed));
            }
            // A path page the resident budget refused is not drawn. Say so
            // instead of letting a partial scene look complete.
            if let Ok(stats) = self.render_stats.lock()
                && stats.budget_omitted > 0
            {
                let omitted = stats.budget_omitted;
                let response = ui.colored_label(
                        Color32::from_rgb(164, 83, 12),
                        format!(
                            "{omitted} requested path page(s) are outside the resident budget and are not drawn; the stock field and the timeline are unaffected."
                        ),
                    );
                crate::app::observe_control("Path pages omitted", response.rect);
            }
            if let Some(target) = self.requested_prefix {
                ui.label(format!(
                    "Requested stock motion {target} of {} — still showing motion {}.",
                    self.motion_count(),
                    self.stock_prefix
                ));
            }
            if stock.meta.dropped_stage_marks > 0 {
                let response = ui.colored_label(Color32::from_rgb(164,83,12), format!("{} stage boundaries are beyond the display checkpoint budget; the timeline still seeks them by replaying from the nearest earlier checkpoint.", stock.meta.dropped_stage_marks));
                crate::app::observe_control("Dropped stage boundaries", response.rect);
            }
        }
    }
}
