use std::{path::PathBuf, process::ExitCode};

const HELP: &str = "Flat V-carve local browser workspace\nUsage: cam-web [--port <0..65535>] [--ui-dir <web/dist>]\n\nBuild the UI with pnpm build before starting. Bind is always 127.0.0.1.\nPort 0 selects an available port; otherwise the port must be free.\nImport, open/migrate, validation, and cancellable background planning are available.\nM5 combined-plan stock verification is available. Machine output and filesystem writes are not exposed.\nCtrl+C cancels computation workers and stops the service.\n";

fn main() -> ExitCode {
    let result = if std::env::args().skip(1).eq(["--planning-worker"]) {
        cam_server::planning_worker::run()
    } else {
        match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime.block_on(run()),
            Err(error) => Err(error.into()),
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("CAM_WEB_ERROR: {error}");
            ExitCode::from(2)
        }
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut port = None;
    let mut ui = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(());
            }
            "--port" if port.is_none() => {
                port = Some(args.next().ok_or("--port needs a number")?.parse::<u16>()?)
            }
            "--ui-dir" if ui.is_none() => {
                ui = Some(PathBuf::from(
                    args.next().ok_or("--ui-dir needs a directory")?,
                ))
            }
            _ => return Err(format!("unknown/repeated argument {arg}; use --help").into()),
        }
    }
    let ui = ui.unwrap_or_else(|| PathBuf::from("web/dist"));
    let assets = cam_server::load_assets(&ui).map_err(|e| {
        format!(
            "Cannot load {}: {e}. Run pnpm build in web first.",
            ui.display()
        )
    })?;
    let listener =
        tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port.unwrap_or(4848)))
            .await?;
    let port = listener.local_addr()?.port();
    let planning = cam_server::planning::Planning::new()?;
    let app = cam_server::router_with_planning(port, assets, planning.clone())?;
    println!("CAM_WEB_URL=http://127.0.0.1:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            planning.shutdown().await;
        })
        .await?;
    Ok(())
}
