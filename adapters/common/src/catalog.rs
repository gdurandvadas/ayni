//! Catalog execution engine: checks tool status and runs installers.
//!
//! `ayni-core` owns the catalog *contract* (`CatalogEntry`, `Installer`,
//! `VersionCheck`, `ToolStatus`, `CatalogRuntime`); this module owns the
//! process execution behind it, keeping tool invocation out of core.

use crate::exec::{run_command_streaming_structured, run_command_structured};
use crate::failure::{catalog_error_from_execution_error, concise_failure_message};
use ayni_core::{
    CatalogEntry, CatalogOperation, CatalogOperationError, CatalogOperationErrorKind,
    CatalogRuntime, ExecutionResolution, Installer, ToolStatus,
};
use std::path::Path;
use std::time::Duration;

/// Shared runtime for catalog entries whose complete behavior is declarative.
pub struct GenericCatalogRuntime;

pub static GENERIC_CATALOG_RUNTIME: GenericCatalogRuntime = GenericCatalogRuntime;

impl CatalogRuntime for GenericCatalogRuntime {
    fn status(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
    ) -> Result<ToolStatus, CatalogOperationError> {
        if let Installer::AdapterManaged { key, .. } = &entry.installer {
            return Err(CatalogOperationError::contract(
                CatalogOperation::Status,
                format!("adapter-managed catalog key `{key}` was passed to the generic runtime"),
            ));
        }
        if let Some(check) = &entry.check {
            return probe(
                check.command,
                check.args,
                check.contains,
                &execution.install_cwd,
                timeout,
            );
        }
        match &entry.installer {
            Installer::Rustup { component } => {
                let output = run_status(
                    "rustup",
                    &["component", "list", "--installed"],
                    &execution.install_cwd,
                    timeout,
                )?;
                if !output.status.success() {
                    return Ok(ToolStatus::Missing);
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(
                    if rustup_installed_lines_contain_component(&stdout, component) {
                        ToolStatus::Current
                    } else {
                        ToolStatus::Missing
                    },
                )
            }
            Installer::GradleTask { task } => gradle_status(execution, &[*task], timeout),
            Installer::GradleTaskAny { tasks } => gradle_status(execution, tasks, timeout),
            Installer::AdapterManaged { .. } => unreachable!("rejected before status dispatch"),
            Installer::Cargo { .. }
            | Installer::GoInstall { .. }
            | Installer::Bundled
            | Installer::Custom { .. } => Ok(ToolStatus::Missing),
        }
    }

    fn install(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError> {
        let (program, args) = generic_install_command(entry)?;
        let Some((program, args)) = program.zip(args) else {
            return Ok(());
        };
        let output = run_command_streaming_structured(
            &execution.install_cwd,
            program,
            &args,
            timeout,
            |line| on_line(line),
        )
        .map_err(|error| catalog_error_from_execution_error(CatalogOperation::Install, &error))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(CatalogOperationError::new(
                CatalogOperation::Install,
                CatalogOperationErrorKind::NonZeroExit,
                Some(crate::exec::format_command(program, &args)),
                Some(execution.install_cwd.clone()),
                output.status.code(),
                format!(
                    "installer for `{}` exited unsuccessfully: {}",
                    entry.name,
                    concise_failure_message(&output)
                ),
            ))
        }
    }
}

fn probe(
    program: &str,
    args: &[&str],
    contains: Option<&str>,
    cwd: &Path,
    timeout: Duration,
) -> Result<ToolStatus, CatalogOperationError> {
    let output = run_status(program, args, cwd, timeout)?;
    if !output.status.success() {
        return Ok(ToolStatus::Missing);
    }
    Ok(match contains {
        Some(required) if !String::from_utf8_lossy(&output.stdout).contains(required) => {
            ToolStatus::Outdated
        }
        _ => ToolStatus::Current,
    })
}

fn run_status(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout: Duration,
) -> Result<std::process::Output, CatalogOperationError> {
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    run_command_structured(cwd, program, &args, timeout)
        .map_err(|error| catalog_error_from_execution_error(CatalogOperation::Status, &error))
}

fn gradle_status(
    execution: &ExecutionResolution,
    tasks: &[&str],
    timeout: Duration,
) -> Result<ToolStatus, CatalogOperationError> {
    let output = run_status(
        &execution.runner,
        &["tasks", "--all", "--quiet"],
        &execution.install_cwd,
        timeout,
    )?;
    if !output.status.success() {
        return Ok(ToolStatus::Missing);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(
        if tasks
            .iter()
            .any(|task| gradle_task_list_contains(&stdout, task))
        {
            ToolStatus::Current
        } else {
            ToolStatus::Missing
        },
    )
}

type InstallCommand = (Option<&'static str>, Option<Vec<String>>);

fn generic_install_command(entry: &CatalogEntry) -> Result<InstallCommand, CatalogOperationError> {
    let command = match &entry.installer {
        Installer::Bundled | Installer::GradleTask { .. } | Installer::GradleTaskAny { .. } => {
            (None, None)
        }
        Installer::Cargo {
            crate_name,
            version,
        } => {
            let mut args = vec!["install".into(), "--locked".into(), (*crate_name).into()];
            if let Some(version) = version {
                args.extend(["--version".into(), (*version).into()]);
            }
            (Some("cargo"), Some(args))
        }
        Installer::Rustup { component } => (
            Some("rustup"),
            Some(vec!["component".into(), "add".into(), (*component).into()]),
        ),
        Installer::GoInstall { module, version } => (
            Some("go"),
            Some(vec![
                "install".into(),
                format!("{}@{}", module, version.unwrap_or("latest")),
            ]),
        ),
        Installer::Custom { program, args } => (
            Some(*program),
            Some(args.iter().map(|arg| (*arg).to_string()).collect()),
        ),
        Installer::AdapterManaged { key, .. } => {
            return Err(CatalogOperationError::contract(
                CatalogOperation::Install,
                format!("adapter-managed catalog key `{key}` was passed to the generic runtime"),
            ));
        }
    };
    Ok(command)
}

/// Whether `rustup component list --installed` contains this component.
///
/// Catalog entries use names accepted by `rustup component add` (for example
/// `llvm-tools-preview`); the installed list uses shorter names such as
/// `llvm-tools-aarch64-apple-darwin`, so we match on stable prefixes and strip
/// the common `-preview` suffix when needed.
fn rustup_installed_lines_contain_component(list_stdout: &str, component: &str) -> bool {
    let prefixes = rustup_component_list_prefixes(component);
    for line in list_stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        for prefix in &prefixes {
            if line == *prefix || line.starts_with(&format!("{prefix}-")) {
                return true;
            }
        }
    }
    false
}

fn rustup_component_list_prefixes(component: &str) -> Vec<&str> {
    let mut out = vec![component];
    if let Some(base) = component.strip_suffix("-preview") {
        out.push(base);
    }
    out
}

fn gradle_task_list_contains(stdout: &str, task: &str) -> bool {
    let suffix = format!(":{task}");
    stdout.lines().any(|line| {
        let first = line.split_whitespace().next().unwrap_or("");
        first == task || first.ends_with(&suffix)
    })
}

#[cfg(test)]
mod tests {
    use super::{GENERIC_CATALOG_RUNTIME, rustup_installed_lines_contain_component};
    use ayni_core::{
        CatalogEntry, CatalogOperation, CatalogOperationErrorKind, CatalogRuntime,
        ExecutionResolution, Installer, ToolStatus, VersionCheck,
    };
    use std::fs;
    use std::io::{self, Write};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;

    fn test_child(test_name: &str, extra: &[String]) -> (&'static str, &'static [&'static str]) {
        let executable = std::env::current_exe().expect("test executable path");
        let mut args = vec![
            String::from("--ignored"),
            String::from("--exact"),
            format!("catalog::tests::{test_name}"),
            String::from("--nocapture"),
        ];
        args.extend_from_slice(extra);
        let program = Box::leak(executable.to_string_lossy().into_owned().into_boxed_str());
        let args = args
            .into_iter()
            .map(|arg| &*Box::leak(arg.into_boxed_str()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        (program, Box::leak(args))
    }

    fn execution(cwd: &Path) -> ExecutionResolution {
        ExecutionResolution::direct("runner", cwd.to_path_buf(), "test", 100)
    }

    #[test]
    fn status_probe_times_out() {
        let dir = TempDir::new().expect("tempdir");
        let (program, args) = test_child("fixture_never_exits", &[]);
        let entry = CatalogEntry {
            name: "timeout-probe",
            check: Some(VersionCheck {
                command: program,
                args,
                contains: None,
            }),
            installer: Installer::Bundled,
            for_signals: &[],
            opt_in: false,
        };
        let error = GENERIC_CATALOG_RUNTIME
            .status(&entry, &execution(dir.path()), Duration::from_millis(100))
            .expect_err("status probe must time out");
        assert_eq!(error.operation, CatalogOperation::Status);
        assert_eq!(error.kind, CatalogOperationErrorKind::Timeout);
        assert!(error.command.is_some());
        assert_eq!(error.cwd.as_deref(), Some(dir.path()));
    }

    #[test]
    fn installer_streams_and_times_out() {
        let dir = TempDir::new().expect("tempdir");
        let release = dir.path().join("never-release");
        let (program, args) = test_child(
            "fixture_streams_then_waits",
            &[release.to_string_lossy().into_owned()],
        );
        let entry = CatalogEntry {
            name: "streaming-installer",
            check: None,
            installer: Installer::Custom { program, args },
            for_signals: &[],
            opt_in: false,
        };
        let mut lines = Vec::new();
        let error = GENERIC_CATALOG_RUNTIME
            .install(
                &entry,
                &execution(dir.path()),
                Duration::from_millis(100),
                &mut |line| lines.push(line.to_string()),
            )
            .expect_err("installer must time out");
        assert!(lines.iter().any(|line| line == "installer-ready"));
        assert_eq!(error.operation, CatalogOperation::Install);
        assert_eq!(error.kind, CatalogOperationErrorKind::Timeout);
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
    fn fixture_streams_then_waits() {
        println!("installer-ready");
        io::stdout().flush().expect("flush fixture stdout");
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn rustup_list_matches_preview_component_names() {
        let list = "cargo-aarch64-apple-darwin\nllvm-tools-aarch64-apple-darwin\n";
        assert!(rustup_installed_lines_contain_component(
            list,
            "llvm-tools-preview"
        ));
        assert!(!rustup_installed_lines_contain_component(list, "rustc-dev"));
    }

    #[test]
    #[cfg(unix)]
    fn gradle_task_any_accepts_alternative_task_names() {
        let dir = TempDir::new().expect("tempdir");
        let runner = dir.path().join("gradlew");
        fs::write(
            &runner,
            "#!/bin/sh\nprintf '%s\\n' 'jacocoTestReport - Generates coverage report'\n",
        )
        .expect("runner");
        let mut perms = fs::metadata(&runner).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&runner, perms).expect("chmod");

        let entry = CatalogEntry {
            name: "coverage-report",
            check: None,
            installer: Installer::GradleTaskAny {
                tasks: &["koverXmlReport", "jacocoTestReport"],
            },
            for_signals: &[],
            opt_in: false,
        };

        assert_eq!(
            GENERIC_CATALOG_RUNTIME
                .status(
                    &entry,
                    &ExecutionResolution::direct(
                        "./gradlew",
                        dir.path().to_path_buf(),
                        "test",
                        100,
                    ),
                    Duration::from_secs(2)
                )
                .expect("status"),
            ToolStatus::Current
        );
    }
}
