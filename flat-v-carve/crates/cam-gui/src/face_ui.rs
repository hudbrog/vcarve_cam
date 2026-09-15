//! Face operation editor (GUI7a) and its coverage controls (GUI7b).
//!
//! The panel only binds document fields: coverage area and margins, travel
//! overrun, the pass pattern/angle, depths and the milling assignment. The
//! planner reports what a setting actually removes and where the tool travels.
use super::*;
use crate::face;
use cam_core::project::{
    FaceArea, FaceEntry, FacePattern, HeightReference, RectXY, SpindleDirection,
};

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
            app.face_entry_choice(ui, ctx);
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

    /// Where the passes enter (plan section 11.1). The choice is the user's,
    /// and the readout is the same resolution the planner performs, so the
    /// panel says where the cutter descends and how much room it has before
    /// anything is generated.
    fn face_entry_choice(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let (entry, angle, stock) = {
            let Some(job) = self.document.as_ref().map(|d| &d.job) else {
                return;
            };
            let Some(settings) = crate::session::face(job, &id) else {
                return;
            };
            (settings.entry, settings.pass_angle_deg, job.setup.stock.xy)
        };
        let preview = self.document.as_ref().and_then(|document| {
            cam_core::project::v5::inspection::face_entry_preview(&document.job, &id)
                .ok()
                .flatten()
        });
        let axis = preview
            .as_ref()
            .map(|preview| preview.axis.to_string())
            .unwrap_or_else(|| {
                if angle == Some(90.) {
                    "Y".into()
                } else {
                    "X".into()
                }
            });
        help::label(ui, "Pass entry");
        ui.horizontal_wrapped(|ui| {
            for (label, value) in [
                (format!("Start at {axis}−"), FaceEntry::Min),
                (format!("Start at {axis}+"), FaceEntry::Max),
                ("Start at…".to_string(), FaceEntry::At { coordinate_mm: 0. }),
                ("Alternate per depth".to_string(), FaceEntry::Alternate),
            ] {
                let selected = matches!(
                    (entry, value),
                    (FaceEntry::Min, FaceEntry::Min)
                        | (FaceEntry::Max, FaceEntry::Max)
                        | (FaceEntry::At { .. }, FaceEntry::At { .. })
                        | (FaceEntry::Alternate, FaceEntry::Alternate)
                );
                let response = ui.selectable_label(selected, &label);
                observe_control(&label, response.rect);
                if response.clicked() && !selected {
                    let id = self.operation_id();
                    self.edit_job(ctx, &[], move |job| face::set_entry(job, &id, value));
                }
            }
        });
        if matches!(entry, FaceEntry::At { .. }) {
            ui.small("Position along the pass direction, in setup coordinates: the pass starts here instead of a travel past the coverage edge.");
            self.operation_numbers(ui, ctx, &[109]);
        } else {
            ui.small("Entry travel reaches past the coverage edge on the end every pass enters from; the exit travel trails the other end.");
        }
        if let Some(preview) = &preview {
            if let Some(stock) = stock {
                ui.small(format!(
                    "Passes run along {axis}; the stock spans {axis} {:.3} … {:.3}, and the clearance below is measured against that rectangle.",
                    if axis == "Y" {
                        stock.min_y_mm
                    } else {
                        stock.min_x_mm
                    },
                    if axis == "Y" {
                        stock.min_y_mm + stock.length_mm
                    } else {
                        stock.min_x_mm + stock.width_mm
                    }
                ));
            }
            if let Some(coordinate) = preview.inside_coverage {
                ui.small(format!(
                    "The entry at {axis} = {coordinate:.3} is inside the pass's span ({axis} {:.3} … {:.3}): a pass starting there would leave the strip behind it unswept. A pass must run the whole span, so it starts at or below {axis} {:.3} or at or above {axis} {:.3}.",
                    preview.pass_low_mm, preview.pass_high_mm, preview.pass_low_mm, preview.pass_high_mm
                ));
            } else if !preview.entries.is_empty() {
                let (lo, hi) = preview
                    .entries
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), entry| {
                        (lo.min(*entry), hi.max(*entry))
                    });
                let where_ = if (hi - lo).abs() < 1e-9 {
                    format!("Every pass enters at {axis} = {lo:.3}")
                } else {
                    format!(
                        "Passes enter at {axis} = {lo:.3} on even layers and {axis} = {hi:.3} on odd"
                    )
                };
                let clearance = preview
                    .clearances
                    .iter()
                    .flatten()
                    .fold(f64::INFINITY, |worst, value| worst.min(*value));
                let verdict = if !clearance.is_finite() {
                    "set the physical stock in Setup to check the entry".to_string()
                } else if clearance >= 0. {
                    format!("the cutter clears the stock by {clearance:.3} mm there")
                } else {
                    format!(
                        "the cutter reaches {:.3} mm into the stock there, so this tool must be able to plunge",
                        -clearance
                    )
                };
                ui.small(format!("{where_} · {verdict}"));
                if clearance < 0.
                    && let Some((required, travel)) = preview.clearing
                {
                    ui.small(format!(
                        "Starting at {axis} = {required:.3} clears the stock: that is {travel:.3} mm of entry travel."
                    ));
                    if matches!(
                        entry,
                        FaceEntry::Min | FaceEntry::Max | FaceEntry::Alternate
                    ) {
                        let response = ui.button(format!("Use {travel:.3} mm of entry travel"));
                        observe_control("Use entry travel", response.rect);
                        if response.clicked() {
                            let id = self.operation_id();
                            self.edit_job(ctx, &[], move |job| {
                                face::set(job, &id, 76, Some(travel))
                            });
                        }
                    } else {
                        let response = ui.button(format!("Start at {axis} = {required:.3}"));
                        observe_control("Use entry position", response.rect);
                        if response.clicked() {
                            let id = self.operation_id();
                            self.edit_job(ctx, &[], move |job| {
                                face::set(job, &id, 109, Some(required))
                            });
                        }
                    }
                }
            }
            let envelope = &preview.envelope;
            ui.small(format!(
                "Allowed travel: X {:.3} … {:.3}, Y {:.3} … {:.3} (coverage, travel and cutter radius).",
                envelope.min_x_mm,
                envelope.min_x_mm + envelope.width_mm,
                envelope.min_y_mm,
                envelope.min_y_mm + envelope.length_mm
            ));
        }
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
            // The same cutter picker every operation uses.
            let operation = app.operation_id();
            let cutter = super::tool_picker::Cutter::milling(&operation, "Face");
            app.tool_picker(ui, ctx, &cutter);
            ui.separator();
            app.operation_numbers(ui, ctx, &[12, 13]);
            ui.separator();
            // Where the passes enter the cut: a face mill that cannot plunge
            // needs entry clearance instead, and the planner refuses to
            // descend through material without this answer (plan section 11).
            app.operation_capabilities(ui, ctx, false);
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
            // The pass entry choice: the two ends of the pass axis, an
            // explicit position, or the per-layer flip.
            "Start at X−",
            "Start at X+",
            "Start at…",
            "Alternate per depth",
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

    /// A face job configured through the panel's own fields, at the setup
    /// origin so the plan's coordinates are the panel's.
    fn configured_face() -> (App, String) {
        let mut app = app();
        let doc = app.document.as_mut().unwrap();
        // The four stock dimensions commit as one group; a partial entry keeps
        // its raw text until the last one arrives.
        for (field, text) in [(42, "100"), (43, "60"), (41, "0"), (40, "0")] {
            let _ = doc.edit(field, text.into());
        }
        for (field, text) in [
            (6, "8"),
            (7, "5"),
            (2, "600"),
            (8, "1"),
            (88, "1"),
            (9, "6"),
            (10, "100"),
            (11, "12000"),
            (75, "0"),
            (76, "20"),
            (77, "2"),
            (87, "-2"),
        ] {
            // Multi-field groups keep their raw text until the group completes.
            let _ = doc.edit(field, text.into());
        }
        for (field, text) in [(12, "10"), (13, "20")] {
            let _ = doc.edit(field, text.into());
        }
        let id = doc.raw.operation.clone();
        crate::authoring::tool_mut_in(&mut doc.job, &id, false)
            .unwrap()
            .capabilities
            .plunge_capable = Some(true);
        (app, id)
    }

    /// The readout is the planner's own resolution: every position the panel
    /// promises is a position the plan descends at, and the coverage it shows
    /// is the coverage the plan reports.
    #[test]
    fn the_entry_readout_matches_the_plan() {
        use cam_core::project::v5::references::ReadinessScope;
        use cam_core::sequence::{GenerationStatus, OperationPlanV5, PlanLimits};

        let (mut app, id) = configured_face();
        for entry in [
            FaceEntry::Min,
            FaceEntry::Max,
            FaceEntry::Alternate,
            FaceEntry::At {
                coordinate_mm: -30.,
            },
        ] {
            let job = app.document.as_mut().unwrap().job.clone();
            let mut job = job;
            face::set_entry(&mut job, &id, entry).unwrap();
            let preview = cam_core::project::v5::inspection::face_entry_preview(&job, &id)
                .unwrap()
                .expect("the fixture resolves");
            let plan = OperationPlanV5::plan_job_v5(
                &job,
                &ReadinessScope::AllEnabled,
                &PlanLimits::default(),
            )
            .unwrap();
            assert_eq!(
                plan.operation_results[0].generation_status,
                GenerationStatus::Complete,
                "{entry:?}: {:?}",
                plan.generation_diagnostics
            );
            let covered = plan.operation_results[0].named_outputs[0]
                .covered
                .expect("a face publishes its coverage");
            assert_eq!(covered, preview.coverage, "{entry:?} coverage");
            // The envelope the panel shows is the region the plan may travel
            // in: nothing the planner emits may leave it.
            let envelope = &preview.envelope;
            for motion in &plan.motions {
                for point in [motion.start, motion.end] {
                    assert!(
                        point.x >= envelope.min_x_mm - 1e-6
                            && point.x <= envelope.min_x_mm + envelope.width_mm + 1e-6
                            && point.y >= envelope.min_y_mm - 1e-6
                            && point.y <= envelope.min_y_mm + envelope.length_mm + 1e-6,
                        "{entry:?}: motion {} leaves the envelope at ({}, {})",
                        motion.id,
                        point.x,
                        point.y
                    );
                }
            }
            // The axis is X at 0 degrees; a descent is any motion that ends
            // lower than the motion before it (the first one starts at the
            // bridge's clearance).
            let descents: Vec<f64> = plan
                .motions
                .iter()
                .enumerate()
                .filter(|(index, motion)| {
                    let previous = if *index == 0 {
                        motion.start.z
                    } else {
                        plan.motions[index - 1].end.z
                    };
                    motion.end.z < previous - 1e-9
                })
                .map(|(_, motion)| motion.end.x)
                .collect();
            assert_eq!(descents, preview.entries, "{entry:?} entries");
        }
    }

    /// Switching to an explicit position keeps the position the passes enter at
    /// now, and the mode is a document field like any other.
    #[test]
    fn the_entry_choice_is_a_document_field_and_keeps_its_position() {
        let (mut app, id) = configured_face();
        let job = &mut app.document.as_mut().unwrap().job;
        assert_eq!(face::entry(job, &id), Some(FaceEntry::Min));
        // `Min` with 20 mm of travel on a pass spanning X -5 … 105 enters at
        // X = -25.
        face::set_entry(job, &id, FaceEntry::At { coordinate_mm: 0. }).unwrap();
        assert_eq!(
            face::value(job, &id, 109),
            Some(-25.),
            "the position the passes enter at now"
        );
        assert_eq!(
            face::set(job, &id, 109, Some(-40.)),
            Ok(()),
            "the position is editable in the `At` mode"
        );
        face::set_entry(job, &id, FaceEntry::Max).unwrap();
        assert_eq!(face::value(job, &id, 109), None);
        assert_eq!(
            face::set(job, &id, 109, Some(-40.)).unwrap_err(),
            "Choose 'Start at…' before setting the entry position"
        );
        // Switching back seeds from the entry the passes now use: the mode
        // carries the position, not the document, so the panel never keeps a
        // position no pass enters at.
        face::set_entry(job, &id, FaceEntry::At { coordinate_mm: 0. }).unwrap();
        assert_eq!(
            face::value(job, &id, 109),
            Some(125.),
            "the maximum end plus 20 mm of travel"
        );
        face::set_entry(job, &id, FaceEntry::Min).unwrap();
        face::set_entry(job, &id, FaceEntry::At { coordinate_mm: 0. }).unwrap();
        assert_eq!(face::value(job, &id, 109), Some(-25.));
        app.document
            .as_mut()
            .unwrap()
            .job
            .validate_structure()
            .unwrap();
    }
}
