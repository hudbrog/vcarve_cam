//! Sketch workspace routing and the single Apply transaction.
use super::*;
use crate::sketch::{Editor, SavedDraft, Tool};
use cam_core::project::v5::{
    ArtworkContent, ArtworkItem, ArtworkItemId, SvgInterpretation, artwork::materialize_item,
    commands,
};

impl App {
    pub(super) fn begin_sketch(&mut self, target: Option<ArtworkItemId>, ctx: &egui::Context) {
        if self.active.is_some() || self.io.is_some() || self.sketch.is_some() {
            return;
        }
        let had_document = self.document.is_some();
        let (name, model, source_revision) = if let Some(id) = &target {
            let Some(item) = self
                .document
                .as_ref()
                .and_then(|d| d.job.artwork.iter().find(|i| &i.id == id))
            else {
                return;
            };
            let ArtworkContent::Sketch(model) = &item.content else {
                return;
            };
            (
                item.name.clone(),
                model.clone(),
                item.authored_revision().ok(),
            )
        } else {
            (
                "Sketch".into(),
                cam_core::sketch::SketchDocument::default(),
                None,
            )
        };
        if self.document.is_none() {
            self.document = Some(Document::new(crate::operation_authoring::empty_job()));
        }
        self.sketch = Some(Editor::new(SavedDraft {
            target,
            source_revision,
            name,
            model,
            had_document,
            constraint_form: Default::default(),
        }));
        if let Some(editor) = self.sketch.as_mut() {
            let panels: (bool, bool) = ctx
                .data(|d| d.get_temp(egui::Id::new("sketch-panel-visibility")))
                .unwrap_or_default();
            editor.show_geometry = panels.0;
            editor.show_constraints = panels.1;
        }
        self.close_resource();
        self.view.pause();
        self.edit_group = None;
        self.recovery.changed(ctx.input(|i| i.time));
    }
    fn cancel_sketch(&mut self, ctx: &egui::Context) {
        if let Some(editor) = self.sketch.take()
            && !editor.saved.had_document
        {
            self.document = None;
        }
        self.status = "Sketch draft cancelled.".into();
        self.recovery.changed(ctx.input(|i| i.time));
    }
    pub(super) fn apply_sketch(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(editor) = &mut self.sketch {
            editor.poll_solver(ctx);
        }
        let Some(editor) = &self.sketch else {
            return Ok(());
        };
        if editor.pending() || editor.solving() {
            return Err(
                "Finish the drawing and wait for the solve, or cancel the gesture with Esc, before applying."
                    .into(),
            );
        }
        editor.saved.validate()?;
        if editor.saved.constraint_form.changed {
            return Err("Add or Update the constraint first, or use Clear operands to discard the form edit.".into());
        }
        if editor.saved.name.trim().is_empty() {
            return Err("Give the sketch a name.".into());
        }
        let doc = self.document.as_ref().ok_or("No job is open")?;
        let (item, outcome, report) = if let Some(id) = &editor.saved.target {
            let original = doc
                .job
                .artwork
                .iter()
                .find(|i| &i.id == id)
                .ok_or("The sketch artwork was removed")?;
            if Some(original.authored_revision().map_err(|e| e.to_string())?)
                != editor.saved.source_revision
            {
                return Err("The artwork changed after this draft was opened. Cancel and reopen the sketch.".into());
            }
            let mut candidate = original.clone();
            candidate.content = ArtworkContent::Sketch(editor.saved.model.clone());
            materialize_item(&candidate, None).map_err(|e| e.to_string())?;
            let applied =
                commands::apply_sketch_with_report(&doc.job, id, editor.saved.model.clone())
                    .map_err(|e| e.to_string())?;
            (id.clone(), applied.outcome, Some(applied.report))
        } else {
            let candidate = ArtworkItem {
                id: ArtworkItemId("draft".into()),
                name: editor.saved.name.trim().into(),
                content: ArtworkContent::Sketch(editor.saved.model.clone()),
                import_settings: SvgInterpretation::default(),
                placement: Default::default(),
            };
            materialize_item(&candidate, None).map_err(|e| e.to_string())?;
            let outcome = commands::add_sketch(
                &doc.job,
                candidate.name,
                editor.saved.model.clone(),
                candidate.import_settings,
                candidate.placement,
            )
            .map_err(|e| e.to_string())?;
            (
                outcome
                    .job
                    .artwork
                    .last()
                    .ok_or("Sketch was not added")?
                    .id
                    .clone(),
                outcome,
                None,
            )
        };
        if outcome.job == doc.job {
            self.sketch = None;
            self.recovery.changed(ctx.input(|i| i.time));
            self.status = "Sketch closed; no geometry changes.".into();
            return Ok(());
        }
        self.remember();
        let doc = self.document.as_mut().unwrap();
        doc.job = outcome.job;
        doc.select_artwork(&item.0);
        self.sketch = None;
        self.edit_group = None;
        self.changed(ctx);
        self.issues = outcome.issues;
        self.sketch_impact = report;
        self.navigate(0);
        self.simulate = false;
        self.status = match &self.sketch_impact {
            Some(report) => format!(
                "Sketch applied · {} selections/anchors preserved · {} to repair. {}",
                report.preserved_count(),
                report.repair_count(),
                if report.geometry_changed {
                    "Generate to update cutting paths."
                } else {
                    "Published geometry is unchanged."
                },
            ),
            None => {
                "Sketch added. Select its geometry in an operation, then generate cutting paths."
                    .into()
            }
        };
        Ok(())
    }

