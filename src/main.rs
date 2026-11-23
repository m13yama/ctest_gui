mod app;
mod discovery;
mod editor;
mod gtest;
mod highlight;
mod location;
mod runner;
mod workspace;

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use eframe::egui;

use crate::workspace::Workspace;

const USAGE: &str = "Usage: ctgui [PATH]

Open a GUI to discover and run ctest tests.
PATH is a CMake project root, a CMake build directory, or a colcon workspace (default: .).

Options:
  -f, --foreground  Keep running attached to the terminal instead of in the background
  -h, --help        Print this help
  -V, --version     Print the version";

fn main() -> ExitCode {
    let mut path: Option<PathBuf> = None;
    let mut foreground = false;
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            Some("-V" | "--version") => {
                println!("ctgui {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            Some("-f" | "--foreground") => foreground = true,
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let root = std::fs::canonicalize(&path).unwrap_or(path);
    let workspace = match Workspace::detect(&root) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("ctgui: {e}");
            return ExitCode::FAILURE;
        }
    };

    if !foreground {
        match detach(&root) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => eprintln!(
                "ctgui: failed to start in the background, staying in the foreground: {e}"
            ),
        }
    }

    let title = format!("ctgui - {}", root.display());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_app_id("ctgui")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("embedded icon is a valid png"),
            )
            .with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    match eframe::run_native(
        "ctgui",
        options,
        Box::new(|_cc| Ok(Box::new(app::CtguiApp::new(workspace)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ctgui: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Starts the GUI as a new session detached from the terminal, like `code .` does.
///
/// The workspace is detected before this so that errors are still reported in the terminal.
fn detach(root: &Path) -> std::io::Result<()> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("--foreground")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn().map(drop)
}
