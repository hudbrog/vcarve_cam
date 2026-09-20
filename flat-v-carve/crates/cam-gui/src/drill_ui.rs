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

    /// The catalogue's marker points, refreshed on the first panel frame and
    /// again whenever the document revision moves (an untouched document
    /// still sits at revision 0, which the revision counter alone cannot
    /// distinguish from "never looked"). Points come from the same import
    /// the other readings do, so the panel never re-parses anything the
    /// reference inspection did not.
    fn refresh_points(&mut self) {
        let Some(doc) = &self.document else { return };
        if self.points_seen && self.points_revision == self.revision {
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
        self.points_seen = true;
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
                app.operation_numbers(ui, ctx, &[120, 121, 122]);
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

    #[test]
    fn drill_dimensions_commit_together_and_preserve_drill_geometry() {
        let mut doc = Document::new(drill_workspace());
        let id = doc.raw.operation.clone();
        assert!(doc.edit(120, "6".into()).is_err());
        assert!(
            doc.pending(),
            "partial drill dimensions must block generation"
        );
        assert!(doc.edit(121, "25".into()).is_err());
        doc.edit(122, "135".into()).unwrap();
        assert!(!doc.pending());
        doc.edit(120, "8".into()).unwrap();
        let Some(cam_core::project::ToolGeometry::Drill(geometry)) =
            &crate::authoring::tool_in(&doc.job, &id, false)
                .unwrap()
                .geometry
        else {
            panic!("editing drill dimensions must not create an endmill");
        };
        assert_eq!(geometry.diameter_mm, 8.);
        assert_eq!(geometry.cutting_length_mm, 25.);
        assert_eq!(geometry.tip_angle_deg, 135.);
        let before = doc.job.clone();
        assert!(doc.edit(122, "180".into()).is_err());
        assert_eq!(
            doc.job, before,
            "invalid angle must preserve the last valid tool"
        );
        assert!(doc.pending());
        doc.edit(122, "118".into()).unwrap();
        assert!(!doc.pending());
    }

    #[test]
    fn drill_geometry_panel_displays_drill_dimensions_and_plunge_capability() {
        let mut doc = Document::new(drill_workspace());
        doc.edit(120, "5".into()).unwrap_err();
        doc.edit(121, "30".into()).unwrap_err();
        doc.edit(122, "118".into()).unwrap();
        let id = doc.raw.operation.clone();
        crate::authoring::tool_mut_in(&mut doc.job, &id, false)
            .unwrap()
            .capabilities
            .plunge_capable = Some(true);
        let mut app = App {
            document: Some(doc),
            inspector_tab: 2,
            operation_tab: 2,
            search: "drill".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
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
        for label in [
            "Drill bit diameter",
            "Drill bit cutting length",
            "Drill point angle",
            "Help Drill bit can plunge",
            "Plunge yes",
        ] {
            assert!(control_rect(label).is_some(), "missing {label}");
        }
        assert!(control_rect("Endmill diameter").is_none());
        assert!(control_rect("Help Endmill can plunge").is_none());
        assert!(control_rect("Ramp unset").is_none());
        let doc = app.document.as_ref().unwrap();
        assert_eq!(
            (doc.text(120), doc.text(121), doc.text(122)),
            ("5".into(), "30".into(), "118".into())
        );
    }

    #[test]
    fn viewport_marker_cache_follows_the_loaded_scene() {
        let mut app = App::default();
        let preview = crate::session::execute(
            &mut cam_service::retained::Retained::new(),
            crate::session::Command::Preview {
                job: drill_workspace().to_json().unwrap(),
            },
        );
        app.view.load_scene(preview);
        app.view.set_drill_selected(true);
        assert!(app.view.is_drill());
        assert_eq!(app.view.drill_points().len(), 2);
        let empty = crate::operation_authoring::empty_job();
        app.view.load_scene(crate::session::execute(
            &mut cam_service::retained::Retained::new(),
            crate::session::Command::Preview {
                job: empty.to_json().unwrap(),
            },
        ));
        assert!(app.view.drill_points().is_empty());
        app.view.set_drill_selected(false);
        assert!(!app.view.is_drill());
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

    /// Render the panel, click one marker's checkbox, and let the app's event
    /// loop adopt the service reply through the in-process test transport.
    #[test]
    fn clicking_a_marker_row_selects_the_point_through_the_worker() {
        let job = drill_workspace();
        let operation_id = job.operations[0].id.clone();
        let mut app = App {
            document: Some(Document::new(job)),
            inspector_tab: 2,
            operation_tab: 0,
            ..Default::default()
        };
        app.port.use_inline_compute();
        let ctx = egui::Context::default();
        let frame = |app: &mut App, events: Vec<egui::Event>| {
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
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let rect = crate::app::control_rect("Drill point artwork-1 / a-point")
            .expect("the marker row is rendered and observed");
        let center = egui::pos2((rect[0] + rect[2]) / 2., (rect[1] + rect[3]) / 2.);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        // The click submitted an artwork command; pump the app's event loop
        // until the worker reply is adopted.
        let selected = |app: &App| {
            crate::session::drill(&app.document.as_ref().unwrap().job, &operation_id)
                .map(|s| s.points.len())
                .unwrap_or(0)
        };
        let mut count = selected(&app);
        for _ in 0..200 {
            if count == 1 && app.active.is_none() {
                break;
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 900.),
                    )),
                    ..Default::default()
                },
                |ctx| app.ui(ctx),
            );
            count = selected(&app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            count, 1,
            "the clicked marker must be selected; status: {}",
            app.status
        );
    }

    #[test]
    fn drilling_preview_clicks_assign_toggle_and_undo_the_live_selection() {
        let job = drill_workspace();
        let operation = job.operations[0].id.clone();
        let mut app = App {
            document: Some(Document::new(job)),
            preview_dirty: true,
            ..Default::default()
        };
        app.port.use_inline_compute();
        let mut settings = app.view.settings();
        settings.tilt_deg = Some(0.);
        settings.yaw = 0.;
        app.view.restore_settings(&settings);
        let ctx = egui::Context::default();
        let time = std::cell::Cell::new(0.);
        let frame = |app: &mut App, events, modifiers| {
            time.set(time.get() + 0.1);
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time.get()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400., 1000.),
                    )),
                    events,
                    modifiers,
                    ..Default::default()
                },
                |ctx| app.ui(ctx),
            );
        };
        let settle = |app: &mut App| {
            for _ in 0..8 {
                frame(app, vec![], egui::Modifiers::default());
            }
            assert!(app.active.is_none(), "{}", app.status);
        };
        settle(&mut app);
        let markers = app.view.drill_points();
        assert_eq!(markers.len(), 2);
        let click = |app: &mut App, index: usize, shift: bool, edge: bool| {
            let [x0, y0, x1, y1] = control_rect("Artwork viewport").unwrap();
            let rect = egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
            let saved = app.view.settings();
            let mut camera = crate::camera::Camera {
                aspect: rect.width() / rect.height(),
                yaw: saved.yaw,
                pan: saved.pan,
                ..Default::default()
            };
            camera.set_tilt(0.);
            camera.set_zoom(saved.zoom);
            let marker = &markers[index];
            let point = crate::artwork_view::screen_point(
                camera,
                app.view.scene_bounds().unwrap(),
                rect,
                cam_core::geometry::Point::new(
                    marker.center[0] + if edge { marker.diameter_mm * 0.45 } else { 0. },
                    marker.center[1],
                ),
            );
            let modifiers = egui::Modifiers {
                shift,
                ..Default::default()
            };
            for pressed in [true, false] {
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    modifiers,
                );
            }
            settle(app);
        };
        let assert_selection = |app: &App, indices: &[usize]| {
            let expected: Vec<_> = indices
                .iter()
                .map(|i| markers[*i].reference.clone())
                .collect();
            assert_eq!(
                crate::session::drill(&app.document.as_ref().unwrap().job, &operation)
                    .unwrap()
                    .points,
                expected,
                "{}",
                app.status
            );
            assert_eq!(app.view.artwork.selected, expected);
        };
        click(&mut app, 0, false, false);
        assert_selection(&app, &[0]);
        click(&mut app, 1, true, true);
        assert_selection(&app, &[0, 1]);
        click(&mut app, 0, true, false);
        assert_selection(&app, &[1]);
        app.undo(&ctx);
        settle(&mut app);
        assert_selection(&app, &[0, 1]);
        let item = markers[0].reference.artwork_item_id.0.clone();
        app.view.artwork.locked.insert(item.clone());
        click(&mut app, 0, false, false);
        assert_selection(&app, &[0, 1]);
        app.view.artwork.locked.clear();
        app.view.artwork.hidden.insert(item);
        click(&mut app, 0, false, false);
        assert_selection(&app, &[0, 1]);
        app.view.artwork.hidden.clear();
        let doc = app.document.as_mut().unwrap();
        doc.job = crate::operation_authoring::apply(
            &doc.job,
            crate::operation_authoring::add(crate::operation_authoring::Kind::Drill, &doc.job),
        )
        .unwrap();
        let other_operation = doc.job.operations.last().unwrap().id.clone();
        assert!(doc.select_operation(&other_operation));
        settle(&mut app);
        assert!(
            app.view.artwork.selected.is_empty(),
            "changing the current operation must clear its old highlight"
        );
    }
}
