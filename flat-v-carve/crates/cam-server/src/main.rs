use std::process::ExitCode;

fn main() -> ExitCode {
    let result = if std::env::args().skip(1).eq(["--planning-worker"]) {
        cam_server::planning_worker::run()
    } else {
        cam_server::serve::run(std::env::args().skip(1), None)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("CAM_WEB_ERROR: {error}");
            ExitCode::from(2)
        }
    }
}
