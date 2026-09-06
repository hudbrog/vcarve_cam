//! Shared startup for `cam serve` and the development `cam-web` executable.
use crate::{Assets, library, load_assets, planning::Planning, router_with_library};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

pub const HELP: &str = "Local browser workspace\nUsage: cam serve [--port <0..65535>] [--open] [--ui-dir <directory>] [--library-dir <directory>]\n\nBind is always 127.0.0.1; default port is 4848. Port 0 selects an available port.\n--open launches the default browser after the service is ready.\nPortable builds serve embedded UI assets. --ui-dir overrides them for development.\nThe library uses local application data unless --library-dir is supplied.\nLibrary creation is explicit in the UI. Ctrl+C cancels workers and stops the service.\n";

#[derive(Default, Debug)]
struct Options {
    port: Option<u16>,
    ui: Option<PathBuf>,
    library: Option<PathBuf>,
    open: bool,
}
impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut options = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => return Ok(None),
                "--port" if options.port.is_none() => {
                    options.port = Some(
                        args.next()
                            .ok_or("--port needs a number")?
                            .parse()
                            .map_err(|_| "--port must be an integer between 0 and 65535")?,
                    );
                }
                "--ui-dir" if options.ui.is_none() => {
                    options.ui = Some(args.next().ok_or("--ui-dir needs a directory")?.into());
                }
                "--library-dir" if options.library.is_none() => {
                    options.library =
                        Some(args.next().ok_or("--library-dir needs a directory")?.into());
                }
                "--open" if !options.open => options.open = true,
                _ => return Err(format!("unknown/repeated argument {arg}; use --help")),
            }
        }
        Ok(Some(options))
    }
}

pub fn run(
    args: impl Iterator<Item = String>,
    embedded: Option<Assets>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(options) = Options::parse(args)? else {
        print!("{HELP}");
        return Ok(());
    };
    let assets = match (options.ui, embedded) {
        (Some(path), _) => load_assets(&path).map_err(|e| format!("Cannot load {}: {e}", path.display()))?,
        (None, Some(assets)) => assets,
        (None, None) => load_assets(&PathBuf::from("web/dist")).map_err(|e| {
            format!("This build has no embedded UI and web/dist could not be loaded: {e}. Use a portable build, or run pnpm build in web and supply --ui-dir <web/dist>.")
        })?,
    };
    let directory = std::path::absolute(match options.library {
        Some(path) => path,
        None => library::default_directory()?,
    })?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            options.port.unwrap_or(4848),
        ))
        .await?;
        let port = listener.local_addr()?.port();
        let planning = Planning::new()?;
        let app = router_with_library(port, assets, planning.clone(), Some(directory))?;
        let url = format!("http://127.0.0.1:{port}");
        println!("CAM_WEB_URL={url}");
        io::stdout().flush()?;
        if options.open
            && let Err(error) = open_browser(&url)
        {
            eprintln!("Could not open the browser: {error}. Open {url} manually.");
        }
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = tokio::signal::ctrl_c().await;
                planning.shutdown().await;
            })
            .await?;
        Ok(())
    })
}

fn open_browser(url: &str) -> io::Result<()> {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let system = std::env::var_os("SystemRoot")
            .ok_or_else(|| io::Error::other("SystemRoot is unavailable"))?;
        let mut command = Command::new(PathBuf::from(system).join("System32/rundll32.exe"));
        command
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x08000000);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Options;
    #[test]
    fn server_arguments_are_explicit_and_strict() {
        let options = Options::parse(
            ["--port", "0", "--open", "--library-dir", "tools"]
                .map(str::to_owned)
                .into_iter(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(options.port, Some(0));
        assert!(options.open);
        assert_eq!(options.library.unwrap().to_str(), Some("tools"));
        for args in [
            vec!["--port"],
            vec!["--port", "65536"],
            vec!["--port", "1", "--port", "2"],
            vec!["--open", "--open"],
            vec!["--host", "0.0.0.0"],
        ] {
            assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(
            Options::parse(["--help".into()].into_iter())
                .unwrap()
                .is_none()
        );
    }
}
