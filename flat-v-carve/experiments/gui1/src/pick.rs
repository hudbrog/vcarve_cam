//! Display picking for the GUI1 viewport: CPU candidate index plus an exact
//! screen-distance test. No CAM meaning is attached to a pick; the returned
//! motion index is a display identity inside the transported motion pages.
//!
//! The camera here mirrors `scene.wgsl` exactly. Vertex positions are the same
//! normalized scene coordinates the shader receives, so a pick reproduces the
//! visible projection instead of a second, guessed transform:
//!
//! ```text
//! u   = x*cos(yaw) - y*sin(yaw)          // rotation
//! v   = x*sin(yaw) + y*cos(yaw)
//! ndc = (u*zoom/aspect, mix(v, 0.65*v + 0.76*z, iso)*zoom)
//! ```
//!
//! Tolerance is declared in **physical pixels** and converted through the
//! reported scale factor, so a display at 200% keeps the same physical hit
//! target instead of a hit target that halves on screen.
use serde::{Deserialize, Serialize};

pub const VERTEX_BYTES: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub iso: bool,
    pub aspect: f32,
    pub zoom: f32,
    pub yaw: f32,
}

impl Camera {
    pub fn ndc(&self, p: [f32; 3]) -> [f32; 2] {
        let (s, c) = self.yaw.sin_cos();
        let u = p[0] * c - p[1] * s;
        let v = p[0] * s + p[1] * c;
        let mixed = if self.iso { 0.65 * v + 0.76 * p[2] } else { v };
        [u * self.zoom / self.aspect, mixed * self.zoom]
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
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub motion: u32,
    pub distance_pixels: f32,
}

#[derive(Clone, Debug, Default)]
struct Cell {
    members: Vec<u32>,
}

/// Uniform grid over the scene's XY plane. The grid never depends on the
/// camera, so rotating or zooming does not rebuild it; the query widens its
/// search radius instead and the exact test stays in projected screen space.
pub struct Picker {
    endpoints: Vec<[f32; 3]>,
    z_reach: f32,
    origin: [f32; 2],
    cell: f32,
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl Picker {
    /// `xyz` holds both endpoints of every motion, in motion order.
    pub fn new(xyz: Vec<[f32; 3]>) -> Result<Self, String> {
        if !xyz.len().is_multiple_of(2) {
            return Err("Picking needs endpoint pairs".into());
        }
        if xyz.is_empty() {
            return Ok(Self {
                endpoints: xyz,
                z_reach: 0.,
                origin: [0., 0.],
                cell: 1.,
                cols: 0,
                rows: 0,
                cells: Vec::new(),
            });
        }
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        let mut z_reach = 0_f32;
        for p in &xyz {
            if !p.iter().all(|v| v.is_finite()) {
                return Err("Picking rejects non-finite scene coordinates".into());
            }
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
            z_reach = z_reach.max(p[2].abs());
        }
        let extent = (max[0] - min[0]).max(max[1] - min[1]).max(1e-3);
        let cell = extent / 64.;
        let cols = (((max[0] - min[0]) / cell) as usize + 1).max(1);
        let rows = (((max[1] - min[1]) / cell) as usize + 1).max(1);
        let mut picker = Self {
            endpoints: xyz,
            z_reach,
            origin: min,
            cell,
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
        };
        for motion in 0..picker.endpoints.len() / 2 {
            let a = picker.endpoints[motion * 2];
            let b = picker.endpoints[motion * 2 + 1];
            let lo = picker.cell_of([a[0].min(b[0]), a[1].min(b[1])]);
            let hi = picker.cell_of([a[0].max(b[0]), a[1].max(b[1])]);
            for row in lo[1]..=hi[1] {
                for col in lo[0]..=hi[0] {
                    picker.cells[row * picker.cols + col]
                        .members
                        .push(motion as u32);
                }
            }
        }
        Ok(picker)
    }

    /// Decode the XY endpoints of every motion from a packed vertex payload
    /// (`f32` position + `f32` colour per vertex, two vertices per motion).
    pub fn from_vertex_bytes(bytes: &[u8]) -> Result<Self, String> {
        if !bytes.len().is_multiple_of(VERTEX_BYTES * 2) {
            return Err("Picking payload is not a whole number of motion pairs".into());
        }
        let mut xy = Vec::with_capacity(bytes.len() / VERTEX_BYTES * 2);
        for vertex in bytes.chunks_exact(VERTEX_BYTES) {
            xy.push([
                f32::from_le_bytes(vertex[0..4].try_into().unwrap()),
                f32::from_le_bytes(vertex[4..8].try_into().unwrap()),
                f32::from_le_bytes(vertex[8..12].try_into().unwrap()),
            ]);
        }
        Self::new(xy)
    }

    pub fn motion_count(&self) -> usize {
        self.endpoints.len() / 2
    }

    pub fn memory_bytes(&self) -> usize {
        self.endpoints.len() * 12
            + self.cells.len() * std::mem::size_of::<Cell>()
            + self
                .cells
                .iter()
                .map(|c| c.members.len() * 4)
                .sum::<usize>()
    }

    pub fn endpoints(&self, motion: u32) -> Option<[[f32; 3]; 2]> {
        let i = motion as usize * 2;
        self.endpoints.get(i..i + 2).map(|p| [p[0], p[1]])
    }

    fn cell_of(&self, p: [f32; 2]) -> [usize; 2] {
        [
            (((p[0] - self.origin[0]) / self.cell) as usize).min(self.cols - 1),
            (((p[1] - self.origin[1]) / self.cell) as usize).min(self.rows - 1),
        ]
    }

    /// Model-space search radius that provably covers every segment whose
    /// projection can be within tolerance. Rotation preserves lengths, so a
    /// ball in model space is a valid superset of the projected tolerance.
    fn search_radius(&self, tolerance_points: f32, rect_points: [f32; 2], cam: &Camera) -> f32 {
        // Points per model unit along each axis. The viewport keeps
        // `aspect == width/height`, but the bound stays conservative if a
        // caller reports a different aspect, so it takes the smallest scale.
        let scale_x = (cam.zoom * rect_points[0] / 2. / cam.aspect.max(1e-6)).max(1e-6);
        let scale_y = (cam.zoom * rect_points[1] / 2.).max(1e-6);
        let along_x = tolerance_points / scale_x;
        let along_y = if cam.iso {
            // In isometric view the shader mixes 0.65*v + 0.76*z, so a
            // candidate with a different z can still project onto the cursor.
            (tolerance_points / scale_y + 0.76 * self.z_reach) / 0.65
        } else {
            tolerance_points / scale_y
        };
        along_x.max(along_y)
    }

    pub fn pick(
        &self,
        cam: &Camera,
        rect_points: [f32; 2],
        cursor_points: [f32; 2],
        tolerance_px: f32,
        pixels_per_point: f32,
    ) -> Option<Pick> {
        let tolerance_points = tolerance_px.max(0.) / pixels_per_point.max(1e-3);
        if self.endpoints.is_empty() {
            return None;
        }
        let cursor_ndc = cam.to_ndc(cursor_points, rect_points);
        let radius = self.search_radius(tolerance_points, rect_points, cam);
        // Cursor to model space: invert the shader's linear part, then search a
        // ball of `radius` around the point that projects onto that pixel.
        let (s, c) = cam.yaw.sin_cos();
        let rotated = [
            cursor_ndc[0] * cam.aspect / cam.zoom.max(1e-6),
            cursor_ndc[1] / cam.zoom.max(1e-6) / if cam.iso { 0.65 } else { 1. },
        ];
        let centre = [
            rotated[0] * c + rotated[1] * s,
            -rotated[0] * s + rotated[1] * c,
        ];
        let lo = self.cell_of([centre[0] - radius, centre[1] - radius]);
        let hi = self.cell_of([centre[0] + radius, centre[1] + radius]);
        let mut best: Option<(u32, f32)> = None;
        let mut seen = std::collections::HashSet::new();
        for row in lo[1]..=hi[1] {
            for col in lo[0]..=hi[0] {
                for &motion in &self.cells[row * self.cols + col].members {
                    if !seen.insert(motion) {
                        continue;
                    }
                    let i = motion as usize * 2;
                    let pa = cam.to_points(cam.ndc(self.endpoints[i]), rect_points);
                    let pb = cam.to_points(cam.ndc(self.endpoints[i + 1]), rect_points);
                    let distance = point_segment_distance(cursor_points, pa, pb);
                    if distance <= tolerance_points
                        && best.is_none_or(|previous| distance < previous.1)
                    {
                        best = Some((motion, distance));
                    }
                }
            }
        }
        best.map(|(motion, distance)| Pick {
            motion,
            distance_pixels: distance * pixels_per_point.max(1e-3),
        })
    }

    /// Test oracle: the same projected distance without the index.
    pub fn brute_force(
        &self,
        cam: &Camera,
        rect_points: [f32; 2],
        cursor_points: [f32; 2],
        tolerance_px: f32,
        pixels_per_point: f32,
    ) -> Option<Pick> {
        let tolerance_points = tolerance_px.max(0.) / pixels_per_point.max(1e-3);
        let mut best: Option<(u32, f32)> = None;
        for motion in 0..self.motion_count() {
            let i = motion * 2;
            let pa = cam.to_points(cam.ndc(self.endpoints[i]), rect_points);
            let pb = cam.to_points(cam.ndc(self.endpoints[i + 1]), rect_points);
            let distance = point_segment_distance(cursor_points, pa, pb);
            if distance <= tolerance_points && best.is_none_or(|previous| distance < previous.1) {
                best = Some((motion as u32, distance));
            }
        }
        best.map(|(motion, distance)| Pick {
            motion,
            distance_pixels: distance * pixels_per_point.max(1e-3),
        })
    }
}

pub fn point_segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length2 <= f32::EPSILON {
        0.
    } else {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / length2).clamp(0., 1.)
    };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Picker {
        let mut xyz = Vec::new();
        for i in 0..600 {
            let t = i as f32 / 600.;
            let x = -0.8 + 1.6 * t;
            let y = 0.5 * (t * 9.).sin();
            xyz.push([x, y, 0.02]);
            xyz.push([x + 0.02, y + 0.01, -0.14 * t]);
        }
        xyz.push([0.4, 0.4, 0.]);
        xyz.push([0.4, 0.4, 0.]);
        Picker::new(xyz).unwrap()
    }

