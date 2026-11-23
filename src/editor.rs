//! Opening source locations in an external editor.

use std::env;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use crate::location::Location;

/// Environment variable holding the editor command template.
pub const EDITOR_ENV: &str = "CTGUI_EDITOR";

/// Editors tried in order when `CTGUI_EDITOR` is not set.
const KNOWN_EDITORS: &[&str] = &[
    "code -g {file}:{line}:{column}",
    "codium -g {file}:{line}:{column}",
    "subl {file}:{line}:{column}",
    "gnome-text-editor +{line}:{column} {file}",
    "gedit +{line}:{column} {file}",
    "kate --line {line} --column {column} {file}",
];

/// Fallback that opens the file without jumping to the line.
const FALLBACK: &str = "xdg-open {file}";

/// Opens `location` in the configured or detected editor.
pub fn open(location: &Location) -> Result<(), String> {
    let template = env::var(EDITOR_ENV).ok().filter(|t| !t.trim().is_empty());
    let template = template.as_deref().unwrap_or_else(|| detect(is_on_path));
    let argv = build_argv(template, location);
    let (program, args) = argv.split_first().ok_or("editor command is empty")?;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            format!("failed to run `{program}`: {e} (set {EDITOR_ENV} to choose an editor)")
        })?;
    // Reap the process when it exits so that it does not remain as a zombie.
    thread::spawn(move || child.wait());
    Ok(())
}

/// Returns the first known editor that is available.
fn detect(available: impl Fn(&str) -> bool) -> &'static str {
    KNOWN_EDITORS
        .iter()
        .find(|template| template.split_whitespace().next().is_some_and(&available))
        .copied()
        .unwrap_or(FALLBACK)
}

/// Expands `{file}`, `{line}` and `{column}` in a whitespace separated command template.
///
/// The file is appended when the template has no `{file}`.
fn build_argv(template: &str, location: &Location) -> Vec<String> {
    let file = location.path.to_string_lossy();
    let line = location.line.to_string();
    let column = location.column.unwrap_or(1).to_string();
    let mut argv: Vec<String> = template
        .split_whitespace()
        .map(|arg| {
            arg.replace("{file}", &file)
                .replace("{line}", &line)
                .replace("{column}", &column)
        })
        .collect();
    if !template.contains("{file}") {
        argv.push(file.into_owned());
    }
    argv
}

fn is_on_path(program: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| is_executable(&dir.join(program))))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn location() -> Location {
        Location {
            path: PathBuf::from("/src/my file.cpp"),
            line: 28,
            column: None,
        }
    }

    #[test]
    fn expands_placeholders() {
        assert_eq!(
            build_argv("code -g {file}:{line}:{column}", &location()),
            ["code", "-g", "/src/my file.cpp:28:1"]
        );
        assert_eq!(
            build_argv("vim +{line}", &location()),
            ["vim", "+28", "/src/my file.cpp"]
        );
    }

    #[test]
    fn detects_available_editor() {
        assert_eq!(
            detect(|p| p == "subl" || p == "gedit"),
            "subl {file}:{line}:{column}"
        );
        assert_eq!(detect(|_| false), FALLBACK);
    }
}
