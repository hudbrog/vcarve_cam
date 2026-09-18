//! Operations list: the workspace's sole execution order.
//!
//! Every row is one stable operation ID: enable/disable, move, rename and
//! delete all go through the shared schema-5 operation commands, so a
//! reordering that breaks a height dependency stays a saveable unresolved
//! draft with a located issue instead of silently changing anything else.
use super::*;
use crate::operation_authoring::{self, Action, Kind};
use crate::session::GenerateScope;

impl App {
    pub(super) fn operation_command(&mut self, action: Action, ctx: &egui::Context) {
        let job = self
            .document
            .as_ref()
            .map(|d| d.job.clone())
            .unwrap_or_else(operation_authoring::empty_job);
        self.submit(
            Command::Operation {
                job: job.to_json().unwrap(),
                action,
            },
            ctx,
        );
    }

    /// Select the operation the inspector edits. Selection is workspace state:
    /// it never changes the document or the machining order.
    pub(super) fn select_operation(&mut self, id: &str, ctx: &egui::Context) {
        let same = self
            .document
            .as_ref()
            .is_some_and(|d| d.raw.operation == id);
        if self
            .document
            .as_mut()
            .is_some_and(|d| d.select_operation(id))
        {
            self.edit_group = None;
            self.search.clear();
            if !same {
                self.operation_scroll = [0.; 3];
                self.operation_ramp_draft = false;
            }
            self.navigate(2);
            self.recovery.changed(ctx.input(|i| i.time));
        }
    }

