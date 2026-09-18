#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
/// Record every panic where the user can find it. The release build is a
/// Windows-subsystem binary with no console, so without this a startup failure
/// looks like nothing happening at all: no window, no message, nothing to
/// report. The log sits beside the executable, the same place the recovery
/// store lives, and falls back to the temporary directory when that is not
/// writable.
#[cfg(not(target_arch = "wasm32"))]
fn install_panic_log() {
    const LIMIT: u64 = 256 * 1024;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let message = format!("cam-gui {} panic: {info}\n", env!("CARGO_PKG_VERSION"));
        let target = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join("cam-gui.log")))
            .unwrap_or_else(|| std::env::temp_dir().join("cam-gui.log"));
        if std::fs::metadata(&target).is_ok_and(|meta| meta.len() > LIMIT) {
            let _ = std::fs::remove_file(&target);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)
        {
            let _ = file.write_all(message.as_bytes());
        }
        eprintln!("{message}logged to {}", target.display());
        previous(info);
    }));
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    install_panic_log();
    let args: Vec<_> = std::env::args_os().collect();
    #[cfg(feature = "ui-review")]
    if args.get(1).is_some_and(|a| a == "--ui-review") {
        return cam_gui_runtime::app::ui_review::run(&args[2..]);
    }
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::install_panic_log;

    /// The point of the hook: a release build has no console, so a startup
    /// failure has to leave a file behind or it leaves nothing.
    #[test]
    fn a_panic_is_written_next_to_the_executable() {
        install_panic_log();
        let before = log_path()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len());
        let _ = std::panic::catch_unwind(|| panic!("startup self-test"));
        let path = log_path().expect("the log path is derivable");
        let after = std::fs::metadata(&path)
            .expect("the log exists after a panic")
            .len();
        assert!(before.unwrap_or(0) < after, "the log grew");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("startup self-test"), "{text}");
        assert!(text.contains("cam-gui"), "{text}");
    }

    fn log_path() -> Option<std::path::PathBuf> {
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.join("cam-gui.log"))
    }
}
