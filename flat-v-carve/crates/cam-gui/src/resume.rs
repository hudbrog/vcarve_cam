use super::*;
use crate::recovery::Workspace;

impl Document {
    pub fn estimated_bytes(&self) -> usize {
        struct Counter(usize);
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter(0);
        serde_json::to_writer(&mut counter, self).expect("validated document");
        counter.0 + 4096
    }
    pub fn validate(&self) -> Result<(), String> {
        engine::open(&self.job.to_json().map_err(|e| e.to_string())?)?;
        Draft::recover(&serde_json::to_string(&self.raw).map_err(|e| e.to_string())?)?;
        if self.raw.artwork_item != self.job.artwork[0].id.0
            || self.raw.operation != self.job.operations[0].id
        {
            return Err("Draft identity does not match the job".into());
        }
        if let Some(finish) = &self.finish_draft {
            finish.validate().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
impl App {
    pub(super) fn trim_history(history: &mut Vec<Document>) {
        while history.len() > 32
            || history.iter().map(Document::estimated_bytes).sum::<usize>() > 16 * 1024 * 1024
        {
            history.remove(0);
        }
    }
    pub(super) fn workspace(&self) -> Workspace {
        let mut view = self.view.settings();
        if let Some((_, prefix)) = &self.resume_view {
            view.prefix = *prefix;
        }
        Workspace {
            inspector: self.inspector_tab,
            inspector_width: self.inspector_width,
            scroll: self.scroll,
            search: self.search.clone(),
            simulate: self.simulate,
            view,
            plan_fingerprint: self.plan_fingerprint.clone(),
            saved_job_hash: self.saved_job_hash.clone(),
        }
    }
    pub fn recovery_snapshot(&self) -> Option<Snapshot> {
        let mut snapshot = self.document.as_ref()?.snapshot();
        snapshot.workspace = self.workspace();
        snapshot.undo = self.undo.clone();
        snapshot.redo = self.redo.clone();
        loop {
            if snapshot.undo.len() + snapshot.redo.len() <= 32
                && serde_json::to_vec(&snapshot).ok()?.len() < crate::recovery::MAX_BYTES - 128
            {
                break;
            }
            if !snapshot.undo.is_empty() {
                snapshot.undo.remove(0);
            } else if !snapshot.redo.is_empty() {
                snapshot.redo.remove(0);
            } else {
                break;
            }
        }
        Some(snapshot)
    }
    pub fn restore(&mut self, snapshot: Snapshot, ctx: &egui::Context) {
        if let Err(error) = snapshot.validate() {
            self.status = error;
            return;
        }
        let Some(text) = snapshot.job else {
            self.status = "Recovery contains no job".into();
            return;
        };
        let job = match engine::open(&text) {
            Ok(job) => job,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        self.port.cancel();
        self.active = None;
        self.cancelled_id = None;
        self.plan = None;
        self.prepared = None;
        self.retained_save = None;
        self.retry = false;
        self.saved_revision = None;
        self.edit_group = None;
        self.document = Some(Document {
            job,
            raw: snapshot.draft,
            finish_draft: snapshot.finish_draft,
        });
        self.undo = snapshot.undo;
        self.redo = snapshot.redo;
        let workspace = snapshot.workspace;
        self.saved_job_hash = workspace.saved_job_hash.clone();
        self.inspector_tab = workspace.inspector;
        self.inspector_width = workspace.inspector_width;
        self.scroll = workspace.scroll;
        self.search = workspace.search.clone();
        self.simulate = workspace.simulate;
        self.view.restore_settings(&workspace.view);
        self.plan_fingerprint = workspace.plan_fingerprint.clone();
        self.resume_view = workspace
            .plan_fingerprint
            .map(|f| (f, workspace.view.prefix));
        self.changed(ctx);
        if self.document.as_ref().is_some_and(|d| {
            !d.pending()
                && self.saved_job_hash.as_ref()
                    == Some(&crate::compute::hash(d.job.to_json().unwrap().as_bytes()))
        }) {
            self.saved_revision = Some(self.revision);
        }
        self.status="Draft and working context restored. Generate to rebuild simulation; no saved execution or output handles were trusted.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        App {
            document: Some(Document::new(engine::open(engine::FLOWER).unwrap())),
            ..Default::default()
        }
    }
    #[test]
    fn recovery_keeps_raw_text_navigation_history_and_no_artifact_authority() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.remember();
        app.document
            .as_mut()
            .unwrap()
            .edit(2, "1700".into())
            .unwrap();
        app.remember();
        assert!(app.document.as_mut().unwrap().edit(0, "-".into()).is_err());
        app.inspector_tab = 0;
        app.search = "Rotation".into();
        app.scroll[0] = 42.;
        app.view.restore_settings(&crate::viewport::ViewSettings {
            isometric: true,
            zoom: 1.4,
            yaw: 0.3,
            stage: 1,
            stock: true,
            prefix: 123,
        });
        app.plan = Some(("do-not-restore".into(), 0));
        app.prepared = Some((json!({"file":"do-not-restore"}), 0));
        let snapshot = app.recovery_snapshot().unwrap();
        let text = serde_json::to_string(&snapshot).unwrap();
        assert!(!text.contains("do-not-restore"));
        let mut restored = App::default();
        restored.restore(serde_json::from_str(&text).unwrap(), &ctx);
        assert_eq!(restored.inspector_tab, 0);
        assert_eq!(restored.search, "Rotation");
        assert!(restored.view.settings().isometric);
        assert_eq!(restored.undo.len(), 2);
        assert!(restored.document.as_ref().unwrap().pending());
        assert!(restored.plan.is_none() && restored.prepared.is_none());
        restored.undo(&ctx);
        assert_eq!(restored.document.as_ref().unwrap().text(0), "1");
        restored.undo(&ctx);
        assert_eq!(restored.document.as_ref().unwrap().text(2), "2000");
        restored.redo(&ctx);
        assert_eq!(restored.document.as_ref().unwrap().text(2), "1700");
    }
    #[test]
    fn late_results_and_save_completions_cannot_clear_newer_edits() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.active = Some((7, 0));
        app.changed(&ctx);
        let result = engine::execute(
            &mut cam_service::retained::Retained::new(),
            Command::Open {
                json: engine::FLOWER.into(),
            },
        );
        app.accept(7, result, &ctx);
        assert_eq!(app.revision, 1);
        assert!(app.plan.is_none());
        app.io = Some((9, IoKind::Save(Some(0))));
        app.event(
            Event::Io {
                id: 9,
                result: Ok(IoValue::Saved("Saved exact bytes".into())),
            },
            &ctx,
        );
        assert_eq!(app.saved_revision, None);
        app.io = Some((10, IoKind::Save(Some(1))));
        app.event(
            Event::Io {
                id: 10,
                result: Ok(IoValue::Saved("Download requested".into())),
            },
            &ctx,
        );
        assert_eq!(app.saved_revision, None);
        app.status = "Current result".into();
        app.cancelled_id = Some(1);
        app.event(Event::Cancelled { id: 0, stop_ms: 1. }, &ctx);
        assert_eq!(app.status, "Current result");
    }
    #[test]
    fn saved_job_identity_survives_restart_but_partial_text_remains_unsaved() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.saved_job_hash = Some(crate::compute::hash(
            app.document
                .as_ref()
                .unwrap()
                .job
                .to_json()
                .unwrap()
                .as_bytes(),
        ));
        let mut restored = App::default();
        restored.restore(app.recovery_snapshot().unwrap(), &ctx);
        assert_eq!(restored.saved_revision, Some(restored.revision));
        assert!(
            restored
                .document
                .as_mut()
                .unwrap()
                .edit(0, "-".into())
                .is_err()
        );
        let snapshot = restored.recovery_snapshot().unwrap();
        restored.restore(snapshot, &ctx);
        assert_eq!(restored.saved_revision, None);
    }
    #[test]
    fn history_obeys_byte_and_transaction_limits() {
        let mut app = app();
        for _ in 0..50 {
            app.remember();
        }
        assert_eq!(app.undo.len(), 32);
        let cam_core::project::v5::ArtworkContent::Svg(source) =
            &mut app.document.as_mut().unwrap().job.artwork[0].content;
        source.svg.push_str(&" ".repeat(1_000_000));
        for _ in 0..20 {
            app.remember();
        }
        assert!(
            app.undo
                .iter()
                .map(Document::estimated_bytes)
                .sum::<usize>()
                <= 16 * 1024 * 1024
        );
        let snapshot = app.recovery_snapshot().unwrap();
        assert!(serde_json::to_vec(&snapshot).unwrap().len() < crate::recovery::MAX_BYTES);
        snapshot.validate().unwrap();
    }
}
