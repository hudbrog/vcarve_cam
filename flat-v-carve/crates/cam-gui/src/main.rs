#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "--worker") {
        let result = args
            .get(2)
            .ok_or_else(|| "Missing worker mailbox".to_owned())
            .and_then(|folder| cam_gui_runtime::worker::serve(std::path::PathBuf::from(folder)));
        if result.is_err() {
            std::process::exit(2);
        }
        return Ok(());
    }
    eframe::run_native(
        "Flat V-carve",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280., 800.])
                .with_min_inner_size([1000., 650.]),
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 24,
            multisampling: 0,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(cam_gui_runtime::app::App::new(cc)))),
    )
}
#[cfg(target_arch = "wasm32")]
fn main() {}
