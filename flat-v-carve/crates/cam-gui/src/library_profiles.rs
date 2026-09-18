//! Profile selection remains visible beside the selected cutting values.
use super::*;

impl App {
    pub(super) fn library_profiles(&mut self, ui: &mut egui::Ui, tool: &mut LibraryTool) {
        if ui.available_width() >= 620. {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(160., 180.),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(160.);
                        self.library_profile_list(ui, tool);
                    },
                );
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    self.profile_details(ui, tool);
                });
            });
        } else {
            let selected = tool
                .cutting_presets
                .iter()
                .find(|p| p.id == self.resources.preset)
                .map(|p| p.name.as_str())
                .or_else(|| {
                    tool.knife_cutting_presets
                        .iter()
                        .find(|p| p.id == self.resources.preset)
                        .map(|p| p.name.as_str())
                })
                .unwrap_or("Tool only");
            let menu = ui.menu_button(format!("Profile · {selected}"), |ui| {
                ui.set_width(220.);
                self.library_profile_list(ui, tool);
            });
            observe_control("Library profile chooser", menu.response.rect);
            self.profile_details(ui, tool);
        }
    }

    fn library_profile_list(&mut self, ui: &mut egui::Ui, tool: &LibraryTool) {
        let area = egui::ScrollArea::vertical()
            .id_salt(("library-profiles", &tool.id))
            .max_height(180.)
            .min_scrolled_height(28.)
            .show(ui, |ui| {
                let r = ui.selectable_value(&mut self.resources.preset, String::new(), "Tool only");
                observe_control("Library profile none", r.rect);
                for (id, name) in tool
                    .cutting_presets
                    .iter()
                    .map(|p| (&p.id, &p.name))
                    .chain(tool.knife_cutting_presets.iter().map(|p| (&p.id, &p.name)))
                {
                    let r = ui
                        .add_sized(
                            [ui.available_width(), 28.],
                            egui::Button::new(name)
                                .selected(self.resources.preset == *id)
                                .truncate(),
                        )
                        .on_hover_text(name);
                    observe_control(&format!("Library profile {id}"), r.rect);
                    if r.clicked() {
                        self.resources.preset = id.clone();
                    }
                }
            });
        observe_control("Library profiles viewport", area.inner_rect);
    }

    fn profile_details(&mut self, ui: &mut egui::Ui, tool: &mut LibraryTool) {
        if matches!(tool.geometry, LibraryGeometry::DragKnife(_)) {
            self.knife_library_profiles(ui, tool);
        } else {
            self.milling_profile_details(ui, tool);
        }
    }

    fn milling_profile_details(&mut self, ui: &mut egui::Ui, tool: &mut LibraryTool) {
        ui.horizontal(|ui| {
            ui.strong("Cutting values");
            let menu = ui.menu_button("Profile actions", |ui| {
                if let Some(p) = tool
                    .cutting_presets
                    .iter_mut()
                    .find(|p| p.id == self.resources.preset)
                {
                    let context = ui.menu_button("Material & machine context", |ui| {
                        ui.set_width(350.);
                        for (label, slot) in [
                            ("Profile material", &mut p.material),
                            ("Profile machine context", &mut p.machine),
                        ] {
                            let mut value = slot.clone().unwrap_or_default();
                            if text(ui, label, &mut value) {
                                *slot = (!value.is_empty()).then_some(value);
                                self.resources.dirty = true;
                            }
                        }
                    });
                    observe_control("Library profile context", context.response.rect);
                    ui.separator();
                }
                if button(ui, "New cutting profile", true).clicked() {
                    let id = unique("profile", tool.cutting_presets.iter().map(|p| p.id.clone()));
                    tool.cutting_presets.push(CuttingPreset {
                        id: id.clone(),
                        name: "New profile".into(),
                        material: None,
                        machine: None,
                        spindle_rpm: None,
                        cutting_feed_mm_min: None,
                        plunge_feed_mm_min: None,
                        max_stepdown_mm: None,
                        stepover_mm: None,
                    });
                    self.resources.preset = id;
                    self.resources.dirty = true;
                    ui.close();
                }
                if button(
                    ui,
                    "Capture assignment as profile",
                    self.document
                        .as_ref()
                        .is_some_and(|d| d.active_operation().is_some()),
                )
                .clicked()
                {
                    let id = unique("profile", tool.cutting_presets.iter().map(|p| p.id.clone()));
                    let doc = self.document.as_ref().unwrap();
                    match crate::resources::capture_assignment_in(
                        &doc.job,
                        &doc.raw.operation,
                        self.resources.role,
                        id.clone(),
                        "Captured cutting values".into(),
                    ) {
                        Ok(profile) => {
                            tool.cutting_presets.push(profile);
                            self.resources.preset = id;
                            self.resources.dirty = true;
                        }
                        Err(error) => {
                            self.resources.status = error.clone();
                            self.resources.error = Some(error);
                        }
                    }
                    ui.close();
                }
                if button(
                    ui,
                    "Duplicate cutting profile",
                    tool.cutting_presets
                        .iter()
                        .any(|p| p.id == self.resources.preset),
                )
                .clicked()
                {
                    let mut copy = tool
                        .cutting_presets
                        .iter()
                        .find(|p| p.id == self.resources.preset)
                        .unwrap()
                        .clone();
                    copy.id = unique("profile", tool.cutting_presets.iter().map(|p| p.id.clone()));
                    copy.name.push_str(" copy");
                    self.resources.preset = copy.id.clone();
                    tool.cutting_presets.push(copy);
                    self.resources.dirty = true;
                    ui.close();
                }
            });
            observe_control("Library profile actions", menu.response.rect);
        });
        if let Some(p) = tool
            .cutting_presets
            .iter_mut()
            .find(|p| p.id == self.resources.preset)
        {
            ui.push_id((&tool.id, p.id.clone()), |ui| {
                preset_form(ui, &tool.id, p, &mut self.resources)
            });
        } else {
            ui.label("Tool only · no cutting values will be applied.");
        }
    }
}
