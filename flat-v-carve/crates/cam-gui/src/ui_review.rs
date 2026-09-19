//! Opt-in review build: actual native wgpu screenshots without loading or
//! writing the user's recovery/catalog. Uses the normal session and UI.
use super::*;
use std::{ffi::OsString, path::PathBuf};

struct Review {
    app: App,
    output: PathBuf,
    frames: usize,
    requested: bool,
    size: egui::Vec2,
    stock_zero: bool,
}

pub fn run(args: &[OsString]) -> eframe::Result {
    let failure =
        |message: &str| eframe::Error::AppCreation(Box::new(std::io::Error::other(message)));
    if args.len() < 3 {
        return Err(failure(
            "--ui-review JOB OUTPUT.png PANEL [WIDTH HEIGHT SCALE]",
        ));
    }
    let job = std::fs::read_to_string(&args[0])
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let output = PathBuf::from(&args[1]);
    let panel = args[2].to_string_lossy().into_owned();
    let number = |index: usize, fallback: f32| {
        args.get(index)
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(fallback)
    };
    let size = egui::vec2(number(3, 1280.), number(4, 800.));
    let scale = number(5, 1.);
    if !size.is_finite() || size.min_elem() < 320. || !(0.5..=3.).contains(&scale) {
        return Err(failure("Invalid review size/scale"));
    }
    eframe::run_native(
        "CAM UI review",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size(size),
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 24,
            multisampling: 0,
            ..Default::default()
        },
        Box::new(move |cc| {
            cc.egui_ctx.set_pixels_per_point(scale);
            let mut app = App {
                view: View::new_viewer(cc),
                ..Default::default()
            };
            App::theme(&cc.egui_ctx);
            app.recovery.enabled = false;
            app.active = Some((1, 0));
            let opened = engine::run(Command::Open { json: job.clone() })?;
            app.accept(1, Ok(opened), &cc.egui_ctx);
            app.active = Some((2, app.revision));
            let scene = engine::run(if panel.starts_with("simulation") {
                Command::generate(job)
            } else {
                Command::Preview { job }
            })?;
            app.accept(2, Ok(scene), &cc.egui_ctx);
            app.preview_dirty = false;
            app.inspector_tab = match panel.as_str() {
                "artwork" => 0,
                "stock" | "stock-zero" => 1,
                "machine" => 3,
                "simulation" | "simulation-section" | "simulation-warnings" => 6,
                "settings" => 7,
                _ => 2,
            };
            app.simulate = panel.starts_with("simulation");
            if app.simulate {
                let mut settings = app.view.settings();
                settings.inspection_tab = match panel.as_str() {
                    "simulation-section" => 1,
                    "simulation-warnings" => 2,
                    _ => 0,
                };
                app.view.restore_settings(&settings);
                // Review generation runs synchronously in this thread; there
                // is no worker-owned execution to rebuild for the same preset.
                app.view.take_preset_request();
            }
            app.operation_tab = match panel.as_str() {
                "operation-cutting" => 1,
                "operation-last" => 2,
                _ => 0,
            };
            app.resources.ready = true;
            app.resources.draft = crate::resources::Catalog::decode(include_str!(
                "../../../fixtures/gui5/library.json"
            ))?;
            if panel == "library-knife" {
                let doc = app.document.as_ref().unwrap();
                let source = doc
                    .job
                    .tools
                    .iter()
                    .find(|t| {
                        matches!(
                            t.geometry,
                            Some(cam_core::project::ToolGeometry::DragKnife(_))
                        )
                    })
                    .ok_or_else(|| failure("Knife review needs a knife job"))?;
                let tool = crate::resources::capture_tool(
                    source,
                    "review-knife".into(),
                    source.name.clone(),
                )
                .map_err(|e| failure(&e))?;
                app.resources.tool = tool.id.clone();
                app.resources.draft.library.tools.push(tool);
            }
            app.resources.base = Some(crate::resources::StoredCatalog {
                revision: app.resources.draft.library.revision,
                snapshot: app.resources.draft.clone(),
            });
            if panel.starts_with("library") {
                app.open_resource(ResourcePage::ToolLibrary);
                if panel == "library-vbit" {
                    app.resources.tool = "vbit".into();
                    app.resources.preset = "finish".into();
                }
            } else if panel == "machines" {
                app.open_resource(ResourcePage::MachineLibrary);
            } else if panel == "tools" {
                app.open_resource(ResourcePage::JobTools);
            } else if panel == "picker" {
                app.operation_tab = 1;
                app.operation_picker = Some(0);
            }
            Ok(Box::new(Review {
                app,
                output,
                frames: 0,
                requested: false,
                size,
                stock_zero: panel == "stock-zero",
            }))
        }),
    )
}

impl eframe::App for Review {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply the review scroll after the initial DPI resize/layout settles.
        if self.stock_zero && self.frames == 8 {
            self.app.scroll[1] = 600.;
        }
        self.app.ui(ctx);
        self.frames += 1;
        if self.frames == 1 {
            // Window creation uses the OS scale; resize in egui points once
            // the explicit review scale is active.
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.size));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(shot) = shot {
            if let Some(parent) = self.output.parent() {
                std::fs::create_dir_all(parent).expect("review output directory");
            }
            let bytes: Vec<_> = shot.pixels.iter().flat_map(|c| c.to_array()).collect();
            image::save_buffer(
                &self.output,
                &bytes,
                shot.width() as u32,
                shot.height() as u32,
                image::ColorType::Rgba8,
            )
            .expect("write native screenshot");
            let metadata = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "pixels": shot.size,
                "pixelsPerPoint": ctx.pixels_per_point(),
                "workspace": self.app.workspace(),
                "controls": CONTROLS.with(|c| c.borrow().clone()),
                "renderer": self.app.view.renderer_probe(),
            });
            std::fs::write(
                self.output.with_extension("json"),
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .expect("write native review metadata");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.frames > 15 && !self.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        assert!(self.frames < 600, "Native review screenshot timed out");
        ctx.request_repaint();
    }
}
