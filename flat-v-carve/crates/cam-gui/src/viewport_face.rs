//! Facing overlay: the coverage the operation has to sweep.
//!
//! **Only the coverage is drawn.** The requested area, the pass span, the
//! travel and the entry positions are stated in the Face panel, where each one
//! has a number and a `(?)` definition attached; drawing them again in the
//! viewport produced boxes and lines that read as areas the user had to decode
//! and then asked to remove (tester, 2026-09-15). The scene still publishes the
//! whole resolution in `gui2.facePlans`, which is what keeps the panel and the
//! viewport from disagreeing and what the scene tests read.
//!
//! Display-only: nothing here plans, measures or exports.
use super::*;
use crate::overlay::Overlay;

const COVERAGE_LINE: [f32; 4] = [0.30, 0.85, 1.0, 0.90];
/// The outline is drawn just above the stock top, so it never z-fights with
/// the surface it annotates.
const LIFT: f64 = 0.05;

impl Viewport {
    pub(super) fn face_overlay(&self, out: &mut Overlay) {
        let Some(scene) = &self.scene else {
            return;
        };
        let bounds = scene.meta.bounds;
        let Some(plans) = scene.meta.report["gui2"]["facePlans"].as_array() else {
            return;
        };
        for plan in plans {
            let Some(values) = plan["coverage"].as_array() else {
                continue;
            };
            let coordinate = |index: usize| values.get(index).and_then(|value| value.as_f64());
            let (Some(x), Some(y), Some(width), Some(length)) =
                (coordinate(0), coordinate(1), coordinate(2), coordinate(3))
            else {
                continue;
            };
            let corners = [
                [x, y],
                [x + width, y],
                [x + width, y + length],
                [x, y + length],
            ];
            for index in 0..4 {
                for corner in [corners[index], corners[(index + 1) % 4]] {
                    out.lines.push(crate::compute::vertex(
                        [corner[0], corner[1], LIFT],
                        bounds,
                        COVERAGE_LINE,
                    ));
                }
            }
        }
    }
}
