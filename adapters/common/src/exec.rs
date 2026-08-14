//! Tool invocation with concurrent output capture and wall-clock timeouts.
//!
//! Every adapter command goes through this module so a hung tool (a stuck
//! Gradle daemon, a wedged test run) can never block an analyze run forever.

use ayni_core::RunContext;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How often the runner checks a live child when no output arrives.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Maximum time spent draining child pipes after the direct process exits or
/// is terminated. Descendants must not be able to retain a pipe indefinitely.
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Fallback timeout for invocations that have no `RunContext` (and therefore
/// no policy) available. Matches the `execution.tool_timeout_seconds` default.
pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(1800);

/// Stable classification for failures owned by the command runner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionErrorKind {
    /// The operating system could not create the child process.
    Spawn,
    /// Waiting for the child or reading one of its pipes failed.
    Wait,
    /// The child exceeded its wall-clock limit and was killed and reaped.
    Timeout,
}

/// A command-runner failure, including output captured before cleanup.
///
/// A non-zero exit is deliberately not an `ExecutionError`: callers receive a
/// normal [`Output`] and retain responsibility for interpreting tool status.
#[derive(Clone, Debug)]
pub struct ExecutionError {
    pub kind: ExecutionErrorKind,
    pub command: String,
    pub cwd: PathBuf,
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timeout: Option<Duration>,
    pub detail: String,
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ExecutionErrorKind::Spawn => write!(
                formatter,
                "failed to execute {} in {}: {}",
                self.command,
                self.cwd.display(),
                self.detail
            ),
            ExecutionErrorKind::Wait => write!(
                formatter,
                "failed while waiting for {} in {}: {}",
                self.command,
                self.cwd.display(),
                self.detail
            ),
            ExecutionErrorKind::Timeout => write!(
                formatter,
                "command timed out after {}s: {}",
                self.timeout.unwrap_or_default().as_secs_f64(),
                self.command
            ),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Result returned by structured command-runner entry points.
pub type ExecutionResult = Result<Output, Box<ExecutionError>>;

/// Formats a program and args for diagnostics (`cargo test --workspace`).
pub fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

/// Runs a command in `workdir`, capturing stdout/stderr, killing it after `timeout`.
///
/// This compatibility entry point retains the historical string error. New
/// infrastructure that needs stable failure mapping should use
/// [`run_command_structured`].
pub fn run_command(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<Output, String> {
    run_command_structured(workdir, program, args, timeout).map_err(|error| error.to_string())
}

/// Structured-error variant of [`run_command`].
pub fn run_command_structured(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
) -> ExecutionResult {
    run_command_streaming_structured(workdir, program, args, timeout, |_| {})
}

/// Like [`run_command`], but invokes `on_line` for every complete stdout and
/// stderr line as it arrives, and for a final non-empty partial line.
pub fn run_command_streaming(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    on_line: impl FnMut(&str),
) -> Result<Output, String> {
    run_command_streaming_structured(workdir, program, args, timeout, on_line)
        .map_err(|error| error.to_string())
}

/// Structured-error variant of [`run_command_streaming`].
pub fn run_command_streaming_structured(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    mut on_line: impl FnMut(&str),
) -> ExecutionResult {
    let command_text = format_command(program, args);
    let mut command = Command::new(program);
    command
        .args(args.iter().map(String::as_str))
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        Box::new(ExecutionError {
            kind: ExecutionErrorKind::Spawn,
            command: command_text.clone(),
            cwd: workdir.to_path_buf(),
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            timeout: None,
            detail: error.to_string(),
        })
    })?;

    let (sender, receiver) = mpsc::channel();
    spawn_reader(Stream::Stdout, child.stdout.take(), sender.clone());
    spawn_reader(Stream::Stderr, child.stderr.take(), sender);

    let started = Instant::now();
    let mut capture = Capture::default();
    let mut status = None;
    let mut execution_failure = None;