    pub(super) fn operation_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use crate::{
            ui_icons::{self, Icon},
            ui_widgets,
        };
        use cam_core::project::v5::{OperationSettingsV5 as Settings, resources};
        let idle = self.active.is_none() && self.io.is_none();
        let job = self.current_job();
        let assignments = resources::assignment_statuses(&job);
        let selected = self
            .document
            .as_ref()
            .map(|d| d.raw.operation.as_str())
            .unwrap_or("")
            .to_owned();
        let count = job.operations.len();
        if count == 0 {
            ui.small("No operations yet");
        }
        for (index, op) in job.operations.iter().enumerate() {
            let id = &op.id;
            let icon = match op.settings {
                Settings::Face(_) => Icon::Face,
                Settings::FlatVcarve(_) => Icon::Carve,
                Settings::Profile(_) => Icon::Profile,
                Settings::DragKnife(_) => Icon::Knife,
            };
            let tools: Vec<_> = assignments
                .iter()
                .filter(|a| a.operation_id == *id)
                .map(|a| {
                    job.tools
                        .iter()
                        .find(|t| t.id == a.tool_id)
                        .filter(|t| t.geometry.is_some())
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| "Tool unset".into())
                })
                .collect();
            let issue = self
                .issues
                .iter()
                .any(|i| i.operation_id.as_deref() == Some(id));
            let detail = format!(
                "{}{}{}",
                if op.enabled { "" } else { "Disabled · " },
                if issue { "Issue · " } else { "" },
                tools.join(" / ")
            );
            ui.push_id(id, |ui| {
                ui.horizontal(|ui| {
                    let mut on = op.enabled;
                    let toggle = ui
                        .add_enabled(idle, egui::Checkbox::without_text(&mut on))
                        .on_hover_text(
                            "Include this operation when it falls within the generated scope",
                        );
                    observe_control(&format!("Operation enabled {id}"), toggle.rect);
                    let label = format!("{:02}  {}", index + 1, op.name);
                    let response = ui
                        .allocate_ui_with_layout(
                            egui::vec2((ui.available_width() - 36.).max(20.), 46.),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui_widgets::navigation_row(
                                    ui,
                                    &label,
                                    Some(&detail),
                                    icon,
                                    selected == *id && self.inspector_tab == 2,
                                )
                            },
                        )
                        .inner;
                    observe_control(&format!("Operation row {id}"), response.rect);
                    if selected == *id {
                        observe_control("Cutting", response.rect);
                    }
                    if response.clicked() {
                        self.select_operation(id, ctx);
                    }
                    if toggle.changed() {
                        self.operation_command(
                            Action::SetEnabled {
                                operation_id: id.clone(),
                                enabled: on,
                            },
                            ctx,
                        );
                    }
                    let menu =
                        ui_icons::button(ui, Icon::More, &format!("Actions for {label}"), false);
                    observe_control(&format!("Operation actions {id}"), menu.rect);
                    if selected == *id {
                        observe_control("Operation actions", menu.rect);
                    }
                    egui::Popup::menu(&menu).show(|ui| {
                        ui.strong(&label);
                        ui.small(id);
                        ui.separator();
                        if button(ui, "Rename", idle).clicked() {
                            self.operation_rename = Some((id.clone(), op.name.clone()));
                            ui.close();
                        }
                        if button(ui, "Move earlier", idle && index > 0).clicked() {
                            self.operation_command(
                                Action::Move {
                                    operation_id: id.clone(),
                                    to_index: index - 1,
                                },
                                ctx,
                            );
                            ui.close();
                        }
                        if button(ui, "Move later", idle && index + 1 < count).clicked() {
                            self.operation_command(
                                Action::Move {
                                    operation_id: id.clone(),
                                    to_index: index + 1,
                                },
                                ctx,
                            );
                            ui.close();
                        }
                        let ready = idle
                            && !self.operation_ramp_draft
                            && self.document.as_ref().is_some_and(|d| !d.pending());
                        let through = button(ui, "Generate through here", ready);
                        observe_control("Generate through operation", through.rect);
                        if through.clicked() {
                            self.generate(
                                GenerateScope::ThroughOperation {
                                    operation_id: id.clone(),
                                },
                                ctx,
                            );
                            ui.close();
                        }
                        ui.separator();
                        if button(ui, "Delete operation", idle).clicked() {
                            self.operation_command(
                                Action::Delete {
                                    operation_id: id.clone(),
                                },
                                ctx,
                            );
                            ui.close();
                        }
                    });
                })
            });
            if self
                .operation_rename
                .as_ref()
                .is_some_and(|(rename, _)| rename == id)
            {
                let mut text = self.operation_rename.as_ref().unwrap().1.clone();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(egui::Id::new(("operation-rename", id)))
                        .hint_text("Operation name")
                        .desired_width(ui.available_width()),
                );
                observe_control("Rename operation", edit.rect);
                if edit.changed() {
                    self.operation_rename = Some((id.clone(), text.clone()));
                }
                ui.horizontal(|ui| {
                    if button(ui, "Apply name", idle && !text.trim().is_empty()).clicked() {
                        self.operation_command(
                            Action::Rename {
                                operation_id: id.clone(),
                                name: text,
                            },
                            ctx,
                        );
                        self.operation_rename = None;
                    }
                    if button(ui, "Cancel rename", true).clicked() {
                        self.operation_rename = None;
                    }
                });
            }
        }
        let menu = ui.menu_button("+ Add operation", |ui| {
            for (label, kind) in [
                ("Add Face", Kind::Face),
                ("Add Flat V-carve", Kind::FlatVcarve),
                ("Add Profile", Kind::Profile),
                ("Add drag knife", Kind::DragKnife),
            ] {
                let label = format!("{label} — {}", operation_authoring::next_id(&job, kind));
                if button(
                    ui,
                    &label,
                    idle && count < operation_authoring::MAX_OPERATIONS,
                )
                .clicked()
                {
                    self.operation_command(operation_authoring::add(kind, &job), ctx);
                    ui.close();
                }
            }
        });
        observe_control("Add operation", menu.response.rect);
    }
    fn current_job(&self) -> CamJobV5 {
        self.document
            .as_ref()
            .map(|d| d.job.clone())
            .unwrap_or_else(operation_authoring::empty_job)
    }

    pub(super) fn adopt_operation(
        &mut self,
        job: CamJobV5,
        active: Option<&str>,
        ctx: &egui::Context,
    ) {
        let previous = self
            .document
            .as_ref()
            .map(|d| d.raw.operation.clone())
            .unwrap_or_default();
        let previous_job = self.document.as_ref().map(|d| d.job.clone());
        if self.document.is_none() {
            self.document = Some(Document::new(operation_authoring::empty_job()));
        }
        self.remember();
        let mut next = Document::new(job);
        if let Some(old) = &self.document {
            // Raw text is keyed by stable operation/artwork identity, so it
            // follows its own entity through add, delete and reorder.
            next.raw.raw = old.raw.raw.clone();
            next.raw.artwork_item = old.raw.artwork_item.clone();
        }
        next.sync_artwork();
        next.raw.operation = active
            .map(str::to_owned)
            .filter(|id| {
                next.job
                    .operations
                    .iter()
                    .any(|operation| &operation.id == id)
            })
            .or_else(|| {
                Some(previous.clone()).filter(|id| {
                    !id.is_empty()
                        && next
                            .job
                            .operations
                            .iter()
                            .any(|operation| &operation.id == id)
                })
            })
            .or_else(|| {
                // A document that replaced its operations (a fresh job, or an
                // add into an empty one) selects whatever is present.
                let added = next.job.operations.iter().find(|operation| {
                    previous_job
                        .as_ref()
                        .is_none_or(|job| !job.operations.iter().any(|old| old.id == operation.id))
                });
                added.map(|operation| operation.id.clone())
            })
            .unwrap_or_else(|| {
                next.job
                    .operations
                    .first()
                    .map(|operation| operation.id.clone())
                    .unwrap_or_default()
            });
        let empty = next.job.operations.is_empty();
        self.document = Some(next);
        self.edit_group = None;
        self.plan = None;
        self.plan_scope = None;
        self.plan_fingerprint = None;
        self.prepared = None;
        self.export_dialog = None;
        self.retained_save = None;
        self.retry = false;
        self.resume_view = None;
        self.operation_ramp_draft = false;
        self.resources.jobs_open = false;
        self.resources.open = false;
        self.issues.clear();
        self.navigate(2);
        self.search.clear();
        self.simulate = false;
        self.changed(ctx);
        self.status = if empty {
            "Operation deleted. Artwork, stock, tools and machine retained. Add an operation or Undo.".into()
        } else {
            "Operations updated. Select a row to edit it; Generate a prefix to inspect one operation's result.".into()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checkbox column and the name-button column are each one straight
    /// line, whatever the names weigh, and a row never asks the fixed-width
    /// panel for more space than it has.
    #[test]
    fn operation_rows_line_up_in_one_column() {
        let empty = operation_authoring::empty_job();
        let job = operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty))
            .unwrap();
        let job =
            operation_authoring::apply(&job, operation_authoring::add(Kind::FlatVcarve, &job))
                .unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1298., 834.),
                )),
                ..Default::default()
            },
            |ctx| app.navigator(ctx),
        );
        let ids: Vec<String> = app
            .document
            .as_ref()
            .map(|d| {
                d.job
                    .operations
                    .iter()
                    .map(|o| o.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let rows: Vec<[f32; 4]> = ids
            .iter()
            .filter_map(|id| crate::app::control_rect(&format!("Operation row {id}")))
            .collect();
        let toggles: Vec<[f32; 4]> = ids
            .iter()
            .filter_map(|id| crate::app::control_rect(&format!("Operation enabled {id}")))
            .collect();
        assert_eq!(rows.len(), 2, "one row per operation: {ids:?}");
        assert_eq!(toggles.len(), 2, "one checkbox per operation: {ids:?}");
        // The checkbox leads the row, so a wide name cannot push its own
        // checkbox out of line with the other rows'.
        assert!(
            (toggles[0][0] - toggles[1][0]).abs() < 0.5,
            "checkboxes share one left edge: {toggles:?}"
        );
        // The name buttons span to the same right edge: the justified,
        // truncating frame always fills the rest of the row.
        assert!(
            (rows[0][2] - rows[1][2]).abs() < 0.5,
            "buttons share one right edge: {rows:?}"
        );
        // A row that leaves the panel widens the scroll content, which
        // grows the panel's reserved space past its painted width — the
        // growth shows up as a black band at the panel edge.
        for rect in rows.iter().chain(toggles.iter()) {
            assert!(
                rect[2] <= crate::ui_theme::NAVIGATOR,
                "row stays inside the panel: {rect:?} of {rows:?} {toggles:?}"
            );
        }
    }

    /// Rows that asked for more width than the navigator had grew its reserved space past its
    /// painted width, and the gap between the two showed through as a black
    /// band between the panel and the canvas.
    #[test]
    fn navigator_content_stays_within_its_default_width() {
        let empty = operation_authoring::empty_job();
        let job = operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty))
            .unwrap();
        let job =
            operation_authoring::apply(&job, operation_authoring::add(Kind::FlatVcarve, &job))
                .unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1298., 834.),
                )),
                ..Default::default()
            },
            |ctx| {
                App::theme(ctx);
                app.ui(ctx);
            },
        );
        let panel_fill = Color32::from_rgb(237, 242, 246);
        let navigator: Vec<egui::Rect> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if rect.fill == panel_fill
                        && rect.rect.min.x == 0.
                        && rect.rect.height() > 400. =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect();
        assert!(
            navigator
                .iter()
                .any(|rect| rect.max.x <= crate::ui_theme::NAVIGATOR + 1.),
            "the navigator paints within its default width: {navigator:?}"
        );
    }

    #[test]
    fn operation_rows_start_at_the_same_left_edge() {
        let empty = operation_authoring::empty_job();
        let job = operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty))
            .unwrap();
        let job =
            operation_authoring::apply(&job, operation_authoring::add(Kind::FlatVcarve, &job))
                .unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280., 800.),
                )),
                ..Default::default()
            },
            |ctx| app.navigator(ctx),
        );
        // There is one painted label per operation. Both start at the same x.
        let texts: Vec<(String, egui::Pos2)> = output
            .shapes
            .into_iter()
            .filter_map(|clipped| match clipped.shape {
                egui::Shape::Text(text) => Some((text.galley.text().trim().to_owned(), text.pos)),
                _ => None,
            })
            .filter(|(text, _)| text.starts_with("01") || text.starts_with("02"))
            .collect();
        let mut closest = (f32::INFINITY, 0., 0.);
        for (first_text, first) in &texts {
            if !first_text.starts_with("01") {
                continue;
            }
            for (second_text, second) in &texts {
                if second_text.starts_with("02") {
                    let gap = (first.y - second.y).abs();
                    if gap < closest.0 {
                        closest = (gap, first.x, second.x);
                    }
                }
            }
        }
        assert!(
            closest.0 < 56.,
            "the two operation rows are adjacent: {:?}",
            texts
        );
        assert!(
            (closest.1 - closest.2).abs() < 0.5,
            "both names start at the same x: {closest:?} of {texts:?}"
        );
    }
}
