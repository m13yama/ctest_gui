//! Finds the parts of ctest output that describe a test result, so that they can be colored.

use std::ops::Range;

use crate::runner::TestStatus;

/// A byte range of a line and the result it represents.
pub type Span = (Range<usize>, TestStatus);

/// Returns the result spans of a single output line (without the trailing newline).
pub fn spans(line: &str) -> Vec<Span> {
    if let Some(span) = result_line_span(line) {
        return vec![span];
    }
    if let Some(status) = whole_line_status(line) {
        return vec![(0..line.len(), status)];
    }
    if let Some(span) = summary_list_span(line) {
        return vec![span];
    }
    bracket_tag_span(line).into_iter().collect()
}

/// `1/1 Test #9: name ......***Failed    0.00 sec` → the `***Failed` part.
fn result_line_span(line: &str) -> Option<Span> {
    let after_number = line.find("Test #")?;
    let body = line[after_number..].trim_end().strip_suffix(" sec")?;
    let colon = after_number + line[after_number..].find(": ")?;
    let dots = colon + line[colon..].find("..")?;
    let start = dots + line[dots..].find(|c| c != '.')?;
    // Strip the elapsed time ("0.00") at the end.
    let time_start = after_number + body.trim_end().rfind(' ')?;
    let end = start + line[start..time_start].trim_end().len();
    let start = start + (line[start..end].len() - line[start..end].trim_start().len());
    if start >= end {
        return None;
    }
    let text = &line[start..end];
    let status = if text.contains("***Timeout") {
        TestStatus::Timeout
    } else if text.contains("***Exception") {
        TestStatus::Crashed
    } else if text.contains("***Skipped") {
        TestStatus::Skipped
    } else if text.contains("***Not Run (Disabled)") {
        TestStatus::Disabled
    } else if text.starts_with("Passed") {
        TestStatus::Passed
    } else {
        TestStatus::Failed
    };
    Some((start..end, status))
}

/// Summary lines printed by ctest after running tests.
fn whole_line_status(line: &str) -> Option<TestStatus> {
    let trimmed = line.trim();
    if trimmed.starts_with("100% tests passed") {
        Some(TestStatus::Passed)
    } else if (trimmed.contains("% tests passed") && trimmed.contains("tests failed out of"))
        || trimmed.starts_with("The following tests FAILED:")
        || trimmed.starts_with("Errors while running CTest")
        || trimmed.starts_with("No tests were found")
    {
        Some(TestStatus::Failed)
    } else if trimmed.starts_with("The following tests did not run:") {
        Some(TestStatus::Skipped)
    } else {
        None
    }
}

/// `\t  9 - name (Failed)   labels` → the `(Failed)` part.
fn summary_list_span(line: &str) -> Option<Span> {
    if !line.starts_with('\t') || !line.contains(" - ") {
        return None;
    }
    let start = line.rfind(" (")? + 1;
    let end = start + line[start..].find(')')? + 1;
    let status = match &line[start + 1..end - 1] {
        "Failed" => TestStatus::Failed,
        "Timeout" => TestStatus::Timeout,
        "Skipped" => TestStatus::Skipped,
        "Disabled" | "Not Run" => TestStatus::Disabled,
        _ => TestStatus::Crashed,
    };
    Some((start..end, status))
}

/// Tags of common test frameworks, e.g. `[  OK  ]`, `[  FAILED  ]` (GoogleTest).
fn bracket_tag_span(line: &str) -> Option<Span> {
    if !line.starts_with('[') {
        return None;
    }
    let end = line.find(']')? + 1;
    let status = match line[1..end - 1].trim() {
        "OK" | "PASSED" => TestStatus::Passed,
        "FAILED" => TestStatus::Failed,
        "SKIPPED" => TestStatus::Skipped,
        _ => return None,
    };
    Some((0..end, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colored(line: &str) -> Vec<(&str, TestStatus)> {
        spans(line)
            .into_iter()
            .map(|(range, status)| (&line[range], status))
            .collect()
    }

    #[test]
    fn colors_result_lines() {
        assert_eq!(
            colored("1/1 Test #1: math_test.add ......   Passed    0.00 sec"),
            [("Passed", TestStatus::Passed)]
        );
        assert_eq!(
            colored("1/1 Test #9: demo_test.a ......***Failed    0.00 sec"),
            [("***Failed", TestStatus::Failed)]
        );
        assert_eq!(
            colored("1/1 Test #10: d.segfault ....***Exception: SegFault  1.08 sec"),
            [("***Exception: SegFault", TestStatus::Crashed)]
        );
        assert_eq!(
            colored("1/1 Test #11: d.u .....Subprocess aborted***Exception:   1.07 sec"),
            [("Subprocess aborted***Exception:", TestStatus::Crashed)]
        );
        assert_eq!(
            colored("1/1 Test #12: d.t ......***Timeout   3.01 sec"),
            [("***Timeout", TestStatus::Timeout)]
        );
        assert_eq!(
            colored("1/1 Test #13: d.s ......***Skipped   0.00 sec"),
            [("***Skipped", TestStatus::Skipped)]
        );
        assert_eq!(
            colored("1/1 Test #19: d.d ....***Not Run (Disabled)   0.00 sec"),
            [("***Not Run (Disabled)", TestStatus::Disabled)]
        );
        assert_eq!(
            colored(
                "1/1 Test #15: d.e ....***Failed  Error regular expression found in output. Regex=[ERROR]  0.00 sec"
            ),
            [(
                "***Failed  Error regular expression found in output. Regex=[ERROR]",
                TestStatus::Failed
            )]
        );
    }

    #[test]
    fn colors_summary_lines() {
        let line = "100% tests passed, 0 tests failed out of 1";
        assert_eq!(colored(line), [(line, TestStatus::Passed)]);
        let line = "0% tests passed, 1 tests failed out of 1";
        assert_eq!(colored(line), [(line, TestStatus::Failed)]);
        assert_eq!(
            colored("Errors while running CTest"),
            [("Errors while running CTest", TestStatus::Failed)]
        );
        assert_eq!(
            colored("\t 10 - demo_test.segfault (SEGFAULT)       demo"),
            [("(SEGFAULT)", TestStatus::Crashed)]
        );
        assert_eq!(
            colored("\t 12 - demo_test.timeout (Timeout)  demo"),
            [("(Timeout)", TestStatus::Timeout)]
        );
    }

    #[test]
    fn colors_framework_tags() {
        assert_eq!(colored("[  OK  ] add"), [("[  OK  ]", TestStatus::Passed)]);
        assert_eq!(
            colored("[  FAILED  ] Suite.Case (0 ms)"),
            [("[  FAILED  ]", TestStatus::Failed)]
        );
        assert!(colored("[ RUN  ] add").is_empty());
    }

    #[test]
    fn leaves_plain_lines_uncolored() {
        assert!(colored("progress 1/10").is_empty());
        assert!(colored("Total Test time (real) =   0.00 sec").is_empty());
        assert!(colored("").is_empty());
    }
}
