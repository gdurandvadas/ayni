//! Tool invocation with concurrent output capture and wall-clock timeouts.
//!
//! Every adapter command goes through this module so a hung tool (a stuck
//! Gradle daemon, a wedged test run) can never block an analyze run forever.

use ayni_core::{CancellationToken, RunContext};
use std::collections::{BTreeMap, VecDeque};
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
#[cfg(not(test))]
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

/// Maximum retained bytes for each child stream. Commands that exceed this
/// bound are terminated because collectors cannot safely parse partial output.
const STREAM_CAPTURE_LIMIT: usize = 16 * 1024 * 1024;

/// Bounds reader-to-runner buffering while still allowing both pipes to drain
/// concurrently. A full queue applies backpressure to the child process.
const OUTPUT_EVENT_BUFFER: usize = 16;

/// Limit one eager drain pass so a command that writes continuously cannot
/// starve the timeout and cancellation checks in the outer runner loop.
const OUTPUT_EVENT_BATCH: usize = OUTPUT_EVENT_BUFFER;

/// A single progress line is diagnostic UI state, not canonical command
/// output. Bound it separately so a newline-free stream cannot create another
/// large allocation while the retained command output remains bounded.
const STREAM_LINE_LIMIT: usize = 64 * 1024;

/// Fallback timeout for invocations that have no `RunContext` (and therefore
/// no policy) available. Matches the `execution.tool_timeout_seconds` default.
pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(1800);

/// Internal marker used by adapter process-fixture tests so instrumented child
/// test binaries do not write partial LLVM profiles when they are terminated.
#[doc(hidden)]
pub const DISCARD_LLVM_PROFILE_ENV: &str = "AYNI_INTERNAL_DISCARD_LLVM_PROFILE";

/// Stable classification for failures owned by the command runner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionErrorKind {
    /// The operating system could not create the child process.
    Spawn,
    /// Waiting for the child or reading one of its pipes failed.
    Wait,
    /// The child exceeded its wall-clock limit and was killed and reaped.
    Timeout,
    /// Orchestration requested cancellation and the process group was stopped.
    Cancelled,
    /// A stream exceeded the bounded capture limit and parsing would be partial.
    OutputLimit,
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
    /// Bytes observed but not retained after stdout reached its capture limit.
    pub stdout_truncated_bytes: u64,
    /// Bytes observed but not retained after stderr reached its capture limit.
    pub stderr_truncated_bytes: u64,
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
            ExecutionErrorKind::Cancelled => {
                write!(formatter, "command cancelled: {}", self.command)
            }
            ExecutionErrorKind::OutputLimit => write!(
                formatter,
                "command output exceeded the {} byte per-stream capture limit: {}",
                STREAM_CAPTURE_LIMIT, self.command
            ),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Result returned by structured command-runner entry points.
pub type ExecutionResult = Result<Output, Box<ExecutionError>>;

/// Successful infrastructure output retained as a rolling tail.
///
/// Unlike collector execution, infrastructure streaming does not fail merely
/// because verbose build logs exceed the capture ceiling. Callers still receive
/// explicit truncation counts for diagnostics.
#[derive(Debug)]
pub struct TruncatedOutput {
    pub output: Output,
    pub stdout_truncated_bytes: u64,
    pub stderr_truncated_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CapturePolicy {
    FailClosed,
    RetainTail,
}

struct CommandSettings<'a> {
    environment: &'a BTreeMap<String, String>,
    cancellation: Option<&'a CancellationToken>,
    capture_policy: CapturePolicy,
}

struct CapturedOutput {
    output: Output,
    stdout_truncated_bytes: u64,
    stderr_truncated_bytes: u64,
}

struct CapturedStreams {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated_bytes: u64,
    stderr_truncated_bytes: u64,
}

enum CommandCompletion {
    Exited(ExitStatus),
    Failed(ExecutionErrorKind, String),
}

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
    on_line: impl FnMut(&str),
) -> ExecutionResult {
    run_command_streaming_structured_with_environment(
        workdir,
        program,
        args,
        timeout,
        CommandSettings {
            environment: &BTreeMap::new(),
            cancellation: None,
            capture_policy: CapturePolicy::FailClosed,
        },
        on_line,
    )
    .map(|captured| captured.output)
}

