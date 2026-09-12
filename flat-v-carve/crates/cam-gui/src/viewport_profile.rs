//! Profile candidate anchors and the generated tab bridges they produce.
//!
//! Candidate markers show where the document asks for a tab or a start; the
//! filled quads come from the generated plan's own tab placements, so the two
//! are never confused. Dragging a candidate is one gesture that becomes one
//! document command on release; nothing here plans, compensates or measures.
use super::*;
use crate::profile::{AnchorKind, project_ring, ring_point};

/// One candidate anchor the viewport draws and drags.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileAnchor {
    pub scope: String,
    pub kind: AnchorKind,
    pub point: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProfileAnchorEvent {
    /// A finished drag: the anchor belongs at this source fraction.
    Moved {
        scope: String,
        kind: AnchorKind,
        fraction: f64,
    },
}

#[derive(Clone, Debug)]
pub(super) struct AnchorDrag {
    pub(super) scope: String,
    pub(super) kind: AnchorKind,
    /// Document revision the gesture started from: a stale gesture is dropped
    /// rather than applied to a document that moved underneath it.
    pub(super) revision: u64,
    pub(super) point: [f64; 2],
}

const CANDIDATE_FILL: [f32; 4] = [1.0, 0.78, 0.18, 0.85];
const CANDIDATE_DRAG_FILL: [f32; 4] = [1.0, 0.45, 0.15, 0.95];
const BRIDGE_FILL: [f32; 4] = [0.20, 0.85, 0.45, 0.55];
const BRIDGE_LINE: [f32; 4] = [0.06, 0.42, 0.22, 0.95];

impl Viewport {
    /// Publish the candidate anchors of the selected profile operation. The
    /// values are display state: the document is the authority.
    pub fn set_profile_anchors(&mut self, anchors: Vec<ProfileAnchor>) {
        let signature = anchors.iter().fold(anchors.len() as u64, |hash, anchor| {
            let mut next = hash;
            for value in [anchor.point[0], anchor.point[1]] {
                next = next
                    .rotate_left(7)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add(value.to_bits());
            }
            next.rotate_left(3).wrapping_add(anchor.scope.len() as u64)
        });
        if self.profile_anchors != anchors || self.profile_anchor_signature != signature {
            self.overlay_signature = None;
        }
        self.profile_anchors = anchors;
        self.profile_anchor_signature = signature;
    }

    pub fn take_profile_anchor_events(&mut self) -> Vec<ProfileAnchorEvent> {
        std::mem::take(&mut self.profile_anchor_events)
    }

    /// Candidate anchors and generated bridges, in the same normalized scene
    /// space as the motion lines.
    pub(super) fn profile_overlay(&self, out: &mut overlay::Overlay) {
        let Some(scene) = &self.scene else {
            return;
        };
        let bounds = scene.meta.bounds;
        let project = |xy: [f64; 2], lift: f64| -> [f32; 3] {
            crate::compute::vertex([xy[0], xy[1], lift], bounds, [1.; 4]).position
        };
        // Generated bridges: exact protected cross-sections of the retained
        // plan, independent of the display grid.
        if let Some(operations) = scene.meta.report["gui2"]["inspection"]["operations"].as_array() {
            for operation in operations {
                let Some(placements) = operation["tabPlacements"].as_array() else {
                    continue;
                };
                for placement in placements {
                    let Some(quad) = placement["footprintMm"].as_array() else {
                        continue;
                    };
                    let points: Vec<[f32; 3]> = quad
                        .iter()
                        .filter_map(|point| Some([point[0].as_f64()?, point[1].as_f64()?]))
                        .map(|xy| project(xy, 0.04))
                        .collect();
                    if points.len() != 4 {
                        continue;
                    }
                    out.triangles.push(crate::compute::Vertex {
                        position: points[0],
                        color: BRIDGE_FILL,
                    });
                    out.triangles.push(crate::compute::Vertex {
                        position: points[1],
                        color: BRIDGE_FILL,
                    });
                    out.triangles.push(crate::compute::Vertex {
                        position: points[2],
                        color: BRIDGE_FILL,
                    });
                    out.triangles.push(crate::compute::Vertex {
                        position: points[0],
                        color: BRIDGE_FILL,
                    });
                    out.triangles.push(crate::compute::Vertex {
                        position: points[2],
                        color: BRIDGE_FILL,
                    });
                    out.triangles.push(crate::compute::Vertex {
                        position: points[3],
                        color: BRIDGE_FILL,
                    });
                    for index in 0..4 {
                        out.lines.push(crate::compute::Vertex {
                            position: points[index],
                            color: BRIDGE_LINE,
                        });
                        out.lines.push(crate::compute::Vertex {
                            position: points[(index + 1) % 4],
                            color: BRIDGE_LINE,
                        });
                    }
                }
            }
        }
        // Candidate markers: the document's own request.
        let dragging = self.anchor_drag.as_ref().map(|drag| &drag.scope);
        for anchor in &self.profile_anchors {
            let point = match &self.anchor_drag {
                Some(drag) if drag.scope == anchor.scope && drag.kind == anchor.kind => drag.point,
                _ => anchor.point,
            };
            let fill = if dragging == Some(&anchor.scope) {
                CANDIDATE_DRAG_FILL
            } else {
                CANDIDATE_FILL
            };
            overlay::candidate_marker(out, project(point, 0.05), fill);
        }
    }

