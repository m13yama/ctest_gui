//! egui application.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use crate::discovery::{self, TestCase};
use crate::editor;
use crate::gtest::CaseResult;
use crate::highlight;
use crate::location::{self, Location};
use crate::runner::{self, RunEvent, RunRequest, TestId, TestStatus};
use crate::workspace::{Suite, Workspace};

struct TestEntry {
    case: TestCase,
    status: TestStatus,
    output: Vec<OutputLine>,
    duration: Option<Duration>,
    /// GoogleTest cases reported by the last run.
    cases: Vec<CaseResult>,
}

impl TestEntry {
    /// Whether the cases add information beyond the test itself, so they are listed in the tree.
    ///
    /// A test registered by `gtest_discover_tests()` has a single case with the same name.
    fn has_child_cases(&self) -> bool {
        match self.cases.as_slice() {
            [] => false,
            [only] => only.name != self.case.name,
            _ => true,
        }
    }
}

/// A line of test output split into differently styled parts.
struct OutputLine {
    text: String,
    segments: Vec<Segment>,
}

struct Segment {
    range: Range<usize>,
    color: Color32,
    link: Option<Location>,
}

struct SuiteState {
    suite: Suite,
    tests: Vec<TestEntry>,
    error: Option<String>,
}

pub struct CtguiApp {
    workspace: Workspace,
    suites: Vec<SuiteState>,
    filter: String,
    selected: Option<TestId>,
    events: Option<Receiver<RunEvent>>,
    cancel: Arc<AtomicBool>,
    /// Error shown in the details panel, e.g. when the editor failed to start.
    notice: Option<String>,
}

impl CtguiApp {
    pub fn new(workspace: Workspace) -> Self {
        let mut app = CtguiApp {
            workspace,
            suites: Vec::new(),
            filter: String::new(),
            selected: None,
            events: None,
            cancel: Arc::new(AtomicBool::new(false)),
            notice: None,
        };
        app.refresh();
        app
    }

    fn is_running(&self) -> bool {
        self.events.is_some()
    }

