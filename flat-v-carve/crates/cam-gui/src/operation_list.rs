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
        if self
            .document
            .as_mut()
            .is_some_and(|d| d.select_operation(id))
        {
            self.edit_group = None;
            self.search.clear();
            self.operation_scroll = [0.; 3];
            self.operation_ramp_draft = false;
            self.navigate(2);
            self.recovery.changed(ctx.input(|i| i.time));
        }
    }

    pub(super) fn operation_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let idle = self.active.is_none() && self.io.is_none();
        let operations: Vec<(String, String, bool, bool)> = self
            .document
            .as_ref()
            .map(|d| {
                d.job
                    .operations
                    .iter()
                    .map(|op| {
                        (
                            op.id.clone(),
                            op.name.clone(),
                            op.enabled,
                            matches!(
                                op.settings,
                                cam_core::project::v5::OperationSettingsV5::Face(_)
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selected = self
            .document
            .as_ref()
            .map(|d| d.raw.operation.clone())
            .unwrap_or_default();
        let count = operations.len();
        for (index, (id, name, enabled, _)) in operations.iter().enumerate() {
            let label = format!("{:02}  {name}", index + 1);
            ui.horizontal(|ui| {
                let response = ui.add_sized(
                    [ui.available_width() - 26., 32.],
                    egui::Button::selectable(selected == *id, label).truncate(),
                );
                observe_control(&format!("Operation row {id}"), response.rect);
                if response.clicked() {
                    self.select_operation(id, ctx);
                }
                let mut on = *enabled;
                let toggle = ui
                    .add_enabled(idle, egui::Checkbox::without_text(&mut on))
                    .on_hover_text(if on {
                        "Enabled: included in every generation scope"
                    } else {
                        "Disabled: excluded from every scope; dependents see missing outputs"
                    });
                observe_control(&format!("Operation enabled {id}"), toggle.rect);
                if toggle.changed() {
                    self.operation_command(
                        Action::SetEnabled {
                            operation_id: id.clone(),
                            enabled: on,
                        },
                        ctx,
                    );
                }
            });
        }
        if let Some((rename_id, rename_text)) = self.operation_rename.clone() {
            let mut text = rename_text;
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(egui::Id::new(("operation-rename", &rename_id)))
                        .hint_text("Operation name"),
                );
                observe_control("Rename operation", edit.rect);
                if edit.changed() {
                    self.operation_rename = Some((rename_id.clone(), text.clone()));
                }
                if button(ui, "Apply name", idle && !text.trim().is_empty()).clicked() {
                    self.operation_command(
                        Action::Rename {
                            operation_id: rename_id.clone(),
                            name: text.clone(),
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
        if let Some((id, name, _, _)) = operations.iter().find(|(id, ..)| *id == selected).cloned()
        {
            let index = operations
                .iter()
                .position(|(candidate, ..)| *candidate == id)
                .unwrap_or(0);
            ui.horizontal_wrapped(|ui| {
                if button(ui, "Move earlier", idle && index > 0).clicked() {
                    self.operation_command(
                        Action::Move {
                            operation_id: id.clone(),
                            to_index: index - 1,
                        },
                        ctx,
                    );
                }
                if button(ui, "Move later", idle && index + 1 < count).clicked() {
                    self.operation_command(
                        Action::Move {
                            operation_id: id.clone(),
                            to_index: index + 1,
                        },
                        ctx,
                    );
                }
                if button(ui, "Rename", idle).clicked() {
                    self.operation_rename = Some((id.clone(), name.clone()));
                }
                if button(ui, "Delete operation", idle).clicked() {
                    self.operation_command(
                        Action::Delete {
                            operation_id: id.clone(),
                        },
                        ctx,
                    );
                }
            });
            let ready = !self.operation_ramp_draft
                && self
                    .document
                    .as_ref()
                    .is_some_and(|d| !d.pending() && !d.job.operations.is_empty());
            let through = button(ui, &format!("Generate through {name}"), idle && ready);
            observe_control("Generate through operation", through.rect);
            if through.clicked() {
                self.generate(
                    GenerateScope::ThroughOperation {
                        operation_id: id.clone(),
                    },
                    ctx,
                );
            }
            ui.small(
                "A prefix generation binds this operation and every enabled operation before it; export then prepares exactly that scope.",
            );
        }
        let job = self.current_job();
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
