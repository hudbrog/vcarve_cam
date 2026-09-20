use super::*;
use crate::{
    artwork_view::{self, GestureMode},
    authoring::Component,
};
use cam_core::{geometry::Point, project::v5::GeometryRef, svg::Placement};

#[derive(Clone)]
pub enum ArtworkEvent {
    KnifeSelection(Vec<GeometryRef>),
    ProfileSelection(Vec<GeometryRef>),
    DrillSelection(Vec<GeometryRef>),
    CarveSelection(Vec<GeometryRef>),
    Placement {
        item: String,
        revision: u64,
        initial: Placement,
        value: Placement,
    },
}
struct Drag {
    item: String,
    revision: u64,
    initial: Placement,
    start: Point,
    candidate: Placement,
}
#[derive(Default)]
pub struct ArtworkInteraction {
    pub enabled: bool,
    pub mode: GestureMode,
    /// The geometry assigned to the current operation, mirrored here so the
    /// viewport can extend or replace that operation's own selection. Pointer
    /// assignment is an explicit document command; it is never display-only.
    pub selected: Vec<GeometryRef>,
    pub revision: u64,
    pub hidden: std::collections::BTreeSet<String>,
    pub locked: std::collections::BTreeSet<String>,
    components: Vec<Component>,
    placement: Placement,
    item: String,
    placements: std::collections::BTreeMap<String, Placement>,
    candidates: Vec<GeometryRef>,
    candidate_index: usize,
    drag: Option<Drag>,
    events: Vec<ArtworkEvent>,
}
impl Viewport {
    pub fn artwork_matches(&self, item: &str, placement: &Placement) -> bool {
        self.artwork.item == item && &self.artwork.placement == placement
    }
    pub fn select_artwork(&mut self, item: &str) {
        if self.artwork.item != item {
            self.artwork.drag = None;
            self.artwork.item = item.into();
            self.artwork.placement = self
                .artwork
                .placements
                .get(item)
                .cloned()
                .unwrap_or_default();
        }
    }
    pub fn update_artwork(
        &mut self,
        revision: u64,
        item: String,
        placements: std::collections::BTreeMap<String, Placement>,
        components: Vec<Component>,
        selection: Vec<GeometryRef>,
    ) {
        self.artwork.revision = revision;
        self.artwork.item = item;
        self.artwork.placement = placements
            .get(&self.artwork.item)
            .cloned()
            .unwrap_or_default();
        self.artwork.placements = placements;
        self.artwork
            .hidden
            .retain(|id| self.artwork.placements.contains_key(id));
        self.artwork
            .locked
            .retain(|id| self.artwork.placements.contains_key(id));
        self.artwork.drag = None;
        self.artwork.components = components;
        self.artwork.selected = selection;
        let known = self.available_references();
        self.artwork.candidates.retain(|r| known.contains(r));
        self.artwork.candidate_index = 0;
    }
    /// Every reference the displayed scene can resolve: filled components for
    /// a carving operation, closed contours for a profile, knife chains for
    /// a drag knife, and marker points for drilling.
    fn available_references(&self) -> Vec<GeometryRef> {
        self.artwork
            .components
            .iter()
            .map(|c| c.reference.clone())
            .chain(self.profile_contours.iter().map(|c| c.reference.clone()))
            .chain(self.knife_chains.iter().map(|c| c.reference.clone()))
            .chain(self.drill_points.iter().map(|p| p.reference.clone()))
            .collect()
    }
    pub fn take_artwork_events(&mut self) -> Vec<ArtworkEvent> {
        std::mem::take(&mut self.artwork.events)
    }
    pub(super) fn artwork_toolbar(&mut self, ui: &mut egui::Ui) {
        self.artwork.candidates.retain(|r| {
            !self.artwork.hidden.contains(&r.artwork_item_id.0)
                && !self.artwork.locked.contains(&r.artwork_item_id.0)
        });
        if !self.artwork.enabled {
            return;
        }
        ui.vertical(|ui| {
            for (label, mode) in [
                ("Select artwork", GestureMode::Select),
                ("Move artwork", GestureMode::Move),
                ("Rotate artwork", GestureMode::Rotate),
                ("Scale artwork", GestureMode::Scale),
            ] {
                let label = if mode == GestureMode::Select && self.is_knife() {
                    "Select knife paths"
                } else if mode == GestureMode::Select && self.is_profile() {
                    "Select profile contours"
                } else if mode == GestureMode::Select && self.is_drill() {
                    "Select drill holes"
                } else {
                    label
                };
                let response = ui.selectable_value(&mut self.artwork.mode, mode, label);
                crate::app::observe_control(label, response.rect);
                if response.clicked() {
                    ui.close();
                }
            }
            if self.artwork.candidates.len() > 1
                && !self.is_knife()
                && !self.is_profile()
                && !self.is_drill()
            {
                let r = ui.button("Next overlap");
                crate::app::observe_control("Next overlap", r.rect);
                if r.clicked() {
                    // Advance from whichever coincident owner is assigned now,
                    // so a completed assignment cycle does not repeat an owner.
                    self.artwork.candidate_index = self
                        .artwork
                        .candidates
                        .iter()
                        .position(|c| self.artwork.selected.contains(c))
                        .map_or(0, |index| (index + 1) % self.artwork.candidates.len());
                    self.artwork.events.push(ArtworkEvent::CarveSelection(vec![
                        self.artwork.candidates[self.artwork.candidate_index].clone(),
                    ]));
                }
            }
        });
        ui.small(if self.artwork.mode != GestureMode::Select {"Drag the whole source · rotate/scale about setup 0,0 (numeric page origin) · Esc cancels"} else if self.is_knife() {"Click a knife path to assign it to this operation · Shift-click adds/removes · or use Geometry to cut in the operation"} else if self.is_profile() {"Click a closed contour to assign it to the profile with its suggested side · Shift-click adds/removes · choose Inside/Outside/On in the operation"} else if self.is_drill() {"Click a hole marker to assign it to this drill operation · Shift-click adds/removes"} else {"Click a filled region to assign it to this operation · Shift-click adds/removes · or use Geometry to carve in the operation"});
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
            if self.is_drill() {
                if response.clicked()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let setup = artwork_view::setup_point(camera, bounds, rect, p);
                    let nearest = self
                        .drill_points
                        .iter()
                        .filter_map(|point| {
                            if self
                                .artwork
                                .hidden
                                .contains(&point.reference.artwork_item_id.0)
                                || self
                                    .artwork
                                    .locked
                                    .contains(&point.reference.artwork_item_id.0)
                            {
                                return None;
                            }
                            let center = Point::new(point.center[0], point.center[1]);
                            let distance =
                                (p - artwork_view::screen_point(camera, bounds, rect, center))
                                    .length();
                            // Accept the drawn marker's interior as well as a fixed
                            // pixel target around its center when zoomed out.
                            let inside = (setup.x - center.x).hypot(setup.y - center.y)
                                <= point.diameter_mm / 2.;
                            (distance <= 8. || inside).then_some((distance, &point.reference))
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0));
                    if let Some((_, reference)) = nearest {
                        let mut selected: Vec<_> = self
                            .artwork
                            .selected
                            .iter()
                            .filter(|r| self.drill_points.iter().any(|p| &p.reference == *r))
                            .cloned()
                            .collect();
                        if ui.input(|i| i.modifiers.shift) {
                            if selected.contains(reference) {
                                selected.retain(|r| r != reference);
                            } else {
                                selected.push(reference.clone());
                            }
                        } else {
                            selected = vec![reference.clone()];
                        }
                        self.artwork
                            .events
                            .push(ArtworkEvent::DrillSelection(selected));
                    }
                }
                return;
            }
            if self.is_knife() {
                if response.clicked()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let mut nearest = None;
                    let mut distance = 8.;
                    for chain in self.knife_chains.iter().filter(|c| {
                        !self.artwork.hidden.contains(&c.reference.artwork_item_id.0)
                            && !self.artwork.locked.contains(&c.reference.artwork_item_id.0)
                    }) {
                        for i in 0..chain
                            .vertices
                            .len()
                            .saturating_sub(usize::from(!chain.closed))
                        {
                            let screen = |xy: [f64; 2]| {
                                artwork_view::screen_point(
                                    camera,
                                    bounds,
                                    rect,
                                    cam_core::geometry::Point::new(xy[0], xy[1]),
                                )
                            };
                            let a = screen(chain.vertices[i]);
                            let b = screen(chain.vertices[(i + 1) % chain.vertices.len()]);
                            let ab = b - a;
                            let t = if ab.length_sq() > 0. {
                                ((p - a).dot(ab) / ab.length_sq()).clamp(0., 1.)
                            } else {
                                0.
                            };
                            let d = (p - (a + ab * t)).length();
                            if d < distance {
                                distance = d;
                                nearest = Some(chain.reference.clone());
                            }
                        }
                    }
                    if let Some(reference) = nearest {
                        // Authoring selection can change while the scene is
                        // retained. Read the live qualified references just
                        // as Profile does, never the old preview's selection.
                        let known = self.available_references();
                        let mut selected: Vec<GeometryRef> = self
                            .artwork
                            .selected
                            .iter()
                            .filter(|r| known.contains(r))
                            .cloned()
                            .collect();
                        if ui.input(|i| i.modifiers.shift) {
                            if selected.contains(&reference) {
                                selected.retain(|r| r != &reference);
                            } else {
                                selected.push(reference);
                            }
                        } else {
                            selected = vec![reference];
                        }
                        self.artwork
                            .events
                            .push(ArtworkEvent::KnifeSelection(selected));
                    }
                }
                return;
            }
            if self.is_profile() {
                if response.clicked()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let mut nearest = None;
                    let mut distance = 8.;
                    for contour in self.profile_contours.iter().filter(|c| {
                        !self.artwork.hidden.contains(&c.reference.artwork_item_id.0)
                            && !self.artwork.locked.contains(&c.reference.artwork_item_id.0)
                    }) {
                        let count = contour.vertices.len();
                        for i in 0..count {
                            let screen = |xy: [f64; 2]| {
                                artwork_view::screen_point(
                                    camera,
                                    bounds,
                                    rect,
                                    cam_core::geometry::Point::new(xy[0], xy[1]),
                                )
                            };
                            let a = screen(contour.vertices[i]);
                            let b = screen(contour.vertices[(i + 1) % count]);
                            let ab = b - a;
                            let t = if ab.length_sq() > 0. {
                                ((p - a).dot(ab) / ab.length_sq()).clamp(0., 1.)
                            } else {
                                0.
                            };
                            let d = (p - (a + ab * t)).length();
                            if d < distance {
                                distance = d;
                                nearest = Some(contour.reference.clone());
                            }
                        }
                    }
                    if let Some(reference) = nearest {
                        let known = self.available_references();
                        let mut selected: Vec<GeometryRef> = self
                            .artwork
                            .selected
                            .iter()
                            .filter(|r| known.contains(r))
                            .cloned()
                            .collect();
                        if ui.input(|i| i.modifiers.shift) {
                            if selected.contains(&reference) {
                                selected.retain(|r| r != &reference);
                            } else {
                                selected.push(reference);
                            }
                        } else {
                            selected = vec![reference];
                        }
                        self.artwork
                            .events
                            .push(ArtworkEvent::ProfileSelection(selected));
                    }
                }
                return;
            }
            if response.clicked()
                && let Some(p) = response.interact_pointer_pos()
            {
                let point = artwork_view::setup_point(camera, bounds, rect, p);
                self.artwork.candidates =
                    artwork_view::hit_candidates(&self.artwork.components, point);
                self.artwork.candidates.retain(|r| {
                    !self.artwork.hidden.contains(&r.artwork_item_id.0)
                        && !self.artwork.locked.contains(&r.artwork_item_id.0)
                });
                self.artwork.candidate_index = 0;
                // Assigning from the viewport replaces or extends the
                // operation's own selection. Unresolved references are not
                // silently rewritten; they simply cannot be re-sent.
                let known = self.available_references();
                let mut selected: Vec<GeometryRef> = self
                    .artwork
                    .selected
                    .iter()
                    .filter(|r| known.contains(r))
                    .cloned()
                    .collect();
                let Some(chosen) = self.artwork.candidates.first().cloned() else {
                    return;
                };
                if ui.input(|i| i.modifiers.shift) {
                    if selected.contains(&chosen) {
                        selected.retain(|r| r != &chosen);
                    } else {
                        selected.push(chosen);
                    }
                } else {
                    selected = vec![chosen];
                }
                self.artwork
                    .events
                    .push(ArtworkEvent::CarveSelection(selected));
            }
            return;
        }
        if self.artwork.hidden.contains(&self.artwork.item)
            || self.artwork.locked.contains(&self.artwork.item)
        {
            self.artwork.drag = None;
            return;
        }
        if response.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            let start = artwork_view::setup_point(camera, bounds, rect, p);
            if self.is_knife()
                || self.artwork.mode != GestureMode::Move
                || artwork_view::hit_candidates(&self.artwork.components, start)
                    .iter()
                    .any(|r| r.artwork_item_id.0 == self.artwork.item)
            {
                self.artwork.drag = Some(Drag {
                    item: self.artwork.item.clone(),
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
                item: drag.item,
                revision: drag.revision,
                initial: drag.initial,
                value: drag.candidate,
            });
        }
    }
    pub(super) fn artwork_overlay(&self, ui: &egui::Ui, rect: egui::Rect) {
        if !self.stock_style.show_artwork {
            return;
        }
        let Some(scene) = &self.scene else {
            return;
        };
        let camera = self.camera(rect);
        let bounds = scene.meta.bounds;
        let painter = ui.painter().with_clip_rect(rect);
        // Draw the live operation selection after the scene and stock. Scene
        // geometry can be shared by several catalogues and retained across
        // edits; neither its draw order nor its old selection owns this signal.
        let selected = |reference: &GeometryRef| {
            self.artwork.selected.contains(reference)
                && !self.artwork.hidden.contains(&reference.artwork_item_id.0)
        };
        let highlight = egui::Stroke::new(2.5, Color32::from_rgb(64, 204, 217));
        let outline = |vertices: &[[f64; 2]], closed: bool| {
            let mut points: Vec<_> = vertices
                .iter()
                .map(|xy| {
                    artwork_view::screen_point(camera, bounds, rect, Point::new(xy[0], xy[1]))
                })
                .collect();
            if closed && let Some(first) = points.first().copied() {
                points.push(first);
            }
            painter.add(egui::Shape::line(
                points.clone(),
                egui::Stroke::new(4.5, Color32::BLACK),
            ));
            painter.add(egui::Shape::line(points, highlight));
        };
        for component in self
            .artwork
            .components
            .iter()
            .filter(|c| selected(&c.reference))
        {
            for ring in &component.rings {
                outline(ring, true);
            }
        }
        for chain in self.knife_chains.iter().filter(|c| selected(&c.reference)) {
            outline(&chain.vertices, chain.closed);
        }
        for contour in self
            .profile_contours
            .iter()
            .filter(|c| selected(&c.reference))
        {
            outline(&contour.vertices, true);
        }
        for point in self.drill_points.iter().filter(|p| selected(&p.reference)) {
            let center = artwork_view::screen_point(
                camera,
                bounds,
                rect,
                Point::new(point.center[0], point.center[1]),
            );
            painter.circle_stroke(center, 7., egui::Stroke::new(4.5, Color32::BLACK));
            painter.circle_stroke(center, 7., highlight);
            for delta in [egui::vec2(5., 0.), egui::vec2(0., 5.)] {
                painter.line_segment([center - delta, center + delta], highlight);
            }
        }
        if !self.artwork.enabled {
            return;
        }
        if let Some(drag) = &self.artwork.drag {
            for component in &self.artwork.components {
                if self
                    .artwork
                    .hidden
                    .contains(&component.reference.artwork_item_id.0)
                    || component.reference.artwork_item_id.0 != drag.item
                {
                    continue;
                }
                for ring in &component.rings {
                    let mut screen = Vec::new();
                    for p in ring {
                        let Ok(page) = drag.initial.to_page(Point::new(p[0], p[1])) else {
                            continue;
                        };
                        let Ok(next) = drag.candidate.to_setup(page) else {
                            continue;
                        };
                        screen.push(artwork_view::screen_point(camera, bounds, rect, next));
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