    /// Re-detects the workspace and discovers the tests of every suite in parallel.
    fn refresh(&mut self) {
        if let Ok(ws) = Workspace::detect(&self.workspace.root) {
            self.workspace = ws;
        }
        let results: Vec<Result<Vec<TestCase>, String>> = thread::scope(|s| {
            let handles: Vec<_> = self
                .workspace
                .suites
                .iter()
                .map(|suite| s.spawn(|| discovery::discover(&suite.build_dir)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err("discovery panicked".into()))
                })
                .collect()
        });
        self.suites = self
            .workspace
            .suites
            .iter()
            .cloned()
            .zip(results)
            .map(|(suite, result)| match result {
                Ok(cases) => SuiteState {
                    suite,
                    tests: cases
                        .into_iter()
                        .map(|case| TestEntry {
                            status: initial_status(&case),
                            case,
                            output: Vec::new(),
                            duration: None,
                            cases: Vec::new(),
                        })
                        .collect(),
                    error: None,
                },
                Err(e) => SuiteState {
                    suite,
                    tests: Vec::new(),
                    error: Some(e),
                },
            })
            .collect();
        self.selected = None;
    }

    fn entry(&self, id: TestId) -> Option<&TestEntry> {
        self.suites.get(id.suite)?.tests.get(id.test)
    }

    fn entry_mut(&mut self, id: TestId) -> Option<&mut TestEntry> {
        self.suites.get_mut(id.suite)?.tests.get_mut(id.test)
    }

    fn matches_filter(&self, case: &TestCase) -> bool {
        let filter = self.filter.trim().to_lowercase();
        filter.is_empty()
            || case.name.to_lowercase().contains(&filter)
            || case
                .labels
                .iter()
                .any(|l| l.to_lowercase().contains(&filter))
    }

    /// Ids of all tests that pass the current filter.
    fn visible_ids(&self) -> Vec<TestId> {
        self.suites
            .iter()
            .enumerate()
            .flat_map(|(si, s)| {
                s.tests
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| self.matches_filter(&t.case))
                    .map(move |(ti, _)| TestId {
                        suite: si,
                        test: ti,
                    })
            })
            .collect()
    }

    fn start_run(&mut self, ids: Vec<TestId>, ctx: &egui::Context) {
        if self.is_running() || ids.is_empty() {
            return;
        }
        let requests: Vec<RunRequest> = ids
            .iter()
            .filter_map(|&id| {
                Some(RunRequest {
                    id,
                    build_dir: self.suites.get(id.suite)?.suite.build_dir.clone(),
                    test_name: self.entry(id)?.case.name.clone(),
                })
            })
            .collect();
        for &id in &ids {
            if let Some(entry) = self.entry_mut(id) {
                entry.status = TestStatus::Queued;
            }
        }
        let (tx, rx) = mpsc::channel();
        self.cancel = Arc::new(AtomicBool::new(false));
        let ctx = ctx.clone();
        runner::spawn_run(
            requests,
            self.workspace.setup_script.clone(),
            self.cancel.clone(),
            tx,
            Arc::new(move || ctx.request_repaint()),
        );
        self.events = Some(rx);
    }

    fn poll_events(&mut self) {
        let Some(rx) = self.events.take() else {
            return;
        };
        let mut done = false;
        for event in rx.try_iter() {
            match event {
                RunEvent::Started(id) => {
                    if let Some(e) = self.entry_mut(id) {
                        e.status = TestStatus::Running;
                        e.output.clear();
                        e.duration = None;
                        e.cases.clear();
                    }
                }
                RunEvent::Cases(id, cases) => {
                    if let Some(e) = self.entry_mut(id) {
                        e.cases = cases;
                    }
                }
                RunEvent::Output(id, text) => {
                    let Some(suite) = self.suites.get(id.suite) else {
                        continue;
                    };
                    let Some(entry) = suite.tests.get(id.test) else {
                        continue;
                    };
                    // Relative paths in the output are resolved against these directories.
                    let working_dir = entry.case.working_directory.as_deref().map(Path::new);
                    let base_dirs: Vec<&Path> = working_dir
                        .into_iter()
                        .chain([suite.suite.build_dir.as_path(), &self.workspace.root])
                        .collect();
                    let line = OutputLine::new(&text, &base_dirs);
                    if let Some(e) = self.entry_mut(id) {
                        e.output.push(line);
                    }
                }
                RunEvent::Finished(id, status, duration) => {
                    if let Some(e) = self.entry_mut(id) {
                        e.status = status;
                        e.duration = Some(duration);
                    }
                }
                RunEvent::Done => done = true,
            }
        }
        if done {
            for entry in self.suites.iter_mut().flat_map(|s| s.tests.iter_mut()) {
                if entry.status == TestStatus::Queued {
                    entry.status = initial_status(&entry.case);
                }
            }
        } else {
            self.events = Some(rx);
        }
    }

    /// Returns the number of all, passed, failed, and skipped tests.
    fn counts(&self) -> (usize, usize, usize, usize) {
        let all = || self.suites.iter().flat_map(|s| s.tests.iter());
        let count = |pred: fn(TestStatus) -> bool| all().filter(|t| pred(t.status)).count();
        (
            all().count(),
            count(|s| s == TestStatus::Passed),
            count(TestStatus::is_failure),
            count(|s| s == TestStatus::Skipped),
        )
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let running = self.is_running();
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.workspace.kind.label()).strong());
            ui.label(self.workspace.root.display().to_string());
            if let Some(setup) = &self.workspace.setup_script {
                ui.label(RichText::new(format!("(source {})", setup.display())).weak());
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("⟳ Refresh"))
                .clicked()
            {
                self.refresh();
            }
            if ui
                .add_enabled(!running, egui::Button::new("▶ Run all"))
                .on_hover_text("Run all tests matching the filter")
                .clicked()
            {
                let ids = self.visible_ids();
                self.start_run(ids, ui.ctx());
            }
            let failed: Vec<TestId> = self
                .visible_ids()
                .into_iter()
                .filter(|&id| self.entry(id).is_some_and(|e| e.status.is_failure()))
                .collect();
            if ui
                .add_enabled(
                    !running && !failed.is_empty(),
                    egui::Button::new("▶ Run failed"),
                )
                .clicked()
            {
                self.start_run(failed, ui.ctx());
            }
            if ui
                .add_enabled(running, egui::Button::new("■ Stop"))
                .clicked()
            {
                self.cancel.store(true, Ordering::SeqCst);
            }
            ui.separator();
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("name or label")
                    .desired_width(200.0),
            );
            ui.separator();
            let (total, passed, failed, skipped) = self.counts();
            ui.label(format!("{total} tests"));
            ui.colored_label(status_color(TestStatus::Passed), format!("{passed} passed"));
            ui.colored_label(status_color(TestStatus::Failed), format!("{failed} failed"));
            ui.colored_label(
                status_color(TestStatus::Skipped),
                format!("{skipped} skipped"),
            );
            if running {
                ui.spinner();
            }
        });
    }

    fn test_tree(&mut self, ui: &mut egui::Ui) {
        let mut to_run: Option<Vec<TestId>> = None;
        let mut to_open: Option<Location> = None;
        let running = self.is_running();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.suites.is_empty() {
                    ui.label("No build directories with tests found.");
                }
                for si in 0..self.suites.len() {
                    let visible: Vec<usize> = self.suites[si]
                        .tests
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| self.matches_filter(&t.case))
                        .map(|(i, _)| i)
                        .collect();
                    let suite = &self.suites[si];
                    if visible.is_empty() && suite.error.is_none() && !self.filter.trim().is_empty()
                    {
                        continue;
                    }
                    let header = format!("{} ({})", suite.suite.name, suite.tests.len());
                    let response = egui::CollapsingHeader::new(RichText::new(header).strong())
                        .id_salt(("suite", si))
                        .default_open(true)
                        .show(ui, |ui| {
                            if let Some(err) = &self.suites[si].error {
                                ui.colored_label(Color32::RED, err);
                            }
                            for &ti in &visible {
                                let id = TestId {
                                    suite: si,
                                    test: ti,
                                };
                                let entry = &self.suites[si].tests[ti];
                                ui.horizontal(|ui| {
                                    status_icon(ui, entry.status);
                                    let resp = ui
                                        .selectable_label(
                                            self.selected == Some(id),
                                            &entry.case.name,
                                        )
                                        .on_hover_text("Double click to run");
                                    if resp.clicked() {
                                        self.selected = Some(id);
                                    }
                                    if resp.double_clicked() && !running {
                                        to_run = Some(vec![id]);
                                    }
                                    resp.context_menu(|ui| {
                                        if ui
                                            .add_enabled(!running, egui::Button::new("▶ Run"))
                                            .clicked()
                                        {
                                            to_run = Some(vec![id]);
                                            ui.close();
                                        }
                                    });
                                });
                                if entry.has_child_cases() {
                                    for case in &entry.cases {
                                        ui.horizontal(|ui| {
                                            ui.add_space(20.0);
                                            status_icon(ui, case.status);
                                            let mut resp = ui.add(
                                                egui::Label::new(RichText::new(&case.name).weak())
                                                    .selectable(false)
                                                    .sense(egui::Sense::click()),
                                            );
                                            if let Some(loc) = &case.location {
                                                resp = resp.on_hover_text(format!(
                                                    "{}:{}\nDouble click to open in editor",
                                                    loc.path.display(),
                                                    loc.line
                                                ));
                                                if resp.double_clicked() {
                                                    to_open = Some(loc.clone());
                                                }
                                            }
                                            if resp.clicked() {
                                                self.selected = Some(id);
                                            }
                                        });
                                    }
                                }
                            }
                        });
                    response.header_response.context_menu(|ui| {
                        if ui
                            .add_enabled(!running, egui::Button::new("▶ Run suite"))
                            .clicked()
                        {
                            to_run = Some(
                                visible
                                    .iter()
                                    .map(|&ti| TestId {
                                        suite: si,
                                        test: ti,
                                    })
                                    .collect(),
                            );
                            ui.close();
                        }
                    });
                }
            });
        if let Some(ids) = to_run {
            self.start_run(ids, ui.ctx());
        }
        if let Some(location) = to_open {
            self.notice = editor::open(&location).err();
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.centered_and_justified(|ui| ui.label("Select a test to see its details."));
            return;
        };
        let Some(entry) = self.entry(id) else {
            return;
        };
        let running = self.is_running();
        let mut run = false;
        ui.visuals_mut().override_text_color = Some(TEXT_COLOR);
        ui.horizontal(|ui| {
            ui.heading(&entry.case.name);
            status_icon(ui, entry.status);
            ui.label(
                RichText::new(status_text(entry.status))
                    .strong()
                    .color(status_color(entry.status)),
            );
            if let Some(d) = entry.duration {
                ui.label(format!("{:.2} s", d.as_secs_f64()));
            }
            run = ui
                .add_enabled(!running, egui::Button::new("▶ Run"))
                .clicked();
        });
        egui::Grid::new("details").num_columns(2).show(ui, |ui| {
            ui.label("Suite:");
            ui.label(&self.suites[id.suite].suite.name);
            ui.end_row();
            ui.label("Command:");
            ui.label(RichText::new(entry.case.command.join(" ")).monospace());
            ui.end_row();
            if !entry.case.labels.is_empty() {
                ui.label("Labels:");
                ui.label(entry.case.labels.join(", "));
                ui.end_row();
            }
            if let Some(dir) = &entry.case.working_directory {
                ui.label("Working dir:");
                ui.label(RichText::new(dir).monospace());
                ui.end_row();
            }
        });
        let mut open: Option<Location> = None;
        if !entry.cases.is_empty() {
            ui.separator();
            open = cases_view(ui, &entry.cases);
        }
        ui.separator();
        if let Some(notice) = &self.notice {
            ui.colored_label(status_color(TestStatus::Failed), notice);
        }
        if entry.output.is_empty() {
            ui.label(RichText::new("(no output)").monospace().weak());
        } else if let Some(location) = output_view(ui, &entry.output) {
            open = Some(location);
        }
        if let Some(location) = open {
            self.notice = editor::open(&location).err();
        }
        if run {
            self.start_run(vec![id], ui.ctx());
        }
    }
}

