use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, TerminalOptions, Viewport};

use crate::ui::layout;

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
    last_line: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub aborted: bool,
}

enum RunnerEvent {
    Started(usize),
    Line(usize, String),
    Finished(usize, ToolState),
    Done(Result<(), String>),
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
    index: usize,
}

impl ToolHandle {
    pub fn started(&self) {
        let _ = self.tx.send(RunnerEvent::Started(self.index));
    }

    pub fn line(&self, line: impl Into<String>) {
        let _ = self.tx.send(RunnerEvent::Line(self.index, line.into()));
    }

    pub fn finished(&self, state: ToolState) {
        let _ = self.tx.send(RunnerEvent::Finished(self.index, state));
    }
}

#[derive(Clone)]
pub struct ExecContext {
    tx: Sender<RunnerEvent>,
    tool_index: Arc<HashMap<String, usize>>,
    abort: Arc<AtomicBool>,
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
            index,
        })
    }

    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.abort.load(Ordering::Relaxed)
    }

    pub fn abort(&self) {
        self.abort.store(true, Ordering::Relaxed);
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
    let abort = Arc::new(AtomicBool::new(false));
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
        abort: Arc::clone(&abort),
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
            last_line: None,
        })
        .collect::<Vec<_>>();
    let mut complete_result = None;
    let mut aborted = false;

    while complete_result.is_none() {
        match rx.recv_timeout(Duration::from_millis(66)) {
            Ok(event) => apply_event(event, &mut tools, &mut complete_result, &mut observer),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        update_elapsed(&mut tools);
        if let Some(terminal) = terminal.as_mut() {
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
                .map_err(|e| format!("failed to draw dashboard: {e}"))?;
        }

        if interactive && poll_ctrl_c() {
            aborted = true;
            abort.store(true, Ordering::Relaxed);
            break;
        }
    }

    if interactive {
        ratatui::restore();
        // Leave the cursor on a clean line; otherwise stdout/stderr can splice into the viewport.
        let _ = io::stdout().write_all(b"\n");
        let _ = io::stderr().write_all(b"\n");
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
    let _ = exec_thread.join();

    if aborted {
        return Ok(RunOutcome { aborted: true });
    }
    if let Some(result) = complete_result {
        result?;
    }
    Ok(RunOutcome { aborted: false })
}

fn apply_event<G>(
    event: RunnerEvent,
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
        RunnerEvent::Line(index, line) => {
            if let Some(tool) = tools.get_mut(index) {
                tool.last_line = Some(line.clone());
                observer(ProgressEvent::Line {
                    language: tool.language.clone(),
                    name: tool.name.clone(),
                    line: line.clone(),
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
}
