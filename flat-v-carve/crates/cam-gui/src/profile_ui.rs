//! Profile operation editor (GUI8a basic closed profile, GUI8b tabs, GUI8c
//! radial finishing and GUI8d starts/entries).
//!
//! The panel only binds document fields and emits explicit document commands.
//! Which contours a profile cuts, on which side, how deep and in which order
//! all live in [`cam_core::project::v5::ProfileSettingsV5`]; the planner owns
//! the compensation, tab placement and entry geometry that the result
//! inspection and the viewport then display.
use super::*;
use crate::profile::{self, SelectionRow};
use cam_core::project::{
    ContourOrder, ContourSide, CutDirection, HeightReference, SpindleDirection, TraversalDirection,
};

const SIDES: [(&str, ContourSide); 3] = [
    ("Inside", ContourSide::Inside),
    ("Outside", ContourSide::Outside),
    ("On contour", ContourSide::On),
];

impl App {
    pub(super) fn profile_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let name = self
            .document
            .as_ref()
            .and_then(|d| d.active_operation())
            .map(|op| op.name.clone())
            .unwrap_or_else(|| "Profile".into());
        ui.heading(name);
        ui.small(
            "Closed-contour milling: explicit sides, depth passes, tabs, finishing and entries.",
        );
        ui.separator();
        let _ = ctx;
    }

    pub(super) fn profile_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.inspector_tab {
            0 => self.artwork_panel(ui, ctx),
            4 => self.job_tool_panel(ui, ctx, false),
            5 => self.job_tool_panel(ui, ctx, true),
            6 => self.view.inspection_controls(ui),
            _ => {
                ui.heading("Profile");
                ui.small("Mills the selected closed contours. Nothing is assumed: unset values stay unset and the planner reports what is still missing.");
                ui.add_space(4.);
                self.profile_geometry(ui, ctx);
                self.profile_tool(ui, ctx);
                self.profile_heights(ui, ctx);
                self.profile_order(ui, ctx);
                self.profile_evidence(ui);
            }
        }
    }

    fn profile_geometry(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let selected = profile::selection(&job, &id);
        let available = self.view.profile_contours();
        let mut rows = selected.clone();
        self.operation_group(ui, "Geometry to cut", true, |app, ui| {
            ui.label(format!(
                "{} selected · {} available closed contours",
                selected.len(),
                available.len()
            ));
            ui.small("Check the closed boundaries this operation should mill, then choose the side it keeps material on. An outer boundary usually cuts Outside; a hole cuts Inside.");
            if available.is_empty() {
                ui.label("No closed contours available. Import or replace an SVG whose shapes enclose an area; text must be converted to paths.");
            } else {
                if button(ui, "Select all profile contours", app.active.is_none()).clicked() {
                    rows = available
                        .iter()
                        .map(|contour| SelectionRow {
                            reference: contour.reference.clone(),
                            side: contour.suggested_side,
                            traversal: (contour.suggested_side == ContourSide::On)
                                .then_some(TraversalDirection::Forward),
                        })
                        .collect();
                }
                if button(ui, "Clear profile selection", app.active.is_none()).clicked() {
                    rows.clear();
                }
                egui::ScrollArea::vertical()
                    .id_salt("profile-geometry-list")
                    .auto_shrink([false, true])
                    .max_height(230.)
                    .show(ui, |ui| {
                        let items = job.artwork.clone();
                        for item in &items {
                            let local = available
                                .iter()
                                .filter(|contour| {
                                    contour.reference.artwork_item_id == item.id
                                })
                                .collect::<Vec<_>>();
                            if local.is_empty() {
                                continue;
                            }
                            ui.strong(&item.name);
                            for contour in local {
                                let index = rows
                                    .iter()
                                    .position(|row| row.reference == contour.reference);
                                let mut on = index.is_some();
                                let response = ui.add_enabled(
                                    app.active.is_none(),
                                    egui::Checkbox::new(
                                        &mut on,
                                        format!(
                                            "{} · {} · {:.1} mm around",
                                            contour.reference.local_geometry_id,
                                            contour.role,
                                            contour.perimeter_mm
                                        ),
                                    ),
                                );
                                observe_control(
                                    &format!(
                                        "Profile contour {} / {}",
                                        item.id.0, contour.reference.local_geometry_id
                                    ),
                                    response.rect,
                                );
                                if response.changed() {
                                    match (on, index) {
                                        (true, None) => rows.push(SelectionRow {
                                            reference: contour.reference.clone(),
                                            side: contour.suggested_side,
                                            traversal: (contour.suggested_side
                                                == ContourSide::On)
                                                .then_some(TraversalDirection::Forward),
                                        }),
                                        (false, Some(at)) => {
                                            rows.remove(at);
                                        }
                                        _ => {}
                                    }
                                }
                                let Some(at) = rows
                                    .iter()
                                    .position(|row| row.reference == contour.reference)
                                else {
                                    continue;
                                };
                                ui.horizontal_wrapped(|ui| {
                                    ui.add_space(18.);
                                    for (label, side) in SIDES {
                                        let key = format!(
                                            "Profile side {label} {} / {}",
                                            item.id.0, contour.reference.local_geometry_id
                                        );
                                        let response = ui.add_enabled(
                                            app.active.is_none(),
                                            egui::Button::new(label)
                                                .selected(rows[at].side == side),
                                        );
                                        observe_control(&key, response.rect);
                                        if response.clicked() && rows[at].side != side {
                                            rows[at].side = side;
                                            rows[at].traversal = match side {
                                                ContourSide::On => Some(
                                                    rows[at]
                                                        .traversal
                                                        .unwrap_or(TraversalDirection::Forward),
                                                ),
                                                _ => rows[at].traversal,
                                            };
                                        }
                                    }
                                    if rows[at].side == ContourSide::On {
                                        for (label, direction) in [
                                            ("Forward", TraversalDirection::Forward),
                                            ("Reverse", TraversalDirection::Reverse),
                                        ] {
                                            let key = format!(
                                                "Profile traversal {label} {} / {}",
                                                item.id.0,
                                                contour.reference.local_geometry_id
                                            );
                                            let response = ui.add_enabled(
                                                app.active.is_none(),
                                                egui::Button::new(label).selected(
                                                    rows[at].traversal == Some(direction),
                                                ),
                                            );
                                            observe_control(&key, response.rect);
                                            if response.clicked() {
                                                rows[at].traversal = Some(direction);
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    });
            }
            let unresolved: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    !available.iter().any(|c| c.reference == row.reference)
                })
                .map(|(index, _)| index)
                .collect();
            if !unresolved.is_empty() {
                let response = ui.colored_label(
                    Color32::DARK_RED,
                    "Some selected contours are unresolved. Replace or remove them explicitly.",
                );
                observe_control("Unresolved profile contours", response.rect);
                for index in unresolved {
                    let reference = rows[index].reference.clone();
                    ui.horizontal_wrapped(|ui| {
                        ui.small(format!(
                            "Unresolved: {} / {}",
                            reference.artwork_item_id.0, reference.local_geometry_id
                        ));
                        if button(ui, &format!("Remove unresolved contour {index}"), true).clicked()
                        {
                            rows.remove(index);
                        }
                    });
                }
            }
        });
        if rows != selected {
            self.profile_selection(rows, ctx);
        }
    }

    /// Send one explicit contour selection through the retained service: the
    /// catalogue is re-imported there, so a stale reference is refused rather
    /// than rebound to whatever now carries the same ID.
    pub(super) fn profile_selection(&mut self, rows: Vec<SelectionRow>, ctx: &egui::Context) {
        self.artwork_command(engine::ArtworkCommand::ProfileSelection { rows }, ctx);
        self.operation_tab = 0;
        self.navigate(2);
    }

    /// A viewport contour click assigns that contour to the profile. Its stored
    /// side is kept where the contour was already selected; a newly selected
    /// contour starts from the importer's advisory side, which the user can
    /// change explicitly in the operation's contour table.
    pub(crate) fn viewport_profile_selection(
        &mut self,
        references: Vec<cam_core::project::v5::GeometryRef>,
        ctx: &egui::Context,
    ) {
        let Some(document) = &self.document else {
            return;
        };
        let id = document.raw.operation.clone();
        let existing = profile::selection(&document.job, &id);
        let available = self.view.profile_contours();
        let rows = references
            .iter()
            .map(|reference| {
                existing
                    .iter()
                    .find(|row| &row.reference == reference)
                    .cloned()
                    .or_else(|| {
                        available
                            .iter()
                            .find(|contour| &contour.reference == reference)
                            .map(|contour| SelectionRow {
                                reference: contour.reference.clone(),
                                side: contour.suggested_side,
                                traversal: (contour.suggested_side == ContourSide::On)
                                    .then_some(TraversalDirection::Forward),
                            })
                    })
            })
            .collect::<Option<Vec<_>>>();
        let Some(rows) = rows else {
            self.status = "Artwork changed; pick the contour again.".into();
            return;
        };
        self.profile_selection(rows, ctx);
    }

    fn profile_tool(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.operation_group(ui, "Tool & cutting", true, |app, ui| {
            let job = app.document.as_ref().unwrap().job.clone();
            let id = app.operation_id();
            let assigned = profile::settings_in(&job, &id).map(|s| s.assignment.tool_id.clone());
            let label = assigned
                .as_ref()
                .and_then(|tool_id| job.tools.iter().find(|tool| &tool.id == tool_id))
                .map(|tool| format!("{} · {}", tool.name, tool.id))
                .unwrap_or_else(|| "no tool assigned".into());
            ui.strong(label);
            let mut selected = assigned.clone().unwrap_or_default();
            let before = selected.clone();
            let response = egui::ComboBox::from_id_salt("profile-tool")
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
            observe_control("Profile assignment tool", response.response.rect);
            if selected != before && !selected.is_empty() {
                let id = app.operation_id();
                app.edit_job(ctx, &[2, 8, 10, 11, 12, 13, 88], move |job| {
                    authoring::assign_tool_in(job, &id, false, &selected)
                });
            }
            if button(ui, "Clear profile cutting values", app.active.is_none()).clicked() {
                let id = app.operation_id();
                app.edit_job(ctx, &[2, 8, 10, 11, 88], move |job| {
                    authoring::clear_assignment_in(job, &id, false);
                    Ok(())
                });
            }
            ui.separator();
            app.operation_numbers(ui, ctx, &[12, 13]);
            ui.separator();
            help::label(ui, "Feeds & speed");
            app.operation_numbers(ui, ctx, &[2, 10, 11, 88]);
            let direction = profile::settings_in(&app.document.as_ref().unwrap().job, &app.operation_id())
                .and_then(|s| s.assignment.spindle_direction);
            ui.horizontal_wrapped(|ui| {
                for (name, v) in [
                    ("unset", None),
                    ("CW", Some(SpindleDirection::Clockwise)),
                    ("CCW", Some(SpindleDirection::Counterclockwise)),
                ] {
                    let label = format!("Profile {name}");
                    let r = ui.selectable_value(&mut direction.clone(), v, &label);
                    observe_control(&label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            profile::set_spindle_direction(job, &id, v)
                        });
                    }
                }
            });
            ui.small("Spindle rotation is required before checked export and does not change the generated paths.");
            ui.separator();
            app.operation_capabilities(ui, ctx, false);
            ui.small("A ramp entry needs a tool you mark ramp capable; a plunge entry needs one you mark plunge capable.");
        });
    }

    fn profile_heights(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = profile::settings_in(&job, &id) else {
            return;
        };
        let top = settings.top.reference.clone();
        let bottom = settings.bottom.reference.clone();
        let faces = profile::published_faces(&job, &id);
        self.operation_group(ui, "Heights & depth", true, |app, ui| {
            help::label(ui, "Profile top");
            ui.horizontal_wrapped(|ui| {
                {
                    let label = "Top: stock top";
                    let r = ui.selectable_label(top == HeightReference::StockTop, label);
                    observe_control(label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            profile::set_height_reference(job, &id, false, HeightReference::StockTop)
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
                            profile::set_height_reference(job, &id, false, reference.clone())
                        });
                    }
                }
            });
            help::label(ui, "Profile bottom");
            ui.horizontal_wrapped(|ui| {
                for (label, reference) in [
                    ("Bottom: stock top", HeightReference::StockTop),
                    ("Bottom: stock bottom", HeightReference::StockBottom),
                    (
                        "Bottom: this operation's top",
                        HeightReference::OperationTop,
                    ),
                ] {
                    let r = ui.selectable_label(bottom == reference, label);
                    observe_control(label, r.rect);
                    if r.clicked() {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            profile::set_height_reference(job, &id, true, reference.clone())
                        });
                    }
                }
            });
            ui.small("The cut depth is measured from the top reference down to the bottom reference. Enter the bottom offset (negative cuts down) or pick the stock bottom to cut through.");
            app.operation_numbers(ui, ctx, &[89, 90]);
            ui.separator();
            help::label(ui, "Depth per pass");
            app.operation_numbers(ui, ctx, &[8, 88, 70]);
            ui.small("Stepdown is the pass depth, never more than the tool's stepdown limit. The through-cut allowance permits the tool to travel below the stock bottom; it does not move the programmed bottom.");
        });
    }

    fn profile_order(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = profile::settings_in(&job, &id) else {
            return;
        };
        let direction = settings.direction;
        let order = settings.order;
        self.operation_group(ui, "Cut direction & order", true, |app, ui| {
            help::label(ui, "Cut direction");
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Climb", Some(CutDirection::Climb)),
                    ("Conventional", Some(CutDirection::Conventional)),
                    ("unset", None),
                ] {
                    let label = format!("Cut {label}");
                    let r = ui.selectable_label(direction == value, &label);
                    observe_control(&label, r.rect);
                    if r.clicked() && direction != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            profile::set_direction(job, &id, value)
                        });
                    }
                }
            });
            ui.small("Climb or conventional is resolved from the retained side of each contour; an on-contour selection uses its explicit traversal instead.");
            ui.separator();
            help::label(ui, "Contour order");
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("Inner before outer", ContourOrder::InnerBeforeOuter),
                    ("Selection order", ContourOrder::Explicit),
                ] {
                    let label = format!("Order: {label}");
                    let r = ui.selectable_label(order == value, &label);
                    observe_control(&label, r.rect);
                    if r.clicked() && order != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[], move |job| {
                            profile::set_order(job, &id, value)
                        });
                    }
                }
            });
            ui.small("Inner before outer cuts nested islands before the material that holds them. Selection order follows the contour table.");
        });
    }

    /// Generated-evidence readout: resolved heights, per-stage passes and the
    /// nominal cutter-center offset. Values come from the retained plan (or the
    /// tool geometry), never from a UI-side offset computation.
    fn profile_evidence(&mut self, ui: &mut egui::Ui) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = profile::settings_in(&job, &id) else {
            return;
        };
        let selection = profile::selection(&job, &id);
        let mut outside = 0;
        let mut inside = 0;
        let mut on = 0;
        for row in &selection {
            match row.side {
                ContourSide::Outside => outside += 1,
                ContourSide::Inside => inside += 1,
                ContourSide::On => on += 1,
            }
        }
        let radius = job
            .tools
            .iter()
            .find(|tool| tool.id == settings.assignment.tool_id)
            .and_then(|tool| match &tool.geometry {
                Some(cam_core::project::ToolGeometry::Endmill(geometry)) => {
                    Some(geometry.diameter_mm / 2.)
                }
                _ => None,
            });
        let plan = self.view.plan_inspection();
        self.operation_group(ui, "Offset & pass inspection", false, |_app, ui| {
            ui.label(format!(
                "Sides: {outside} outside · {inside} inside · {on} on-contour"
            ));
            ui.label(match settings.direction {
                Some(CutDirection::Climb) => "Direction: climb",
                Some(CutDirection::Conventional) => "Direction: conventional",
                None => "Direction: not chosen yet",
            });
            if let Some(radius) = radius {
                ui.label(format!(
                    "Cutter radius {radius:.4} mm — the rough centerline stands off the contour by this nominal amount"
                ));
                if settings.finish.enabled {
                    ui.label(format!(
                        "Finishing allowance {} mm — the finish centerline adds it to the cutter radius",
                        settings
                            .finish
                            .radial_allowance_mm
                            .map(|v| format!("{v:.4}"))
                            .unwrap_or_else(|| "unset".into())
                    ));
                }
            } else {
                ui.label("Assign a cutter with geometry to see its nominal standoff.");
            }
            let Some(plan) = plan else {
                ui.small("Generate to inspect the plan's resolved heights, passes and stock checkpoints.");
                return;
            };
            if let Some(inspection) = plan.operations.iter().find(|o| o.operation_id == id) {
                match inspection.heights {
                    Some(heights) => {
                        let bottom = heights
                            .bottom_z_mm
                            .map(|z| format!("{z:.4} mm"))
                            .unwrap_or_else(|| "unresolved".into());
                        ui.label(format!(
                            "Resolved heights: top {:.4} mm · bottom {bottom}",
                            heights.top_z_mm
                        ));
                    }
                    None => {
                        ui.label("Resolved heights: unresolved — the planner reported the reason.");
                    }
                }
                ui.label(format!(
                    "Generated tabs: {}",
                    inspection.tab_placements.len()
                ));
            }
            for stage in plan.stages.iter().filter(|s| s.operation_id == id) {
                ui.label(format!(
                    "{} — {} motions · tool {}",
                    match stage.role {
                        cam_core::sequence::StageRole::ProfileRough => "Rough pass",
                        cam_core::sequence::StageRole::ProfileFinish => "Finish pass",
                        _ => "Pass",
                    },
                    stage.motion_count,
                    stage.tool_id
                ));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cam_service::retained::Retained;

    /// The established lettering artwork with every closed contour selected on
    /// its advisory side, as a preview scene the editor can list.
    fn app() -> App {
        let job = crate::profile::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        let id = job.operations[0].id.clone();
        let rows = crate::profile::contours(&job)
            .unwrap()
            .into_iter()
            .map(|contour| crate::profile::SelectionRow {
                reference: contour.reference,
                side: contour.suggested_side,
                traversal: None,
            })
            .collect::<Vec<_>>();
        let job = crate::profile::select_in(&job, &id, &rows).unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            inspector_tab: 2,
            ..Default::default()
        };
        let job = app.document.as_ref().unwrap().job.to_json().unwrap();
        let preview =
            crate::session::execute(&mut Retained::new(), Command::Preview { job }).unwrap();
        app.view.load_scene(Ok(preview));
        app
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
    fn profile_editor_owns_its_contour_table_and_cut_fields() {
        let mut app = app();
        let ctx = egui::Context::default();
        let controls = render(&mut app, &ctx);
        for label in [
            "Select all profile contours",
            "Clear profile selection",
            "Cut Climb",
            "Cut Conventional",
            "Order: Inner before outer",
            "Order: Selection order",
            "Stepdown",
            "Roughing feed",
            "Plunge feed",
            "Spindle speed",
            "Tool stepdown limit",
            "Through-cut allowance",
            "Profile top offset",
            "Profile bottom offset",
            "Profile CW",
        ] {
            assert!(controls.contains_key(label), "{label} missing");
        }
        // Every contour row and its three sides are named by the contour's own
        // qualified identity, not by a row position.
        assert!(
            controls
                .keys()
                .any(|key| key.starts_with("Profile contour artwork-1 / ")),
            "contour rows missing: {:?}",
            controls.keys().collect::<Vec<_>>()
        );
        for side in ["Inside", "Outside", "On contour"] {
            assert!(
                controls
                    .keys()
                    .any(|key| key.starts_with(&format!("Profile side {side} artwork-1 / "))),
                "{side} missing"
            );
        }
        // A profile has no carving mode and no V-bit tab content.
        assert!(!controls.contains_key("Combined"));
        assert!(!controls.contains_key("Endmill only"));
    }

    #[test]
    fn profile_fields_commit_into_the_profile_settings() {
        let mut app = app();
        let doc = app.document.as_mut().unwrap();
        doc.edit(8, "0.5".into()).unwrap();
        assert_eq!(
            crate::profile::settings_in(&doc.job, &doc.raw.operation)
                .unwrap()
                .stepdown_mm,
            Some(0.5)
        );
        doc.edit(89, "-0.2".into()).unwrap();
        assert_eq!(
            crate::profile::settings_in(&doc.job, &doc.raw.operation)
                .unwrap()
                .top
                .offset_mm,
            -0.2
        );
        // A field that belongs to another operation kind is refused with its
        // own reason instead of writing into the profile.
        assert!(
            doc.edit(1, "0.4".into()).is_err(),
            "wall allowance is a carving field"
        );
        doc.validate().unwrap();
    }
}
