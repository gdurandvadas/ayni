//! Tool invocation with wall-clock timeouts.
//!
//! Every adapter command goes through this module so a hung tool (a stuck
//! Gradle daemon, a wedged test run) can never block an analyze run forever.

use ayni_core::RunContext;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How often the runner polls a child process for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Fallback timeout for invocations that have no `RunContext` (and therefore
/// no policy) available. Matches the `execution.tool_timeout_seconds` default.
pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(1800);

/// Formats a program and args for diagnostics (`cargo test --workspace`).
pub fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

/// Runs a command in `workdir`, capturing stdout/stderr, killing it after `timeout`.
pub fn run_command(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<Output, String> {
    run_command_streaming(workdir, program, args, timeout, |_| {})
}

/// Like [`run_command`], but invokes `on_line` for every stdout and stderr
/// line as it arrives, so callers can surface live progress.
pub fn run_command_streaming(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    mut on_line: impl FnMut(&str),
) -> Result<Output, String> {
    let mut command = Command::new(program);
    command
        .args(args.iter().map(String::as_str))
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to execute {program}: {error}"))?;

    let stdout_rx = spawn_reader(child.stdout.take());
    let stderr_rx = spawn_reader(child.stderr.take());

    let status = wait_with_timeout(&mut child, timeout).map_err(|()| {
        format!(
            "command timed out after {}s: {}",
            timeout.as_secs(),
            format_command(program, args)
        )
    })?;

    let stdout = drain_lines(stdout_rx, &mut on_line);
    let stderr = drain_lines(stderr_rx, &mut on_line);
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Runs a command in the context's execution cwd with the policy timeout and
/// debug diagnostics. This is the standard entry point for collectors.
pub fn run_command_for_context(
    context: &RunContext,
    program: &str,
    args: &[String],
) -> Result<Output, String> {
    run_command_for_context_streaming(context, program, args, |_| {})
}

/// Streaming variant of [`run_command_for_context`].
pub fn run_command_for_context_streaming(
    context: &RunContext,
    program: &str,
    args: &[String],
    on_line: impl FnMut(&str),
) -> Result<Output, String> {
    let timeout = context_timeout(context);
    let output =
        run_command_streaming(&context.execution.exec_cwd, program, args, timeout, on_line)?;
    if context.debug {
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
    Ok(output)
}

/// The wall-clock timeout configured for tool invocations in this run.
pub fn context_timeout(context: &RunContext) -> Duration {
    Duration::from_secs(context.policy.execution.tool_timeout_seconds)
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, ()> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(());
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
        }
    }
}

fn spawn_reader(stream: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let Some(mut stream) = stream else {
            return;
        };
        let mut buffer = [0u8; 8192];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if sender.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

fn drain_lines(receiver: mpsc::Receiver<Vec<u8>>, mut on_line: impl FnMut(&str)) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut line_start = 0;
    while let Ok(chunk) = receiver.recv() {
        collected.extend_from_slice(&chunk);
        while let Some(offset) = collected[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let line_end = line_start + offset;
            let line = String::from_utf8_lossy(&collected[line_start..line_end]);
            on_line(line.trim_end_matches('\r'));
            line_start = line_end + 1;
        }
    }
    if line_start < collected.len() {
        let line = String::from_utf8_lossy(&collected[line_start..]);
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if !trimmed.is_empty() {
            on_line(trimmed);
        }
    }
    collected
}

#[cfg(test)]
mod tests {
    use super::{format_command, run_command, run_command_streaming};
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn formats_command_with_and_without_args() {
        assert_eq!(format_command("cargo", &[]), "cargo");
        assert_eq!(
            format_command("cargo", &[String::from("test")]),
            "cargo test"
        );
    }

    #[test]
    fn captures_stdout_and_streams_lines() {
        let mut lines = Vec::new();
        let output = run_command_streaming(
            Path::new("."),
            "sh",
            &[String::from("-c"), String::from("echo one; echo two")],
            Duration::from_secs(10),
            |line| lines.push(line.to_string()),
        )
        .expect("command runs");
        assert!(output.status.success());
        assert_eq!(lines, vec!["one", "two"]);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "one\ntwo\n");
    }

    #[test]
    fn kills_command_after_timeout() {
        let error = run_command(
            Path::new("."),
            "sh",
            &[String::from("-c"), String::from("sleep 30")],
            Duration::from_millis(200),
        )
        .expect_err("must time out");
        assert!(error.contains("timed out"), "unexpected error: {error}");
    }

    #[test]
    fn reports_missing_program() {
        let error = run_command(
            Path::new("."),
            "ayni-definitely-not-a-real-tool",
            &[],
            Duration::from_secs(1),
        )
        .expect_err("must fail");
        assert!(error.contains("failed to execute"));
    }
}
