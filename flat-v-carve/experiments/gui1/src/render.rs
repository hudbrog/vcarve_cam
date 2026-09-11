//! Paged wgpu scene batches plus translucent overlay geometry.
//!
//! Motion geometry is uploaded page by page. A page whose fingerprint matches
//! the resident page is never copied again, so an idle frame, a camera change
//! or a reload of the identical scene performs no transfer at all. Pages the
//! display no longer needs are evicted once the resident budget is exceeded;
//! they are re-uploaded from the retained CPU payload rather than re-planned.
use crate::compute::Vertex;
use crate::pages::PageTable;
use crate::paging::Pager;
use eframe::egui_wgpu::{self, wgpu};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub const VERTEX_BYTES: usize = std::mem::size_of::<Vertex>();
pub const DEFAULT_PAGE_BUDGET: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub upload_bytes: u64,
    pub uploads: u64,
    pub uploads_skipped: u64,
    pub page_uploads: u64,
    pub overlay_bytes: u64,
    pub resident_pages: usize,
    pub resident_bytes: u64,
    /// Pages the display asked for but the resident budget did not admit.
    pub budget_omitted: usize,
    pub admitted_pages: usize,
    pub evictions: u64,
    pub recoveries: u64,
    pub injected_errors: u64,
    pub last_error: Option<String>,
    pub scene_buffer_bytes: u64,
}

pub type SharedStats = Arc<Mutex<Stats>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Drill {
    #[default]
    None,
    /// Destroy and rebuild every persistent GPU resource from retained CPU data.
    Recover,
    /// Submit a real invalid wgpu operation and capture the reported error.
    InjectError,
}

/// Page-aligned byte base for the motion section inside the GPU buffer.
pub fn page_base_bytes(contour_vertices: usize) -> usize {
    (contour_vertices * VERTEX_BYTES).div_ceil(VERTEX_BYTES) * VERTEX_BYTES
}

pub struct Resources {
    line_pipeline: wgpu::RenderPipeline,
    triangle_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    camera: wgpu::Buffer,
    scene: wgpu::Buffer,
    overlay_lines: wgpu::Buffer,
    overlay_triangles: wgpu::Buffer,
    scene_capacity: u64,
    lines_capacity: u64,
    triangles_capacity: u64,
    format: wgpu::TextureFormat,
    pager: Pager,
    overlay_revision: u64,
    pub stats: SharedStats,
}

impl Resources {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, stats: SharedStats) -> Self {
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GUI1 camera"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (line_pipeline, triangle_pipeline, layout, bind) = build_gpu(device, format, &camera);
        let resources = Self {
            line_pipeline,
            triangle_pipeline,
            layout,
            bind,
            camera,
            scene: buffer(device, 4, wgpu::BufferUsages::VERTEX),
            overlay_lines: buffer(device, 4, wgpu::BufferUsages::VERTEX),
            overlay_triangles: buffer(device, 4, wgpu::BufferUsages::VERTEX),
            scene_capacity: 4,
            lines_capacity: 4,
            triangles_capacity: 4,
            format,
            pager: Pager::new(DEFAULT_PAGE_BUDGET),
            overlay_revision: u64::MAX,
            stats,
        };
        resources.stats.lock().unwrap().scene_buffer_bytes = resources.scene_capacity;
        resources
    }

    /// Rebuild pipelines, bind group and buffers. Recovery uses this path, so
    /// every page is re-copied from the retained CPU payload on the next frame.
    fn recreate(&mut self, device: &wgpu::Device) {
        let (line_pipeline, triangle_pipeline, layout, bind) =
            build_gpu(device, self.format, &self.camera);
        self.line_pipeline = line_pipeline;
        self.triangle_pipeline = triangle_pipeline;
        self.layout = layout;
        self.bind = bind;
        self.scene = buffer(device, self.scene_capacity, wgpu::BufferUsages::VERTEX);
        self.overlay_lines = buffer(device, self.lines_capacity, wgpu::BufferUsages::VERTEX);
        self.overlay_triangles =
            buffer(device, self.triangles_capacity, wgpu::BufferUsages::VERTEX);
        self.overlay_revision = u64::MAX;
        self.pager.forget();
        let mut stats = self.stats.lock().unwrap();
        stats.recoveries += 1;
        stats.resident_pages = 0;
        stats.resident_bytes = 0;
        stats.last_error = None;
        stats.scene_buffer_bytes = self.scene_capacity;
    }

    fn ensure_scene(&mut self, device: &wgpu::Device, need: u64) {
        if need > self.scene_capacity {
            self.scene_capacity = need.next_power_of_two();
            self.scene = buffer(device, self.scene_capacity, wgpu::BufferUsages::VERTEX);
            self.pager.forget();
            self.stats.lock().unwrap().scene_buffer_bytes = self.scene_capacity;
        }
    }
}

fn build_gpu(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera: &wgpu::Buffer,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::BindGroupLayout,
    wgpu::BindGroup,
) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("GUI1 scene"),
        source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("GUI1 scene layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("GUI1 scene bind"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = |topology: wgpu::PrimitiveTopology| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GUI1 scene"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: VERTEX_BYTES as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology,
                ..Default::default()
            },
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
        })
    };
    (
        pipeline(wgpu::PrimitiveTopology::LineList),
        pipeline(wgpu::PrimitiveTopology::TriangleList),
        layout,
        bind,
    )
}

