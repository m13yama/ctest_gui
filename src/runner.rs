//! Background execution of ctest tests.

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::gtest;

/// Identifies a test by its suite index and test index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TestId {
    pub suite: usize,
    pub test: usize,
}

/// Execution state of a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    NotRun,
    Queued,
    Running,
    Passed,
    Failed,
    /// Killed by ctest because it exceeded its `TIMEOUT`.
    Timeout,
    /// Terminated by a signal or an uncaught exception.
    Crashed,
    /// Exited with its `SKIP_RETURN_CODE`.
    Skipped,
    /// Has the `DISABLED` property and is never run.
    Disabled,
    Cancelled,
}

impl TestStatus {
    /// Whether the test finished without passing.
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            TestStatus::Failed | TestStatus::Timeout | TestStatus::Crashed
        )
    }
}

/// A request to run a single test.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub id: TestId,
    pub build_dir: PathBuf,
    pub test_name: String,
}

/// Progress notifications sent from the runner thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    Started(TestId),
    Output(TestId, String),
    /// Results of the GoogleTest cases inside the test, sent before `Finished`.
    Cases(TestId, Vec<gtest::CaseResult>),
    Finished(TestId, TestStatus, Duration),
    Done,
}

/// Callback used to wake up the UI after an event is sent.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Runs the requested tests one by one on a background thread.
pub fn spawn_run(
    requests: Vec<RunRequest>,
    setup_script: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    tx: Sender<RunEvent>,
    notify: Notify,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for req in requests {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            run_one(&req, setup_script.as_deref(), &cancel, &tx, &notify);
        }
        let _ = tx.send(RunEvent::Done);
        notify();
    })
}

fn run_one(
    req: &RunRequest,
    setup_script: Option<&Path>,
    cancel: &AtomicBool,
    tx: &Sender<RunEvent>,
    notify: &Notify,
) {
    let _ = tx.send(RunEvent::Started(req.id));
    notify();
    let start = Instant::now();

    let report = gtest::new_report_path();
    let mut cmd = build_command(&req.build_dir, &req.test_name, setup_script);
    cmd.env(gtest::OUTPUT_ENV, format!("json:{}", report.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let status = match cmd.spawn() {
        Err(e) => {
            let _ = tx.send(RunEvent::Output(
                req.id,
                format!("failed to run ctest: {e}\n"),
            ));
            TestStatus::Failed
        }
        Ok(mut child) => {
            let stdout = child
                .stdout
                .take()
                .map(|s| forward_lines(s, req.id, tx, notify));
            let stderr = child
                .stderr
                .take()
                .map(|s| forward_lines(s, req.id, tx, notify));
            let exit = wait_or_cancel(&mut child, cancel);
            let stdout = stdout.and_then(|h| h.join().ok()).unwrap_or_default();
            if let Some(h) = stderr {
                let _ = h.join();
            }
            match exit {
                Some(success) => classify(&stdout, success),
                None => TestStatus::Cancelled,
            }
        }
    };

    if let Some(cases) = gtest::take_report(&report) {
        let _ = tx.send(RunEvent::Cases(req.id, cases));
    }
    let _ = std::fs::remove_file(&report);
    let _ = tx.send(RunEvent::Finished(req.id, status, start.elapsed()));
    notify();
}

/// Waits for ctest to exit and returns whether it succeeded, or `None` when cancelled.
fn wait_or_cancel(child: &mut Child, cancel: &AtomicBool) -> Option<bool> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) => {}
            Err(_) => return Some(false),
        }
        if cancel.load(Ordering::SeqCst) {
            // The child is the leader of its own process group, so this also stops the test process.
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(30));
    }
}

fn forward_lines(
    stream: impl Read + Send + 'static,
    id: TestId,
    tx: &Sender<RunEvent>,
    notify: &Notify,
) -> thread::JoinHandle<String> {
    let tx = tx.clone();
    let notify = notify.clone();
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        let mut all = String::new();
        while let Ok(n) = reader.read_until(b'\n', &mut buf) {
            if n == 0 {
                break;
            }
            let line = String::from_utf8_lossy(&buf).into_owned();
            all.push_str(&line);
            let _ = tx.send(RunEvent::Output(id, line));
            notify();
            buf.clear();
        }
        all
    })
}

/// Determines the test result from ctest's result line, e.g.
/// `1/1 Test #3: name .....***Timeout   3.01 sec`.
pub fn classify(ctest_stdout: &str, success: bool) -> TestStatus {
    let result_line = ctest_stdout
        .lines()
        .find(|line| line.contains("Test #") && line.trim_end().ends_with(" sec"));
    if let Some(line) = result_line {
        if line.contains("***Timeout") {
            return TestStatus::Timeout;
        }
        if line.contains("***Exception") {
            return TestStatus::Crashed;
        }
        if line.contains("***Skipped") {
            return TestStatus::Skipped;
        }
        if line.contains("***Not Run (Disabled)") {
            return TestStatus::Disabled;
        }
    }
    if success {
        TestStatus::Passed
    } else {
        TestStatus::Failed
    }
}

/// Builds the ctest command that runs exactly one test.
pub fn build_command(build_dir: &Path, test_name: &str, setup_script: Option<&Path>) -> Command {
    let args = ctest_args(test_name);
    let mut cmd = match setup_script {
        Some(setup) => {
            let mut script = format!(
                "source {} >/dev/null 2>&1 || exit 1; exec ctest",
                shell_quote(&setup.to_string_lossy())
            );
            for arg in &args {
                script.push(' ');
                script.push_str(&shell_quote(arg));
            }
            let mut cmd = Command::new("bash");
            cmd.arg("-c").arg(script);
            cmd
        }
        None => {
            let mut cmd = Command::new("ctest");
            cmd.args(&args);
            cmd
        }
    };
    cmd.current_dir(build_dir);
    cmd
}

