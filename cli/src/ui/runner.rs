use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ayni_core::CancellationToken;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, TerminalOptions, Viewport};

use crate::ui::cancellation::SignalCancellation;
use crate::ui::layout;

const PROGRESS_LINE_LIMIT: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug)]
pub struct PlanTool {
    pub id: String,
    pub language: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub tools: Vec<PlanTool>,
}

#[derive(Clone, Debug)]
pub struct ToolView {
    pub language: String,
    pub state: ToolState,
}

#[derive(Clone, Debug)]
pub struct DashboardView {
    pub tools: Vec<ToolView>,
}

#[derive(Clone, Debug)]
struct ToolRuntimeState {
    language: String,
    name: String,
    state: ToolState,
    started_at: Option<Instant>,
    elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub aborted: bool,
}

enum RunnerEvent {
    Started(usize),
    Line(usize),
    Finished(usize, ToolState),
    Done(Result<(), String>),
}

#[derive(Default)]
struct RunnerLoopOutcome {
    complete_result: Option<Result<(), String>>,
    aborted: bool,
    runner_error: Option<String>,
}

#[derive(Default)]
struct PendingLine {
    value: Option<String>,
    queued: bool,
}

struct ProgressLines {
    slots: Mutex<Vec<PendingLine>>,
}

impl ProgressLines {
    fn new(tool_count: usize) -> Self {
        Self {
            slots: Mutex::new(
                std::iter::repeat_with(PendingLine::default)
                    .take(tool_count)
                    .collect(),
            ),
        }
    }

    fn update(&self, index: usize, line: String) -> bool {
        let Ok(mut slots) = self.slots.lock() else {
            return false;
        };
        let Some(slot) = slots.get_mut(index) else {
            return false;
        };
        slot.value = Some(bounded_progress_line(line));
        if slot.queued {
            false
        } else {
            slot.queued = true;
            true
        }
    }

    fn take(&self, index: usize) -> Option<String> {
        let Ok(mut slots) = self.slots.lock() else {
            return None;
        };
        let slot = slots.get_mut(index)?;
        slot.queued = false;
        slot.value.take()
    }
}

#[derive(Clone, Debug)]
pub enum ProgressEvent {
    Started {
        language: String,
        name: String,
    },
    Line {
        language: String,
        name: String,
        line: String,
    },
    Finished {
        language: String,
        name: String,
        state: ToolState,
        elapsed: Duration,
    },
}

#[derive(Clone)]
pub struct ToolHandle {
    tx: Sender<RunnerEvent>,
    progress_lines: Arc<ProgressLines>,
    index: usize,
}

impl ToolHandle {
    pub fn started(&self) {
        let _ = self.tx.send(RunnerEvent::Started(self.index));
    }

    pub fn line(&self, line: impl Into<String>) {
        if self.progress_lines.update(self.index, line.into()) {
            let _ = self.tx.send(RunnerEvent::Line(self.index));
        }
    }

    pub fn finished(&self, state: ToolState) {
        let _ = self.tx.send(RunnerEvent::Finished(self.index, state));
    }
}

#[derive(Clone)]
pub struct ExecContext {
    tx: Sender<RunnerEvent>,
    tool_index: Arc<HashMap<String, usize>>,
    progress_lines: Arc<ProgressLines>,
    cancellation: CancellationToken,
}

