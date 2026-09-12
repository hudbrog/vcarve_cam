use super::*;
use crate::operation_authoring::{self, Action};

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
    pub(super) fn operation_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let idle = self.active.is_none() && self.io.is_none();
        let has_operation = self
            .document
            .as_ref()
            .is_some_and(|d| !d.job.operations.is_empty());
        let menu = ui.menu_button("+ Add operation", |ui| {
            if has_operation { ui.label("Delete the current operation first. Multiple-operation sequences are not available yet."); }
            for (label, action) in [("Add Flat V-carve", Action::AddVcarve), ("Add drag knife", Action::AddKnife)] {
                if button(ui, label, idle && !has_operation).clicked() { self.operation_command(action, ctx); ui.close(); }
            }
        });
        observe_control("Add operation", menu.response.rect);
        if has_operation && button(ui, "Delete operation", idle).clicked() {
            self.operation_command(Action::Delete, ctx);
        }
    }
    pub(super) fn adopt_operation(&mut self, job: CamJobV5, ctx: &egui::Context) {
        if self.document.is_none() {
            self.document = Some(Document::new(operation_authoring::empty_job()));
        }
        self.remember();
        let mut next = Document::new(job);
        if let Some(old) = &self.document {
            for field in [6, 7, 23, 25, 30, 31, 34, 35, 36, 40, 41, 42, 43, 44, 45] {
                if let Some(text) = old.raw.raw.get(&old.raw.key(field)) {
                    next.raw.raw.insert(next.raw.key(field), text.clone());
                }
            }
            for item in &next.job.artwork {
                for field in 26..=29 {
                    if let Some(text) = old.raw.raw.get(&old.raw.key_for(&item.id.0, field)) {
                        next.raw
                            .raw
                            .insert(next.raw.key_for(&item.id.0, field), text.clone());
                    }
                }
            }
            if next
                .job
                .artwork
                .iter()
                .any(|i| i.id.0 == old.raw.artwork_item)
            {
                next.raw.artwork_item = old.raw.artwork_item.clone();
            }
        }
        self.document = Some(next);
        self.edit_group = None;
        self.plan = None;
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
        self.status = if self.document.as_ref().unwrap().job.operations.is_empty() { "Operation deleted. Artwork, stock, tools and machine retained. Add an operation or Undo." } else { "Operation added. Select its geometry and configure its tool and cutting values." }.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_changes_preserve_shared_drafts_and_recoverable_undo() {
        let original = crate::authoring::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        let mut app = App {
            document: Some(Document::new(original.clone())),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        app.document
            .as_mut()
            .unwrap()
            .edit(6, "-".into())
            .unwrap_err();
        app.document
            .as_mut()
            .unwrap()
            .edit(26, "3.".into())
            .unwrap_err();
        app.edit_group = Some(26);
        app.adopt_operation(
            operation_authoring::apply(&original, Action::Delete).unwrap(),
            &ctx,
        );
        let empty = app.document.as_ref().unwrap();
        assert!(app.edit_group.is_none());
        assert!(empty.job.operations.is_empty());
        assert_eq!(empty.raw.raw[&empty.raw.key(6)], "-");
        assert_eq!(empty.raw.raw[&empty.raw.key(26)], "3.");
        app.recovery_snapshot().unwrap().validate().unwrap();
        app.adopt_operation(
            operation_authoring::apply(&empty.job, Action::AddKnife).unwrap(),
            &ctx,
        );
        assert!(app.document.as_ref().unwrap().pending());
        assert!(app.plan.is_none() && app.prepared.is_none());
        app.undo(&ctx);
        assert!(app.document.as_ref().unwrap().job.operations.is_empty());
        app.undo(&ctx);
        assert_eq!(app.document.as_ref().unwrap().job, original);
        app.redo(&ctx);
        app.redo(&ctx);
        assert!(crate::knife::settings(&app.document.as_ref().unwrap().job).is_some());
        app.recovery_snapshot().unwrap().validate().unwrap();
    }

    #[test]
    fn empty_job_and_new_knife_render_all_panels() {
        for job in [
            operation_authoring::empty_job(),
            operation_authoring::apply(&operation_authoring::empty_job(), Action::AddKnife)
                .unwrap(),
        ] {
            let mut app = App {
                document: Some(Document::new(job.clone())),
                ..Default::default()
            };
            app.resources.ready = true;
            app.view.load_scene(engine::run(Command::Preview {
                job: job.to_json().unwrap(),
            }));
            let ctx = egui::Context::default();
            for tab in 0..8 {
                app.inspector_tab = tab;
                app.resources.open = tab == 3;
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280., 800.),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        app.navigator(ctx);
                        app.inspector(ctx);
                        app.resource_windows(ctx);
                    },
                );
            }
            app.document.unwrap().validate().unwrap();
        }
    }
}