fn ctest_args(test_name: &str) -> Vec<String> {
    vec![
        "-R".into(),
        format!("^{}$", regex_escape(test_name)),
        "--output-on-failure".into(),
        "--no-tests=error".into(),
    ]
}

/// Escapes characters that have a special meaning in CMake regular expressions.
pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Quotes a string for safe use in a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn escapes_regex_special_characters() {
        assert_eq!(regex_escape("plain_name"), "plain_name");
        assert_eq!(regex_escape("a.b(c)+"), r"a\.b\(c\)\+");
        assert_eq!(regex_escape("Suite[1]/x"), r"Suite\[1\]/x");
    }

    #[test]
    fn quotes_shell_strings() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn builds_plain_ctest_command() {
        let cmd = build_command(Path::new("/tmp/b"), "x.y", None);
        assert_eq!(cmd.get_program(), "ctest");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
        assert_eq!(args[..2], ["-R", r"^x\.y$"]);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/tmp/b")));
    }

    #[test]
    fn builds_sourced_ctest_command() {
        let cmd = build_command(
            Path::new("/ws/build/pkg"),
            "t",
            Some(Path::new("/ws/install/setup.bash")),
        );
        assert_eq!(cmd.get_program(), "bash");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-c");
        assert!(args[1].starts_with("source '/ws/install/setup.bash'"));
        assert!(args[1].contains("exec ctest '-R' '^t$'"));
    }

    fn collect_run(dir: &Path, names: &[&str], setup: Option<PathBuf>) -> Vec<RunEvent> {
        let (tx, rx) = mpsc::channel();
        let requests = names
            .iter()
            .enumerate()
            .map(|(i, name)| RunRequest {
                id: TestId { suite: 0, test: i },
                build_dir: dir.to_path_buf(),
                test_name: (*name).to_owned(),
            })
            .collect();
        spawn_run(
            requests,
            setup,
            Arc::new(AtomicBool::new(false)),
            tx,
            Arc::new(|| {}),
        )
        .join()
        .unwrap();
        rx.into_iter().collect()
    }

    #[test]
    fn runs_tests_with_real_ctest() {
        if Command::new("ctest").arg("--version").output().is_err() {
            eprintln!("ctest not found; skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("CTestTestfile.cmake"),
            "add_test(ok \"/bin/true\")\nadd_test(ng \"/bin/sh\" \"-c\" \"echo boom; exit 1\")\n",
        )
        .unwrap();

        let events = collect_run(tmp.path(), &["ok", "ng"], None);
        let finished: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                RunEvent::Finished(id, status, _) => Some((id.test, *status)),
                _ => None,
            })
            .collect();
        assert_eq!(finished, [(0, TestStatus::Passed), (1, TestStatus::Failed)]);
        assert!(events.iter().any(
            |e| matches!(e, RunEvent::Output(id, line) if id.test == 1 && line.contains("boom"))
        ));
        assert_eq!(events.last(), Some(&RunEvent::Done));
    }

    fn finished_statuses(events: &[RunEvent]) -> Vec<(usize, TestStatus)> {
        events
            .iter()
            .filter_map(|e| match e {
                RunEvent::Finished(id, status, _) => Some((id.test, *status)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sources_setup_script_before_running() {
        if Command::new("ctest").arg("--version").output().is_err() {
            eprintln!("ctest not found; skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let setup = tmp.path().join("setup.bash");
        std::fs::write(&setup, "export CTGUI_TEST_ENV=sourced\n").unwrap();
        std::fs::write(
            tmp.path().join("CTestTestfile.cmake"),
            "add_test(env \"/bin/sh\" \"-c\" \"test \\\"$CTGUI_TEST_ENV\\\" = sourced\")\n",
        )
        .unwrap();

        assert_eq!(
            finished_statuses(&collect_run(tmp.path(), &["env"], None)),
            [(0, TestStatus::Failed)]
        );
        assert_eq!(
            finished_statuses(&collect_run(tmp.path(), &["env"], Some(setup))),
            [(0, TestStatus::Passed)]
        );
    }

    #[test]
    fn classifies_ctest_result_lines() {
        let line = |result: &str| format!("1/1 Test #1: t ....{result}   0.01 sec\n");
        assert_eq!(classify(&line("   Passed"), true), TestStatus::Passed);
        assert_eq!(classify(&line("***Failed"), false), TestStatus::Failed);
        assert_eq!(classify(&line("***Timeout"), false), TestStatus::Timeout);
        assert_eq!(
            classify(&line("***Exception: SegFault"), false),
            TestStatus::Crashed
        );
        assert_eq!(
            classify(&line("Subprocess aborted***Exception:"), false),
            TestStatus::Crashed
        );
        assert_eq!(classify(&line("***Skipped"), true), TestStatus::Skipped);
        assert_eq!(
            classify(&line("***Not Run (Disabled)"), false),
            TestStatus::Disabled
        );
        // Markers printed by the test itself are not ctest results.
        assert_eq!(classify("***Timeout\n", false), TestStatus::Failed);
        assert_eq!(classify("", true), TestStatus::Passed);
    }
}