impl eframe::App for CtguiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::left("tests")
            .resizable(true)
            .default_size(360.0)
            .show(ui, |ui| self.test_tree(ui));
        egui::CentralPanel::default().show(ui, |ui| self.details(ui));
    }
}

/// Color of plain text in the details panel.
const TEXT_COLOR: Color32 = Color32::from_gray(240);

impl OutputLine {
    /// Splits a line into plain text, result keywords, and source locations.
    fn new(text: &str, base_dirs: &[&Path]) -> Self {
        let text = text.trim_end_matches(['\n', '\r']).to_owned();
        let colors: Vec<(Range<usize>, Color32)> = highlight::spans(&text)
            .into_iter()
            .map(|(range, status)| (range, status_color(status)))
            .collect();
        let links = location::find_links(&text, base_dirs);

        let mut bounds: Vec<usize> = vec![0, text.len()];
        for range in colors
            .iter()
            .map(|(r, _)| r)
            .chain(links.iter().map(|(r, _)| r))
        {
            bounds.extend([range.start, range.end]);
        }
        bounds.sort_unstable();
        bounds.dedup();

        let segments = bounds
            .windows(2)
            .map(|w| w[0]..w[1])
            .filter(|range| !range.is_empty())
            .map(|range| {
                let inside = |r: &Range<usize>| r.start <= range.start && range.end <= r.end;
                Segment {
                    color: colors
                        .iter()
                        .find(|(r, _)| inside(r))
                        .map_or(TEXT_COLOR, |(_, c)| *c),
                    link: links
                        .iter()
                        .find(|(r, _)| inside(r))
                        .map(|(_, l)| l.clone()),
                    range,
                }
            })
            .collect();
        OutputLine { text, segments }
    }
}