/// Runs a command with cooperative cancellation and fail-closed output capture.
pub fn run_command_structured_cancellable(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    cancellation: &CancellationToken,
) -> ExecutionResult {
    run_command_streaming_structured_with_environment(
        workdir,
        program,
        args,
        timeout,
        CommandSettings {
            environment: &BTreeMap::new(),
            cancellation: Some(cancellation),
            capture_policy: CapturePolicy::FailClosed,
        },
        |_| {},
    )
    .map(|captured| captured.output)
}

/// Streams infrastructure logs while retaining only the most recent bounded
/// stdout/stderr tails. Capture truncation is reported but does not terminate
/// an otherwise healthy build command.
pub fn run_command_streaming_truncated(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    on_line: impl FnMut(&str),
) -> Result<TruncatedOutput, String> {
    run_command_streaming_structured_with_environment(
        workdir,
        program,
        args,
        timeout,
        CommandSettings {
            environment: &BTreeMap::new(),
            cancellation: None,
            capture_policy: CapturePolicy::RetainTail,
        },
        on_line,
    )
    .map(|captured| TruncatedOutput {
        output: captured.output,
        stdout_truncated_bytes: captured.stdout_truncated_bytes,
        stderr_truncated_bytes: captured.stderr_truncated_bytes,
    })
    .map_err(|error| error.to_string())
}

fn run_command_streaming_structured_with_environment(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    settings: CommandSettings<'_>,
    mut on_line: impl FnMut(&str),
) -> Result<CapturedOutput, Box<ExecutionError>> {
    let command_text = format_command(program, args);
    let mut child = spawn_command(workdir, program, args, &settings, &command_text)?;
    let (sender, receiver) = mpsc::sync_channel(OUTPUT_EVENT_BUFFER);
    spawn_reader(Stream::Stdout, child.stdout.take(), sender.clone());
    spawn_reader(Stream::Stderr, child.stderr.take(), sender);
    let mut capture = Capture::default();

    let status = match wait_for_command(
        &mut child,
        &receiver,
        &mut capture,
        timeout,
        &settings,
        &mut on_line,
    ) {
        CommandCompletion::Exited(status) => status,
        CommandCompletion::Failed(kind, mut detail) => {
            let cleanup_status = terminate_and_reap(&mut child, &mut detail);
            let mut failure = Some((kind, detail));
            capture.drain_to_end(
                &receiver,
                settings.capture_policy,
                &mut on_line,
                &mut failure,
                OUTPUT_DRAIN_TIMEOUT,
            );
            let (kind, detail) = failure.expect("cleanup preserves the primary failure");
            let captured = capture.into_streams();
            return Err(Box::new(ExecutionError {
                kind,
                command: command_text,
                cwd: workdir.to_path_buf(),
                status: cleanup_status,
                stdout: captured.stdout,
                stderr: captured.stderr,
                stdout_truncated_bytes: captured.stdout_truncated_bytes,
                stderr_truncated_bytes: captured.stderr_truncated_bytes,
                timeout: (kind == ExecutionErrorKind::Timeout).then_some(timeout),
                detail,
            }));
        }
    };

    let mut drain_failure = None;
    capture.drain_to_end(
        &receiver,
        settings.capture_policy,
        &mut on_line,
        &mut drain_failure,
        OUTPUT_DRAIN_TIMEOUT,
    );
    if let Some((kind, mut detail)) = drain_failure {
        let cleanup_status = terminate_after_exit(&mut child, &mut detail).or(Some(status));
        let mut failure = Some((kind, detail));
        capture.drain_to_end(
            &receiver,
            settings.capture_policy,
            &mut on_line,
            &mut failure,
            OUTPUT_DRAIN_TIMEOUT,
        );
        let (kind, detail) = failure.expect("post-exit cleanup preserves the primary failure");
        let captured = capture.into_streams();
        return Err(Box::new(ExecutionError {
            kind,
            command: command_text,
            cwd: workdir.to_path_buf(),
            status: cleanup_status,
            stdout: captured.stdout,
            stderr: captured.stderr,
            stdout_truncated_bytes: captured.stdout_truncated_bytes,
            stderr_truncated_bytes: captured.stderr_truncated_bytes,
            timeout: None,
            detail,
        }));
    }
    let captured = capture.into_streams();
    Ok(CapturedOutput {
        output: Output {
            status,
            stdout: captured.stdout,
            stderr: captured.stderr,
        },
        stdout_truncated_bytes: captured.stdout_truncated_bytes,
        stderr_truncated_bytes: captured.stderr_truncated_bytes,
    })
}

