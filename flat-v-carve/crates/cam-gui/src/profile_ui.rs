//! Profile operation editor (GUI8a basic closed profile, GUI8b tabs, GUI8c
//! radial finishing and GUI8d starts/entries).
//!
//! The panel only binds document fields and emits explicit document commands.
//! Which contours a profile cuts, on which side, how deep and in which order
//! all live in [`cam_core::project::v5::ProfileSettingsV5`]; the planner owns
//! the compensation, tab placement and entry geometry that the result
//! inspection and the viewport then display.
use super::*;
use crate::profile::{self, AnchorKind, SelectionRow};
use crate::resources::ResourceCommand as R;
use cam_core::project::{
    ContourOrder, ContourSide, CutDirection, HeightReference, LeadSpec, ProfileEntry,
    SpindleDirection, TabShape, TraversalDirection,
    v5::{
        StartSelectionV5,
        resources::{self as core, AssignmentRole as Role},
    },
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
                self.profile_tabs(ui, ctx);
                self.profile_finishing(ui, ctx);
                self.profile_starts(ui, ctx);
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

    /// The candidate anchors of the selected profile operation: where the
    /// document asks for tabs and for the start, walked along each contour's
    /// anchor ring. Display state; the document stays the authority.
    pub(crate) fn profile_candidates(&self) -> Vec<crate::viewport::ProfileAnchor> {
        let Some(document) = &self.document else {
            return vec![];
        };
        if self.operation_kind() != Some(crate::session::OperationKind::Profile) {
            return vec![];
        }
        let id = document.raw.operation.clone();
        let contours = self.view.profile_contours();
        let point_of = |scope: &str, fraction: f64| {
            let contour = contours.iter().find(|contour| contour.wire_id == scope)?;
            profile::ring_point(&contour.anchor_ring, fraction)
        };
        let mut out = vec![];
        if let Some(row) = profile::start_row(&document.job, &id)
            && let Some(point) = point_of(&row.scope, row.fraction)
        {
            out.push(crate::viewport::ProfileAnchor {
                scope: row.scope,
                kind: AnchorKind::Start,
                point,
            });
        }
        for row in profile::tab_rows(&document.job, &id) {
            if let Some(point) = point_of(&row.scope, row.fraction) {
                out.push(crate::viewport::ProfileAnchor {
                    scope: row.scope,
                    kind: AnchorKind::Tab,
                    point,
                });
            }
        }
        out
    }

    /// One anchor's own numeric row. The text is scoped to the contour the
    /// anchor belongs to, so switching rows never carries text across anchors.
    pub(super) fn anchor_number(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        scope: &str,
        kind: AnchorKind,
        field: usize,
        value: Option<f64>,
    ) {
        if !FIELDS[field]
            .to_lowercase()
            .contains(&self.search.to_lowercase())
        {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let mut text = document.anchor_text(scope, field, value);
        let response = ui
            .with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                let label = ui
                    .horizontal_wrapped(|ui| {
                        let response = ui.label(FIELDS[field]);
                        help::icon(ui, FIELDS[field]);
                        response
                    })
                    .inner;
                ui.horizontal(|ui| {
                    let response = ui
                        .add(
                            egui::TextEdit::singleline(&mut text)
                                .id(egui::Id::new((
                                    "carving-field",
                                    scope,
                                    &document.raw.operation,
                                    field,
                                )))
                                .desired_width((ui.available_width() - 58.).clamp(65., 160.))
                                .char_limit(128)
                                .hint_text("Unset"),
                        )
                        .labelled_by(label.id);
                    ui.small("share of the contour, 0 up to 1");
                    response
                })
                .inner
            })
            .inner;
        observe_control(FIELDS[field], response.rect);
        observe_control(&format!("Anchor {} {scope}", FIELDS[field]), response.rect);
        if self.issue_focus.as_deref() == Some(FIELDS[field]) {
            response.scroll_to_me(Some(egui::Align::Center));
            response.request_focus();
            self.issue_focus = None;
        }
        if response.changed() {
            if self.edit_group != Some(field) {
                self.remember();
                self.edit_group = Some(field);
            }
            let result =
                self.document
                    .as_mut()
                    .unwrap()
                    .edit_anchor(scope, field, text.clone(), kind);
            self.changed(ctx);
            self.status = match result {
                Ok(()) => "Setting changed; generate to update simulation.".into(),
                Err(error) => {
                    self.issues = vec![cam_core::operations::LocatedDiagnostic {
                        code: "EDITOR_VALUE".into(),
                        message: error.clone(),
                        operation_id: {
                            let selected = &self.document.as_ref().unwrap().raw.operation;
                            (!selected.is_empty()).then(|| selected.clone())
                        },
                        tool_id: None,
                        field_path: Some(format!("editor.fields.{}", FIELDS[field])),
                    }];
                    error
                }
            };
        }
        if response.lost_focus() {
            self.edit_group = None;
        }
        if let Err(error) = Draft::parse(&text) {
            ui.colored_label(Color32::from_rgb(176, 42, 35), error);
        }
    }

    fn profile_tool(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.operation_group(ui, "Tool & cutting", true, |app, ui| {
            app.profile_resources(ui, ctx);
            ui.separator();
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

    /// The profile's cutter and cutting values, addressed as the operation's
    /// single `Milling` assignment: the job tool in use, its Applied/Modified
    /// state, a job-tool picker and the loaded library's tools and cutting
    /// profiles (GUI5's reusable-tool workflow, applied to a profile).
    fn profile_resources(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let operation = self.operation_id();
        let Some(state) = core::assignment_statuses(&job)
            .into_iter()
            .find(|status| status.operation_id == operation && status.role == Role::Milling)
        else {
            return;
        };
        let tools = job
            .tools
            .iter()
            .filter(|tool| {
                matches!(
                    tool.geometry,
                    Some(cam_core::project::ToolGeometry::Endmill(_))
                )
            })
            .map(|tool| (tool.id.clone(), tool.name.clone()))
            .collect::<Vec<_>>();
        ui.strong(format!(
            "Job tool: {}",
            job.tools
                .iter()
                .find(|tool| tool.id == state.tool_id)
                .map(|tool| tool.name.as_str())
                .unwrap_or(&state.tool_id)
        ));
        ui.label(match state.status {
            core::ProfileStatus::Applied => "Cutting values: applied from a library profile",
            core::ProfileStatus::Modified => {
                "Cutting values: modified after the library profile was applied"
            }
            core::ProfileStatus::Custom => "Cutting values: custom — no library profile applied",
        });
        if let Some(applied) = &state.applied {
            ui.small(format!(
                "{} · copied revision {}",
                applied.name_at_application, applied.revision_at_application
            ));
        }
        ui.horizontal_wrapped(|ui| {
            if button(
                ui,
                "Reset profile overrides",
                state.status != core::ProfileStatus::Custom && self.active.is_none(),
            )
            .clicked()
            {
                self.resource_command(
                    R::Reset {
                        operation: operation.clone(),
                        role: Role::Milling,
                    },
                    ctx,
                );
            }
            let menu = ui.menu_button("Choose job endmill", |ui| {
                if tools.is_empty() {
                    ui.label("No job tool carries cutter geometry yet.");
                    return;
                }
                for (id, name) in &tools {
                    if button(ui, name, self.active.is_none()).clicked() {
                        self.resource_command(
                            R::UseTool {
                                operation: operation.clone(),
                                role: Role::Milling,
                                tool: id.clone(),
                            },
                            ctx,
                        );
                        ui.close();
                    }
                }
            });
            observe_control("Choose job endmill", menu.response.rect);
        });
        if !self.resources.ready && !self.resources.busy {
            self.request_resources(ResourceIntent::Load, ctx);
        }
        if let Some(catalog) = self.resources.base.as_ref().map(|s| s.snapshot.clone()) {
            let menu = ui.menu_button("Apply library endmill", |ui| {
                for tool in &catalog.library.tools {
                    if !matches!(
                        tool.geometry,
                        cam_core::tool_library::LibraryGeometry::Endmill(_)
                    ) {
                        continue;
                    }
                    if button(
                        ui,
                        &format!("{} · tool only", tool.name),
                        self.active.is_none(),
                    )
                    .clicked()
                    {
                        self.resource_command(
                            R::SelectLibraryTool {
                                catalog: catalog.clone(),
                                tool: tool.id.clone(),
                                operation: operation.clone(),
                                role: Role::Milling,
                            },
                            ctx,
                        );
                        ui.close();
                    }
                    for preset in &tool.cutting_presets {
                        if button(
                            ui,
                            &format!("{} / {}", tool.name, preset.name),
                            self.active.is_none(),
                        )
                        .clicked()
                        {
                            self.resource_command(
                                R::ApplyToolProfile {
                                    catalog: catalog.clone(),
                                    tool: tool.id.clone(),
                                    preset: preset.id.clone(),
                                    operation: operation.clone(),
                                    role: Role::Milling,
                                },
                                ctx,
                            );
                            ui.close();
                        }
                    }
                }
            });
            observe_control("Apply library endmill", menu.response.rect);
        } else {
            ui.small(
                "Load the tool library in Setup → Tool library to copy a reviewed cutter and its cutting profile here.",
            );
        }
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

    /// Tabs (GUI8b): visible material bridges that keep the part in the stock.
    /// Automatic placement states a count or a spacing; manual placement
    /// anchors one tab per selected contour at an explicit source fraction,
    /// which is the numeric equivalent of dragging its marker in the viewport.
    fn profile_tabs(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = profile::settings_in(&job, &id) else {
            return;
        };
        let tabs = settings.tabs.clone();
        let selection = profile::selection(&job, &id);
        let contours = self.view.profile_contours();
        let rows = profile::tab_rows(&job, &id);
        let cell = self.view.display_cell_mm();
        let owner_of = |wire: &str| {
            contours
                .iter()
                .find(|contour| contour.wire_id == wire)
                .map(|contour| {
                    format!(
                        "{} / {}",
                        contour.owner, contour.reference.local_geometry_id
                    )
                })
                .unwrap_or_else(|| wire.into())
        };
        // An operation that already carries tabs opens its own group; nothing
        // is hidden behind a collapsed header that the job depends on.
        self.operation_group(ui, "Tabs", tabs.is_some(), |app, ui| {
            let Some(tabs) = tabs.clone() else {
                ui.small("Tabs leave visible bridges of material that hold the part in the stock while the rest of the contour is cut through.");
                if button(ui, "Add tabs", app.active.is_none()).clicked() {
                    let id = app.operation_id();
                    app.edit_job(ctx, &[], move |job| {
                        profile::set_tabs_enabled(job, &id, true)
                    });
                }
                return;
            };
            if button(ui, "Cut without tabs", app.active.is_none()).clicked() {
                let id = app.operation_id();
                app.edit_job(ctx, &[93, 94, 95, 96, 97, 108], move |job| {
                    profile::set_tabs_enabled(job, &id, false)
                });
            }
            ui.separator();
            help::label(ui, "Tab shape");
            ui.horizontal_wrapped(|ui| {
                let r = ui.selectable_label(tabs.shape == TabShape::Rectangular, "Rectangular");
                observe_control("Rectangular", r.rect);
                if r.clicked() && tabs.shape != TabShape::Rectangular {
                    let id = app.operation_id();
                    app.edit_job(ctx, &[], move |job| {
                        profile::set_tab_shape(job, &id, TabShape::Rectangular)
                    });
                }
                let r = ui
                    .add_enabled(false, egui::Button::new("Ramped"))
                    .on_disabled_hover_text(
                        "Ramped tab shoulders are not implemented; the planner reports them instead of changing the shape.",
                    );
                observe_control("Ramped", r.rect);
            });
            if tabs.shape != TabShape::Rectangular {
                ui.colored_label(
                    Color32::from_rgb(164, 83, 12),
                    "This job stores ramped tabs, which this milestone does not cut: the planner reports it, and choosing Rectangular is the explicit change.",
                );
            }
            app.operation_numbers(ui, ctx, &[93, 94]);
            ui.small("Tab height is measured up from the physical stock bottom; the width is the protected band around the tab, left standing by rough and finish passes.");
            ui.separator();
            help::label(ui, "Tab placement");
            let automatic = matches!(tabs.placement, cam_core::project::v5::TabPlacementV5::Automatic { .. });
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [("Automatic placement", true), ("Manual anchors", false)] {
                    let r = ui.selectable_label(automatic == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && automatic != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[95, 96, 97], move |job| {
                            profile::set_tab_placement_mode(job, &id, value)
                        });
                    }
                }
            });
            if automatic {
                app.operation_numbers(ui, ctx, &[95, 96]);
                ui.small("Give a count or a spacing between tabs; a count takes precedence. The planner distributes them around each selected contour and reports what it placed.");
            } else {
                ui.small("Each manual tab is anchored on one selected contour. Use automatic placement for several tabs around a single contour.");
                if rows.is_empty() {
                    ui.label("No manual tabs yet.");
                }
                for row in &rows {
                    let label = owner_of(&row.scope);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if row.resolved {
                            format!("Tab on {label}")
                        } else {
                            format!("Tab on {label} (unresolved source)")
                        });
                    });
                    if !row.resolved {
                        let response = ui.colored_label(
                            Color32::DARK_RED,
                            "This anchor's source geometry changed.",
                        );
                        observe_control(&format!("Unresolved tab {}", row.index), response.rect);
                        let menu = ui.menu_button(
                            format!("Reattach tab {} to…", row.index),
                            |ui| {
                                for contour in selection.iter() {
                                    let wire = profile::anchor_scope(&contour.reference);
                                    let name = owner_of(&wire);
                                    if button(ui, &name, app.active.is_none()).clicked() {
                                        app.artwork_command(
                                            engine::ArtworkCommand::ProfileAnchorReattach {
                                                start: false,
                                                    index: row.index,
                                                    wire_id: wire,
                                                    fraction: Some(row.fraction),
                                            },
                                            ctx,
                                        );
                                        ui.close();
                                    }
                                }
                            },
                        );
                        observe_control(&format!("Reattach tab {}", row.index), menu.response.rect);
                    }
                    app.anchor_number(ui, ctx, &row.scope, AnchorKind::Tab, 97, Some(row.fraction));
                    if button(
                        ui,
                        &format!("Remove tab {}", row.index),
                        app.active.is_none(),
                    )
                    .clicked()
                    {
                        app.artwork_command(
                            engine::ArtworkCommand::ProfileTabAnchor {
                                action: profile::TabAnchorAction::Remove {
                                    scope: row.scope.clone(),
                                },
                            },
                            ctx,
                        );
                    }
                    ui.separator();
                }
                let anchored: Vec<String> = rows.iter().map(|row| row.scope.clone()).collect();
                let available: Vec<&crate::profile::Contour> = selection
                    .iter()
                    .filter_map(|row| {
                        let wire = profile::anchor_scope(&row.reference);
                        (!anchored.contains(&wire))
                            .then(|| contours.iter().find(|c| c.wire_id == wire))
                            .flatten()
                    })
                    .collect();
                let menu = ui.menu_button("Add tab on…", |ui| {
                    if available.is_empty() {
                        ui.label("Every selected contour already carries a manual tab.");
                        return;
                    }
                    ui.small("Choose a contour and a starting position; the tab can then be dragged in the viewport or typed exactly.");
                    for contour in &available {
                        for fraction in [0., 0.25, 0.5, 0.75] {
                            if button(
                                ui,
                                &format!(
                                    "{} / {} · {:.0}%",
                                    contour.owner,
                                    contour.reference.local_geometry_id,
                                    fraction * 100.
                                ),
                                app.active.is_none(),
                            )
                            .clicked()
                            {
                                app.artwork_command(
                                    engine::ArtworkCommand::ProfileTabAnchor {
                                        action: profile::TabAnchorAction::Add {
                                            wire_id: contour.wire_id.clone(),
                                            fraction,
                                        },
                                    },
                                    ctx,
                                );
                                ui.close();
                            }
                        }
                    }
                });
                observe_control("Add tab on", menu.response.rect);
                ui.small("Drag a tab marker in the viewport to move it; the released position is one undo step.");
            }
            ui.separator();
            ui.small(match cell {
                Some(cell) => format!(
                    "Display grid: {cell:.4} mm cells. A tab narrower than one cell may not appear in the heightfield; the drawn bridge boundary is exact and is not a raster claim."
                ),
                None => "Generate to see the display grid size; a tab narrower than one cell may not appear in the heightfield.".into(),
            });
            if app.view.is_profile() {
                ui.small("Yellow squares are the requested anchors; the green bridges are the plan's generated tab placements.");
            }
        });
    }

    /// Radial finishing (GUI8c): the rough passes leave a radial allowance and
    /// a final pass cuts the finished wall with its own feed. The planner owns
    /// the offsets; this group only states the values and the pass order.
    fn profile_finishing(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = profile::settings_in(&job, &id) else {
            return;
        };
        let enabled = settings.finish.enabled;
        let on_contour = settings
            .contours
            .iter()
            .any(|contour| contour.side == ContourSide::On);
        self.operation_group(ui, "Radial finishing", false, |app, ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [("Add radial finishing", true), ("Rough passes only", false)]
                {
                    let r = ui.selectable_label(enabled == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && enabled != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[91, 92], move |job| {
                            profile::set_finish_enabled(job, &id, value)
                        });
                    }
                }
            });
            if !enabled {
                ui.small("The rough passes cut straight to the finished wall in depth steps. Turning finishing on keeps a radial allowance for a final pass.");
                return;
            }
            ui.small("The rough passes stand off the wall by the allowance; the finishing pass then cuts the finished wall, depth-stepped like the rough work, at the finishing feed.");
            app.operation_numbers(ui, ctx, &[91, 92]);
            ui.small("Zero allowance is allowed and means the finishing pass follows the same wall as the roughing; the feed still applies. Both passes use this operation's one tool assignment — only the feed differs.");
            if on_contour {
                ui.colored_label(
                    Color32::from_rgb(164, 83, 12),
                    "A zero-offset (on-contour) selection cannot carry a radial allowance; set the allowance to zero or give the contour a retained side.",
                );
            }
        });
    }

    /// Starts and entries (GUI8d): where the cut begins, how it descends and
    /// whether it approaches and leaves the contour with a lead. Every choice
    /// is a document value; the planner resolves the seam, the ramp and the
    /// lead geometry and reports a located reason when one does not fit.
    fn profile_starts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.operation_id();
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(settings) = profile::settings_in(&job, &id) else {
            return;
        };
        let start = settings.start.clone();
        let entry = settings.entry.clone();
        let lead_in = settings.lead_in.clone();
        let lead_out = settings.lead_out.clone();
        let row = profile::start_row(&job, &id);
        let selection = profile::selection(&job, &id);
        let contours = self.view.profile_contours();
        let owner_of = |wire: &str| {
            contours
                .iter()
                .find(|contour| contour.wire_id == wire)
                .map(|contour| {
                    format!(
                        "{} / {}",
                        contour.owner, contour.reference.local_geometry_id
                    )
                })
                .unwrap_or_else(|| wire.into())
        };
        self.operation_group(ui, "Start & entry", true, |app, ui| {
            help::label(ui, "Cut start");
            let anchored = matches!(start, StartSelectionV5::Anchor(_));
            if anchored {
                let row = row.clone().expect("an anchored start has a row");
                ui.label(format!("Anchored start on {}", owner_of(&row.scope)));
                app.anchor_number(ui, ctx, &row.scope, AnchorKind::Start, 108, Some(row.fraction));
                if !row.resolved {
                    let response = ui.colored_label(
                        Color32::DARK_RED,
                        "This start's source geometry changed; reattach it or return to the automatic seam.",
                    );
                    observe_control("Unresolved start anchor", response.rect);
                    let menu = ui.menu_button("Reattach start to…", |ui| {
                        for selection_row in selection.iter() {
                            let wire = profile::anchor_scope(&selection_row.reference);
                            let name = owner_of(&wire);
                            if button(ui, &name, app.active.is_none()).clicked() {
                                app.artwork_command(
                                    engine::ArtworkCommand::ProfileAnchorReattach {
                                        start: true,
                                        index: 0,
                                        wire_id: wire,
                                        fraction: Some(row.fraction),
                                    },
                                    ctx,
                                );
                                ui.close();
                            }
                        }
                    });
                    observe_control("Reattach start anchor", menu.response.rect);
                }
                if button(ui, "Use automatic start", app.active.is_none()).clicked() {
                    app.artwork_command(
                        engine::ArtworkCommand::ProfileStart {
                            wire_id: None,
                            fraction: 0.,
                        },
                        ctx,
                    );
                }
                ui.small("Automatic keeps the source seam the planner already uses.");
            } else {
                ui.label("Automatic start: the planner cuts from the source contour's own seam.");
                let menu = ui.menu_button("Anchor the start…", |ui| {
                    if selection.is_empty() {
                        ui.label("Select contours first.");
                        return;
                    }
                    ui.small("Choose a contour and a starting position; the start can then be dragged in the viewport or typed exactly.");
                    for selection_row in selection.iter() {
                        let wire = profile::anchor_scope(&selection_row.reference);
                        let name = owner_of(&wire);
                        for fraction in [0., 0.25, 0.5, 0.75] {
                            if button(
                                ui,
                                &format!("{name} · {:.0}%", fraction * 100.),
                                app.active.is_none(),
                            )
                            .clicked()
                            {
                                app.artwork_command(
                                    engine::ArtworkCommand::ProfileStart {
                                        wire_id: Some(wire.clone()),
                                        fraction,
                                    },
                                    ctx,
                                );
                                ui.close();
                            }
                        }
                    }
                });
                observe_control("Anchor the start", menu.response.rect);
            }
            ui.small("Drag the yellow square in the viewport to move the start; the release is one undo step and the numeric row states the same value.");
            ui.separator();
            help::label(ui, "Entry");
            let ramp = matches!(entry, ProfileEntry::Ramp { .. });
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [("Plunge entry", false), ("Ramp entry", true)] {
                    let r = ui.selectable_label(ramp == value, label);
                    observe_control(label, r.rect);
                    if r.clicked() && ramp != value {
                        let id = app.operation_id();
                        app.edit_job(ctx, &[98, 99], move |job| {
                            profile::set_entry(job, &id, value)
                        });
                    }
                }
            });
            if ramp {
                app.operation_numbers(ui, ctx, &[98, 99]);
                ui.small("A ramp descends along the loop instead of straight down. It needs a tool you mark ramp capable, and it must fit before any tab bridge — the planner reports a located reason otherwise.");
            } else {
                ui.small("A plunge descends straight at the seam. Choose a ramp to enter along the contour instead.");
            }
            ui.separator();
            for (label, out, spec) in [("Lead-in", false, &lead_in), ("Lead-out", true, &lead_out)] {
                help::label(ui, label);
                ui.horizontal_wrapped(|ui| {
                    for (name, value) in [
                        ("No lead", LeadSpec::None),
                        ("Tangent line", LeadSpec::TangentLine {
                            length_mm: None,
                            feed_mm_min: None,
                        }),
                        ("Tangent arc", LeadSpec::TangentArc {
                            radius_mm: None,
                            sweep_deg: None,
                            feed_mm_min: None,
                        }),
                    ] {
                        let selected = matches!(
                            (&value, spec),
                            (LeadSpec::None, LeadSpec::None)
                                | (
                                    LeadSpec::TangentLine { .. },
                                    LeadSpec::TangentLine { .. }
                                )
                                | (LeadSpec::TangentArc { .. }, LeadSpec::TangentArc { .. })
                        );
                        let control = format!("{label}: {name}");
                        let r = ui.selectable_label(selected, name);
                        observe_control(&control, r.rect);
                        if r.clicked() && !selected {
                            let id = app.operation_id();
                            app.edit_job(ctx, &[100, 101, 102, 103, 104, 105, 106, 107], move |job| {
                                profile::set_lead(job, &id, out, value)
                            });
                        }
                    }
                });
                match spec {
                    LeadSpec::None => {
                        ui.small("The cut starts and ends on the contour.");
                    }
                    LeadSpec::TangentLine { .. } => {
                        app.operation_numbers(
                            ui,
                            ctx,
                            if out { &[104, 105] } else { &[100, 101] },
                        );
                        ui.small("A tangent line is collinear with the loop: it approaches along the contour's own direction.");
                    }
                    LeadSpec::TangentArc { .. } => {
                        app.operation_numbers(
                            ui,
                            ctx,
                            if out { &[106, 107, 105] } else { &[102, 103, 101] },
                        );
                        ui.small("A tangent arc bulges into the scrap side, so it needs a retained side (inside or outside) and room beside the contour and its tabs.");
                    }
                }
            }
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
                        "Finishing allowance {} mm at {} mm/min — the rough centerline adds the allowance to the cutter radius, the finish pass cuts at the radius",
                        settings
                            .finish
                            .radial_allowance_mm
                            .map(|v| format!("{v:.4}"))
                            .unwrap_or_else(|| "unset".into()),
                        settings
                            .finish
                            .feed_mm_min
                            .map(|v| format!("{v:.1}"))
                            .unwrap_or_else(|| "unset".into()),
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

    /// Turn tabs on with one manual anchor on the first selected contour.
    fn manual_tabs(app: &mut App) -> String {
        let doc = app.document.as_mut().unwrap();
        let id = doc.raw.operation.clone();
        crate::profile::set_tabs_enabled(&mut doc.job, &id, true).unwrap();
        crate::profile::set_tab_placement_mode(&mut doc.job, &id, false).unwrap();
        doc.edit(93, "0.5".into()).unwrap();
        doc.edit(94, "2".into()).unwrap();
        let scope = crate::profile::selection(&doc.job, &id)[0]
            .reference
            .clone();
        let scope = crate::profile::anchor_scope(&scope);
        crate::profile::add_tab_anchor(&mut doc.job, &id, &scope, 0.5).unwrap();
        scope
    }

    #[test]
    fn tab_controls_state_their_placement_mode_and_anchor_rows() {
        let mut app = app();
        let ctx = egui::Context::default();
        let doc = app.document.as_mut().unwrap();
        crate::profile::set_tabs_enabled(&mut doc.job, &doc.raw.operation, true).unwrap();
        doc.edit(93, "0.5".into()).unwrap();
        doc.edit(94, "3".into()).unwrap();
        let controls = render(&mut app, &ctx);
        for label in [
            "Cut without tabs",
            "Rectangular",
            "Ramped",
            "Automatic placement",
            "Manual anchors",
            "Tab height",
            "Tab width",
            "Tab count",
            "Tab spacing",
        ] {
            assert!(controls.contains_key(label), "{label} missing");
        }
        let scope = manual_tabs(&mut app);
        let controls = render(&mut app, &ctx);
        assert!(
            controls.contains_key("Add tab on"),
            "the add menu is missing"
        );
        assert!(
            controls
                .keys()
                .any(|key| key == &format!("Anchor Tab anchor fraction {scope}")),
            "the anchor's own numeric row is missing: {:?}",
            controls.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_anchor_draft_stays_with_its_own_contour() {
        let mut app = app();
        let scope = manual_tabs(&mut app);
        let doc = app.document.as_mut().unwrap();
        let id = doc.raw.operation.clone();
        // A committed move leaves no pending text.
        doc.edit_anchor(&scope, 97, "0.4".into(), AnchorKind::Tab)
            .unwrap();
        assert_eq!(
            crate::profile::tab_rows(&doc.job, &id)[0].fraction,
            0.4,
            "the anchor's own row defines its value"
        );
        assert!(!doc.pending());
        // Partial text stays attached to that anchor and blocks generate/save.
        assert!(
            doc.edit_anchor(&scope, 97, "-".into(), AnchorKind::Tab)
                .is_err()
        );
        assert_eq!(doc.anchor_text(&scope, 97, Some(0.4)), "-");
        assert!(doc.pending(), "partial text is pending");
        // The scoped draft validates and survives a recovery round trip.
        doc.validate().unwrap();
        let recovered =
            crate::state::Draft::recover(&serde_json::to_string(&doc.raw).unwrap()).unwrap();
        assert_eq!(recovered, doc.raw);
        // Anchor text must be scoped to a qualified contour whose owner is in
        // the document: a malformed scope or a gone artwork item is rejected.
        let mut invalid = doc.raw.clone();
        invalid.raw.insert(
            format!(
                "gone:contour:letter/{}/{}",
                doc.raw.operation,
                crate::state::FIELDS[97]
            ),
            "0.5".into(),
        );
        assert!(invalid.validate_job(&doc.job).is_err());
        invalid.raw.clear();
        invalid.raw.insert(
            format!(
                "not-a-wire-id/{}/{}",
                doc.raw.operation,
                crate::state::FIELDS[97]
            ),
            "0.5".into(),
        );
        assert!(invalid.validate_job(&doc.job).is_err());
    }

    #[test]
    fn a_profile_selection_reply_is_adopted_not_reported_as_unknown() {
        let mut app = app();
        let ctx = egui::Context::default();
        let id = app.operation_id();
        let job = app.document.as_ref().unwrap().job.clone();
        // Re-select only the two outer contours: the reply must arrive, be
        // adopted into the document and keep the selection semantics.
        let rows = crate::profile::selection(&job, &id)
            .into_iter()
            .filter(|row| {
                crate::profile::contours(&job)
                    .unwrap()
                    .iter()
                    .any(|contour| contour.reference == row.reference && contour.role == "outer")
            })
            .collect::<Vec<_>>();
        let (meta, payload) = crate::session::execute(
            &mut Retained::new(),
            Command::Artwork {
                job: job.to_json().unwrap(),
                operation_id: id.clone(),
                action: engine::ArtworkCommand::ProfileSelection { rows },
            },
        )
        .unwrap();
        let request = app.id();
        app.active = Some((request, app.revision));
        app.accept(request, Ok((meta, payload)), &ctx);
        assert_eq!(
            app.status,
            "Operation geometry updated. Undo restores the previous selection."
        );
        assert_eq!(
            crate::profile::selection(&app.document.as_ref().unwrap().job, &id).len(),
            2,
            "the adopted document carries only the outer contours"
        );
    }

    #[test]
    fn the_tool_group_offers_the_job_tool_and_library_actions() {
        let mut app = app();
        let ctx = egui::Context::default();
        let controls = render(&mut app, &ctx);
        // The profile addresses its single milling assignment through the same
        // role-aware actions the carving stages use: choose a job tool, reset
        // the applied baseline, and load the library when it is not resident.
        for label in [
            "Choose job endmill",
            "Reset profile overrides",
            "Clear profile cutting values",
        ] {
            assert!(controls.contains_key(label), "{label} missing");
        }
        // The old ad-hoc job-tool combo is gone: every tool change now goes
        // through the resource commands that keep provenance and baselines.
        assert!(!controls.contains_key("Profile assignment tool"));
    }

    #[test]
    fn start_entry_and_lead_controls_state_their_supported_choices() {
        let mut app = app();
        let ctx = egui::Context::default();
        let controls = render(&mut app, &ctx);
        for label in [
            "Anchor the start",
            "Plunge entry",
            "Ramp entry",
            "Lead-in: No lead",
            "Lead-in: Tangent line",
            "Lead-in: Tangent arc",
            "Lead-out: No lead",
            "Lead-out: Tangent line",
            "Lead-out: Tangent arc",
        ] {
            assert!(controls.contains_key(label), "{label} missing");
        }
        // Choosing a ramp states its own two values; the mode change leaves
        // them unset so the planner reports what is missing.
        let doc = app.document.as_mut().unwrap();
        let id = doc.raw.operation.clone();
        crate::profile::set_entry(&mut doc.job, &id, true).unwrap();
        assert!(
            crate::profile::settings_in(&doc.job, &id).unwrap().entry
                == cam_core::project::ProfileEntry::Ramp {
                    max_angle_deg: None,
                    feed_mm_min: None,
                }
        );
        doc.edit(98, "15".into()).unwrap();
        doc.edit(99, "90".into()).unwrap();
        let scope =
            crate::profile::anchor_scope(&crate::profile::selection(&doc.job, &id)[0].reference);
        crate::profile::set_lead(
            &mut doc.job,
            &id,
            false,
            cam_core::project::LeadSpec::TangentLine {
                length_mm: None,
                feed_mm_min: None,
            },
        )
        .unwrap();
        crate::profile::set_start(&mut doc.job, &id, Some(&scope), 0.25).unwrap();
        let controls = render(&mut app, &ctx);
        for label in [
            "Entry ramp angle",
            "Entry ramp feed",
            "Lead-in length",
            "Lead-in feed",
        ] {
            assert!(controls.contains_key(label), "{label} missing");
        }
        // A start anchor adds its own scoped numeric row.
        assert!(
            controls
                .keys()
                .any(|key| key == &format!("Anchor Start fraction {scope}")),
            "the start's own numeric row is missing: {:?}",
            controls.keys().collect::<Vec<_>>()
        );
        assert!(controls.contains_key("Use automatic start"));
    }
}