    pub(super) fn export_artwork_svg(&mut self, item: &ArtworkItem, ctx: &egui::Context) {
        match cam_core::project::v5::artwork::export_svg(item) {
            Ok(svg) => {
                // A derived export must never mark the portable job as saved.
                self.save(format!("{}.svg", item.id.0), svg.into_bytes(), None, ctx);
            }
            Err(e) => self.status = format!("SVG export failed: {e}"),
        }
    }

    pub(super) fn sketch_impact_panel(
        &mut self,
        ui: &mut egui::Ui,
        item: &ArtworkItemId,
        ctx: &egui::Context,
    ) {
        let Some(report) = self
            .sketch_impact
            .clone()
            .filter(|r| &r.artwork_item_id == item)
        else {
            return;
        };
        ui.separator();
        ui.strong("Sketch applied");
        let response = ui.label(format!(
            "Selections / anchors: {} preserved · {} to repair",
            report.preserved_count(),
            report.repair_count()
        ));
        observe_control("Sketch apply report", response.rect);
        if report.geometry_changed {
            ui.small("Generate again before exporting cutting paths.");
        }
        for (i, binding) in report
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.change.needs_repair())
        {
            let owner = self
                .document
                .as_ref()
                .and_then(|d| {
                    d.job
                        .operations
                        .iter()
                        .find(|o| o.id == binding.operation_id)
                })
                .map(|o| o.name.as_str())
                .unwrap_or(&binding.operation_id);
            ui.colored_label(
                crate::ui_theme::WARNING,
                format!(
                    "{owner} · {}: {}",
                    binding.local_id,
                    binding.change.description()
                ),
            );
            let label = if binding.field_path.ends_with(".start") {
                "Repair start"
            } else if binding.field_path.contains(".tabs.placement.anchors[") {
                "Repair tab"
            } else {
                "Repair selection"
            };
            let repair = ui
                .push_id(&binding.field_path, |ui| {
                    button(ui, label, self.active.is_none())
                })
                .inner;
            observe_control(&format!("Repair sketch binding {}", i + 1), repair.rect);
            if repair.clicked() {
                self.reveal_issue(
                    &cam_core::operations::LocatedDiagnostic {
                        code: "SKETCH_BINDING_REPAIR".into(),
                        message: binding.change.description().into(),
                        operation_id: Some(binding.operation_id.clone()),
                        tool_id: None,
                        field_path: Some(binding.field_path.clone()),
                    },
                    ctx,
                );
            }
        }
        if !report.outputs.is_empty() {
            ui.collapsing("Changed sketch outputs", |ui| {
                for output in &report.outputs {
                    let change = match output.change {
                        commands::OutputChange::Added => "added",
                        commands::OutputChange::Removed => "removed",
                        commands::OutputChange::TopologyChanged => "topology changed",
                    };
                    ui.small(format!("{} · {change}", output.local_id));
                }
            });
        }
    }
    pub(super) fn sketch_workspace(&mut self, ctx: &egui::Context) {
        let mut apply = false;
        let mut cancel = false;
        let editor = self.sketch.as_mut().unwrap();
        editor.poll_solver(ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            editor.message =
                "Apply or Cancel the sketch before closing the app. The draft remains open.".into();
        }
        if !ctx.input(|i| i.raw.dropped_files.is_empty())
            || ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
            })
        {
            editor.message="Apply or Cancel the sketch before opening or saving a job. The draft remains open.".into();
        }
        if !self.ime && !ctx.wants_keyboard_input() {
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                ) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
            }) {
                editor.redo();
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                editor.undo();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                editor.cancel_gesture();
                editor.tool = Tool::Select;
                editor.shortcut_prefix_clear();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                editor.cancel_gesture();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
                && let Err(e) = editor.delete()
            {
                editor.message = e;
            }
        }
        if !self.ime {
            editor.shortcuts(ctx);
        } else {
            editor.shortcut_prefix_clear();
        }
        egui::TopBottomPanel::top("sketch-header").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Sketch");
                if editor.saved.target.is_none() {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut editor.saved.name)
                            .desired_width(150.)
                            .char_limit(1000),
                    );
                    observe_control("Sketch name", r.rect);
                } else {
                    ui.add_sized(
                        [180., ui.spacing().interact_size.y],
                        egui::Label::new(RichText::new(&editor.saved.name).strong()).truncate(),
                    )
                    .on_hover_text(&editor.saved.name);
                }
                ui.label("Local draft · mm");
                apply =
                    button(ui, "Apply sketch", !editor.pending() && !editor.solving()).clicked();
                cancel = button(ui, "Cancel sketch", true).clicked();
                let (undo, redo) = editor.history_counts();
                if button(ui, "Sketch undo", undo > 0 || editor.pending()).clicked() {
                    editor.undo();
                }
                if button(ui, "Sketch redo", redo > 0).clicked() {
                    editor.redo();
                }
            });
            ui.horizontal_wrapped(|ui| {
                let r = ui.checkbox(&mut editor.snapping, "Snap");
                observe_control("Sketch snap", r.rect);
                let r = ui.checkbox(&mut editor.grid_snap, "Grid snap");
                observe_control("Sketch grid snap", r.rect);
                ui.add(
                    egui::DragValue::new(&mut editor.grid_mm)
                        .range(0.001..=10000.)
                        .speed(0.1)
                        .suffix(" mm grid"),
                );
                if button(ui, "Fit sketch", true).clicked() {
                    editor.fit();
                }
                if button(
                    ui,
                    "Finish wire",
                    editor.tool == Tool::Polyline && editor.pending(),
                )
                .clicked()
                {
                    editor.cancel_gesture();
                }
                let r = ui.selectable_label(editor.show_geometry, "Geometry list");
                observe_control("Geometry list", r.rect);
                if r.clicked() {
                    editor.show_geometry = !editor.show_geometry;
                }
                let r = ui.selectable_label(editor.show_constraints, "Constraints & dimensions");
                observe_control("Constraints & dimensions", r.rect);
                if r.clicked() {
                    editor.show_constraints = !editor.show_constraints;
                }
            });
        });
        egui::TopBottomPanel::bottom("sketch-footer").show(ctx,|ui| {
            if !editor.prefix_hint().is_empty() { ui.label(editor.prefix_hint()); }
            else { ui.small(editor.tool.hint()); }
            ui.small(editor.solver_status());
            ui.small(format!("{} entities · {} wires · {} selected · {} saved constraints · Snaps do not create constraints",editor.saved.model.entities.len(),editor.saved.model.wires.len(),editor.selected.len(),editor.saved.model.constraints.len()));
            if let Some(scale)=editor.saved.target.as_ref().and_then(|id|self.document.as_ref()?.job.artwork.iter().find(|i| &i.id==id).map(|i|i.placement.scale)) && scale != 1. {
                ui.colored_label(crate::ui_theme::WARNING,format!("Artwork placement scale: {scale}×. Dimensions here are local millimetres."));
            }
            if !editor.message.is_empty() {ui.colored_label(crate::ui_theme::ERROR,&editor.message);}
            if self.recovery.failed {ui.colored_label(crate::ui_theme::ERROR,&self.recovery.status);}
            if self.recovery.offered.is_some() {
                ui.colored_label(crate::ui_theme::WARNING,"An earlier recovery draft is waiting. Cancel this sketch to restore or discard it; automatic recovery is paused.");
            }
        });
        egui::SidePanel::left("sketch-tools")
            .default_width(245.)
            .min_width(165.)
            .max_width((ctx.content_rect().width() * 0.3).max(165.))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sketch-tools-scroll")
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: false,
                        ..Default::default()
                    })
                    .show(ui, |ui| editor.tools_panel(ui));
            });
        if editor.show_constraints || editor.show_geometry {
            egui::SidePanel::right("sketch-lists")
                .default_width(290.)
                .min_width(200.)
                .max_width((ctx.content_rect().width() * 0.38).max(200.))
                .show(ctx, |ui| {
                    if editor.show_geometry {
                        egui::ScrollArea::vertical()
                            .id_salt("sketch-geometry-scroll")
                            .max_height(if editor.show_constraints {
                                ui.available_height() * 0.3
                            } else {
                                ui.available_height()
                            })
                            .show(ui, |ui| editor.geometry_panel(ui));
                    }
                    if editor.show_constraints {
                        egui::ScrollArea::vertical()
                            .id_salt("sketch-constraints-scroll")
                            .show(ui, |ui| editor.constraint_panel(ui));
                    }
                });
        }
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("sketch-panel-visibility"),
                (editor.show_geometry, editor.show_constraints),
            )
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| editor.canvas(ui));
        editor.poll_solver(ctx);
        if cancel {
            self.cancel_sketch(ctx);
        } else if apply && let Err(e) = self.apply_sketch(ctx) {
            self.sketch.as_mut().unwrap().message = e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cam_core::geometry::Point;
    #[test]
    fn solved_apply_is_one_cam_transaction_with_repair_and_undo_redo() {
        use cam_core::project::v5::{
            ContourAnchorV5, OperationSettingsV5, StartSelectionV5, TabPlacementV5, TabSettingsV5,
        };
        use cam_core::sketch::ConstraintKind;
        let mut job = engine::open(include_str!(
            "../../../fixtures/v5/sketch-lifecycle-job.json"
        ))
        .unwrap();
        let owner = job.artwork[1].id.clone();
        let catalogue = materialize_item(&job.artwork[1], None).unwrap().catalogue;
        let OperationSettingsV5::Profile(s) = &mut job.operations[0].settings else {
            panic!()
        };
        let anchor = ContourAnchorV5 {
            geometry: s.contours[1].geometry.clone(),
            source_geometry_fingerprint: catalogue
                .contour("contour-plate")
                .unwrap()
                .source_fingerprint
                .clone(),
            fraction_along_source_contour: 0.25,
        };
        s.start = StartSelectionV5::Anchor(Box::new(anchor.clone()));
        s.tabs = Some(TabSettingsV5 {
            height_mm: Some(0.5),
            width_mm: Some(2.),
            shape: Default::default(),
            placement: TabPlacementV5::Manual {
                anchors: vec![anchor],
            },
        });
        let original = job.clone();
        let ctx = egui::Context::default();
        let mut app = App {
            document: Some(Document::new(job)),
            plan: Some(("retained-original".into(), 0)),
            prepared: Some((json!({"file":{"sha256":"old"}}), 0)),
            ..Default::default()
        };
        assert!(app.current());
        app.begin_sketch(Some(owner.clone()), &ctx);
        let e = app.sketch.as_mut().unwrap();
        let ConstraintKind::Length { value_mm, .. } = &mut e.saved.model.constraints[8].definition
        else {
            panic!()
        };
        *value_mm = 25.;
        let solved = cam_sketch_solver::solve(&e.saved.model).unwrap();
        assert!(solved.accepted());
        e.saved.model = solved.candidate;
        app.apply_sketch(&ctx).unwrap();
        let changed = app.document.as_ref().unwrap().job.clone();
        assert_eq!(app.undo.len(), 1);
        assert!(!app.current());
        assert!(app.prepared.is_none());
        assert_eq!(changed.setup, original.setup);
        assert_eq!(changed.artwork[0], original.artwork[0]);
        assert_eq!(changed.operations[1], original.operations[1]);
        assert_eq!(app.sketch_impact.as_ref().unwrap().preserved_count(), 1);
        assert_eq!(app.sketch_impact.as_ref().unwrap().repair_count(), 2);
        assert_eq!(app.issues.len(), 2);
        assert!(
            app.issues
                .iter()
                .any(|i| i.field_path.as_deref() == Some("start")
                    && i.operation_id.as_deref() == Some("profile-1"))
        );
        app.undo(&ctx);
        assert_eq!(app.document.as_ref().unwrap().job, original);
        assert!(app.sketch_impact.is_none());
        assert!(
            commands::apply_sketch_with_report(
                &original,
                &owner,
                match original.artwork[1].content.clone() {
                    ArtworkContent::Sketch(s) => s,
                    _ => panic!(),
                }
            )
            .unwrap()
            .outcome
            .issues
            .is_empty()
        );
        app.redo(&ctx);
        assert_eq!(app.document.as_ref().unwrap().job, changed);
        assert!(!app.current());
        // Explicitly choosing the boundary repairs both bindings through the
        // existing commands; the geometry selection did not need repair.
        let pick = cam_core::project::v5::artwork::GeometryPick {
            artwork_item_id: owner,
            kind: cam_core::project::v5::GeometryRefKind::ClosedContour,
            local_geometry_id: "contour-plate".into(),
        };
        let repaired = commands::reattach_anchor(
            &changed,
            "profile-1",
            commands::AnchorTarget::Start,
            &pick,
            Some(0.3),
        )
        .unwrap();
        let repaired = commands::reattach_anchor(
            &repaired.job,
            "profile-1",
            commands::AnchorTarget::Tab(0),
            &pick,
            Some(0.6),
        )
        .unwrap();
        assert!(repaired.issues.is_empty());
        assert_eq!(
            engine::open(&repaired.job.to_json().unwrap()).unwrap(),
            repaired.job
        );
    }
    fn app() -> App {
        App {
            document: Some(Document::new(
                engine::open(include_str!("../../../fixtures/v5/sketch-job.json")).unwrap(),
            )),
            ..Default::default()
        }
    }
    fn rectangle(app: &mut App) {
        let editor = app.sketch.as_mut().unwrap();
        editor.set_tool(Tool::Rectangle);
        editor.click(Point::new(1., 1.)).unwrap();
        editor.click(Point::new(30., 20.)).unwrap();
    }
    #[test]
    fn new_apply_cancel_history_and_roundtrip_preserve_setup_and_placement() {
        let mut app = app();
        let ctx = egui::Context::default();
        let original = app.document.as_ref().unwrap().job.clone();
        let text = original.to_json().unwrap();
        app.begin_sketch(None, &ctx);
        rectangle(&mut app);
        assert_eq!(app.document.as_ref().unwrap().job.to_json().unwrap(), text);
        assert_eq!(app.revision, 0);
        app.cancel_sketch(&ctx);
        assert_eq!(app.document.as_ref().unwrap().job.to_json().unwrap(), text);
        assert!(app.undo.is_empty());
        app.begin_sketch(None, &ctx);
        rectangle(&mut app);
        app.apply_sketch(&ctx).unwrap();
        let applied = app.document.as_ref().unwrap().job.clone();
        assert_eq!(app.undo.len(), 1);
        assert_eq!(applied.setup, original.setup);
        assert_eq!(applied.artwork[0], original.artwork[0]);
        let reopened = engine::open(&applied.to_json().unwrap()).unwrap();
        assert_eq!(reopened.artwork, applied.artwork);
        let id = applied.artwork.last().unwrap().id.clone();
        app.begin_sketch(Some(id), &ctx);
        let e = app.sketch.as_mut().unwrap();
        let id = e.saved.model.entities[0].id.clone();
        e.select_wire(Some(&id), false);
        let before = e.saved.model.clone();
        e.translate(&before, Point::new(2., 2.)).unwrap();
        app.apply_sketch(&ctx).unwrap();
        assert_eq!(app.document.as_ref().unwrap().job.setup, original.setup);
        assert_eq!(
            app.document
                .as_ref()
                .unwrap()
                .job
                .artwork
                .last()
                .unwrap()
                .placement,
            applied.artwork.last().unwrap().placement
        );
        app.undo(&ctx);
        assert_eq!(
            app.document.as_ref().unwrap().job.to_json().unwrap(),
            applied.to_json().unwrap()
        );
    }
    #[test]
    fn invalid_apply_retains_draft_and_never_mutates_job() {
        let mut app = app();
        let ctx = egui::Context::default();
        let text = app.document.as_ref().unwrap().job.to_json().unwrap();
        app.begin_sketch(None, &ctx);
        rectangle(&mut app);
        let e = app.sketch.as_mut().unwrap();
        e.set_tool(Tool::Circle);
        e.click(Point::new(29., 10.)).unwrap();
        e.click(Point::new(34., 10.)).unwrap();
        assert!(app.apply_sketch(&ctx).is_err());
        assert!(app.sketch.is_some());
        assert_eq!(app.document.as_ref().unwrap().job.to_json().unwrap(), text);
        assert!(app.undo.is_empty());
        app.sketch.as_mut().unwrap().undo();
        app.apply_sketch(&ctx).unwrap();
    }
    #[test]
    fn recovery_restores_local_draft_and_shortcut_undo_stays_local() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.begin_sketch(None, &ctx);
        rectangle(&mut app);
        let snapshot = app.recovery_snapshot().unwrap();
        snapshot.validate().unwrap();
        let mut restored = App::default();
        restored.restore(snapshot, &ctx);
        assert_eq!(
            restored.sketch.as_ref().unwrap().saved,
            app.sketch.as_ref().unwrap().saved
        );
        let text = app.document.as_ref().unwrap().job.to_json().unwrap();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.ui(ctx));
        assert!(app.sketch.as_ref().unwrap().saved.model.entities.is_empty());
        assert_eq!(app.document.as_ref().unwrap().job.to_json().unwrap(), text);
        assert!(app.undo.is_empty());
    }
    #[test]
    fn editor_layout_keeps_apply_cancel_and_canvas_visible() {
        for (width, height) in [(640., 400.), (853., 533.), (1280., 800.)] {
            let mut app = app();
            let ctx = egui::Context::default();
            App::theme(&ctx);
            app.begin_sketch(None, &ctx);
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(10000., 10000.),
                    )),
                    ..Default::default()
                },
                |ctx| app.ui(ctx),
            );
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.ui(ctx),
                );
            }
            for label in ["Apply sketch", "Cancel sketch", "Sketch canvas"] {
                let r = control_rect(label).unwrap();
                assert!(
                    r[0] >= 0. && r[2] <= width && r[1] >= 0. && r[3] <= height,
                    "{label}: {r:?}"
                );
                if label == "Sketch canvas" {
                    assert!(r[3] - r[1] > 100., "canvas too short: {r:?}");
                }
            }
            assert_eq!(
                app.sketch.as_ref().unwrap().canvas_probe()["center"],
                json!([50., 50.])
            );
            assert!(
                app.sketch.as_ref().unwrap().canvas_probe()["scale"]
                    .as_f64()
                    .unwrap()
                    < width as f64 / 100.
            );
        }
    }
    #[test]
    fn native_pointer_drawing_applies_outline_and_hole_without_changing_stock() {
        fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.ui(ctx),
            );
        }
        fn click(app: &mut App, ctx: &egui::Context, p: egui::Pos2) {
            for pressed in [true, false] {
                frame(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(p),
                        egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            frame(app, ctx, vec![]);
        }
        fn control(app: &mut App, ctx: &egui::Context, label: &str) {
            let r = control_rect(label).unwrap();
            click(app, ctx, egui::pos2((r[0] + r[2]) / 2., (r[1] + r[3]) / 2.));
        }
        fn point(app: &mut App, ctx: &egui::Context, x: f64, y: f64) {
            let r = control_rect("Sketch canvas").unwrap();
            let probe = app.sketch.as_ref().unwrap().canvas_probe();
            let s = probe["scale"].as_f64().unwrap();
            let cx = probe["center"][0].as_f64().unwrap();
            let cy = probe["center"][1].as_f64().unwrap();
            click(
                app,
                ctx,
                egui::pos2(
                    (r[0] + r[2]) / 2. + ((x - cx) * s) as f32,
                    (r[1] + r[3]) / 2. - ((y - cy) * s) as f32,
                ),
            );
        }
        let mut app = app();
        let ctx = egui::Context::default();
        App::theme(&ctx);
        let stock = app.document.as_ref().unwrap().job.setup.clone();
        frame(&mut app, &ctx, vec![]);
        control(&mut app, &ctx, "New sketch");
        frame(&mut app, &ctx, vec![]);
        control(&mut app, &ctx, "Sketch Rectangle");
        point(&mut app, &ctx, 10., 10.);
        point(&mut app, &ctx, 80., 70.);
        control(&mut app, &ctx, "Sketch Circle");
        assert_eq!(
            app.sketch.as_ref().unwrap().tool,
            Tool::Circle,
            "Circle control must activate its tool"
        );
        point(&mut app, &ctx, 30., 30.);
        point(&mut app, &ctx, 40., 30.);
        assert_eq!(app.sketch.as_ref().unwrap().saved.model.entities.len(), 5);
        control(&mut app, &ctx, "Sketch Select");
        control(&mut app, &ctx, "Constraints & dimensions");
        point(&mut app, &ctx, 45., 10.);
        control(&mut app, &ctx, "Add constraint");
        let editor = app.sketch.as_ref().unwrap();
        assert_eq!(editor.saved.model.constraints.len(), 1);
        assert!(editor.solver_status().contains("Underconstrained"));
        control(&mut app, &ctx, "Apply sketch");
        assert!(app.sketch.is_none());
        let job = &app.document.as_ref().unwrap().job;
        assert_eq!(job.setup, stock);
        assert_eq!(
            materialize_item(job.artwork.last().unwrap(), None)
                .unwrap()
                .catalogue
                .contours
                .len(),
            2
        );
        assert_eq!(app.undo.len(), 1);
        engine::open(&job.to_json().unwrap()).unwrap();
    }
    #[test]
    fn cancel_first_sketch_restores_welcome_and_removes_draft_from_recovery() {
        let mut app = App::default();
        let ctx = egui::Context::default();
        app.begin_sketch(None, &ctx);
        rectangle(&mut app);
        assert!(app.recovery_snapshot().unwrap().workspace.sketch.is_some());
        app.cancel_sketch(&ctx);
        assert!(app.document.is_none());
        assert!(app.recovery_snapshot().unwrap().workspace.sketch.is_none());
    }
}
