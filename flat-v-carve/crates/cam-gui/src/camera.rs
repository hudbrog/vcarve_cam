//! The preview camera: one orthographic orbit shared by the GPU renderer, the
//! CPU picker and every screen-space overlay.
//!
//! Vertices arrive in the normalized scene coordinates `compute::vertex`
//! writes, so the camera never needs the document, the stock rectangle or the
//! frame size:
//!
//! ```text
//! u        = x*cos(yaw) - y*sin(yaw)
//! v        = x*sin(yaw) + y*cos(yaw)
//! screen_y = v*cos(tilt) + z*sin(tilt)
//! depth    = z*cos(tilt) - v*sin(tilt)
//! ndc      = ((u + pan.x)*zoom/aspect, (screen_y + pan.y)*zoom)
//! ```
//!
//! `tilt` is the elevation: `0` looks straight down and is the historical top
//! view, `ISO_TILT` reproduces the historical isometric mix
//! `0.65*v + 0.76*z`, larger values swing the camera toward the side of the
//! part and negative values look from below the table. `pan` is a view-target
//! offset in the same normalized scene units, applied *before* `zoom`, so a
//! panned point stays where it is when the zoom changes.
//!
//! This module is the single source of the projection. `scene.wgsl` and
//! `stock.wgsl` read [`Camera::uniform`], `pick.rs` and `artwork_view.rs` use
//! [`Camera::ndc`] and [`Camera::ground_point`], and the tests below pin the
//! historical isometric constants, the inverse conversion and the interaction
//! deltas.
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// Elevation of the historical isometric view: `atan2(0.76, 0.65)`. The
/// historical pair is not exactly orthonormal (`0.65² + 0.76² = 1.0001`), so
/// this reproduces the mix `0.65*v + 0.76*z` to within 4e-5 of its own scale.
pub const ISO_TILT: f32 = 0.863_254_7;
/// How far the view may tilt, in radians (85°). The limit keeps the projection
/// away from the exactly edge-on view, where the ground-plane pointer
/// conversion has no usable solution.
pub const TILT_LIMIT: f32 = 85. * PI / 180.;
/// Zoom limits. One is the framing the scene normalization was chosen for: at
/// zoom 1 the scene fills 80% of the viewport height.
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 64.;
/// Azimuth and elevation change per viewport point of an orbit drag.
pub const ORBIT_PER_POINT: f32 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub yaw: f32,
    pub tilt: f32,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub aspect: f32,
}

impl Default for Camera {
    /// The default view is the top view of the framed scene.
    fn default() -> Self {
        Self {
            yaw: 0.,
            tilt: 0.,
            zoom: 1.,
            pan: [0., 0.],
            aspect: 1.,
        }
    }
}

impl Camera {
    pub fn new(aspect: f32) -> Self {
        Self {
            aspect: aspect.max(0.2),
            ..Default::default()
        }
    }