impl ExecContext {
    pub fn tool(&self, id: &str) -> Result<ToolHandle, String> {
        let index = self
            .tool_index
            .get(id)
            .copied()
            .ok_or_else(|| format!("unknown tool id: {id}"))?;
        Ok(ToolHandle {
            tx: self.tx.clone(),
            progress_lines: Arc::clone(&self.progress_lines),
            index,
        })
    }

    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn abort(&self) {
        self.cancellation.cancel();
    }

    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

pub fn run<F>(plan: Plan, exec: F) -> Result<RunOutcome, String>
where
    F: FnOnce(ExecContext) -> Result<(), String> + Send + 'static,
{
    run_internal(plan, exec, true, |_| {})
}

pub fn run_plain<F, G>(plan: Plan, exec: F, observer: G) -> Result<RunOutcome, String>
where
    F: FnOnce(ExecContext) -> Result<(), String> + Send + 'static,
    G: FnMut(ProgressEvent),
{
    run_internal(plan, exec, false, observer)
}

fn run_internal<F, G>(
    plan: Plan,
    exec: F,
    interactive: bool,
    mut observer: G,
) -> Result<RunOutcome, String>
where
    F: FnOnce(ExecContext) -> Result<(), String> + Send + 'static,
    G: FnMut(ProgressEvent),
{
    let (tx, rx): (Sender<RunnerEvent>, Receiver<RunnerEvent>) = mpsc::channel();
    let signal_cancellation = SignalCancellation::install()?;
    let cancellation = signal_cancellation.token();
    let progress_lines = Arc::new(ProgressLines::new(plan.tools.len()));
    let tool_index = Arc::new(
        plan.tools
            .iter()
            .enumerate()
            .map(|(idx, t)| (t.id.clone(), idx))
            .collect::<HashMap<_, _>>(),
    );
    let exec_ctx = ExecContext {
        tx: tx.clone(),
        tool_index,
        progress_lines: Arc::clone(&progress_lines),
        cancellation: cancellation.clone(),
    };
    let exec_thread = thread::spawn(move || {
        let result = exec(exec_ctx);
        let _ = tx.send(RunnerEvent::Done(result));
    });

    let mut terminal = interactive.then(|| init_terminal(calc_height(&plan)));
    let mut tools = plan
        .tools
        .iter()
        .map(|t| ToolRuntimeState {
            language: t.language.clone(),
            name: t.name.clone(),
            state: ToolState::Queued,
            started_at: None,
            elapsed: Duration::ZERO,
        })
        .collect::<Vec<_>>();
    let control = RunnerControl {
        interactive,
        cancellation: &cancellation,
        signal_cancellation: &signal_cancellation,
    };
    let loop_outcome = drive_runner_loop(
        &rx,
        &progress_lines,
        &mut terminal,
        &mut tools,
        &mut observer,
        &control,
    );

    restore_terminal(interactive);
    let join_error = exec_thread
        .join()
        .err()
        .map(|_| String::from("analysis execution thread panicked"));
    finish_runner(loop_outcome, join_error)
}

struct RunnerControl<'a> {
    interactive: bool,
    cancellation: &'a CancellationToken,
    signal_cancellation: &'a SignalCancellation,
}

fn drive_runner_loop<G>(
    rx: &Receiver<RunnerEvent>,
    progress_lines: &ProgressLines,
    terminal: &mut Option<DefaultTerminal>,
    tools: &mut [ToolRuntimeState],
    observer: &mut G,
    control: &RunnerControl<'_>,
) -> RunnerLoopOutcome
where
    G: FnMut(ProgressEvent),
{
    let mut outcome = RunnerLoopOutcome::default();
    while outcome.complete_result.is_none() {
        if !receive_runner_event(
            rx,
            progress_lines,
            tools,
            &mut outcome.complete_result,
            observer,
        ) {
            break;
        }
        update_elapsed(tools);
        if let Err(error) = draw_dashboard(terminal, tools) {
            control.cancellation.cancel();
            outcome.runner_error = Some(error);
            break;
        }
        if abort_requested(control.interactive, control.signal_cancellation) {
            control.cancellation.cancel();
            outcome.aborted = true;
            break;
        }
    }
    outcome
}

fn receive_runner_event<G>(
    rx: &Receiver<RunnerEvent>,
    progress_lines: &ProgressLines,
    tools: &mut [ToolRuntimeState],
    complete_result: &mut Option<Result<(), String>>,
    observer: &mut G,
) -> bool
where
    G: FnMut(ProgressEvent),
{
    match rx.recv_timeout(Duration::from_millis(66)) {
        Ok(event) => {
            apply_event(event, progress_lines, tools, complete_result, observer);
            true
        }
        Err(RecvTimeoutError::Timeout) => true,
        Err(RecvTimeoutError::Disconnected) => false,
    }
}

fn draw_dashboard(
    terminal: &mut Option<DefaultTerminal>,
    tools: &[ToolRuntimeState],
) -> Result<(), String> {
    let Some(terminal) = terminal.as_mut() else {
        return Ok(());
    };
    let view = DashboardView {
        tools: tools
            .iter()
            .map(|tool| ToolView {
                language: tool.language.clone(),
                state: tool.state,
            })
            .collect(),
    };
    terminal
        .draw(|frame| layout::render(frame, &view))
        .map(|_| ())
        .map_err(|error| format!("failed to draw dashboard: {error}"))
}

fn abort_requested(interactive: bool, signal_cancellation: &SignalCancellation) -> bool {
    (interactive && poll_ctrl_c()) || signal_cancellation.interrupted()
}

fn restore_terminal(interactive: bool) {
    if !interactive {
        return;
    }
    ratatui::restore();
    // Leave the cursor on a clean line; otherwise stdout/stderr can splice into the viewport.
    let _ = io::stdout().write_all(b"\n");
    let _ = io::stderr().write_all(b"\n");
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
}

fn finish_runner(
    loop_outcome: RunnerLoopOutcome,
    join_error: Option<String>,
) -> Result<RunOutcome, String> {
    if let Some(error) = loop_outcome.runner_error.or(join_error) {
        return Err(error);
    }
    if loop_outcome.aborted {
        return Ok(RunOutcome { aborted: true });
    }
    if let Some(result) = loop_outcome.complete_result {
        result?;
    }
    Ok(RunOutcome { aborted: false })
}

