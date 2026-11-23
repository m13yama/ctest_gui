//! Detection of source locations (`path:line[:column]`) in test output.

use std::ops::Range;
use std::path::{Path, PathBuf};

/// A position in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
    pub column: Option<u32>,
}

/// A location found in a line of output, with the byte range of the text that refers to it.
pub type Link = (Range<usize>, Location);

/// Characters that cannot be part of a path in output text.
fn is_path_delimiter(c: char) -> bool {
    c.is_whitespace() || "\"'`()[]<>{}=,;|".contains(c)
}

/// Finds locations in `line` that point to existing files.
///
/// Relative paths are resolved against `base_dirs` in order.
pub fn find_links(line: &str, base_dirs: &[&Path]) -> Vec<Link> {
    find_candidates(line)
        .into_iter()
        .filter_map(|(range, path, line_no, column)| {
            let path = resolve(path, base_dirs)?;
            Some((
                range,
                Location {
                    path,
                    line: line_no,
                    column,
                },
            ))
        })
        .collect()
}

/// Finds texts that look like locations, without checking the file system.
fn find_candidates(line: &str) -> Vec<(Range<usize>, &str, u32, Option<u32>)> {
    let mut found = Vec::new();
    find_python_style(line, &mut found);
    find_colon_style(line, &mut found);
    found.sort_by_key(|(range, ..)| range.start);
    found
}

/// `path/to/file.cpp:12` or `path/to/file.cpp:12:5`.
fn find_colon_style<'a>(line: &'a str, found: &mut Vec<(Range<usize>, &'a str, u32, Option<u32>)>) {
    let mut search_from = 0;
    while let Some(offset) = line[search_from..].find(':') {
        let colon = search_from + offset;
        search_from = colon + 1;

        let Some((line_no, mut end)) = parse_number(line, colon + 1) else {
            continue;
        };
        let mut column = None;
        if line[end..].starts_with(':')
            && let Some((col, col_end)) = parse_number(line, end + 1)
        {
            column = Some(col);
            end = col_end;
        }

        let start = line[..colon].rfind(is_path_delimiter).map_or(0, |i| {
            i + line[i..].chars().next().map_or(1, char::len_utf8)
        });
        let path = &line[start..colon];
        if !looks_like_path(path) || overlaps(found, start..end) {
            continue;
        }
        found.push((start..end, path, line_no, column));
        search_from = end;
    }
}

/// `File "path/to/file.py", line 12` (Python tracebacks).
fn find_python_style<'a>(
    line: &'a str,
    found: &mut Vec<(Range<usize>, &'a str, u32, Option<u32>)>,
) {
    const PREFIX: &str = "File \"";
    const MIDDLE: &str = "\", line ";
    let mut search_from = 0;
    while let Some(offset) = line[search_from..].find(PREFIX) {
        let path_start = search_from + offset + PREFIX.len();
        search_from = path_start;
        let Some(path_len) = line[path_start..].find(MIDDLE) else {
            continue;
        };
        let path_end = path_start + path_len;
        let Some((line_no, end)) = parse_number(line, path_end + MIDDLE.len()) else {
            continue;
        };
        let start = path_start - PREFIX.len();
        found.push((start..end, &line[path_start..path_end], line_no, None));
        search_from = end;
    }
}

/// Parses a positive decimal number starting at `start`, returning it and its end offset.
fn parse_number(line: &str, start: usize) -> Option<(u32, usize)> {
    let digits = line[start..].len()
        - line[start..]
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    let number = line[start..start + digits]
        .parse()
        .ok()
        .filter(|&n| n > 0)?;
    Some((number, start + digits))
}

fn looks_like_path(path: &str) -> bool {
    !path.is_empty() && !path.contains("://") && (path.contains('/') || path.contains('.'))
}

fn overlaps(found: &[(Range<usize>, &str, u32, Option<u32>)], range: Range<usize>) -> bool {
    found
        .iter()
        .any(|(r, ..)| r.start < range.end && range.start < r.end)
}

fn resolve(path: &str, base_dirs: &[&Path]) -> Option<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    base_dirs
        .iter()
        .map(|dir| dir.join(path))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(line: &str) -> Vec<(&str, &str, u32, Option<u32>)> {
        find_candidates(line)
            .into_iter()
            .map(|(range, path, l, c)| (&line[range], path, l, c))
            .collect()
    }

    #[test]
    fn finds_colon_style_locations() {
        assert_eq!(
            candidates("/src/demo_test.cpp:28: CHECK failed: 41 == 42"),
            [("/src/demo_test.cpp:28", "/src/demo_test.cpp", 28, None)]
        );
        assert_eq!(
            candidates("../src/a.cpp:3:14: error: expected ';'"),
            [("../src/a.cpp:3:14", "../src/a.cpp", 3, Some(14))]
        );
        assert_eq!(
            candidates("    #0 0x4005 in main (a.out) foo.cc:7:2"),
            [("foo.cc:7:2", "foo.cc", 7, Some(2))]
        );
        assert_eq!(
            candidates("at (test/x_test.cpp:10) and \"y.h:2\""),
            [
                ("test/x_test.cpp:10", "test/x_test.cpp", 10, None),
                ("y.h:2", "y.h", 2, None)
            ]
        );
    }

    #[test]
    fn finds_python_style_locations() {
        assert_eq!(
            candidates("  File \"/ws/src/pkg/test/test_a.py\", line 12, in test_a"),
            [(
                "File \"/ws/src/pkg/test/test_a.py\", line 12",
                "/ws/src/pkg/test/test_a.py",
                12,
                None
            )]
        );
    }

    #[test]
    fn ignores_non_locations() {
        assert!(candidates("progress 1/10").is_empty());
        assert!(candidates("Total Test time (real) =   0.00 sec").is_empty());
        assert!(candidates("started at 12:34:56").is_empty());
        assert!(candidates("see http://example.com:8080/x").is_empty());
        assert!(candidates("file.cpp:0").is_empty());
        assert!(candidates("label: value").is_empty());
    }

    #[test]
    fn resolves_only_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        let file = tmp.path().join("src").join("a.cpp");
        std::fs::write(&file, "").unwrap();

        let line = format!(
            "{}:5: Failure / src/a.cpp:6 / missing.cpp:7",
            file.display()
        );
        let links = find_links(&line, &[Path::new("/nonexistent"), tmp.path()]);
        let found: Vec<_> = links
            .iter()
            .map(|(_, l)| (l.path.clone(), l.line))
            .collect();
        assert_eq!(found, [(file.clone(), 5), (file, 6)]);
    }
}