    while status.is_none() && execution_failure.is_none() {
        drain_available(
            &receiver,
            &mut capture,
            &mut on_line,
            &mut execution_failure,
        );
        match child.try_wait() {
            Ok(Some(exit_status)) => status = Some(exit_status),
            Ok(None) if started.elapsed() >= timeout => {
                execution_failure = Some((ExecutionErrorKind::Timeout, String::new()));
            }
            Ok(None) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                let wait = POLL_INTERVAL.min(remaining);
                if let Ok(event) = receiver.recv_timeout(wait) {
                    capture.handle(event, &mut on_line, &mut execution_failure);
                }
            }
            Err(error) => {
                execution_failure = Some((ExecutionErrorKind::Wait, error.to_string()));
            }
        }
    }

    if let Some((kind, mut detail)) = execution_failure {
        let cleanup_status = terminate_and_reap(&mut child, &mut detail);
        capture.drain_to_end(&receiver, &mut on_line, &mut detail, OUTPUT_DRAIN_TIMEOUT);
        return Err(Box::new(ExecutionError {
            kind,
            command: command_text,
            cwd: workdir.to_path_buf(),
            status: cleanup_status,
            stdout: capture.stdout.bytes,
            stderr: capture.stderr.bytes,
            timeout: (kind == ExecutionErrorKind::Timeout).then_some(timeout),
            detail,
        }));
    }

    let mut detail = String::new();
    capture.drain_to_end(&receiver, &mut on_line, &mut detail, OUTPUT_DRAIN_TIMEOUT);
    if !detail.is_empty() {
        return Err(Box::new(ExecutionError {
            kind: ExecutionErrorKind::Wait,
            command: command_text,
            cwd: workdir.to_path_buf(),
            status,
            stdout: capture.stdout.bytes,
            stderr: capture.stderr.bytes,
            timeout: None,
            detail,
        }));
    }
    Ok(Output {
        status: status.expect("runner loop exits normally only with child status"),
        stdout: capture.stdout.bytes,
        stderr: capture.stderr.bytes,
    })
}

/// Runs a command in the context's execution cwd with the policy timeout and
/// debug diagnostics. This is the standard entry point for collectors.
pub fn run_command_for_context(
    context: &RunContext,
    program: &str,
    args: &[String],
) -> Result<Output, String> {
    run_command_for_context_structured(context, program, args).map_err(|error| error.to_string())
}

/// Streaming variant of [`run_command_for_context`].
pub fn run_command_for_context_streaming(
    context: &RunContext,
    program: &str,
    args: &[String],
    on_line: impl FnMut(&str),
) -> Result<Output, String> {
    run_command_for_context_streaming_structured(context, program, args, on_line)
        .map_err(|error| error.to_string())
}

/// Structured-error variant of [`run_command_for_context`].
pub fn run_command_for_context_structured(
    context: &RunContext,
    program: &str,
    args: &[String],
) -> ExecutionResult {
    run_command_for_context_streaming_structured(context, program, args, |_| {})
}

/// Structured-error variant of [`run_command_for_context_streaming`].
pub fn run_command_for_context_streaming_structured(
    context: &RunContext,
    program: &str,
    args: &[String],
    on_line: impl FnMut(&str),
) -> ExecutionResult {
    let output = run_command_streaming_structured(
        &context.execution.exec_cwd,
        program,
        args,
        context_timeout(context),
        on_line,
    )?;
    debug_output(context, program, args, &output);
    Ok(output)
}

/// The wall-clock timeout configured for tool invocations in this run.
pub fn context_timeout(context: &RunContext) -> Duration {
    Duration::from_secs(context.policy.execution.tool_timeout_seconds)
}

fn debug_output(context: &RunContext, program: &str, args: &[String], output: &Output) {
    if !context.debug {
        return;
    }
    eprintln!(
        "[debug] runner={} source={} kind={} resolved_from={} confidence={} ambiguous={}",
        context.execution.runner,
        context.execution.source,
        context.execution.kind,
        context.execution.resolved_from.display(),
        context.execution.confidence,
        context.execution.ambiguous
    );
    eprintln!(
        "[debug] cwd={} command={} {}",
        context.execution.exec_cwd.display(),
        program,
        args.join(" ")
    );
    eprintln!("[debug] exit={}", output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        eprintln!("[debug] stdout:\n{}", stdout.trim_end());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        eprintln!("[debug] stderr:\n{}", stderr.trim_end());
    }
}