fn apply_event<G>(
    event: RunnerEvent,
    progress_lines: &ProgressLines,
    tools: &mut [ToolRuntimeState],
    complete_result: &mut Option<Result<(), String>>,
    observer: &mut G,
) where
    G: FnMut(ProgressEvent),
{
    match event {
        RunnerEvent::Started(index) => {
            if let Some(tool) = tools.get_mut(index) {
                tool.state = ToolState::Running;
                tool.started_at = Some(Instant::now());
                observer(ProgressEvent::Started {
                    language: tool.language.clone(),
                    name: tool.name.clone(),
                });
            }
        }
        RunnerEvent::Line(index) => {
            if let (Some(tool), Some(line)) = (tools.get_mut(index), progress_lines.take(index)) {
                observer(ProgressEvent::Line {
                    language: tool.language.clone(),
                    name: tool.name.clone(),
                    line,
                });
            }
        }
        RunnerEvent::Finished(index, state) => {
            if let Some(tool) = tools.get_mut(index) {
                tool.state = state;
                if let Some(started_at) = tool.started_at.take() {
                    tool.elapsed = started_at.elapsed();
                }
                observer(ProgressEvent::Finished {
                    language: tool.language.clone(),
                    name: tool.name.clone(),
                    state,
                    elapsed: tool.elapsed,
                });
            }
        }
        RunnerEvent::Done(result) => {
            *complete_result = Some(result);
        }
    }
}

fn bounded_progress_line(line: String) -> String {
    if line.len() <= PROGRESS_LINE_LIMIT {
        return line;
    }
    let mut start = line.len() - PROGRESS_LINE_LIMIT;
    while !line.is_char_boundary(start) {
        start += 1;
    }
    let omitted = start;
    format!("[... {omitted} bytes omitted ...] {}", &line[start..])
}

fn update_elapsed(tools: &mut [ToolRuntimeState]) {
    for tool in tools {
        if tool.state == ToolState::Running
            && let Some(started_at) = tool.started_at
        {
            tool.elapsed = started_at.elapsed();
        }
    }
}

fn calc_height(plan: &Plan) -> u16 {
    let target_count = plan
        .tools
        .iter()
        .map(|tool| tool.language.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    // 1 header row + one row per target + a small buffer.
    let rows = 1usize + target_count.max(1) + 2;
    rows.min(u16::MAX as usize) as u16
}

fn init_terminal(height: u16) -> DefaultTerminal {
    ratatui::init_with_options(TerminalOptions {
        viewport: Viewport::Inline(height),
    })
}

fn poll_ctrl_c() -> bool {
    if event::poll(Duration::from_millis(1)).unwrap_or(false)
        && let Ok(Event::Key(key)) = event::read()
    {
        return key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_plain_emits_tool_events_in_order() {
        let plan = Plan {
            tools: vec![PlanTool {
                id: String::from("rust:test"),
                language: String::from("rust"),
                name: String::from("cargo test"),
            }],
        };
        let mut events = Vec::new();
        let result = run_plain(
            plan,
            |ctx| {
                let tool = ctx.tool("rust:test")?;
                tool.started();
                tool.line("compiling");
                tool.finished(ToolState::Done);
                Ok(())
            },
            |event| events.push(event),
        );
        assert!(result.is_ok());
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], ProgressEvent::Started { .. }));
        assert!(matches!(events[1], ProgressEvent::Line { .. }));
        assert!(matches!(
            events[2],
            ProgressEvent::Finished {
                state: ToolState::Done,
                ..
            }
        ));
    }

    #[test]
    fn progress_lines_coalesce_while_control_events_remain_ordered() {
        let (tx, rx) = mpsc::channel();
        let progress_lines = Arc::new(ProgressLines::new(1));
        let tool = ToolHandle {
            tx,
            progress_lines: Arc::clone(&progress_lines),
            index: 0,
        };
        tool.started();
        tool.line("first");
        tool.line("second");
        tool.line("last");
        tool.finished(ToolState::Done);

        assert!(matches!(
            rx.recv().expect("started"),
            RunnerEvent::Started(0)
        ));
        assert!(matches!(rx.recv().expect("line"), RunnerEvent::Line(0)));
        assert_eq!(progress_lines.take(0).as_deref(), Some("last"));
        assert!(matches!(
            rx.recv().expect("finished"),
            RunnerEvent::Finished(0, ToolState::Done)
        ));
        assert!(
            rx.try_recv().is_err(),
            "only one line notification is queued"
        );
    }

    #[test]
    fn progress_lines_retain_only_a_bounded_tail() {
        let line = format!("prefix-{}-tail", "x".repeat(PROGRESS_LINE_LIMIT * 2));
        let bounded = bounded_progress_line(line);
        assert!(bounded.len() < PROGRESS_LINE_LIMIT + 64);
        assert!(bounded.ends_with("-tail"));
        assert!(bounded.starts_with("[... "));
    }
}
