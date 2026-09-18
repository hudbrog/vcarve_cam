//! Resource navigation does not own the document, inspector selection or drafts.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResourcePage {
    ToolLibrary,
    MachineLibrary,
    JobTools,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_footer_stays_visible_at_small_effective_sizes() {
        for (width, height) in [(640., 400.), (853., 533.), (1280., 800.), (1440., 900.)] {
            for page in [ResourcePage::ToolLibrary, ResourcePage::MachineLibrary] {
                let mut app = App::default();
                app.document = Some(Document::new(
                    CamJobV5::from_json(include_str!("../../../fixtures/gui4/lettering.job.json"))
                        .unwrap(),
                ));
                app.resources.ready = true;
                app.resources.draft = crate::resources::Catalog::decode(include_str!(
                    "../../../fixtures/gui5/library.json"
                ))
                .unwrap();
                app.open_resource(page);
                let ctx = egui::Context::default();
                App::theme(&ctx);
                for _ in 0..4 {
                    CONTROLS.with(|c| c.borrow_mut().clear());
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, height),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            app.commands(ctx);
                            app.navigator(ctx);
                            app.resource_windows(ctx);
                        },
                    );
                }
                CONTROLS.with(|controls| {
                    let controls = controls.borrow();
                    for label in ["Save library", "Close library", "Library footer"] {
                        let r = controls[label];
                        assert!(
                            r[0] >= 0. && r[2] <= width && r[3] <= height - 24.,
                            "{page:?} {width}×{height} {label}: {r:?}"
                        );
                    }
                    let viewport = controls["Resource viewport"];
                    assert!(viewport[3] <= controls["Library footer"][1]);
                    assert!(viewport[3] - viewport[1] >= 16.);
                });
            }
        }
    }

    #[test]
    fn resource_navigation_retains_job_and_both_editing_drafts() {
        let job =
            CamJobV5::from_json(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            ..Default::default()
        };
        let doc = app.document.as_mut().unwrap();
        doc.raw.raw.insert(doc.raw.key(6), "-".into());
        let raw = doc.raw.clone();
        let job = doc.job.to_json().unwrap();
        app.resources.draft =
            crate::resources::Catalog::decode(include_str!("../../../fixtures/gui5/library.json"))
                .unwrap();
        app.resources
            .raw
            .insert("endmill/diameter".into(), "1.".into());
        app.resources
            .job_raw
            .insert("endmill/length".into(), "-".into());
        app.resources.dirty = true;
        let draft = app.resource_stamp();
        app.simulate = true;
        app.inspector_tab = 6;
        app.operation_scroll = [10., 20., 30.];
        for page in [
            ResourcePage::ToolLibrary,
            ResourcePage::MachineLibrary,
            ResourcePage::JobTools,
        ] {
            app.open_resource(page);
            assert_eq!(app.resource_page, Some(page));
            assert_eq!(app.document.as_ref().unwrap().raw, raw);
            assert_eq!(app.document.as_ref().unwrap().job.to_json().unwrap(), job);
            assert_eq!(app.resource_stamp(), draft);
            assert_eq!(app.resources.job_raw["endmill/length"], "-");
            assert_eq!(app.operation_scroll, [10., 20., 30.]);
        }
        app.close_resource();
        assert!(app.simulate);
        assert_eq!(app.inspector_tab, 6);
        app.open_resource(ResourcePage::ToolLibrary);
        app.navigate(1);
        assert!(app.resource_page.is_none());
        assert_eq!(app.document.as_ref().unwrap().raw, raw);
    }
}

impl App {
    pub(super) fn open_resource(&mut self, page: ResourcePage) {
        self.view.pause();
        self.operation_picker = None;
        self.resource_page = Some(page);
    }

    pub(super) fn close_resource(&mut self) {
        self.resource_page = None;
    }

    pub(super) fn library_open(&self) -> bool {
        matches!(
            self.resource_page,
            Some(ResourcePage::ToolLibrary | ResourcePage::MachineLibrary)
        )
    }

    pub(super) fn machines_open(&self) -> bool {
        self.resource_page == Some(ResourcePage::MachineLibrary)
    }
}