#[derive(Clone, Copy, Debug)]
enum Stream {
    Stdout,
    Stderr,
}

enum ReaderEvent {
    Data(Stream, Vec<u8>),
    Done(Stream, Option<String>),
}

fn spawn_reader(
    stream_name: Stream,
    stream: Option<impl Read + Send + 'static>,
    sender: mpsc::Sender<ReaderEvent>,
) {
    thread::spawn(move || {
        let Some(mut stream) = stream else {
            let _ = sender.send(ReaderEvent::Done(
                stream_name,
                Some(String::from("child pipe was unavailable")),
            ));
            return;
        };
        let mut buffer = [0u8; 8192];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(ReaderEvent::Done(stream_name, None));
                    return;
                }
                Ok(read) => {
                    if sender
                        .send(ReaderEvent::Data(stream_name, buffer[..read].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(ReaderEvent::Done(stream_name, Some(error.to_string())));
                    return;
                }
            }
        }
    });
}

#[derive(Default)]
struct StreamCapture {
    bytes: Vec<u8>,
    line_start: usize,
    done: bool,
}

impl StreamCapture {
    fn push(&mut self, chunk: &[u8], on_line: &mut impl FnMut(&str)) {
        self.bytes.extend_from_slice(chunk);
        while let Some(offset) = self.bytes[self.line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let line_end = self.line_start + offset;
            let line = String::from_utf8_lossy(&self.bytes[self.line_start..line_end]);
            on_line(line.trim_end_matches('\r'));
            self.line_start = line_end + 1;
        }
    }

    fn finish(&mut self, on_line: &mut impl FnMut(&str)) {
        if self.done {
            return;
        }
        if self.line_start < self.bytes.len() {
            let line = String::from_utf8_lossy(&self.bytes[self.line_start..]);
            let line = line.trim_end_matches('\r');
            if !line.is_empty() {
                on_line(line);
            }
        }
        self.done = true;
    }
}

#[derive(Default)]
struct Capture {
    stdout: StreamCapture,
    stderr: StreamCapture,
}

impl Capture {
    fn handle(
        &mut self,
        event: ReaderEvent,
        on_line: &mut impl FnMut(&str),
        failure: &mut Option<(ExecutionErrorKind, String)>,
    ) {
        match event {
            ReaderEvent::Data(Stream::Stdout, bytes) => self.stdout.push(&bytes, on_line),
            ReaderEvent::Data(Stream::Stderr, bytes) => self.stderr.push(&bytes, on_line),
            ReaderEvent::Done(stream, error) => {
                self.stream_mut(stream).finish(on_line);
                if let Some(error) = error {
                    *failure = Some((
                        ExecutionErrorKind::Wait,
                        format!("failed to read {}: {error}", stream.name()),
                    ));
                }
            }
        }
    }

    fn stream_mut(&mut self, stream: Stream) -> &mut StreamCapture {
        match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        }
    }

    fn drain_to_end(
        &mut self,
        receiver: &mpsc::Receiver<ReaderEvent>,
        on_line: &mut impl FnMut(&str),
        detail: &mut String,
        timeout: Duration,
    ) {
        let started = Instant::now();
        while !self.stdout.done || !self.stderr.done {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                append_detail(detail, "timed out while draining child output pipes");
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok(event) => {
                    let mut failure = None;
                    self.handle(event, on_line, &mut failure);
                    if let Some((_, error)) = failure {
                        append_detail(detail, &error);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    append_detail(detail, "timed out while draining child output pipes");
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    append_detail(detail, "output readers disconnected");
                    break;
                }
            }
        }
    }
}