/// Shows an underlined text that opens `location` when clicked, and returns whether it was clicked.
fn location_link(ui: &mut egui::Ui, text: RichText, location: &Location) -> bool {
    ui.add(
        egui::Label::new(text.underline())
            .extend()
            .selectable(false)
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(format!(
        "Open {}:{} in editor",
        location.path.display(),
        location.line
    ))
    .clicked()
}

/// Shows the GoogleTest cases of a test and returns the location the user clicked.
fn cases_view(ui: &mut egui::Ui, cases: &[CaseResult]) -> Option<Location> {
    let mut clicked = None;
    egui::CollapsingHeader::new(format!("Test cases ({})", cases.len()))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("cases")
                .max_height(200.0)
                .show(ui, |ui| {
                    egui::Grid::new("cases").num_columns(4).show(ui, |ui| {
                        for case in cases {
                            status_icon(ui, case.status);
                            ui.label(&case.name);
                            match &case.location {
                                Some(location) => {
                                    let file = location.path.file_name().map_or_else(
                                        || location.path.display().to_string(),
                                        |n| n.to_string_lossy().into_owned(),
                                    );
                                    let text = RichText::new(format!("{file}:{}", location.line))
                                        .monospace();
                                    if location_link(ui, text, location) {
                                        clicked = Some(location.clone());
                                    }
                                }
                                None => {
                                    ui.label("");
                                }
                            }
                            match case.duration {
                                Some(d) => ui.label(format!("{:.3} s", d.as_secs_f64())),
                                None => ui.label(""),
                            };
                            ui.end_row();
                        }
                    });
                });
        });
    clicked
}

