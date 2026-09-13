//! Persistent tiled stock buffer. Only tiles whose version changed since the
//! last upload are copied, so switching checkpoints transfers the changed part
//! of the field instead of the whole grid.
use crate::sim::TILE;
use crate::stock_style::{PALETTE_ENTRIES, StockUniform};
use eframe::egui_wgpu::{self, wgpu};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::sync::{Arc, Mutex};

pub const TILE_BYTES: usize = TILE * TILE * 4;
/// Bytes of one palette entry (RGBA).
const PALETTE_BYTES: u64 = (PALETTE_ENTRIES * 16) as u64;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub tile_uploads: u64,
    pub tile_bytes: u64,
    pub skipped_tiles: u64,
    /// Changed tiles the last frame's copy budget pushed to a later frame.
    pub pending_tiles: usize,
    /// Frames that ended with tiles still pending, i.e. how long a cold load
    /// took. Zero on an idle scene.
    pub frames_loading: u64,
    pub full_frame_bytes: u64,
    pub buffer_bytes: u64,
    pub recoveries: u64,
    pub grid: [usize; 2],
}

pub type SharedStats = Arc<Mutex<Stats>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Drill {
    #[default]
    None,
    Recover,
}

pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    camera: wgpu::Buffer,
    grid: wgpu::Buffer,
    cells: wgpu::Buffer,
    style: wgpu::Buffer,
    palette: wgpu::Buffer,
    palette_revision: Option<u64>,
    capacity: u64,
    format: wgpu::TextureFormat,
    identity: u64,
    uploaded_versions: Vec<u32>,
    pub stats: SharedStats,
}

impl Resources {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, stats: SharedStats) -> Self {
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAM GUI stock camera"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAM GUI stock grid"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cells = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAM GUI stock cells"),
            size: TILE_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let style = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAM GUI stock style"),
            size: std::mem::size_of::<StockUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAM GUI stock palette"),
            size: PALETTE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (pipeline, layout, bind) =
            build_gpu(device, format, &camera, &grid, &cells, &style, &palette);
        Self {
            pipeline,
            layout,
            bind,
            camera,
            grid,
            cells,
            style,
            palette,
            palette_revision: None,
            capacity: TILE_BYTES as u64,
            format,
            identity: u64::MAX,
            uploaded_versions: Vec::new(),
            stats,
        }
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        let (pipeline, layout, bind) = build_gpu(
            device,
            self.format,
            &self.camera,
            &self.grid,
            &self.cells,
            &self.style,
            &self.palette,
        );
        self.pipeline = pipeline;
        self.layout = layout;
        self.bind = bind;
    }

    fn recreate(&mut self, device: &wgpu::Device) {
        self.cells = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAM GUI stock cells"),
            size: self.capacity.max(TILE_BYTES as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uploaded_versions.clear();
        self.palette_revision = None;
        self.rebuild(device);
        self.stats.lock().unwrap().recoveries += 1;
    }
}