    /// Drag one candidate anchor along its contour. The gesture captures the
    /// anchor's identity and revision at press time; releasing emits exactly
    /// one move event, which the application commits as one Undo transaction.
    pub(super) fn profile_anchor_pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
    ) {
        if !self.profile_selected || !self.artwork.enabled {
            self.anchor_drag = None;
            return;
        }
        let Some(scene) = &self.scene else {
            return;
        };
        let bounds = scene.meta.bounds;
        let camera = self.camera(rect);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.anchor_drag = None;
            return;
        }
        if response.drag_started()
            && let Some(press) = ui.input(|i| i.pointer.press_origin())
        {
            let mut nearest: Option<(f32, &ProfileAnchor)> = None;
            for anchor in &self.profile_anchors {
                let screen = crate::artwork_view::screen_point(
                    camera,
                    bounds,
                    rect,
                    cam_core::geometry::Point::new(anchor.point[0], anchor.point[1]),
                );
                let distance = press.distance(screen);
                if distance <= 12. && nearest.is_none_or(|(best, _)| distance < best) {
                    nearest = Some((distance, anchor));
                }
            }
            if let Some((_, anchor)) = nearest {
                self.anchor_drag = Some(AnchorDrag {
                    scope: anchor.scope.clone(),
                    kind: anchor.kind,
                    revision: self.artwork.revision,
                    point: anchor.point,
                });
                self.overlay_signature = None;
            }
        }
        let Some(scope) = self.anchor_drag.as_ref().map(|drag| drag.scope.clone()) else {
            return;
        };
        if let Some(position) = response.interact_pointer_pos() {
            let point = crate::artwork_view::setup_point(camera, bounds, rect, position);
            if let Some(ring) = self.anchor_ring(&scope)
                && let Some((projected, _)) = project_ring(&ring, [point.x, point.y])
                && let Some(drag) = self.anchor_drag.as_mut()
            {
                drag.point = projected;
                self.overlay_signature = None;
            }
        }
        if response.drag_stopped() {
            let Some(drag) = self.anchor_drag.take() else {
                return;
            };
            if drag.revision != self.artwork.revision {
                self.status = "Document changed during the drag; drag the anchor again.".into();
                return;
            }
            let Some(ring) = self.anchor_ring(&drag.scope) else {
                self.status = "That contour is no longer in the artwork.".into();
                return;
            };
            let Some((_, fraction)) = project_ring(&ring, drag.point) else {
                return;
            };
            self.profile_anchor_events.push(ProfileAnchorEvent::Moved {
                scope: drag.scope,
                kind: drag.kind,
                fraction,
            });
            self.overlay_signature = None;
        }
    }

    fn anchor_ring(&self, scope: &str) -> Option<Vec<[f64; 2]>> {
        self.profile_contours
            .iter()
            .find(|contour| contour.wire_id == scope)
            .map(|contour| contour.anchor_ring.clone())
    }

    /// The candidate position of one anchor, for the editor's readout.
    pub fn profile_anchor_point(&self, scope: &str, fraction: f64) -> Option<[f64; 2]> {
        ring_point(&self.anchor_ring(scope)?, fraction)
    }
}