fn spawn_command(
    workdir: &Path,
    program: &str,
    args: &[String],
    settings: &CommandSettings<'_>,
    command_text: &str,
) -> Result<Child, Box<ExecutionError>> {
    if settings
        .cancellation
        .is_some_and(CancellationToken::is_cancelled)
    {
        return Err(Box::new(ExecutionError {
            kind: ExecutionErrorKind::Cancelled,
            command: command_text.to_owned(),
            cwd: workdir.to_path_buf(),
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated_bytes: 0,
            stderr_truncated_bytes: 0,
            timeout: None,
            detail: String::from("cancellation requested before process start"),
        }));
    }
    #[cfg(windows)]
    let resolved_program = resolve_windows_program(program, workdir, settings.environment);
    #[cfg(windows)]
    let mut command = Command::new(
        resolved_program
            .as_deref()
            .unwrap_or_else(|| Path::new(program)),
    );
    #[cfg(not(windows))]
    let mut command = Command::new(program);
    command
        .args(args.iter().map(String::as_str))
        .envs(settings.environment)
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(test)]
    configure_test_profile_discard(&mut command);
    if settings.environment.contains_key(DISCARD_LLVM_PROFILE_ENV) {
        configure_profile_discard(&mut command);
        command.env_remove(DISCARD_LLVM_PROFILE_ENV);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map_err(|error| {
        Box::new(ExecutionError {
            kind: ExecutionErrorKind::Spawn,
            command: command_text.to_owned(),
            cwd: workdir.to_path_buf(),
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated_bytes: 0,
            stderr_truncated_bytes: 0,
            timeout: None,
            detail: error.to_string(),
        })
    })
}

