//! Test discovery using `ctest --show-only=json-v1`.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// A single test registered to ctest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    pub name: String,
    pub command: Vec<String>,
    pub labels: Vec<String>,
    pub working_directory: Option<String>,
    pub disabled: bool,
}

#[derive(Deserialize)]
struct CtestInfo {
    #[serde(default)]
    tests: Vec<RawTest>,
}

#[derive(Deserialize)]
struct RawTest {
    name: String,
    #[serde(default)]
    command: Vec<String>,
    #[serde(default)]
    properties: Vec<RawProperty>,
}

#[derive(Deserialize)]
struct RawProperty {
    name: String,
    value: serde_json::Value,
}

/// Lists the tests registered in `build_dir`.
pub fn discover(build_dir: &Path) -> Result<Vec<TestCase>, String> {
    let output = Command::new("ctest")
        .arg("--show-only=json-v1")
        .current_dir(build_dir)
        .output()
        .map_err(|e| format!("failed to run ctest: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ctest failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_tests(&String::from_utf8_lossy(&output.stdout))
}

/// Parses the output of `ctest --show-only=json-v1`.
pub fn parse_tests(json: &str) -> Result<Vec<TestCase>, String> {
    let info: CtestInfo =
        serde_json::from_str(json).map_err(|e| format!("invalid ctest json: {e}"))?;
    Ok(info.tests.into_iter().map(TestCase::from).collect())
}

impl From<RawTest> for TestCase {
    fn from(raw: RawTest) -> Self {
        let mut labels = Vec::new();
        let mut working_directory = None;
        let mut disabled = false;
        for prop in raw.properties {
            match (prop.name.as_str(), prop.value) {
                ("LABELS", serde_json::Value::Array(values)) => {
                    labels = values
                        .into_iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                }
                ("WORKING_DIRECTORY", serde_json::Value::String(dir)) => {
                    working_directory = Some(dir);
                }
                ("DISABLED", serde_json::Value::Bool(value)) => disabled = value,
                _ => {}
            }
        }
        TestCase {
            name: raw.name,
            command: raw.command,
            labels,
            working_directory,
            disabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "kind": "ctestInfo",
      "version": {"major": 1, "minor": 0},
      "tests": [
        {
          "name": "pass_test",
          "command": ["/usr/bin/true"],
          "properties": [
            {"name": "LABELS", "value": ["fast", "unit"]},
            {"name": "WORKING_DIRECTORY", "value": "/tmp/build"}
          ]
        },
        {"name": "no_props", "command": ["/bin/sh", "-c", "exit 1"]},
        {"name": "off", "properties": [{"name": "DISABLED", "value": true}]}
      ]
    }"#;

    #[test]
    fn parses_tests_and_properties() {
        let tests = parse_tests(SAMPLE).unwrap();
        assert_eq!(tests.len(), 3);
        assert!(!tests[0].disabled);
        assert!(tests[2].disabled);
        assert_eq!(tests[0].name, "pass_test");
        assert_eq!(tests[0].labels, ["fast", "unit"]);
        assert_eq!(tests[0].working_directory.as_deref(), Some("/tmp/build"));
        assert_eq!(tests[1].command, ["/bin/sh", "-c", "exit 1"]);
        assert!(tests[1].labels.is_empty());
    }

    #[test]
    fn parses_empty_test_list() {
        assert!(parse_tests(r#"{"kind":"ctestInfo"}"#).unwrap().is_empty());
        assert!(parse_tests("not json").is_err());
    }

    #[test]
    fn discovers_tests_with_real_ctest() {
        if Command::new("ctest").arg("--version").output().is_err() {
            eprintln!("ctest not found; skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("CTestTestfile.cmake"),
            "add_test(a_test \"/bin/true\")\nset_tests_properties(a_test PROPERTIES LABELS \"x;y\")\n",
        )
        .unwrap();

        let tests = discover(tmp.path()).unwrap();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].name, "a_test");
        assert_eq!(tests[0].labels, ["x", "y"]);
    }
}