fn build_gpu(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera: &wgpu::Buffer,
    grid: &wgpu::Buffer,
    cells: &wgpu::Buffer,
    style: &wgpu::Buffer,
    palette: &wgpu::Buffer,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::BindGroup) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("CAM GUI stock"),
        source: wgpu::ShaderSource::Wgsl(include_str!("stock.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("CAM GUI stock layout"),
        entries: &[
            entry(0, wgpu::BufferBindingType::Uniform),
            entry(1, wgpu::BufferBindingType::Uniform),
            entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            entry_with(
                3,
                wgpu::BufferBindingType::Uniform,
                wgpu::ShaderStages::VERTEX_FRAGMENT,
            ),
            entry_with(
                4,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::ShaderStages::FRAGMENT,
            ),
        ],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("CAM GUI stock bind"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: cells.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: style.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: palette.as_entire_binding(),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("CAM GUI stock cells"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_stock"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_stock"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    (pipeline, layout, bind)
}

fn entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    entry_with(binding, ty, wgpu::ShaderStages::VERTEX)
}

fn entry_with(
    binding: u32,
    ty: wgpu::BufferBindingType,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub struct Callback {
    /// Whole scene payload, or locally re-integrated cells for a seek between
    /// transported checkpoints.
    pub payload: Option<Arc<Vec<u8>>>,
    pub cells: Range<usize>,
    pub local: Option<Arc<Vec<u8>>>,
    pub cols: usize,
    pub rows: usize,
    pub tiles_x: usize,
    pub tiles_y: usize,
    /// Per-tile change counter from the transported field.
    pub versions: Arc<Vec<u32>>,
    /// Identity of the scene's field evolution; a change re-uploads every tile.
    pub identity: u64,
    pub revision: u64,
    /// The `camera::Camera::uniform` value `stock.wgsl` reads.
    pub camera: [f32; 8],
    pub grid: [f32; 10],
    /// Display-only style: colour mode, ramps, light and appearance.
    pub style: StockUniform,
    /// One RGBA per stage index followed by one per tool index.
    pub palette: Arc<Vec<[f32; 4]>>,
    /// Changes when the palette contents change, so the buffer is uploaded on
    /// change instead of every frame.
    pub palette_revision: u64,
    pub drill: Drill,
}

impl egui_wgpu::CallbackTrait for Callback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &egui_wgpu::ScreenDescriptor,
        _: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        if self.drill == Drill::Recover {
            r.recreate(device);
        }
        let tiles = self.tiles_x * self.tiles_y;
        if r.identity != self.identity || r.uploaded_versions.len() != tiles {
            r.identity = self.identity;
            r.uploaded_versions = vec![u32::MAX; tiles];
        }
        let needed = (tiles * TILE_BYTES) as u64;
        if needed > r.capacity {
            r.capacity = needed.next_power_of_two();
            r.cells = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("CAM GUI stock cells"),
                size: r.capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            r.rebuild(device);
            r.uploaded_versions = vec![u32::MAX; tiles];
            r.stats.lock().unwrap().buffer_bytes = r.capacity;
        }
        queue.write_buffer(&r.camera, 0, bytemuck::cast_slice(&self.camera));
        queue.write_buffer(&r.grid, 0, bytemuck::cast_slice(&self.grid));
        queue.write_buffer(&r.style, 0, bytemuck::bytes_of(&self.style));
        if r.palette_revision != Some(self.palette_revision) {
            let mut bytes = Vec::with_capacity(PALETTE_BYTES as usize);
            for entry in self.palette.iter().take(PALETTE_ENTRIES) {
                bytes.extend_from_slice(bytemuck::cast_slice(entry));
            }
            // A palette shorter than the buffer keeps its empty slots black
            // rather than reading whatever the driver left behind.
            bytes.resize(PALETTE_BYTES as usize, 0);
            queue.write_buffer(&r.palette, 0, &bytes);
            r.palette_revision = Some(self.palette_revision);
        }
        let mut uploaded = 0_u64;
        let mut skipped = 0_u64;
        let source: &[u8] = match (&self.local, &self.payload) {
            (Some(local), _) => local,
            (None, Some(payload)) => &payload[self.cells.clone()],
            (None, None) => return Vec::new(),
        };
        // One frame copies a bounded number of changed tiles. Everything else
        // keeps its version mismatch and is copied on the following frames, so
        // a cold load at the finest preset cannot stall a frame.
        let batch = crate::stock_preview::next_tile_batch(
            &self.versions,
            &r.uploaded_versions,
            crate::stock_preview::TILE_UPLOADS_PER_FRAME,
        );
        let pending_before =
            crate::stock_preview::dirty_tiles(&self.versions, &r.uploaded_versions).len();
        skipped += (tiles as u64).saturating_sub(pending_before as u64);
        for tile in batch.iter().map(|tile| *tile as usize) {
            let offset = tile * TILE_BYTES;
            let start = offset;
            let end = (start + TILE_BYTES).min(source.len());
            if start >= source.len() {
                skipped += 1;
                continue;
            }
            queue.write_buffer(&r.cells, offset as u64, &source[start..end]);
            let version = self.versions.get(tile).copied().unwrap_or(u32::MAX);
            r.uploaded_versions[tile] = version;
            uploaded += 1;
        }
        {
            let mut stats = r.stats.lock().unwrap();
            let remaining =
                crate::stock_preview::dirty_tiles(&self.versions, &r.uploaded_versions).len();
            stats.tile_uploads += uploaded;
            stats.skipped_tiles += skipped;
            stats.tile_bytes += uploaded * TILE_BYTES as u64;
            stats.pending_tiles = remaining;
            if remaining > 0 {
                stats.frames_loading += 1;
            }
            stats.full_frame_bytes = (self.cols * self.rows * 4) as u64;
            stats.buffer_bytes = r.capacity;
            stats.grid = [self.cols, self.rows];
        }
        Vec::new()
    }

    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(r) = resources.get::<Resources>() {
            pass.set_pipeline(&r.pipeline);
            pass.set_bind_group(0, &r.bind, &[]);
            pass.draw(0..draw_vertices(self.cols, self.rows), 0..1);
        }
    }
}