#[cfg(windows)]
fn resolve_windows_program(
    program: &str,
    workdir: &Path,
    environment: &BTreeMap<String, String>,
) -> Option<PathBuf> {
    let program_path = Path::new(program);
    let path_value = environment
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| std::ffi::OsString::from(value))
        .or_else(|| std::env::var_os("PATH"));
    let path_ext = environment
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATHEXT"))
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var("PATHEXT").ok())
        .unwrap_or_else(|| String::from(".COM;.EXE;.BAT;.CMD"));
    let extensions = path_ext
        .split(';')
        .filter(|extension| !extension.is_empty())
        .collect::<Vec<_>>();

    let explicit_path =
        program_path.is_absolute() || program.contains('/') || program.contains('\\');
    let directories = if explicit_path {
        vec![if program_path.is_absolute() {
            PathBuf::new()
        } else {
            workdir.to_path_buf()
        }]
    } else {
        std::env::split_paths(&path_value?).collect::<Vec<_>>()
    };
    for directory in directories {
        let directory = if directory.as_os_str().is_empty() || directory.is_relative() {
            workdir.join(directory)
        } else {
            directory
        };
        let candidate = directory.join(program_path);
        if candidate.extension().is_some() && candidate.is_file() {
            return Some(candidate);
        }
        if candidate.extension().is_none()
            && let (Some(parent), Some(name)) = (candidate.parent(), candidate.file_name())
        {
            let name = name.to_string_lossy();
            if let Some(candidate) = extensions
                .iter()
                .map(|extension| parent.join(format!("{name}{extension}")))
                .find(|candidate| candidate.is_file())
            {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
fn configure_test_profile_discard(command: &mut Command) {
    configure_profile_discard(command);
}

fn configure_profile_discard(command: &mut Command) {
    // Fixture binaries are process plumbing rather than coverage evidence. They
    // and every descendant inherit the platform null device, so forced
    // termination cannot leave a corrupt or checkout-local profile.
    #[cfg(unix)]
    command.env("LLVM_PROFILE_FILE", "/dev/null");
    #[cfg(windows)]
    command.env("LLVM_PROFILE_FILE", "NUL");
}

fn wait_for_command(
    child: &mut Child,
    receiver: &mpsc::Receiver<ReaderEvent>,
    capture: &mut Capture,
    timeout: Duration,
    settings: &CommandSettings<'_>,
    on_line: &mut impl FnMut(&str),
) -> CommandCompletion {
    let started = Instant::now();
    loop {
        let mut execution_failure = None;
        drain_available(
            receiver,
            capture,
            settings.capture_policy,
            on_line,
            &mut execution_failure,
        );
        if let Some((kind, detail)) = execution_failure {
            return CommandCompletion::Failed(kind, detail);
        }
        if settings
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return CommandCompletion::Failed(
                ExecutionErrorKind::Cancelled,
                String::from("cancellation requested by orchestrator"),
            );
        }
        match child.try_wait() {
            Ok(Some(exit_status)) => return CommandCompletion::Exited(exit_status),
            Ok(None) if started.elapsed() >= timeout => {
                return CommandCompletion::Failed(ExecutionErrorKind::Timeout, String::new());
            }
            Ok(None) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                let wait = POLL_INTERVAL.min(remaining);
                if let Ok(event) = receiver.recv_timeout(wait) {
                    capture.handle(
                        event,
                        settings.capture_policy,
                        on_line,
                        &mut execution_failure,
                    );
                    if let Some((kind, detail)) = execution_failure {
                        return CommandCompletion::Failed(kind, detail);
                    }
                }
            }
            Err(error) => {
                return CommandCompletion::Failed(ExecutionErrorKind::Wait, error.to_string());
            }
        }
    }
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
    let started = Instant::now();
    let result = run_command_streaming_structured_with_environment(
        &context.execution.exec_cwd,
        program,
        args,
        context_timeout(context),
        CommandSettings {
            environment: &context.execution.environment,
            cancellation: Some(&context.cancellation),
            capture_policy: CapturePolicy::FailClosed,
        },
        on_line,
    );
    if context.debug {
        let status = match &result {
            Ok(captured) => captured
                .output
                .status
                .code()
                .map_or_else(|| String::from("signal"), |code| code.to_string()),
            Err(error) => format!("runner_error:{:?}", error.kind),
        };
        eprintln!(
            "[profile] command={} elapsed_ms={} status={status}",
            format_command(program, args),
            started.elapsed().as_millis(),
        );
    }
    let output = result?.output;
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
    sender: mpsc::SyncSender<ReaderEvent>,
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
    bytes: VecDeque<u8>,
    line: LineCapture,
    done: bool,
    truncated_bytes: u64,
}

impl StreamCapture {
    fn push(
        &mut self,
        chunk: &[u8],
        policy: CapturePolicy,
        on_line: &mut impl FnMut(&str),
    ) -> bool {
        self.line.push(chunk, on_line);
        let was_truncated = self.truncated_bytes > 0;
        let truncated = match policy {
            CapturePolicy::FailClosed => {
                let remaining = STREAM_CAPTURE_LIMIT.saturating_sub(self.bytes.len());
                let retained = remaining.min(chunk.len());
                self.bytes.extend(&chunk[..retained]);
                (chunk.len() - retained) as u64
            }
            CapturePolicy::RetainTail => retain_tail(&mut self.bytes, chunk, STREAM_CAPTURE_LIMIT),
        };
        self.truncated_bytes = self.truncated_bytes.saturating_add(truncated);
        policy == CapturePolicy::FailClosed && !was_truncated && self.truncated_bytes > 0
    }

    fn finish(&mut self, on_line: &mut impl FnMut(&str)) {
        if !self.done {
            self.line.finish(on_line);
            self.done = true;
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes.into_iter().collect()
    }
}

#[derive(Default)]
struct LineCapture {
    bytes: VecDeque<u8>,
    truncated_bytes: u64,
}

impl LineCapture {
    fn push(&mut self, mut chunk: &[u8], on_line: &mut impl FnMut(&str)) {
        while let Some(line_end) = chunk.iter().position(|byte| *byte == b'\n') {
            self.append(&chunk[..line_end]);
            self.emit(true, on_line);
            chunk = &chunk[line_end + 1..];
        }
        self.append(chunk);
    }

    fn append(&mut self, bytes: &[u8]) {
        self.truncated_bytes = self.truncated_bytes.saturating_add(retain_tail(
            &mut self.bytes,
            bytes,
            STREAM_LINE_LIMIT,
        ));
    }

    fn finish(&mut self, on_line: &mut impl FnMut(&str)) {
        if !self.bytes.is_empty() || self.truncated_bytes > 0 {
            self.emit(false, on_line);
        }
    }

    fn emit(&mut self, complete: bool, on_line: &mut impl FnMut(&str)) {
        let bytes = self.bytes.drain(..).collect::<Vec<_>>();
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim_end_matches('\r');
        if self.truncated_bytes > 0 {
            let rendered = format!(
                "[... {} line bytes omitted ...] {line}",
                self.truncated_bytes
            );
            on_line(&rendered);
        } else if complete || !line.is_empty() {
            on_line(line);
        }
        self.truncated_bytes = 0;
    }
}

fn retain_tail(buffer: &mut VecDeque<u8>, chunk: &[u8], limit: usize) -> u64 {
    if chunk.len() >= limit {
        let truncated = buffer
            .len()
            .saturating_add(chunk.len().saturating_sub(limit));
        buffer.clear();
        buffer.extend(&chunk[chunk.len() - limit..]);
        return truncated as u64;
    }
    let overflow = buffer
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(limit);
    if overflow > 0 {
        buffer.drain(..overflow);
    }
    buffer.extend(chunk);
    overflow as u64
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
        policy: CapturePolicy,
        on_line: &mut impl FnMut(&str),
        failure: &mut Option<(ExecutionErrorKind, String)>,
    ) {
        match event {
            ReaderEvent::Data(stream, bytes) => {
                let truncated = self.stream_mut(stream).push(&bytes, policy, on_line);
                if truncated {
                    record_failure(
                        failure,
                        ExecutionErrorKind::OutputLimit,
                        format!(
                            "{} exceeded the {} byte capture limit",
                            stream.name(),
                            STREAM_CAPTURE_LIMIT
                        ),
                    );
                }
            }
            ReaderEvent::Done(stream, error) => {
                self.stream_mut(stream).finish(on_line);
                if let Some(error) = error {
                    record_failure(
                        failure,
                        ExecutionErrorKind::Wait,
                        format!("failed to read {}: {error}", stream.name()),
                    );
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
        policy: CapturePolicy,
        on_line: &mut impl FnMut(&str),
        failure: &mut Option<(ExecutionErrorKind, String)>,
        timeout: Duration,
    ) {
        let started = Instant::now();
        while !self.stdout.done || !self.stderr.done {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                record_failure(
                    failure,
                    ExecutionErrorKind::Wait,
                    String::from("timed out while draining child output pipes"),
                );
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok(event) => {
                    self.handle(event, policy, on_line, failure);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    record_failure(
                        failure,
                        ExecutionErrorKind::Wait,
                        String::from("timed out while draining child output pipes"),
                    );
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    record_failure(
                        failure,
                        ExecutionErrorKind::Wait,
                        String::from("output readers disconnected"),
                    );
                    break;
                }
            }
        }
    }

    fn into_streams(self) -> CapturedStreams {
        CapturedStreams {
            stdout_truncated_bytes: self.stdout.truncated_bytes,
            stderr_truncated_bytes: self.stderr.truncated_bytes,
            stdout: self.stdout.into_bytes(),
            stderr: self.stderr.into_bytes(),
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
    policy: CapturePolicy,
    on_line: &mut impl FnMut(&str),
    failure: &mut Option<(ExecutionErrorKind, String)>,
) {
    for _ in 0..OUTPUT_EVENT_BATCH {
        let Ok(event) = receiver.try_recv() else {
            break;
        };
        capture.handle(event, policy, on_line, failure);
        if failure.is_some() {
            break;
        }
    }
}

fn record_failure(
    failure: &mut Option<(ExecutionErrorKind, String)>,
    kind: ExecutionErrorKind,
    detail: String,
) {
    if let Some((first_kind, first_detail)) = failure {
        if failure_priority(kind) > failure_priority(*first_kind) {
            *first_kind = kind;
        }
        append_detail(first_detail, &detail);
    } else {
        *failure = Some((kind, detail));
    }
}

fn failure_priority(kind: ExecutionErrorKind) -> u8 {
    match kind {
        // The operation-level cause remains authoritative while cleanup drains
        // any already-buffered child output.
        ExecutionErrorKind::Timeout | ExecutionErrorKind::Cancelled => 3,
        // Partial collector output must never be mislabeled as a generic pipe
        // wait merely because the other stream reported its error first.
        ExecutionErrorKind::OutputLimit => 2,
        ExecutionErrorKind::Spawn | ExecutionErrorKind::Wait => 1,
    }
}

fn terminate_and_reap(child: &mut Child, detail: &mut String) -> Option<ExitStatus> {
    #[cfg(unix)]
    terminate_process_group(child.id(), detail);
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

fn terminate_after_exit(child: &mut Child, detail: &mut String) -> Option<ExitStatus> {
    #[cfg(unix)]
    terminate_process_group(child.id(), detail);
    child.wait().map_or_else(
        |error| {
            append_detail(detail, &format!("failed to confirm child exit: {error}"));
            None
        },
        Some,
    )
}

#[cfg(unix)]
fn terminate_process_group(child_id: u32, detail: &mut String) {
    let process_group = -(i64::from(child_id) as libc::pid_t);
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

fn append_detail(detail: &mut String, addition: &str) {
    if !detail.is_empty() {
        detail.push_str("; ");
    }
    detail.push_str(addition);
}

#[cfg(test)]
mod tests {
    use super::{
        Capture, CapturePolicy, ExecutionErrorKind, ReaderEvent, STREAM_CAPTURE_LIMIT, Stream,
        format_command, run_command, run_command_for_context_streaming_structured,
        run_command_for_context_structured, run_command_streaming,
        run_command_streaming_structured, run_command_streaming_truncated, run_command_structured,
    };
    use ayni_core::{AyniPolicy, CancellationToken, ExecutionResolution, RunContext, Scope};
    use std::fs;
    use std::io::{self, Write};
    use std::path::Path;
    use std::process;
    #[cfg(unix)]
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

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
    fn context_environment_is_applied_only_to_the_child() {
        const NAME: &str = "AYNI_TEST_CONTEXT_ENVIRONMENT";
        let cwd = std::env::current_dir().expect("current directory");
        let mut execution = ExecutionResolution::direct("runner", cwd.clone(), "test", 100);
        execution
            .environment
            .insert(NAME.to_owned(), "target-value".to_owned());
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd,
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution,
            cancellation: Default::default(),
            debug: false,
        };
        let (program, args) = test_child("fixture_prints_context_environment", &[]);
        let output = run_command_for_context_structured(&context, &program, &args)
            .expect("context command runs");
        assert!(String::from_utf8_lossy(&output.stdout).contains("target-value"));
        assert!(std::env::var_os(NAME).is_none());
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

    #[test]
    fn output_limit_terminates_the_child_and_reports_truncation() {
        let (program, args) = test_child("fixture_exceeds_capture_limit", &[]);
        let error =
            run_command_structured(Path::new("."), &program, &args, Duration::from_secs(10))
                .expect_err("oversized output must fail closed");
        assert_eq!(error.kind, ExecutionErrorKind::OutputLimit);
        assert_eq!(error.stdout.len(), STREAM_CAPTURE_LIMIT);
        assert!(error.stdout_truncated_bytes > 0);
        assert_eq!(error.stderr_truncated_bytes, 0);
    }

    #[test]
    fn infrastructure_streaming_retains_a_tail_without_failing() {
        let (program, args) = test_child("fixture_exceeds_capture_limit_with_tail", &[]);
        let mut lines = Vec::new();
        let captured = run_command_streaming_truncated(
            Path::new("."),
            &program,
            &args,
            Duration::from_secs(10),
            |line| lines.push(line.to_owned()),
        )
        .expect("verbose infrastructure output is truncated, not failed");
        assert!(captured.output.status.success());
        assert_eq!(captured.output.stdout.len(), STREAM_CAPTURE_LIMIT);
        assert!(captured.stdout_truncated_bytes > 0);
        assert!(captured.output.stdout.ends_with(b"tail-marker"));
        assert!(
            lines
                .last()
                .is_some_and(|line| line.ends_with("tail-marker"))
        );
    }

    #[test]
    fn final_drain_preserves_output_limit_as_the_first_failure() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(ReaderEvent::Done(
                Stream::Stderr,
                Some(String::from("earlier read error")),
            ))
            .expect("stderr done");
        sender
            .send(ReaderEvent::Data(
                Stream::Stdout,
                vec![b'x'; STREAM_CAPTURE_LIMIT + 1],
            ))
            .expect("data event");
        sender
            .send(ReaderEvent::Done(Stream::Stdout, None))
            .expect("stdout done");
        drop(sender);

        let mut capture = Capture::default();
        let mut failure = None;
        capture.drain_to_end(
            &receiver,
            CapturePolicy::FailClosed,
            &mut |_| {},
            &mut failure,
            Duration::from_secs(1),
        );
        let (kind, detail) = failure.expect("drain must fail");
        assert_eq!(kind, ExecutionErrorKind::OutputLimit);
        assert!(detail.contains("earlier read error"));
    }

    #[test]
    fn cancellation_terminates_an_active_child() {
        let cwd = std::env::current_dir().expect("current directory");
        let cancellation = CancellationToken::default();
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("runner", cwd, "test", 100),
            cancellation: cancellation.clone(),
            debug: false,
        };
        let (program, args) = test_child("fixture_never_exits", &[]);
        let handle =
            thread::spawn(move || run_command_for_context_structured(&context, &program, &args));
        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        cancellation.cancel();
        let error = handle
            .join()
            .expect("runner thread joins")
            .expect_err("cancelled child must fail");
        assert_eq!(error.kind, ExecutionErrorKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(error.status.is_some_and(|status| !status.success()));
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

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_descendants_that_retain_output_pipes() {
        let cwd = std::env::current_dir().expect("current directory");
        let cancellation = CancellationToken::default();
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("runner", cwd, "test", 100),
            cancellation: cancellation.clone(),
            debug: false,
        };
        let (program, args) = test_child("fixture_spawns_descendant", &[]);
        let started = Instant::now();
        let error =
            run_command_for_context_streaming_structured(&context, &program, &args, |line| {
                if line.ends_with("descendant-ready") {
                    cancellation.cancel();
                }
            })
            .expect_err("process tree must be cancelled");
        assert_eq!(error.kind, ExecutionErrorKind::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "surviving descendant retained an output pipe"
        );
    }

    #[cfg(unix)]
    #[test]
    fn post_exit_drain_failure_kills_remaining_process_group() {
        let (program, args) = test_child("fixture_exits_with_descendant", &[]);
        let started = Instant::now();
        let error =
            run_command_structured(Path::new("."), &program, &args, Duration::from_secs(10))
                .expect_err("a descendant retained the output pipe");
        assert_eq!(error.kind, ExecutionErrorKind::Wait);
        assert!(started.elapsed() < Duration::from_secs(2));

        let stdout = String::from_utf8_lossy(&error.stdout);
        let marker = "descendant-pid=";
        let start = stdout.find(marker).expect("descendant pid is reported") + marker.len();
        let pid = stdout[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<libc::pid_t>()
            .expect("valid descendant pid");
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if !process_is_running(pid) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant process survived cleanup"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn process_is_running(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 performs a read-only existence check for the
        // bounded PID emitted by the test child.
        let exists = unsafe { libc::kill(pid, 0) } == 0;
        if !exists && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return false;
        }
        #[cfg(target_os = "linux")]
        if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            // A container without an init reaper can retain a killed orphan as
            // a zombie until the container exits. It no longer executes or
            // owns the output pipe, so it satisfies the cleanup guarantee.
            if stat
                .rsplit_once(") ")
                .and_then(|(_, fields)| fields.chars().next())
                == Some('Z')
            {
                return false;
            }
        }
        exists
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
        println!("descendant-ready");
        io::stdout().flush().expect("flush readiness");
        loop {
            std::thread::park();
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    #[allow(clippy::zombie_processes)]
    fn fixture_exits_with_descendant() {
        let (program, args) = test_child("fixture_never_exits", &[]);
        let descendant = Command::new(program)
            .args(args)
            .spawn()
            .expect("spawn descendant");
        println!("descendant-pid={}", descendant.id());
        io::stdout().flush().expect("flush descendant pid");
        process::exit(0);
    }

    #[test]
    #[ignore]
    fn fixture_prints_context_environment() {
        print!(
            "{}",
            std::env::var("AYNI_TEST_CONTEXT_ENVIRONMENT").unwrap_or_default()
        );
        io::stdout().flush().expect("flush fixture stdout");
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
    fn fixture_exceeds_capture_limit() {
        let chunk = [b'x'; 8192];
        let mut stdout = io::stdout().lock();
        for _ in 0..=(STREAM_CAPTURE_LIMIT / chunk.len()) {
            stdout.write_all(&chunk).expect("write oversized output");
        }
        stdout.flush().expect("flush oversized output");
        process::exit(0);
    }

    #[test]
    #[ignore]
    fn fixture_exceeds_capture_limit_with_tail() {
        let chunk = [b'x'; 8192];
        let mut stdout = io::stdout().lock();
        for _ in 0..=(STREAM_CAPTURE_LIMIT / chunk.len()) {
            stdout.write_all(&chunk).expect("write oversized output");
        }
        stdout.write_all(b"tail-marker").expect("write tail marker");
        stdout.flush().expect("flush oversized output");
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