    /// Project a normalized scene point to NDC, mirroring `scene.wgsl`.
    pub fn ndc(&self, p: [f32; 3]) -> [f32; 2] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let u = p[0] * cos_yaw - p[1] * sin_yaw;
        let v = p[0] * sin_yaw + p[1] * cos_yaw;
        let (sin_tilt, cos_tilt) = self.tilt.sin_cos();
        [
            (u + self.pan[0]) * self.zoom / self.aspect,
            (v * cos_tilt + p[2] * sin_tilt + self.pan[1]) * self.zoom,
        ]
    }

    /// The renderer's depth term for the same point, mirroring `scene.wgsl`.
    pub fn depth(&self, p: [f32; 3]) -> f32 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let v = p[0] * sin_yaw + p[1] * cos_yaw;
        let (sin_tilt, cos_tilt) = self.tilt.sin_cos();
        (0.5 - (p[2] * cos_tilt - v * sin_tilt) * 0.2).clamp(0.01, 0.99)
    }

    /// NDC to points inside `rect_points` (width, height). Y is inverted
    /// because window coordinates grow downward.
    pub fn to_points(&self, ndc: [f32; 2], rect_points: [f32; 2]) -> [f32; 2] {
        [ndc[0] * rect_points[0] / 2., -ndc[1] * rect_points[1] / 2.]
    }

    pub fn to_ndc(&self, point: [f32; 2], rect_points: [f32; 2]) -> [f32; 2] {
        [
            point[0] * 2. / rect_points[0].max(1.),
            -point[1] * 2. / rect_points[1].max(1.),
        ]
    }

    /// The `Camera` uniform `scene.wgsl` and `stock.wgsl` read, in field order.
    pub fn uniform(&self) -> [f32; 8] {
        [
            self.yaw,
            self.tilt,
            self.zoom,
            self.pan[0],
            self.pan[1],
            self.aspect,
            0.,
            0.,
        ]
    }

    pub fn set_tilt(&mut self, tilt: f32) {
        self.tilt = tilt.clamp(-TILT_LIMIT, TILT_LIMIT);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Orbit by a pointer drag in viewport points. The camera follows the
    /// pointer: dragging right swings the camera around the part to the right,
    /// dragging down dips it toward the side view.
    pub fn orbit(&mut self, delta_points: [f32; 2]) {
        self.yaw += delta_points[0] * ORBIT_PER_POINT;
        self.set_tilt(self.tilt + delta_points[1] * ORBIT_PER_POINT);
    }

    /// Pan by a pointer drag in viewport points, so the scene follows the
    /// pointer.
    pub fn pan_by(&mut self, delta_points: [f32; 2], rect_points: [f32; 2]) {
        let per_point = 2. / (self.zoom * rect_points[1].max(1.));
        self.pan[0] += delta_points[0] * per_point;
        self.pan[1] -= delta_points[1] * per_point;
    }

    /// Zoom by `factor`, keeping the scene point under `cursor_ndc` fixed.
    pub fn zoom_toward(&mut self, cursor_ndc: [f32; 2], factor: f32) {
        let before = self.zoom;
        self.set_zoom(self.zoom * factor);
        let k = 1. / self.zoom - 1. / before;
        if k == 0. {
            return;
        }
        self.pan[0] += cursor_ndc[0] * self.aspect * k;
        self.pan[1] += cursor_ndc[1] * k;
    }

    /// The `z = 0` scene point under a cursor position, in normalized scene
    /// units. `None` when the ground plane is edge-on.
    pub fn ground_point(&self, cursor_points: [f32; 2], rect_points: [f32; 2]) -> Option<[f32; 2]> {
        let cos_tilt = self.tilt.cos();
        if cos_tilt.abs() < 1e-3 {
            return None;
        }
        let ndc = self.to_ndc(cursor_points, rect_points);
        let v = (ndc[1] / self.zoom - self.pan[1]) / cos_tilt;
        let u = ndc[0] * self.aspect / self.zoom - self.pan[0];
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Some([u * cos_yaw + v * sin_yaw, -u * sin_yaw + v * cos_yaw])
    }

    /// Points per normalized scene unit along both axes, matching the
    /// projection whenever `aspect == width/height`.
    pub fn points_per_unit(&self, rect_points: [f32; 2]) -> f32 {
        self.zoom * rect_points[1].max(1.) / 2.
    }

    /// World-space screen basis of this view: `(right, up, to_camera)`.
    ///
    /// Derived from the same projection the shader applies, so a light or a
    /// two-sided normal test built from it stays consistent with what the
    /// viewer actually sees. All three are unit length.
    pub fn screen_basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_tilt, cos_tilt) = self.tilt.sin_cos();
        let right = [cos_yaw, -sin_yaw, 0.];
        let up = [sin_yaw * cos_tilt, cos_yaw * cos_tilt, sin_tilt];
        let to_camera = [-sin_yaw * sin_tilt, -cos_yaw * sin_tilt, cos_tilt];
        (right, up, to_camera)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    /// Saved views, the browser review scenarios and the historical renderer
    /// all assume this mix. It must not drift.
    #[test]
    fn the_isometric_preset_reproduces_the_historical_mix() {
        let camera = Camera {
            yaw: 0.,
            tilt: ISO_TILT,
            zoom: 1.,
            pan: [0., 0.],
            aspect: 1.,
        };
        let (sin_tilt, cos_tilt) = camera.tilt.sin_cos();
        assert!(close(cos_tilt, 0.65, 1e-4), "cos {cos_tilt}");
        assert!(close(sin_tilt, 0.76, 1e-4), "sin {sin_tilt}");
        for p in [[0.5, -0.25, 0.1], [-0.7, 0.3, 0.], [0., 0., -0.2]] {
            let (sin_yaw, cos_yaw) = camera.yaw.sin_cos();
            let u = p[0] * cos_yaw - p[1] * sin_yaw;
            let v = p[0] * sin_yaw + p[1] * cos_yaw;
            let projected = camera.ndc(p);
            assert!(close(projected[0], u * camera.zoom, 1e-6), "{projected:?}");
            assert!(
                close(projected[1], (0.65 * v + 0.76 * p[2]) * camera.zoom, 1e-4),
                "{projected:?}"
            );
            assert!(close(
                camera.depth(p),
                0.5 - (p[2] * 0.65 - v * 0.76) * 0.2,
                1e-4
            ));
        }
    }

    #[test]
    fn the_top_preset_is_the_plain_plane_view() {
        let camera = Camera {
            yaw: 0.4,
            tilt: 0.,
            zoom: 2.,
            pan: [0., 0.],
            aspect: 1.5,
        };
        let (sin_yaw, cos_yaw) = camera.yaw.sin_cos();
        let p = [0.3, -0.6, 0.15];
        let v = p[0] * sin_yaw + p[1] * cos_yaw;
        assert_eq!(camera.ndc(p)[1], v * camera.zoom);
        assert_eq!(
            camera.ndc(p)[0],
            (p[0] * cos_yaw - p[1] * sin_yaw) * camera.zoom / camera.aspect
        );
    }

    #[test]
    fn orbit_follows_the_pointer_and_stops_at_the_limits() {
        let mut camera = Camera::new(1.);
        camera.orbit([20., 0.]);
        assert!(close(camera.yaw, 20. * ORBIT_PER_POINT, 1e-6));
        assert_eq!(camera.tilt, 0.);
        camera.orbit([0., 40.]);
        assert!(close(camera.tilt, 40. * ORBIT_PER_POINT, 1e-6));
        camera.orbit([0., -10_000.]);
        assert_eq!(camera.tilt, -TILT_LIMIT);
        camera.orbit([0., 10_000.]);
        assert_eq!(camera.tilt, TILT_LIMIT);
    }

    #[test]
    fn pan_moves_the_scene_with_the_pointer() {
        let mut camera = Camera::new(1.);
        camera.pan_by([10., -5.], [800., 400.]);
        // 2 / (zoom * height) normalized units per point.
        assert!(close(camera.pan[0], 10. * 2. / 400., 1e-6));
        assert!(close(camera.pan[1], 5. * 2. / 400., 1e-6));
        // The point that was under the cursor follows it: a drag up-screen
        // moves the scene up-screen.
        let rect = [800., 400.];
        let cursor = [-100., -50.];
        let before = Camera::new(2.).ground_point(cursor, rect).unwrap();
        let mut moved = Camera::new(2.);
        moved.pan_by([10., -5.], rect);
        let after = moved
            .ground_point([cursor[0] + 10., cursor[1] - 5.], rect)
            .unwrap();
        assert!(close(before[0], after[0], 1e-5) && close(before[1], after[1], 1e-5));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let rect = [900., 500.];
        for tilt in [0., ISO_TILT, 1.2] {
            for factor in [1.5, 0.4] {
                let mut camera = Camera::new(rect[0] / rect[1]);
                camera.tilt = tilt;
                camera.pan = [0.05, -0.02];
                let cursor = [120., -80.];
                let before = camera.ground_point(cursor, rect).unwrap();
                let ndc = camera.to_ndc(cursor, rect);
                camera.zoom_toward(ndc, factor);
                let after = camera.ground_point(cursor, rect).unwrap();
                assert!(
                    close(before[0], after[0], 1e-4) && close(before[1], after[1], 1e-4),
                    "tilt {tilt} factor {factor}: {before:?} -> {after:?}"
                );
            }
        }
        // The limit still applies, and a clamped zoom keeps the cursor point.
        let mut camera = Camera::new(1.);
        camera.zoom_toward([0.5, 0.5], 1e9);
        assert_eq!(camera.zoom, MAX_ZOOM);
    }

    #[test]
    fn ground_point_inverts_the_projection() {
        for tilt in [-1.0, 0., ISO_TILT, 1.4] {
            for yaw in [0., 0.7, -2.1] {
                let mut camera = Camera::new(1.7);
                camera.yaw = yaw;
                camera.tilt = tilt;
                camera.zoom = 2.3;
                camera.pan = [-0.11, 0.07];
                let rect = [1024., 640.];
                for cursor in [[0., 0.], [200., -140.], [-320., 90.]] {
                    let ground = camera.ground_point(cursor, rect).unwrap();
                    let projected = camera.ndc([ground[0], ground[1], 0.]);
                    let expected = camera.to_ndc(cursor, rect);
                    assert!(
                        close(projected[0], expected[0], 1e-4)
                            && close(projected[1], expected[1], 1e-4),
                        "tilt {tilt} yaw {yaw} cursor {cursor:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn zoom_and_tilt_stay_inside_their_limits() {
        let mut camera = Camera::new(1.);
        camera.set_zoom(1e6);
        assert_eq!(camera.zoom, MAX_ZOOM);
        camera.set_zoom(0.);
        assert_eq!(camera.zoom, MIN_ZOOM);
        camera.set_tilt(-9.);
        assert_eq!(camera.tilt, -TILT_LIMIT);
        // Edge-on views are refused rather than divided by zero.
        camera.tilt = PI / 2.;
        assert!(camera.ground_point([0., 0.], [100., 100.]).is_none());
    }

    #[test]
    fn the_uniform_carries_every_field_in_shader_order() {
        let mut camera = Camera::new(2.);
        camera.yaw = 0.3;
        camera.tilt = 0.7;
        camera.zoom = 1.4;
        camera.pan = [-0.2, 0.4];
        assert_eq!(camera.uniform(), [0.3, 0.7, 1.4, -0.2, 0.4, 2., 0., 0.]);
    }

    /// The stock pass lights itself from this basis and flips a normal that
    /// faces away from the viewer; both have to agree with the projection.
    #[test]
    fn the_screen_basis_matches_the_projection() {
        for yaw in [0., 0.7, -2.1] {
            for tilt in [0., ISO_TILT, -1.2] {
                let mut camera = Camera::new(1.3);
                camera.yaw = yaw;
                camera.tilt = tilt;
                let (right, up, to_camera) = camera.screen_basis();
                for axis in [right, up, to_camera] {
                    let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
                    assert!((length - 1.).abs() < 1e-5, "unit length: {axis:?}");
                }
                // A point offset along `right` moves only across the screen, one
                // along `up` moves only up it, and one along `to_camera` comes
                // toward the viewer without moving on screen.
                let scale = 0.05;
                let along_right: [f32; 3] = right.map(|v| v * scale);
                let along_up: [f32; 3] = up.map(|v| v * scale);
                let along_camera: [f32; 3] = to_camera.map(|v| v * scale);
                assert!(camera.ndc(along_right)[0] > 0.);
                assert!(camera.ndc(along_up)[1] > 0.);
                assert!(camera.ndc(along_camera)[1].abs() < 1e-5);
                assert!(camera.depth(along_camera) < camera.depth([0., 0., 0.]));
            }
        }
    }
}