impl Stream {
    fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

fn drain_available(
    receiver: &mpsc::Receiver<ReaderEvent>,
    capture: &mut Capture,
    on_line: &mut impl FnMut(&str),
    failure: &mut Option<(ExecutionErrorKind, String)>,
) {
    while let Ok(event) = receiver.try_recv() {
        capture.handle(event, on_line, failure);
    }
}

fn terminate_and_reap(child: &mut Child, detail: &mut String) -> Option<ExitStatus> {
    #[cfg(unix)]
    {
        let process_group = -(i64::from(child.id()) as libc::pid_t);
        // SAFETY: the spawned child is placed in a new process group whose ID
        // equals its PID. `kill` receives that bounded process-group ID and a
        // constant signal; no pointers or borrowed memory cross the FFI call.
        let result = unsafe { libc::kill(process_group, libc::SIGKILL) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                append_detail(
                    detail,
                    &format!("failed to kill child process group: {error}"),
                );
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = child.kill() {
        append_detail(detail, &format!("failed to kill child: {error}"));
    }
    match child.wait() {
        Ok(status) => Some(status),
        Err(error) => {
            append_detail(detail, &format!("failed to reap child: {error}"));
            None
        }
    }
}

fn append_detail(detail: &mut String, addition: &str) {
    if !detail.is_empty() {
        detail.push_str("; ");
    }
    detail.push_str(addition);
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionErrorKind, format_command, run_command, run_command_streaming,
        run_command_streaming_structured, run_command_structured,
    };
    use std::fs;
    use std::io::{self, Write};
    use std::path::Path;
    use std::process;
    #[cfg(unix)]
    use std::process::Command;
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::Instant;

    fn test_child(test_name: &str, extra: &[String]) -> (String, Vec<String>) {
        let executable = std::env::current_exe().expect("test executable path");
        let mut args = vec![
            String::from("--ignored"),
            String::from("--exact"),
            format!("exec::tests::{test_name}"),
            String::from("--nocapture"),
        ];
        args.extend_from_slice(extra);
        (executable.to_string_lossy().into_owned(), args)
    }

    #[test]
    fn formats_command_with_and_without_args() {
        assert_eq!(format_command("cargo", &[]), "cargo");
        assert_eq!(
            format_command("cargo", &[String::from("test")]),
            "cargo test"
        );
    }

    #[test]
    fn callback_runs_before_child_exit() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let release = temporary.path().join("callback-received");
        let (program, args) = test_child(
            "fixture_waits_for_callback",
            &[release.to_string_lossy().into_owned()],
        );
        let mut saw_ready = false;
        let output = run_command_streaming_structured(
            Path::new("."),
            &program,
            &args,
            Duration::from_secs(10),
            |line| {
                if line == "callback-ready" {
                    saw_ready = true;
                    fs::write(&release, b"release").expect("release child");
                }
            },
        )
        .expect("child is released by live callback");
        assert!(output.status.success());
        assert!(saw_ready);
    }

    #[test]
    fn timeout_kills_and_classifies_child() {
        let (program, args) = test_child("fixture_never_exits", &[]);
        let error =
            run_command_structured(Path::new("."), &program, &args, Duration::from_millis(100))
                .expect_err("child must time out");
        assert_eq!(error.kind, ExecutionErrorKind::Timeout);
        assert_eq!(error.timeout, Some(Duration::from_millis(100)));
        assert!(
            error.status.is_some_and(|status| !status.success()),
            "killed child must be reaped with an unsuccessful status"
        );
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendants_that_retain_output_pipes() {
        let (program, args) = test_child("fixture_spawns_descendant", &[]);
        let started = Instant::now();
        let error =
            run_command_structured(Path::new("."), &program, &args, Duration::from_millis(100))
                .expect_err("process tree must time out");
        assert_eq!(error.kind, ExecutionErrorKind::Timeout);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "surviving descendant retained an output pipe"
        );
    }

    #[test]
    fn captures_partial_stdout_and_stderr() {
        let (program, args) = test_child("fixture_writes_partial_output", &[]);
        let mut lines = Vec::new();
        let output = run_command_streaming_structured(
            Path::new("."),
            &program,
            &args,
            Duration::from_secs(10),
            |line| lines.push(line.to_string()),
        )
        .expect("fixture runs");
        assert!(output.status.success());
        assert!(
            output
                .stdout
                .windows(b"stdout-partial".len())
                .any(|bytes| bytes == b"stdout-partial")
        );
        assert!(
            output
                .stderr
                .windows(b"stderr-partial".len())
                .any(|bytes| bytes == b"stderr-partial")
        );
        assert!(lines.iter().any(|line| line.ends_with("stdout-partial")));
        assert!(lines.iter().any(|line| line.ends_with("stderr-partial")));
    }

    #[test]
    fn preserves_nonzero_status_and_output() {
        let (program, args) = test_child("fixture_exits_nonzero", &[]);
        let output =
            run_command_structured(Path::new("."), &program, &args, Duration::from_secs(10))
                .expect("non-zero status is normal output");
        assert_eq!(output.status.code(), Some(17));
        assert!(
            output
                .stdout
                .windows(b"nonzero-stdout".len())
                .any(|bytes| bytes == b"nonzero-stdout")
        );
        assert!(
            output
                .stderr
                .windows(b"nonzero-stderr".len())
                .any(|bytes| bytes == b"nonzero-stderr")
        );
    }

    #[test]
    fn compatibility_streaming_api_captures_stdout() {
        let (program, args) = test_child("fixture_writes_partial_output", &[]);
        let mut lines = Vec::new();
        let output = run_command_streaming(
            Path::new("."),
            &program,
            &args,
            Duration::from_secs(10),
            |line| lines.push(line.to_string()),
        )
        .expect("compatibility command runs");
        assert!(output.status.success());
        assert!(lines.iter().any(|line| line.ends_with("stdout-partial")));
    }

    #[test]
    fn reports_missing_program() {
        let error = run_command_structured(
            Path::new("."),
            "ayni-definitely-not-a-real-tool",
            &[],
            Duration::from_secs(1),
        )
        .expect_err("must fail");
        assert_eq!(error.kind, ExecutionErrorKind::Spawn);
        assert!(error.to_string().contains("failed to execute"));

        let compatibility_error = run_command(
            Path::new("."),
            "ayni-definitely-not-a-real-tool",
            &[],
            Duration::from_secs(1),
        )
        .expect_err("compatibility API must fail");
        assert!(compatibility_error.contains("failed to execute"));
    }

    #[test]
    #[ignore]
    fn fixture_waits_for_callback() {
        let release = std::env::args()
            .next_back()
            .expect("release marker argument");
        println!("callback-ready");
        io::stdout().flush().expect("flush fixture stdout");
        while !Path::new(&release).exists() {
            std::thread::yield_now();
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    #[allow(clippy::zombie_processes)]
    fn fixture_spawns_descendant() {
        let (program, args) = test_child("fixture_never_exits", &[]);
        let _descendant = Command::new(program)
            .args(args)
            .spawn()
            .expect("spawn descendant");
        loop {
            std::thread::park();
        }
    }

    #[test]
    #[ignore]
    fn fixture_never_exits() {
        loop {
            std::thread::park();
        }
    }

    #[test]
    #[ignore]
    fn fixture_writes_partial_output() {
        io::stdout()
            .write_all(b"stdout-partial")
            .expect("write fixture stdout");
        io::stdout().flush().expect("flush fixture stdout");
        io::stderr()
            .write_all(b"stderr-partial")
            .expect("write fixture stderr");
        io::stderr().flush().expect("flush fixture stderr");
        process::exit(0);
    }

    #[test]
    #[ignore]
    fn fixture_exits_nonzero() {
        print!("nonzero-stdout");
        eprint!("nonzero-stderr");
        io::stdout().flush().expect("flush fixture stdout");
        io::stderr().flush().expect("flush fixture stderr");
        process::exit(17);
    }
}