/// Vertices `stock.wgsl` emits for a `cols × rows` field: one quad per cell,
/// one wall quad per boundary cell (the four edges, corners counted twice) and
/// the stock's bottom face. The shader derives the same three ranges from
/// `grid.cols`/`grid.rows`, so this must stay in step with it.
pub fn draw_vertices(cols: usize, rows: usize) -> u32 {
    (cols * rows * 6 + (2 * cols + 2 * rows) * 6 + 6) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror of `stock.wgsl`'s perimeter walk: the `(cell, along x, edge)`
    /// triple of every wall quad, in the order the shader emits them.
    fn wall_quads(cols: usize, rows: usize) -> Vec<((usize, usize), bool, u8)> {
        (0..2 * cols + 2 * rows)
            .map(|wall| {
                if wall < cols {
                    ((wall, 0), true, 0)
                } else if wall < cols + rows {
                    ((0, wall - cols), false, 0)
                } else if wall < 2 * cols + rows {
                    ((wall - (cols + rows), rows - 1), true, 1)
                } else {
                    ((cols - 1, wall - (2 * cols + rows)), false, 1)
                }
            })
            .collect()
    }

    /// The four walls must tile their whole edge: every boundary cell owns the
    /// quad next to it, so a faced plate loses its rim and a partially faced
    /// one steps down instead of leaving a gap.
    #[test]
    fn the_four_walls_cover_every_boundary_cell_once() {
        let (cols, rows) = (4, 3);
        let quads = wall_quads(cols, rows);
        assert_eq!(
            quads.len(),
            2 * cols + 2 * rows,
            "one quad per boundary cell"
        );
        let mut edges = [vec![], vec![], vec![], vec![]];
        for (cell, along_x, high) in quads {
            let (along, edge) = match (along_x, high) {
                (true, 0) => {
                    assert_eq!(cell.1, 0, "the low-Y edge walks row 0");
                    (cell.0, 0)
                }
                (false, 0) => {
                    assert_eq!(cell.0, 0, "the low-X edge walks column 0");
                    (cell.1, 1)
                }
                (true, _) => {
                    assert_eq!(cell.1, rows - 1, "the high-Y edge walks the last row");
                    (cell.0, 2)
                }
                _ => {
                    assert_eq!(cell.0, cols - 1, "the high-X edge walks the last column");
                    (cell.1, 3)
                }
            };
            edges[edge].push(along);
        }
        for (edge, covered) in edges.iter_mut().enumerate() {
            covered.sort();
            let expect = if edge % 2 == 0 { cols } else { rows };
            assert_eq!(*covered, (0..expect).collect::<Vec<_>>(), "edge {edge}");
        }
    }

    /// Mirror of `stock.wgsl`'s wall vertex: one corner of one wall quad, in
    /// stock coordinates, from the packed field.
    #[allow(clippy::too_many_arguments)]
    fn wall_corner(
        cells: &[u32],
        cols: usize,
        cell: (usize, usize),
        along_x: bool,
        high: f32,
        corner: [f32; 2],
        extent: [f32; 2],
        cell_mm: f32,
        thickness: f32,
    ) -> [f32; 3] {
        let packed = cells[cell.1 * cols + cell.0];
        let depth = (packed & 0xffff) as f32 / 65535.;
        let along_extent = if along_x { extent[0] } else { extent[1] };
        let edge_extent = if along_x { extent[1] } else { extent[0] };
        let along = (((if along_x { cell.0 } else { cell.1 }) as f32 + corner[0]) * cell_mm)
            .min(along_extent);
        let fixed = high * edge_extent;
        let (x, y) = if along_x {
            (along, fixed)
        } else {
            (fixed, along)
        };
        let surface = -depth * thickness;
        [x, y, surface + (-thickness - surface) * corner[1]]
    }

    /// The walk alone is not enough to place a wall: it has to stand on the
    /// stock boundary its edge names, stay inside the rectangle, and take its
    /// height from its own cell. A wall measured against the axis it runs along
    /// instead of the edge it stands on lands beside the stock or inside it,
    /// which is exactly what the review build showed.
    #[test]
    fn every_wall_quad_stands_on_its_own_boundary_and_follows_its_cell() {
        let (cols, rows) = (5, 3);
        let extent = [200_f32, 100_f32];
        let thickness = 18_f32;
        let cell_mm = extent[0] / cols as f32;
        // Boundary cells of the low-Y edge are faced 4 mm; everything else is
        // untouched, so the test covers both a stepped wall and a full one.
        let mut cells = vec![0u32; cols * rows];
        for cell in cells.iter_mut().take(cols) {
            *cell = 65535 / 18 * 4;
        }
        let corners = [[0., 0.], [1., 0.], [0., 1.], [1., 1.]];
        for (cell, along_x, high) in wall_quads(cols, rows) {
            let mut zs = vec![];
            for corner in corners {
                let [x, y, z] = wall_corner(
                    &cells,
                    cols,
                    cell,
                    along_x,
                    high as f32,
                    corner,
                    extent,
                    cell_mm,
                    thickness,
                );
                assert!(
                    x >= -1e-3 && x <= extent[0] + 1e-3 && y >= -1e-3 && y <= extent[1] + 1e-3,
                    "wall {cell:?} along_x {along_x} edge {high} leaves the stock at ({x}, {y})"
                );
                let on_boundary = if along_x {
                    y == high as f32 * extent[1]
                } else {
                    x == high as f32 * extent[0]
                };
                assert!(
                    on_boundary,
                    "wall {cell:?} along_x {along_x} edge {high} is not on its edge: ({x}, {y})"
                );
                zs.push(z);
            }
            let depth = (cells[cell.1 * cols + cell.0] & 0xffff) as f32 / 65535.;
            assert!(
                zs.iter().all(
                    |z| (*z - -depth * thickness).abs() < 1e-3 || (*z + thickness).abs() < 1e-3
                ),
                "wall {cell:?} spans {zs:?} instead of its cell's remaining material"
            );
            assert!(
                zs.iter().any(|z| *z < 0.),
                "an untouched boundary cell keeps the full stock thickness"
            );
        }
        // A cell cut through leaves no wall height at all.
        cells[0] = 65535;
        let faced = wall_corner(
            &cells,
            cols,
            (0, 0),
            true,
            0.,
            [0., 0.],
            extent,
            cell_mm,
            thickness,
        );
        assert_eq!(faced[2], -thickness, "a through cut leaves no wall");
    }

    /// The stock pass is one un-indexed draw whose vertex count `stock.wgsl`
    /// splits into three ranges by index. If either side changes alone the pass
    /// draws a truncated frame or indexes past the field, so the ranges are
    /// pinned against each other here.
    #[test]
    fn the_drawn_vertex_count_matches_the_shader_ranges() {
        let shader = include_str!("stock.wgsl");
        assert!(shader.contains("count*6u"), "cell range");
        assert!(shader.contains("(2u*cols+2u*rows)*6u"), "wall range");
        assert!(
            shader.contains("array<u32,6>(4u,5u,6u,4u,6u,7u)"),
            "bottom face"
        );
        for (cols, rows) in [(0, 0), (1, 1), (2, 1), (500, 250)] {
            assert_eq!(
                draw_vertices(cols, rows),
                (cols * rows * 6 + (2 * cols + 2 * rows) * 6 + 6) as u32,
                "{cols}x{rows}"
            );
        }
        // An empty field still draws the stock's bottom face.
        assert_eq!(draw_vertices(0, 0), 6);
        // A fine 200 × 100 mm field at the standard 0.4 mm cells adds one wall
        // quad per boundary cell: 1.2% more vertices than the surface alone.
        assert_eq!(draw_vertices(500, 250) - 500 * 250 * 6, 1_500 * 6 + 6);
    }

    /// Mirror of `stock.wgsl`'s surface normal: `(t·∂d/∂x, t·∂d/∂y, 1)`, where
    /// `d` is the *removed* fraction and `t` the stock thickness. The shading
    /// only reads the direction, so the test pins that rather than pixels.
    fn surface_normal(
        left: f32,
        right: f32,
        down: f32,
        up: f32,
        thickness: f32,
        cell: f32,
    ) -> [f32; 3] {
        let step = (2. * cell).max(1e-6);
        [
            (right - left) * thickness / step,
            (up - down) * thickness / step,
            1.,
        ]
    }

    #[test]
    fn the_surface_normal_points_up_and_leans_with_the_gradient() {
        // Flat material: straight up, whatever the depth.
        assert_eq!(surface_normal(0.2, 0.2, 0.2, 0.2, 18., 0.4), [0., 0., 1.]);
        // Deeper material to the +x side means the surface falls away in +x, so
        // the normal leans that way: the step face reads as an edge, and a
        // V-bit flank reads as a slope instead of a flat colour.
        let falling = surface_normal(0.1, 0.3, 0.2, 0.2, 18., 0.4);
        assert!(falling[0] > 0. && falling[2] > 0., "{falling:?}");
        let mirrored = surface_normal(0.3, 0.1, 0.2, 0.2, 18., 0.4);
        assert!((mirrored[0] + falling[0]).abs() < 1e-6);
        // A finer display cell reports a steeper gradient for the same step,
        // which is what keeps shading consistent across display presets.
        let coarse = surface_normal(0.1, 0.2, 0.1, 0.1, 18., 0.8);
        let fine = surface_normal(0.1, 0.2, 0.1, 0.1, 18., 0.2);
        assert!(fine[0] > coarse[0]);
    }
}
