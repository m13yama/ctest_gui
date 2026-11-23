//! Reading GoogleTest JSON reports (`GTEST_OUTPUT=json:<path>`).
//!
//! GoogleTest reads `GTEST_OUTPUT` from the environment, and ctest passes its environment on to
//! the tests, so setting it when running ctest gives the result and source location of every
//! `TEST()` without changing how the tests are registered.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::Deserialize;

use crate::location::Location;
use crate::runner::TestStatus;

/// Environment variable read by GoogleTest to write a report.
pub const OUTPUT_ENV: &str = "GTEST_OUTPUT";

/// Result of a single GoogleTest test case (`TEST()`) inside a ctest test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    /// `Suite.Name`.
    pub name: String,
    pub status: TestStatus,
    pub location: Option<Location>,
    pub duration: Option<Duration>,
}

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    testsuites: Vec<Suite>,
}

#[derive(Deserialize)]
struct Suite {
    name: String,
    #[serde(default)]
    testsuite: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    file: Option<String>,
    line: Option<u32>,
    status: Option<String>,
    result: Option<String>,
    time: Option<String>,
    #[serde(default)]
    failures: Vec<serde_json::Value>,
}

/// Returns a unique path for a report file in the temporary directory.
pub fn new_report_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "ctgui-{}-{}.json",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Reads and removes the report at `path`, returning `None` when the test did not write one.
pub fn take_report(path: &Path) -> Option<Vec<CaseResult>> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    parse_report(&text).ok()
}

/// Parses a GoogleTest JSON report.
pub fn parse_report(json: &str) -> Result<Vec<CaseResult>, String> {
    let report: Report =
        serde_json::from_str(json).map_err(|e| format!("invalid gtest report: {e}"))?;
    Ok(report
        .testsuites
        .into_iter()
        .flat_map(|suite| {
            let suite_name = suite.name;
            suite.testsuite.into_iter().map(move |case| CaseResult {
                name: format!("{suite_name}.{}", case.name),
                status: case_status(&case),
                location: case.file.zip(case.line).map(|(file, line)| Location {
                    path: PathBuf::from(file),
                    line,
                    column: None,
                }),
                duration: case.time.as_deref().and_then(parse_time),
            })
        })
        .collect())
}

fn case_status(case: &Case) -> TestStatus {
    if case.status.as_deref() == Some("NOTRUN") {
        TestStatus::Disabled
    } else if !case.failures.is_empty() {
        TestStatus::Failed
    } else if case.result.as_deref() == Some("SKIPPED") {
        TestStatus::Skipped
    } else {
        TestStatus::Passed
    }
}

/// Parses durations such as `"0.012s"`.
fn parse_time(time: &str) -> Option<Duration> {
    let seconds: f64 = time.strip_suffix('s')?.parse().ok()?;
    Duration::try_from_secs_f64(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"{
      "tests": 4, "name": "AllTests",
      "testsuites": [
        {"name": "Math", "testsuite": [
          {"name": "Add", "file": "/src/t.cpp", "line": 2, "status": "RUN", "result": "COMPLETED", "time": "0.012s", "classname": "Math"},
          {"name": "Broken", "file": "/src/t.cpp", "line": 3, "status": "RUN", "result": "COMPLETED", "time": "0s",
           "failures": [{"failure": "/src/t.cpp:3\nExpected equality", "type": ""}]}
        ]},
        {"name": "Text", "testsuite": [
          {"name": "Skip", "file": "/src/t.cpp", "line": 4, "status": "RUN", "result": "SKIPPED", "time": "0s",
           "skipped": [{"message": "not today"}]},
          {"name": "DISABLED_Off", "file": "/src/t.cpp", "line": 5, "status": "NOTRUN", "result": "SUPPRESSED", "time": "0s"}
        ]}
      ]
    }"#;

    #[test]
    fn parses_case_results() {
        let cases = parse_report(REPORT).unwrap();
        let summary: Vec<_> = cases
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.status,
                    c.location.as_ref().map(|l| l.line),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("Math.Add", TestStatus::Passed, Some(2)),
                ("Math.Broken", TestStatus::Failed, Some(3)),
                ("Text.Skip", TestStatus::Skipped, Some(4)),
                ("Text.DISABLED_Off", TestStatus::Disabled, Some(5)),
            ]
        );
        assert_eq!(cases[0].duration, Some(Duration::from_millis(12)));
        assert_eq!(
            cases[0].location.as_ref().unwrap().path,
            PathBuf::from("/src/t.cpp")
        );
    }

    #[test]
    fn rejects_invalid_reports() {
        assert!(parse_report("not json").is_err());
        assert!(parse_report("{}").unwrap().is_empty());
    }

    #[test]
    fn creates_unique_report_paths() {
        assert_ne!(new_report_path(), new_report_path());
    }
}