    fn rects() -> Vec<[f32; 2]> {
        vec![[800., 600.], [1280., 800.], [640., 900.]]
    }

    #[test]
    fn indexed_pick_matches_brute_force_for_cameras_and_dpi() {
        let picker = fixture();
        for iso in [false, true] {
            for zoom in [0.5, 1., 2.4] {
                for yaw in [0., 0.7, -1.9] {
                    for rect in rects() {
                        // The viewport always reports aspect = width/height; the
                        // last entry deliberately disagrees to exercise the
                        // conservative radius.
                        for aspect in [rect[0] / rect[1], 0.8] {
                            let cam = Camera {
                                iso,
                                aspect,
                                zoom,
                                yaw,
                            };
                            for step in 0..48 {
                                let cursor = [
                                    -rect[0] / 2. + rect[0] * step as f32 / 47.,
                                    -rect[1] / 2. + rect[1] * ((step * 7) % 47) as f32 / 47.,
                                ];
                                for ppp in [1., 1.25, 1.5, 2.] {
                                    let index = picker.pick(&cam, rect, cursor, 8., ppp);
                                    let brute = picker.brute_force(&cam, rect, cursor, 8., ppp);
                                    match (index, brute) {
                                        (None, None) => {}
                                        (Some(a), Some(b)) => {
                                            assert_eq!(a.motion, b.motion);
                                            assert!(
                                                (a.distance_pixels - b.distance_pixels).abs()
                                                    < 1e-3
                                            );
                                        }
                                        other => panic!(
                                            "index/brute-force mismatch for {cam:?} {rect:?} {cursor:?} ppp {ppp}: {other:?}"
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hit_target_is_physical_pixels_at_every_scale_factor() {
        // One isolated horizontal segment, so the only question is how the
        // declared physical-pixel tolerance converts at each scale factor.
        let picker = Picker::new(vec![[-0.5, 0., 0.], [0.5, 0., 0.]]).unwrap();
        let cam = Camera {
            iso: false,
            aspect: 1.5,
            zoom: 1.,
            yaw: 0.,
        };
        let rect = [900., 600.];
        let anchor = [0., 0.];
        for ppp in [1., 1.25, 1.5, 2.] {
            // Ten physical pixels above the anchor, expressed in points.
            let cursor = [anchor[0], anchor[1] - 10. / ppp];
            let hit = picker
                .pick(&cam, rect, cursor, 12., ppp)
                .unwrap_or_else(|| panic!("ten physical pixels must be inside at {ppp}× DPI"));
            assert!(hit.distance_pixels <= 12.);
            assert_eq!(hit.motion, 0);
            // Thirty physical pixels is outside a twelve-pixel tolerance at
            // every scale factor.
            let miss = picker.pick(&cam, rect, [anchor[0], anchor[1] - 30. / ppp], 12., ppp);
            assert!(
                miss.is_none(),
                "tolerance must not grow with the DPI scale factor: {miss:?}"
            );
        }
        // The same cursor in points is inside at 100% DPI (12 point tolerance)
        // and outside at 200% DPI (6 point tolerance), which is the observable
        // difference a physical-pixel tolerance is defined on.
        let cursor = [0., -8.];
        assert!(picker.pick(&cam, rect, cursor, 12., 1.).is_some());
        assert!(picker.pick(&cam, rect, cursor, 12., 2.).is_none());
    }

    #[test]
    fn empty_and_malformed_payloads_are_rejected() {
        let empty = Picker::new(Vec::new()).unwrap();
        assert_eq!(empty.motion_count(), 0);
        assert!(
            empty
                .pick(
                    &Camera {
                        iso: false,
                        aspect: 1.,
                        zoom: 1.,
                        yaw: 0.
                    },
                    [100., 100.],
                    [0., 0.],
                    8.,
                    1.
                )
                .is_none()
        );
        assert!(Picker::from_vertex_bytes(&[0u8; 7]).is_err());
        assert!(Picker::new(vec![[f32::NAN, 0., 0.]]).is_err());
    }

    #[test]
    fn decoded_payload_matches_direct_endpoints() {
        let mut bytes = Vec::new();
        for (x, y) in [(0.1f32, 0.2f32), (0.3, 0.4), (-0.5, 0.25), (-0.6, 0.3)] {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
            bytes.extend_from_slice(&0f32.to_le_bytes());
            bytes.extend_from_slice(&[0u8; 16]);
        }
        let picker = Picker::from_vertex_bytes(&bytes).unwrap();
        assert_eq!(picker.motion_count(), 2);
        assert_eq!(
            picker.endpoints(1),
            Some([[-0.5, 0.25, 0.], [-0.6, 0.3, 0.]])
        );
    }
}
