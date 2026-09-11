//! One persistent packed height buffer; upload only when the checkpoint changes.
use eframe::egui_wgpu::{self, wgpu};
use std::sync::Arc;
pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    camera: wgpu::Buffer,
    grid: wgpu::Buffer,
    cells: wgpu::Buffer,
    capacity: u64,
    revision: (u64, usize),
    pub upload_bytes: u64,
}
impl Resources {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GUI1 stock"),
            source: wgpu::ShaderSource::Wgsl(include_str!("stock.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Uniform),
                entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let camera = buffer(device, 16, wgpu::BufferUsages::UNIFORM);
        let grid = buffer(device, 32, wgpu::BufferUsages::UNIFORM);
        let cells = buffer(device, 4, wgpu::BufferUsages::STORAGE);
        let bind = bind(device, &layout, &camera, &grid, &cells);
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
        Self {
            pipeline,
            layout,
            bind,
            camera,
            grid,
            cells,
            capacity: 4,
            revision: (u64::MAX, usize::MAX),
            upload_bytes: 0,
        }
    }
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
fn buffer(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("GUI1 stock retained"),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera: &wgpu::Buffer,
    grid: &wgpu::Buffer,
    cells: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
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
    })
}
pub struct Callback {
    pub cells: Arc<Vec<u32>>,
    pub revision: (u64, usize),
    pub camera: [f32; 4],
    pub grid: [f32; 8],
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
        if let Some(r) = resources.get_mut::<Resources>() {
            if r.revision != self.revision {
                let bytes = bytemuck::cast_slice(&self.cells);
                if bytes.len() as u64 > r.capacity {
                    r.capacity = bytes.len() as u64;
                    r.cells = buffer(device, r.capacity, wgpu::BufferUsages::STORAGE);
                    r.bind = bind(device, &r.layout, &r.camera, &r.grid, &r.cells);
                }
                queue.write_buffer(&r.cells, 0, bytes);
                queue.write_buffer(&r.grid, 0, bytemuck::cast_slice(&self.grid));
                r.revision = self.revision;
                r.upload_bytes += bytes.len() as u64;
            }
            queue.write_buffer(&r.camera, 0, bytemuck::cast_slice(&self.camera));
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
            pass.draw(0..self.cells.len() as u32 * 6 + 30, 0..1);
        }
    }
}
