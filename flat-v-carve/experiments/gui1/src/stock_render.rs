//! Persistent tiled stock buffer. Only tiles whose version changed since the
//! last upload are copied, so switching checkpoints transfers the changed part
//! of the field instead of the whole grid.
use crate::sim::TILE;
use eframe::egui_wgpu::{self, wgpu};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::sync::{Arc, Mutex};

pub const TILE_BYTES: usize = TILE * TILE * 4;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub tile_uploads: u64,
    pub tile_bytes: u64,
    pub skipped_tiles: u64,
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
    capacity: u64,
    format: wgpu::TextureFormat,
    identity: u64,
    uploaded_versions: Vec<u32>,
    pub stats: SharedStats,
}

impl Resources {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, stats: SharedStats) -> Self {
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GUI1 stock camera"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GUI1 stock grid"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cells = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GUI1 stock cells"),
            size: TILE_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (pipeline, layout, bind) = build_gpu(device, format, &camera, &grid, &cells);
        Self {
            pipeline,
            layout,
            bind,
            camera,
            grid,
            cells,
            capacity: TILE_BYTES as u64,
            format,
            identity: u64::MAX,
            uploaded_versions: Vec::new(),
            stats,
        }
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        let (pipeline, layout, bind) =
            build_gpu(device, self.format, &self.camera, &self.grid, &self.cells);
        self.pipeline = pipeline;
        self.layout = layout;
        self.bind = bind;
    }

    fn recreate(&mut self, device: &wgpu::Device) {
        self.cells = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GUI1 stock cells"),
            size: self.capacity.max(TILE_BYTES as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uploaded_versions.clear();
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
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::BindGroup) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("GUI1 stock"),
        source: wgpu::ShaderSource::Wgsl(include_str!("stock.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("GUI1 stock layout"),
        entries: &[
            entry(0, wgpu::BufferBindingType::Uniform),
            entry(1, wgpu::BufferBindingType::Uniform),
            entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
        ],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("GUI1 stock bind"),
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
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("GUI1 stock cells"),
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
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
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
    pub camera: [f32; 4],
    pub grid: [f32; 10],
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
                label: Some("GUI1 stock cells"),
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
        let mut uploaded = 0_u64;
        let mut skipped = 0_u64;
        let source: &[u8] = match (&self.local, &self.payload) {
            (Some(local), _) => local,
            (None, Some(payload)) => &payload[self.cells.clone()],
            (None, None) => return Vec::new(),
        };
        for tile in 0..tiles {
            let version = self.versions.get(tile).copied().unwrap_or(u32::MAX);
            if r.uploaded_versions[tile] == version {
                skipped += 1;
                continue;
            }
            let offset = tile * TILE_BYTES;
            let start = offset;
            let end = (start + TILE_BYTES).min(source.len());
            if start >= source.len() {
                skipped += 1;
                continue;
            }
            queue.write_buffer(&r.cells, offset as u64, &source[start..end]);
            r.uploaded_versions[tile] = version;
            uploaded += 1;
        }
        {
            let mut stats = r.stats.lock().unwrap();
            stats.tile_uploads += uploaded;
            stats.skipped_tiles += skipped;
            stats.tile_bytes += uploaded * TILE_BYTES as u64;
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
            pass.draw(0..self.cols as u32 * self.rows as u32 * 6 + 30, 0..1);
        }
    }
}
