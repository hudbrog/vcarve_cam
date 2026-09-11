#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "--capture") {
        let folder =
            std::path::PathBuf::from(args.get(2).expect("--capture requires an output directory"));
        std::fs::create_dir_all(&folder).expect("create capture folder");
        let flower = args.get(3).is_some_and(|v| v == "flower");
        let export = args.get(4).is_some_and(|v| v == "export");
        let begin = std::time::Instant::now();
        let result =
            cam_gui1::compute::run(cam_gui1::compute::Request::Reference { flower, export });
        let mut report = match result {
            Ok(scene) => scene.report,
            Err(error) => serde_json::json!({"error":error}),
        };
        report["elapsedMs"] = serde_json::json!(begin.elapsed().as_secs_f64() * 1000.);
        std::fs::write(
            folder.join(if flower {
                "flower-reference.json"
            } else {
                "small-reference.json"
            }),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .expect("write evidence");
        if report.get("error").is_some() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.get(1).is_some_and(|a| a == "--worker") {
        let result = (|| -> Result<(), String> {
            let input =
                std::fs::read(args.get(2).ok_or("Missing request")?).map_err(|e| e.to_string())?;
            let request: cam_gui1::compute::Request =
                serde_json::from_slice(&input).map_err(|e| e.to_string())?;
            if matches!(request, cam_gui1::compute::Request::Crash) {
                std::process::exit(9);
            }
            let reply = cam_gui1::compute::run(request);
            let file = std::fs::File::create(args.get(3).ok_or("Missing output")?)
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(file, &reply).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            std::process::exit(2);
        }
        return Ok(());
    }
    eframe::run_native(
        "Flat V-carve · GUI1 experiment",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280., 800.])
                .with_min_inner_size([1000., 650.]),
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 24,
            multisampling: 0,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(cam_gui1::app::App::new(cc)))),
    )
}
#[cfg(target_arch = "wasm32")]
fn main() {}
