//! Compact display controls. These edit camera/presentation state only.
use super::*;
use crate::{app::observe_control, ui_icons, ui_theme as theme};

// Paging, sliders and collapsible keys are controls inside a popup: keep it
// open until the user dismisses it or a command explicitly calls ui.close().
pub(super) fn settings_menu(
    ui: &mut egui::Ui,
    label: &str,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    egui::containers::menu::MenuButton::new(label)
        .config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .ui(ui, |ui| {
            let height = (ui.ctx().content_rect().height() - 64.).max(120.);
            ui.set_max_height(height);
            let area = egui::ScrollArea::vertical()
                .max_height(height)
                .show(ui, content);
            observe_control("Viewport options viewport", area.inner_rect);
        })
        .0
}

impl Viewport {
    pub(super) fn view_toolbar(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let width = ui.available_width();
        let toolbar = ui.horizontal(|ui| {
            ui.set_min_height(theme::ROW);
            if self.artwork.enabled && width >= 380. {
                self.artwork_menu(ui);
            }
            if width >= 560. {
                self.view_presets(ui, false);
            }
            let fit = if width < 300. {
                ui_icons::button(ui, ui_icons::Icon::Fit, "Fit view", false)
            } else {
                ui.button("Fit view")
            };
            observe_control("Fit", fit.rect);
            observe_control("Fit view", fit.rect);
            if fit.clicked() {
                self.fit();
            }
            let view = settings_menu(ui, "View", |ui| {
                ui.set_width(250.);
                self.view_presets(ui, true);
                ui.separator();
                let mut tilt = self.camera.tilt.to_degrees();
                let response = ui.add(
                    egui::Slider::new(&mut tilt, -85.0..=85.0)
                        .suffix("°")
                        .text("Elevation"),
                );
                observe_control("View", response.rect);
                if response.changed() {
                    self.camera.set_tilt(tilt.to_radians());
                }
                let mut zoom = self.camera.zoom;
                let response = ui.add(
                    egui::Slider::new(&mut zoom, camera::MIN_ZOOM..=camera::MAX_ZOOM)
                        .logarithmic(true)
                        .text("Zoom"),
                );
                observe_control("Zoom", response.rect);
                if response.changed() {
                    self.camera.set_zoom(zoom);
                }
                crate::app::help::label(ui, "View controls");
                if self.artwork.enabled && width < 380. {
                    ui.separator();
                    self.artwork_toolbar(ui);
                }
            });
            observe_control("View menu", view.rect);
            let display = settings_menu(ui, "Display", |ui| {
                ui.set_width(250.);
                ui.small("Display only");
                self.stock_display_controls(ui);
                if width < 300. {
                    ui.separator();
                    self.layer_controls(ui);
                }
            });
            observe_control("Display menu", display.rect);
            if width >= 300. {
                let layers = settings_menu(ui, "Layers", |ui| self.layer_controls(ui));
                observe_control("Layers menu", layers.rect);
            }
            if width >= 800. {
                ui.weak(format!(
                    "{} · {}",
                    self.stock_style.surface.label(),
                    if self.stock_style.appearance == stock_style::Appearance::XRay {
                        "X-ray"
                    } else {
                        "Solid"
                    }
                ));
            }
        });
        observe_control("View toolbar", toolbar.response.rect);
        toolbar.response.rect
    }

    fn artwork_menu(&mut self, ui: &mut egui::Ui) {
        use crate::artwork_view::GestureMode;
        let label = match self.artwork.mode {
            GestureMode::Select => "Select",
            GestureMode::Move => "Move",
            GestureMode::Rotate => "Rotate",
            GestureMode::Scale => "Scale",
        };
        let menu = ui.menu_button(label, |ui| {
            ui.set_width(280.);
            self.artwork_toolbar(ui);
        });
        observe_control("Artwork tools", menu.response.rect);
    }

    fn view_presets(&mut self, ui: &mut egui::Ui, elevations: bool) {
        for (label, tilt) in [("Top", 0.), ("Isometric", camera::ISO_TILT)] {
            let response = ui.selectable_label((self.camera.tilt - tilt).abs() < 1e-3, label);
            observe_control(label, response.rect);
            if response.clicked() {
                self.camera.set_tilt(tilt);
                if elevations {
                    ui.close();
                }
            }
        }
        if elevations {
            for (label, yaw) in camera::ELEVATIONS {
                let response = ui.selectable_label(
                    self.camera.is_elevation() && (self.camera.yaw - yaw).abs() < 1e-3,
                    label,
                );
                observe_control(label, response.rect);
                if response.clicked() {
                    self.camera.yaw = yaw;
                    self.camera.set_tilt(camera::TILT_LIMIT);
                    ui.close();
                }
            }
        }
    }

