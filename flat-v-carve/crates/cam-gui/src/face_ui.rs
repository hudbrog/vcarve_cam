//! Face operation editor (GUI7a) and its coverage controls (GUI7b).
//!
//! The panel only binds document fields: coverage area and margins, travel
//! overrun, the pass pattern/angle, depths and the milling assignment. The
//! planner reports what a setting actually removes and where the tool travels.
use super::*;
use crate::face;
use cam_core::project::{FaceArea, FacePattern, HeightReference, RectXY, SpindleDirection};

impl App {
    pub(super) fn face_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(doc) = &self.document else { return };
        let id = doc.raw.operation.clone();
        let Some(settings) = crate::session::face(&doc.job, &id) else {
            return;
        };
        let _ = settings;
        ui.heading("Face");
        ui.small("Source-free facing: stock, tool and cutting values only.");
        ui.separator();
        let _ = ctx;
    }

    pub(super) fn face_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        if id.is_empty() {
            return;
        }
        ui.heading("Face");
        ui.small("Faces the requested area with one endmill. Nothing is assumed: unset values stay unset and the planner reports what is still missing.");
        ui.add_space(4.);
        self.face_coverage(ui, ctx);
        self.face_heights(ui, ctx);
        self.face_tool(ui, ctx);
        self.face_passes(ui, ctx);
    }

    fn face_coverage(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| &d.job) else {
            return;
        };
        let Some(settings) = crate::session::face(job, &id) else {
            return;
        };
        let area = settings.area.clone();
        let pattern = settings.pattern;
        let stock_xy = job.setup.stock.xy;
        self.operation_group(ui, "Coverage", true, |app, ui| {
            help::label(ui, "Requested face area");
            ui.horizontal_wrapped(|ui| {
                for (label, whole) in [("Entire stock", true), ("Rectangle", false)] {
                    let selected = matches!(area, FaceArea::EntireStock) == whole;
                    let r = ui.selectable_label(selected, label);
                    observe_control(label, r.rect);
                    if r.clicked() && !selected {
                        let target = if whole {
                            FaceArea::EntireStock
                        } else {
                            FaceArea::Rectangle {
                                rect: stock_xy.unwrap_or(RectXY {
                                    min_x_mm: 0.,
                                    min_y_mm: 0.,
                                    width_mm: 0.,
                                    length_mm: 0.,
                                }),
                            }
                        };
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            face::set_area(job, &id, target.clone())
                        });
                    }
                }
            });
            if matches!(area, FaceArea::EntireStock) {
                ui.small("Coverage is the physical stock rectangle plus the margins. Set stock XY in Setup.");
            } else {
                ui.small("Requested coverage rectangle in setup coordinates; margins expand it per side.");
                app.operation_numbers(ui, ctx, &[82, 83, 84, 85]);
            }
            ui.separator();
            help::label(ui, "Coverage margins");
            ui.small("Positive margins extend coverage outward; negative values would pull it inward and are rejected.");
            app.operation_numbers(ui, ctx, &[78, 79, 80, 81]);
            ui.separator();
            help::label(ui, "Travel overrun");
            ui.small("Entry/exit overrun is travel beyond the requested coverage. Overhang is allowed travel; it is not a claim that material outside the request is faced.");
            app.operation_numbers(ui, ctx, &[76, 77]);
            ui.separator();
            help::label(ui, "Pass pattern");
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Zig-zag passes", FacePattern::ZigZag),
                    ("One-way passes", FacePattern::OneWay),
                ] {
                    let r = ui.selectable_label(pattern == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && pattern != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            face::set_pattern(job, &id, value)
                        });
                    }
                }
            });
            help::label(ui, "Pass angle");
            ui.horizontal_wrapped(|ui| {
                for (label, angle) in [("0° (rows along X)", 0.), ("90° (rows along Y)", 90.)] {
                    let selected = face::value(
                        &app.document.as_ref().unwrap().job,
                        &app.operation_id(),
                        75,
                    ) == Some(angle);
                    let r = ui.selectable_label(selected, label);
                    observe_control(label, r.rect);
                    if r.clicked() && !selected {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            face::set(job, &id, 75, Some(angle))
                        });
                    }
                }
            });
            app.operation_numbers(ui, ctx, &[75]);
            ui.small("Only 0° and 90° raster facing ships in this milestone; another angle is rejected with a located reason.");
        });
    }

    fn face_heights(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| &d.job) else {
            return;
        };
        let Some(settings) = crate::session::face(job, &id) else {
            return;
        };
        let top = settings.top.reference.clone();
        let bottom = settings.bottom.reference.clone();
        let faces = face::published_faces(job, &id);
        self.operation_group(ui, "Heights & depth", true, |app, ui| {
            help::label(ui, "Face top");
            ui.horizontal_wrapped(|ui| {
                {
                    let label = "Top: stock top";
                    let r = ui.selectable_label(top == HeightReference::StockTop, label);
                    observe_control(label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            face::set_height_reference(
                                job,
                                &id,
                                false,
                                HeightReference::StockTop,
                            )
                        });
                    }
                }
                for face_id in &faces {
                    let reference = HeightReference::FaceResult {
                        operation_id: face_id.clone(),
                    };
                    let label = format!("Top: {face_id} result");
                    let r = ui.selectable_label(top == reference, &label);
                    observe_control(&label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        let reference = reference.clone();
                        app.edit_job(ctx, &[], move |job| {
                            face::set_height_reference(job, &id, false, reference.clone())
                        });
                    }
                }
            });
            help::label(ui, "Face bottom");
            ui.horizontal_wrapped(|ui| {
                for (label, reference) in [
                    ("Bottom: stock top", HeightReference::StockTop),
                    ("Bottom: stock bottom", HeightReference::StockBottom),
                    ("Bottom: this operation's top", HeightReference::OperationTop),
                ] {
                    let r = ui.selectable_label(bottom == reference, label);
                    observe_control(label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            face::set_height_reference(job, &id, true, reference.clone())
                        });
                    }
                }
            });
            ui.small("The facing depth is the distance from the top reference down to the bottom reference. Enter the bottom offset (negative removes material) or pick stock bottom.");
            app.operation_numbers(ui, ctx, &[86, 87]);
            ui.separator();
            help::label(ui, "Depth per pass");
            app.operation_numbers(ui, ctx, &[8, 88]);
            ui.small("Stepdown is the pass depth; the planner never exceeds the tool's stepdown limit.");
        });
    }

    fn face_tool(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.operation_group(ui, "Tool & cutting", true, |app, ui| {
            let job = app.document.as_ref().unwrap().job.clone();
            let id = app.operation_id();
            let assigned = crate::session::face(&job, &id).map(|s| s.assignment.tool_id.clone());
            let label = assigned
                .as_ref()
                .and_then(|tool_id| job.tools.iter().find(|tool| &tool.id == tool_id))
                .map(|tool| format!("{} · {}", tool.name, tool.id))
                .unwrap_or_else(|| "no tool assigned".into());
            ui.strong(label);
            let mut selected = assigned.clone().unwrap_or_default();
            let before = selected.clone();
            let response = egui::ComboBox::from_id_salt("face-tool")
                .width(ui.available_width())
                .selected_text("Choose a job tool…")
                .show_ui(ui, |ui| {
                    for tool in &job.tools {
                        if matches!(
                            tool.geometry,
                            None | Some(cam_core::project::ToolGeometry::Endmill(_))
                        ) {
                            ui.selectable_value(
                                &mut selected,
                                tool.id.clone(),
                                format!("{} · {}", tool.name, tool.id),
                            );
                        }
                    }
                });
            observe_control("Face assignment tool", response.response.rect);
            if selected != before && !selected.is_empty() {
                let id = app.operation_id();
                app.edit_job(ctx, &[2, 8, 9, 10, 11, 12, 13, 88], move |job| {
                    authoring::assign_tool_in(job, &id, false, &selected)
                });
            }
            if ui
                .button("Clear face cutting values")
                .clicked()
            {
                let id = app.operation_id();
                app.edit_job(ctx, &[2, 8, 9, 10, 11, 88], move |job| {
                    authoring::clear_assignment_in(job, &id, false);
                    Ok(())
                });
            }
            ui.separator();
            app.operation_numbers(ui, ctx, &[12, 13]);
            ui.separator();
            help::label(ui, "Feeds & speed");
            app.operation_numbers(ui, ctx, &[2, 10, 11]);
            let direction = crate::session::face(&app.document.as_ref().unwrap().job, &app.operation_id())
                .and_then(|s| s.assignment.spindle_direction);
            ui.horizontal_wrapped(|ui| {
                for (name, v) in [
                    ("unset", None),
                    ("CW", Some(SpindleDirection::Clockwise)),
                    ("CCW", Some(SpindleDirection::Counterclockwise)),
                ] {
                    let label = format!("Face {name}");
                    let r = ui.selectable_value(&mut direction.clone(), v, &label);
                    observe_control(&label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            face::set_spindle_direction(job, &id, v)
                        });
                    }
                }
            });
            ui.small("Spindle direction is required before checked export; it does not change the generated paths.");
        });
    }

    fn face_passes(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.operation_group(ui, "Stepover & limits", true, |app, ui| {
            help::label(ui, "Stepover");
            app.operation_numbers(ui, ctx, &[9]);
            ui.small("Stepover must be positive and no greater than the cutter diameter; a larger value is rejected with the allowed range.");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CONTROLS;
    use crate::operation_authoring::{self, Kind};

    fn app() -> App {
        let empty = operation_authoring::empty_job();
        let job = operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty))
            .unwrap();
        App {
            document: Some(Document::new(job)),
            inspector_tab: 2,
            ..Default::default()
        }
    }

    fn render(app: &mut App, ctx: &egui::Context) -> std::collections::BTreeMap<String, [f32; 4]> {
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
        CONTROLS.with(|c| c.borrow().clone())
    }

    #[test]
    fn face_editor_binds_its_own_fields_without_milling_assumptions() {
        let mut app = app();
        let ctx = egui::Context::default();
        let controls = render(&mut app, &ctx);
        // Face coverage controls (GUI7a/b).
        for label in [
            "Entire stock",
            "Rectangle",
            // Stepdown/stepover are the operation's own assignment values.
            "Stepdown",
            "Stepover",
            // The binder renames field 2 to "Cutting feed" on screen; its
            // stable probe key stays the field label.
            "Roughing feed",
            "Plunge feed",
            "Spindle speed",
            "Face pass angle",
            "Face entry overrun",
            "Face exit overrun",
            "Face margin min X",
            "Face top offset",
            "Face bottom offset",
            "Tool stepdown limit",
            "Zig-zag passes",
            "One-way passes",
            "0° (rows along X)",
            "90° (rows along Y)",
        ] {
            assert!(controls.contains_key(label), "{label} missing");
        }
        // A face operation has no V-bit tab content and no carving mode.
        assert!(!controls.contains_key("Combined"));
        assert!(!controls.contains_key("Endmill only"));
        // Editing a face field commits into the face settings, not a carving.
        let doc = app.document.as_mut().unwrap();
        doc.edit(8, "0.5".into()).unwrap();
        assert_eq!(
            crate::session::face(&doc.job, &doc.raw.operation)
                .unwrap()
                .stepdown_mm,
            Some(0.5)
        );
        doc.edit(87, "-0.5".into()).unwrap();
        assert_eq!(
            crate::session::face(&doc.job, &doc.raw.operation)
                .unwrap()
                .bottom
                .offset_mm,
            -0.5
        );
        doc.validate().unwrap();
    }

    #[test]
    fn a_configured_face_job_leaves_no_pending_text() {
        let mut app = app();
        let doc = app.document.as_mut().unwrap();
        doc.edit(6, "6".into()).unwrap();
        // The four stock dimensions commit together; a partial entry keeps its
        // raw text and reports that the group is incomplete.
        for (field, text) in [(42, "40"), (43, "30"), (41, "0"), (40, "0")] {
            let _ = doc.edit(field, text.into());
        }
        assert!(
            doc.job.setup.stock.xy.is_some(),
            "stock dimensions committed"
        );
        for (field, text) in [
            (2, "300"),
            (8, "1"),
            (88, "1"),
            (9, "1.5"),
            (10, "100"),
            (11, "12000"),
            (75, "0"),
            (87, "-1"),
            (78, "0"),
            (76, "0"),
        ] {
            doc.edit(field, text.into())
                .unwrap_or_else(|error| panic!("field {field}: {error}"));
        }
        let offenders: Vec<usize> = (0..crate::state::FIELDS.len())
            .filter(|&field| {
                !matches!(field, 26..=29)
                    && crate::authoring::active_in(&doc.job, &doc.raw.operation, field)
                    && crate::state::Draft::parse(&doc.text(field)).ok()
                        != Some(crate::app::value(&doc.job, &doc.raw.operation, field))
            })
            .collect();
        assert!(offenders.is_empty(), "pending fields {offenders:?}");
        doc.validate().unwrap();
    }

    #[test]
    fn switching_between_operations_keeps_each_editors_own_text() {
        let empty = operation_authoring::empty_job();
        let face = operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty))
            .unwrap();
        let job =
            operation_authoring::apply(&face, operation_authoring::add(Kind::FlatVcarve, &face))
                .unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            inspector_tab: 2,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        {
            let doc = app.document.as_mut().unwrap();
            doc.select_operation("face-1");
            doc.edit(8, "0.5".into()).unwrap();
            doc.select_operation("carving-1");
            doc.edit(8, "2".into()).unwrap();
        }
        // Reordering operations must not move the text to another entity.
        let ordered = operation_authoring::apply(
            &app.document.as_ref().unwrap().job,
            operation_authoring::Action::Move {
                operation_id: "carving-1".into(),
                to_index: 0,
            },
        )
        .unwrap();
        app.adopt_operation(ordered, None, &ctx);
        let doc = app.document.as_mut().unwrap();
        doc.select_operation("face-1");
        assert_eq!(doc.text(8), "0.5", "the face keeps its own edited text");
        doc.select_operation("carving-1");
        assert_eq!(doc.text(8), "2", "the carve keeps its own edited text");
        doc.snapshot().validate().unwrap();
    }
}