/// Shows test output, one row per line, and returns the location the user clicked.
fn output_view(ui: &mut egui::Ui, lines: &[OutputLine]) -> Option<Location> {
    let mut clicked = None;
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    ui.spacing_mut().interact_size.y = row_height;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                ui.horizontal(|ui| {
                    for segment in &line.segments {
                        let text = RichText::new(&line.text[segment.range.clone()])
                            .monospace()
                            .color(segment.color);
                        let Some(location) = &segment.link else {
                            ui.add(egui::Label::new(text).extend());
                            continue;
                        };
                        if location_link(ui, text, location) {
                            clicked = Some(location.clone());
                        }
                    }
                });
            }
        });
    clicked
}

/// Status of a test that has not been run in this session.
fn initial_status(case: &TestCase) -> TestStatus {
    if case.disabled {
        TestStatus::Disabled
    } else {
        TestStatus::NotRun
    }
}

fn status_color(status: TestStatus) -> Color32 {
    match status {
        TestStatus::NotRun => Color32::from_rgb(165, 165, 165),
        TestStatus::Queued => Color32::from_rgb(148, 163, 184),
        TestStatus::Running => Color32::YELLOW,
        TestStatus::Passed => Color32::from_rgb(80, 200, 120),
        TestStatus::Failed => Color32::from_rgb(230, 80, 80),
        TestStatus::Timeout => Color32::from_rgb(245, 158, 11),
        TestStatus::Crashed => Color32::from_rgb(200, 90, 230),
        TestStatus::Skipped => Color32::from_rgb(96, 165, 250),
        TestStatus::Disabled => Color32::from_rgb(125, 125, 125),
        TestStatus::Cancelled => Color32::from_rgb(190, 190, 190),
    }
}

fn status_text(status: TestStatus) -> &'static str {
    match status {
        TestStatus::NotRun => "not run",
        TestStatus::Queued => "queued",
        TestStatus::Running => "running",
        TestStatus::Passed => "passed",
        TestStatus::Failed => "failed",
        TestStatus::Timeout => "timeout",
        TestStatus::Crashed => "crashed",
        TestStatus::Skipped => "skipped",
        TestStatus::Disabled => "disabled",
        TestStatus::Cancelled => "cancelled",
    }
}

fn status_icon(ui: &mut egui::Ui, status: TestStatus) {
    // Fixed size so that names stay aligned while the spinner is shown.
    let size = [16.0, 16.0];
    if status == TestStatus::Running {
        ui.add_sized(size, egui::Spinner::new().size(12.0));
    } else {
        ui.add_sized(
            size,
            egui::Label::new(RichText::new("●").color(status_color(status))),
        )
        .on_hover_text(status_text(status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_output_line_into_text_and_links() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.cpp"), "").unwrap();

        let line = OutputLine::new("a.cpp:28: CHECK failed\n", &[tmp.path()]);
        let parts: Vec<_> = line
            .segments
            .iter()
            .map(|s| (&line.text[s.range.clone()], s.link.as_ref().map(|l| l.line)))
            .collect();
        assert_eq!(parts, [("a.cpp:28", Some(28)), (": CHECK failed", None)]);
    }

    #[test]
    fn colors_result_keywords_in_output_line() {
        let line = OutputLine::new("1/1 Test #1: t ....***Failed    0.00 sec", &[]);
        let failed = line
            .segments
            .iter()
            .find(|s| &line.text[s.range.clone()] == "***Failed")
            .unwrap();
        assert_eq!(failed.color, status_color(TestStatus::Failed));
        assert!(line.segments.iter().all(|s| s.link.is_none()));
    }
}