fn buffer(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("GUI1 retained batch"),
        size: size.max(4),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub struct Callback {
    /// Whole scene payload; pages are copied straight out of it.
    pub payload: Arc<Vec<u8>>,
    /// Identity of the payload contents; a new identity re-uploads pages.
    pub identity: u64,
    pub table: PageTable,
    /// Fingerprint per page, computed once by the display process.
    pub hashes: Arc<Vec<Option<u64>>>,
    /// Pages the current display needs, nearest the playhead first.
    pub required: Vec<usize>,
    pub budget_bytes: u64,
    pub contour_vertices: usize,
    pub revision: u64,
    pub camera: [f32; 4],
    pub lines: Arc<Vec<Vertex>>,
    pub triangles: Arc<Vec<Vertex>>,
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
        if r.pager.budget() != self.budget_bytes {
            r.pager.set_budget(self.budget_bytes);
            r.overlay_revision = u64::MAX;
        }
        let base_bytes = page_base_bytes(self.contour_vertices);
        r.ensure_scene(device, base_bytes as u64 + self.table.buffer_bytes() as u64);
        queue.write_buffer(&r.camera, 0, bytemuck::cast_slice(&self.camera));
        let table = self.table;
        let plan = r.pager.plan(
            self.identity,
            &table,
            &self.hashes,
            &self.required,
            |page| table.bytes_of(page),
        );
        for upload in &plan.uploads {
            let offset = base_bytes as u64 + upload.slot_offset as u64;
            queue.write_buffer(
                &r.scene,
                offset,
                &self.payload[upload.offset..upload.offset + upload.len],
            );
        }
        // Overlay geometry is small and changes with the selection or playhead.
        if r.overlay_revision != self.revision {
            let line_bytes = bytemuck::cast_slice(&self.lines);
            let triangle_bytes = bytemuck::cast_slice(&self.triangles);
            if line_bytes.len() as u64 > r.lines_capacity {
                r.lines_capacity = (line_bytes.len() as u64).next_power_of_two();
                r.overlay_lines = buffer(device, r.lines_capacity, wgpu::BufferUsages::VERTEX);
            }
            if triangle_bytes.len() as u64 > r.triangles_capacity {
                r.triangles_capacity = (triangle_bytes.len() as u64).next_power_of_two();
                r.overlay_triangles =
                    buffer(device, r.triangles_capacity, wgpu::BufferUsages::VERTEX);
            }
            if !line_bytes.is_empty() {
                queue.write_buffer(&r.overlay_lines, 0, line_bytes);
            }
            if !triangle_bytes.is_empty() {
                queue.write_buffer(&r.overlay_triangles, 0, triangle_bytes);
            }
            r.overlay_revision = self.revision;
            r.stats.lock().unwrap().overlay_bytes +=
                (line_bytes.len() + triangle_bytes.len()) as u64;
        }
        {
            let mut stats = r.stats.lock().unwrap();
            stats.uploads += plan.uploads.len() as u64;
            stats.page_uploads += plan.uploads.len() as u64;
            stats.uploads_skipped += plan.skipped as u64;
            stats.upload_bytes += plan.upload_bytes();
            stats.evictions += plan.evicted as u64;
            stats.budget_omitted = plan.omitted;
            stats.admitted_pages = plan.admitted;
            stats.resident_pages = plan.resident_pages;
            stats.resident_bytes = plan.resident_bytes;
        }
        if self.drill == Drill::InjectError {
            inject_validation_error(device, r);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = resources.get::<Resources>() else {
            return;
        };
        let base_vertex = (page_base_bytes(self.contour_vertices) / VERTEX_BYTES) as u32;
        let page_vertices = (self.table.page_motions * 2) as u32;
        if self.contour_vertices > 0 {
            pass.set_pipeline(&r.line_pipeline);
            pass.set_bind_group(0, &r.bind, &[]);
            pass.set_vertex_buffer(0, r.scene.slice(..));
            pass.draw(0..self.contour_vertices as u32, 0..1);
        }
        pass.set_pipeline(&r.line_pipeline);
        pass.set_bind_group(0, &r.bind, &[]);
        pass.set_vertex_buffer(0, r.scene.slice(..));
        for page in &self.required {
            if !r.pager.is_resident(*page) {
                continue;
            }
            let motions = self.table.motions_in(*page);
            let count = ((motions.end - motions.start) * 2) as u32;
            if count == 0 {
                continue;
            }
            let start = base_vertex + *page as u32 * page_vertices;
            pass.draw(start..start + count, 0..1);
        }
        if !self.triangles.is_empty() {
            pass.set_pipeline(&r.triangle_pipeline);
            pass.set_vertex_buffer(0, r.overlay_triangles.slice(..));
            pass.draw(0..self.triangles.len() as u32, 0..1);
        }
        if !self.lines.is_empty() {
            pass.set_pipeline(&r.line_pipeline);
            pass.set_vertex_buffer(0, r.overlay_lines.slice(..));
            pass.draw(0..self.lines.len() as u32, 0..1);
        }
    }
}

/// Submit a real invalid operation and read the reported error. The device
/// stays usable: only the invalid request is dropped.
fn inject_validation_error(device: &wgpu::Device, resources: &mut Resources) {
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let invalid = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("GUI1 invalid size probe"),
        size: 3,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: false,
    });
    drop(invalid);
    let message = drain_error_scope(device);
    let mut stats = resources.stats.lock().unwrap();
    stats.injected_errors += 1;
    stats.last_error =
        Some(message.unwrap_or_else(|| "error scope did not resolve on this backend".into()));
}

fn drain_error_scope(device: &wgpu::Device) -> Option<String> {
    let mut scope = std::pin::pin!(device.pop_error_scope());
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    for _ in 0..256 {
        if let std::task::Poll::Ready(error) = scope.as_mut().poll(&mut context) {
            return error.map(|error| error.to_string());
        }
        if device.poll(wgpu::PollType::Poll).is_err() {
            break;
        }
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::yield_now();
    }
    None
}
