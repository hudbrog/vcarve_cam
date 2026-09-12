use super::*;
use crate::{
    artwork_view::{self, GestureMode},
    authoring::Component,
};
use cam_core::{geometry::Point, project::v5::GeometryRef, svg::Placement};

#[derive(Clone)]
pub enum ArtworkEvent {
    Selection(Vec<GeometryRef>),
    Placement {
        revision: u64,
        initial: Placement,
        value: Placement,
    },
}
struct Drag {
    revision: u64,
    initial: Placement,
    start: Point,
    candidate: Placement,
}
#[derive(Default)]
pub struct ArtworkInteraction {
    pub enabled: bool,
    pub mode: GestureMode,
    pub selected: Vec<GeometryRef>,
    pub revision: u64,
    components: Vec<Component>,
    placement: Placement,
    candidates: Vec<GeometryRef>,
    candidate_index: usize,
    drag: Option<Drag>,
    events: Vec<ArtworkEvent>,
}
impl Viewport {
    pub fn artwork_matches(&self, placement: &Placement) -> bool {
        &self.artwork.placement == placement
    }
    pub fn update_artwork(
        &mut self,
        revision: u64,
        placement: Placement,
        components: Vec<Component>,
    ) {
        self.artwork.revision = revision;
        self.artwork.placement = placement;
        self.artwork.components = components;
        self.artwork
            .candidates
            .retain(|r| self.artwork.components.iter().any(|c| &c.reference == r));
        self.artwork.candidate_index = 0;
        self.artwork
            .selected
            .retain(|r| self.artwork.components.iter().any(|c| &c.reference == r));
    }
    pub fn take_artwork_events(&mut self) -> Vec<ArtworkEvent> {
        std::mem::take(&mut self.artwork.events)
    }
    pub(super) fn artwork_toolbar(&mut self, ui: &mut egui::Ui) {
        if !self.artwork.enabled {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for (label, mode) in [
                ("Select artwork", GestureMode::Select),
                ("Move artwork", GestureMode::Move),
                ("Rotate artwork", GestureMode::Rotate),
                ("Scale artwork", GestureMode::Scale),
            ] {
                let response = ui.selectable_value(&mut self.artwork.mode, mode, label);
                crate::app::observe_control(label, response.rect);
            }
            if self.artwork.candidates.len() > 1 {
                let r = ui.button("Next overlap");
                crate::app::observe_control("Next overlap", r.rect);
                if r.clicked() {
                    self.artwork.candidate_index =
                        (self.artwork.candidate_index + 1) % self.artwork.candidates.len();
                    self.artwork.selected =
                        vec![self.artwork.candidates[self.artwork.candidate_index].clone()];
                    self.artwork
                        .events
                        .push(ArtworkEvent::Selection(self.artwork.selected.clone()));
                }
            }
        });
        ui.small(if self.artwork.mode==GestureMode::Select {"Pick filled regions · Shift-click adds/removes · selection does not change the cut"} else {"Drag the whole source · rotate/scale about setup 0,0 (numeric page origin) · Esc cancels"});
    }
    pub(super) fn artwork_pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
    ) {
        if !self.artwork.enabled {
            self.artwork.drag = None;
            return;
        }
        let Some(scene) = &self.scene else {
            return;
        };
        let bounds = scene.meta.bounds;
        let camera = self.camera(rect);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.artwork.drag = None;
            return;
        }
        if self.artwork.mode == GestureMode::Select {
            if response.clicked()
                && let Some(p) = response.interact_pointer_pos()
            {
                let point = artwork_view::setup_point(camera, bounds, rect, p);
                self.artwork.candidates =
                    artwork_view::hit_candidates(&self.artwork.components, point);
                self.artwork.candidate_index = 0;
                let chosen = self.artwork.candidates.first().cloned();
                if ui.input(|i| i.modifiers.shift) {
                    if let Some(chosen) = chosen {
                        if self.artwork.selected.contains(&chosen) {
                            self.artwork.selected.retain(|r| r != &chosen);
                        } else {
                            self.artwork.selected.push(chosen);
                        }
                    }
                } else {
                    self.artwork.selected = chosen.into_iter().collect();
                }
                self.artwork
                    .events
                    .push(ArtworkEvent::Selection(self.artwork.selected.clone()));
            }
            return;
        }
        if response.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            let start = artwork_view::setup_point(camera, bounds, rect, p);
            if self.artwork.mode != GestureMode::Move
                || !artwork_view::hit_candidates(&self.artwork.components, start).is_empty()
            {
                self.artwork.drag = Some(Drag {
                    revision: self.artwork.revision,
                    initial: self.artwork.placement.clone(),
                    candidate: self.artwork.placement.clone(),
                    start,
                });
            }
        }
        if let Some(drag) = &mut self.artwork.drag
            && let Some(p) = response.interact_pointer_pos()
            && let Ok(value) = artwork_view::candidate(
                &drag.initial,
                self.artwork.mode,
                drag.start,
                artwork_view::setup_point(camera, bounds, rect, p),
            )
        {
            drag.candidate = value;
        }
        if response.drag_stopped()
            && let Some(drag) = self.artwork.drag.take()
            && drag.initial != drag.candidate
        {
            self.artwork.events.push(ArtworkEvent::Placement {
                revision: drag.revision,
                initial: drag.initial,
                value: drag.candidate,
            });
        }
    }
    pub(super) fn artwork_overlay(&self, ui: &egui::Ui, rect: egui::Rect) {
        if !self.artwork.enabled {
            return;
        }
        let Some(scene) = &self.scene else {
            return;
        };
        let camera = self.camera(rect);
        let bounds = scene.meta.bounds;
        let painter = ui.painter().with_clip_rect(rect);
        for component in &self.artwork.components {
            if self.artwork.drag.is_none() && !self.artwork.selected.contains(&component.reference)
            {
                continue;
            }
            for ring in &component.rings {
                let mut screen = Vec::new();
                for p in ring {
                    let mut point = Point::new(p[0], p[1]);
                    if let Some(drag) = &self.artwork.drag {
                        let Ok(page) = drag.initial.to_page(point) else {
                            continue;
                        };
                        let Ok(next) = drag.candidate.to_setup(page) else {
                            continue;
                        };
                        point = next;
                    }
                    screen.push(artwork_view::screen_point(camera, bounds, rect, point));
                }
                if let Some(first) = screen.first().copied() {
                    screen.push(first);
                }
                painter.add(egui::Shape::line(
                    screen,
                    egui::Stroke::new(2., Color32::from_rgb(226, 130, 30)),
                ));
            }
        }
        if matches!(self.artwork.mode, GestureMode::Rotate | GestureMode::Scale) {
            let p = artwork_view::screen_point(camera, bounds, rect, Point::new(0., 0.));
            painter.circle_stroke(
                p,
                6.,
                egui::Stroke::new(2., Color32::from_rgb(226, 130, 30)),
            );
        }
    }
}
