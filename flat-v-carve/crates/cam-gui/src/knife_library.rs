use super::*;
use cam_core::tool_library::KnifeCuttingPreset;

impl App {
    pub(super) fn knife_library_profiles(&mut self, ui: &mut egui::Ui, tool: &mut LibraryTool) {
        ui.heading("Knife cutting profiles");
        for p in &tool.knife_cutting_presets {
            let r = ui.selectable_value(&mut self.resources.preset, p.id.clone(), &p.name);
            observe_control(&format!("Library profile {}", p.id), r.rect);
        }
        if button(
            ui,
            "New knife profile",
            tool.knife_cutting_presets.len() < 100,
        )
        .clicked()
        {
            let id = unique(
                "profile",
                tool.knife_cutting_presets.iter().map(|p| p.id.clone()),
            );
            tool.knife_cutting_presets.push(KnifeCuttingPreset {
                id: id.clone(),
                name: "New knife profile".into(),
                material: None,
                machine: None,
                cutting_feed_mm_min: None,
                plunge_feed_mm_min: None,
                swivel_feed_mm_min: None,
                max_stepdown_mm: None,
            });
            self.resources.preset = id;
            self.resources.dirty = true;
        }
        if button(
            ui,
            "Capture knife assignment",
            self.document
                .as_ref()
                .is_some_and(|d| crate::knife::settings(&d.job).is_some()),
        )
        .clicked()
        {
            let a = &crate::knife::settings(&self.document.as_ref().unwrap().job)
                .unwrap()
                .assignment;
            let id = unique(
                "profile",
                tool.knife_cutting_presets.iter().map(|p| p.id.clone()),
            );
            tool.knife_cutting_presets.push(KnifeCuttingPreset {
                id: id.clone(),
                name: "Captured knife values".into(),
                material: None,
                machine: None,
                cutting_feed_mm_min: a.cutting_feed_mm_min,
                plunge_feed_mm_min: a.plunge_feed_mm_min,
                swivel_feed_mm_min: a.swivel_feed_mm_min,
                max_stepdown_mm: a.max_stepdown_mm,
            });
            self.resources.preset = id;
            self.resources.dirty = true;
        }
        if let Some(index) = tool
            .knife_cutting_presets
            .iter()
            .position(|p| p.id == self.resources.preset)
        {
            let p = &mut tool.knife_cutting_presets[index];
            self.resources.dirty |= text(ui, "Profile name", &mut p.name);
            for (label, field, value) in [
                ("Profile cutting feed", "feed", &mut p.cutting_feed_mm_min),
                ("Profile plunge feed", "plunge", &mut p.plunge_feed_mm_min),
                ("Profile swivel feed", "swivel", &mut p.swivel_feed_mm_min),
                ("Profile stepdown", "stepdown", &mut p.max_stepdown_mm),
            ] {
                number(
                    ui,
                    label,
                    &format!("{}/{}/{field}", tool.id, p.id),
                    value,
                    false,
                    &mut self.resources,
                );
            }
            if button(
                ui,
                "Duplicate knife profile",
                tool.knife_cutting_presets.len() < 100,
            )
            .clicked()
            {
                let mut p = tool.knife_cutting_presets[index].clone();
                p.id = unique(
                    "profile",
                    tool.knife_cutting_presets.iter().map(|p| p.id.clone()),
                );
                p.name.push_str(" copy");
                self.resources.preset = p.id.clone();
                tool.knife_cutting_presets.push(p);
                self.resources.dirty = true;
            }
        }
    }
}