    fn stock_display_controls(&mut self, ui: &mut egui::Ui) {
        let mut surface = self.stock_style.surface;
        ui.label("Stock color");
        for mode in stock_style::ColorMode::ALL {
            let response = ui.selectable_value(&mut surface, mode, mode.label());
            observe_control(&format!("Stock color {}", mode.label()), response.rect);
        }
        if surface != self.stock_style.surface {
            self.stock_style.surface = surface;
            self.palette = None;
        }
        ui.separator();
        ui.horizontal(|ui| {
            let solid = ui.selectable_value(
                &mut self.stock_style.appearance,
                stock_style::Appearance::Opaque,
                "Solid",
            );
            observe_control("Solid stock", solid.rect);
            let xray = ui.selectable_value(
                &mut self.stock_style.appearance,
                stock_style::Appearance::XRay,
                "X-ray",
            );
            observe_control("X-ray stock", xray.rect);
        });
        if self.stock_style.appearance == stock_style::Appearance::XRay {
            let response = ui.add(
                egui::Slider::new(&mut self.stock_style.xray_opacity, 0.05..=1.).text("Opacity"),
            );
            observe_control("Stock opacity", response.rect);
        }
        ui.separator();
        ui.label("Wall color");
        for mode in stock_style::WallMode::ALL {
            let response = ui.selectable_value(&mut self.stock_style.walls, mode, mode.label());
            observe_control(&format!("Stock walls {}", mode.label()), response.rect);
        }
    }

    fn layer_controls(&mut self, ui: &mut egui::Ui) {
        ui.small("Display only");
        if self.stock.is_some() {
            let response = ui.checkbox(&mut self.show_stock, "Stock preview");
            observe_control("Stock preview", response.rect);
        }
        let artwork = ui.checkbox(&mut self.stock_style.show_artwork, "Artwork");
        observe_control("Layer Artwork", artwork.rect);
        if artwork.changed() {
            self.overlay_signature = None;
        }
        for (name, value) in [
            ("Cutting", &mut self.stock_style.show_cutting),
            ("Travel", &mut self.stock_style.show_travel),
            ("Edges", &mut self.stock_style.show_edges),
        ] {
            let response = ui.checkbox(value, name);
            observe_control(&format!("Layer {name}"), response.rect);
        }
        ui.separator();
        let key = ui.collapsing("View key", |ui| {
            use cam_core::sequence::StageRole;
            ui.set_max_width(270.);
            ui.small("Teal artwork is selected. Other artwork keeps its source color, or neutral gray.");
            for (label, role) in [
                ("Face", StageRole::Face),
                ("Endmill carve", StageRole::VcarveRough),
                ("V-bit carve", StageRole::VcarveFinish),
                ("Profile rough", StageRole::ProfileRough),
                ("Profile finish", StageRole::ProfileFinish),
                ("Knife holder path", StageRole::Knife),
            ] {
                let [r,g,b,_] = crate::scene::role_color(role);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16., 12.), egui::Sense::hover());
                    ui.painter().rect_filled(rect.shrink2(egui::vec2(0., 4.)), 0., Color32::from_rgb((r*255.) as u8, (g*255.) as u8, (b*255.) as u8));
                    ui.small(label);
                });
            }
            ui.small("Faint gray paths are travel. Gold marks a picked motion; orange marks the current partial motion. The translucent cutter follows the simulated tip.");
        });
        observe_control("View key", key.header_response.rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_stays_on_one_bounded_row_at_reduced_effective_width() {
        for width in [200., 296., 380., 560., 688., 1000.] {
            let ctx = egui::Context::default();
            crate::ui_theme::apply(&ctx);
            let mut view = Viewport::default();
            view.artwork.enabled = true;
            let original = view.settings();
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 500.),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let bounds = ui.available_rect_before_wrap();
                            let toolbar = view.view_toolbar(ui);
                            assert!(
                                ui.min_rect().right() <= bounds.right() + 1.,
                                "toolbar exceeds {width} points"
                            );
                            assert!(
                                toolbar.height() <= 36.,
                                "toolbar exceeds one row at {width}"
                            );
                        });
                    },
                );
            }
            assert_eq!(
                view.settings(),
                original,
                "rendering controls changed saved display state"
            );
        }
    }
}
