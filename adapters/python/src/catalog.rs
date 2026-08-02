use crate::package_manager::PackageManager;
use ayni_adapters_common::catalog::GENERIC_CATALOG_RUNTIME;
use ayni_adapters_common::exec::{
    format_command, run_command_streaming_structured, run_command_structured,
};
use ayni_adapters_common::failure::{catalog_error_from_execution_error, concise_failure_message};
use ayni_core::{
    CatalogEntry, CatalogOperation, CatalogOperationError, CatalogOperationErrorKind,
    CatalogRuntime, ExecutionResolution, Installer, SignalKind, ToolStatus,
};
use std::process::Output;
use std::time::Duration;

const RUNTIME: &str = "python.runtime";
const PYTEST: &str = "python.requirement.pytest";
const PYTEST_JSON: &str = "python.requirement.pytest-json-report";
const PYTEST_COV: &str = "python.requirement.pytest-cov";
const COVERAGE: &str = "python.requirement.coverage";
const COMPLEXIPY: &str = "python.uv-tool.complexipy";
const MUTMUT: &str = "python.requirement.mutmut";
const RUNTIME_PROGRAMS: [&str; 2] = ["python3", "python"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Requirement {
    Runtime,
    Package {
        package: &'static str,
        import_name: &'static str,
        version: Option<&'static str>,
        dev: bool,
    },
    UvTool {
        package: &'static str,
        version: Option<&'static str>,
    },
}

pub(crate) struct PythonCatalogRuntime;
pub(crate) static PYTHON_CATALOG_RUNTIME: PythonCatalogRuntime = PythonCatalogRuntime;

impl CatalogRuntime for PythonCatalogRuntime {
    fn status(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
    ) -> Result<ToolStatus, CatalogOperationError> {
        let Installer::AdapterManaged { key, .. } = &entry.installer else {
            return GENERIC_CATALOG_RUNTIME.status(entry, execution, timeout);
        };
        match requirement(key, CatalogOperation::Status)? {
            Requirement::Runtime => runtime_status(execution, timeout),
            Requirement::Package { import_name, .. } => {
                import_status(execution, import_name, timeout)
            }
            Requirement::UvTool { package, version } => {
                uv_tool_status(execution, package, version, timeout)
            }
        }
    }

    fn install(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError> {
        let Installer::AdapterManaged { key, .. } = &entry.installer else {
            return GENERIC_CATALOG_RUNTIME.install(entry, execution, timeout, on_line);
        };
        match requirement(key, CatalogOperation::Install)? {
            Requirement::Runtime => Ok(()),
            Requirement::Package {
                package,
                version,
                dev,
                ..
            } => {
                let manager = manager(execution, CatalogOperation::Install)?;
                let target = version.map_or_else(
                    || package.to_string(),
                    |version| format!("{package}=={version}"),
                );
                run_install(
                    entry.name,
                    execution,
                    &execution.runner,
                    manager.add_dependency_args(&target, dev),
                    timeout,
                    on_line,
                )
            }
            Requirement::UvTool { package, version } => {
                let target = version.map_or_else(
                    || package.to_string(),
                    |version| format!("{package}=={version}"),
                );
                run_install(
                    entry.name,
                    execution,
                    uv_program(execution),
                    vec![
                        String::from("tool"),
                        String::from("install"),
                        String::from("--force"),
                        String::from("--upgrade"),
                        target,
                    ],
                    timeout,
                    on_line,
                )
            }
        }
    }
}

fn requirement(
    key: &str,
    operation: CatalogOperation,
) -> Result<Requirement, CatalogOperationError> {
    let package = |package, import_name| Requirement::Package {
        package,
        import_name,
        version: None,
        dev: true,
    };
    match key {
        RUNTIME => Ok(Requirement::Runtime),
        PYTEST => Ok(package("pytest", "pytest")),
        PYTEST_JSON => Ok(package("pytest-json-report", "pytest_jsonreport")),
        PYTEST_COV => Ok(package("pytest-cov", "pytest_cov")),
        COVERAGE => Ok(package("coverage", "coverage")),
        COMPLEXIPY => Ok(Requirement::UvTool {
            package: "complexipy",
            version: None,
        }),
        MUTMUT => Ok(package("mutmut", "mutmut")),
        _ => Err(CatalogOperationError::contract(
            operation,
            format!("unknown Python adapter-managed catalog key `{key}`"),
        )),
    }
}

fn manager(
    execution: &ExecutionResolution,
    operation: CatalogOperation,
) -> Result<PackageManager, CatalogOperationError> {
    PackageManager::from_runner(&execution.runner).ok_or_else(|| {
        CatalogOperationError::contract(
            operation,
            format!(
                "Python execution runner `{}` is not a supported package manager",
                execution.runner
            ),
        )
    })
}

fn runtime_status(
    execution: &ExecutionResolution,
    timeout: Duration,
) -> Result<ToolStatus, CatalogOperationError> {
    let mut last_spawn_error = None;
    for program in RUNTIME_PROGRAMS {
        match status_command(execution, program, &["--version"], timeout) {
            Ok(output) if output.status.success() => return Ok(ToolStatus::Current),
            Ok(_) => {}
            Err(error) if error.kind == CatalogOperationErrorKind::Spawn => {
                last_spawn_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(error) = last_spawn_error {
        Err(error)
    } else {
        Ok(ToolStatus::Missing)
    }
}

fn import_status(
    execution: &ExecutionResolution,
    import_name: &str,
    timeout: Duration,
) -> Result<ToolStatus, CatalogOperationError> {
    let manager = manager(execution, CatalogOperation::Status)?;
    let script = format!(
        "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec('{import_name}') else 1)"
    );
    let (program, args) = if manager == PackageManager::Pip {
        (execution.runner.clone(), vec![String::from("-c"), script])
    } else {
        let (_, args) = manager.run_command("python", &["-c", &script]);
        (execution.runner.clone(), args)
    };
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = status_command(execution, &program, &refs, timeout)?;
    Ok(if output.status.success() {
        ToolStatus::Current
    } else {
        ToolStatus::Missing
    })
}

fn uv_tool_status(
    execution: &ExecutionResolution,
    package: &str,
    version: Option<&str>,
    timeout: Duration,
) -> Result<ToolStatus, CatalogOperationError> {
    let program = uv_program(execution);
    let output = status_command(execution, program, &["tool", "list"], timeout)?;
    if !output.status.success() {
        return Ok(ToolStatus::Missing);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some(package))
    else {
        return Ok(ToolStatus::Missing);
    };
    if version.is_some_and(|version| !line.contains(version)) {
        return Ok(ToolStatus::Outdated);
    }
    let output = status_command(
        execution,
        program,
        &["tool", "run", package, "--help"],
        timeout,
    )?;
    Ok(if output.status.success() {
        ToolStatus::Current
    } else {
        ToolStatus::Missing
    })
}

fn uv_program(execution: &ExecutionResolution) -> &str {
    if PackageManager::from_runner(&execution.runner) == Some(PackageManager::Uv) {
        &execution.runner
    } else {
        "uv"
    }
}

fn status_command(
    execution: &ExecutionResolution,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, CatalogOperationError> {
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    run_command_structured(&execution.install_cwd, program, &args, timeout)
        .map_err(|error| catalog_error_from_execution_error(CatalogOperation::Status, &error))
}

fn run_install(
    label: &str,
    execution: &ExecutionResolution,
    program: &str,
    args: Vec<String>,
    timeout: Duration,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), CatalogOperationError> {
    let output =
        run_command_streaming_structured(&execution.install_cwd, program, &args, timeout, |line| {
            on_line(line)
        })
        .map_err(|error| catalog_error_from_execution_error(CatalogOperation::Install, &error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CatalogOperationError::new(
            CatalogOperation::Install,
            CatalogOperationErrorKind::NonZeroExit,
            Some(format_command(program, &args)),
            Some(execution.install_cwd.clone()),
            output.status.code(),
            format!(
                "{label} exited unsuccessfully: {}",
                concise_failure_message(&output)
            ),
        ))
    }
}

pub static PYTHON_CATALOG: &[CatalogEntry] = &[
    managed(
        "python",
        RUNTIME,
        "install: (python runtime on PATH)",
        &[
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Size,
            SignalKind::Complexity,
            SignalKind::Deps,
            SignalKind::Mutation,
        ],
        false,
    ),
    managed(
        "pytest",
        PYTEST,
        "install: add Python devDependency pytest via package manager",
        &[SignalKind::Test, SignalKind::Coverage],
        false,
    ),
    managed(
        "pytest-json-report",
        PYTEST_JSON,
        "install: add Python devDependency pytest-json-report via package manager",
        &[SignalKind::Test],
        false,
    ),
    managed(
        "pytest-cov",
        PYTEST_COV,
        "install: add Python devDependency pytest-cov via package manager",
        &[SignalKind::Coverage],
        false,
    ),
    managed(
        "coverage",
        COVERAGE,
        "install: add Python devDependency coverage via package manager",
        &[SignalKind::Coverage],
        false,
    ),
    managed(
        "complexipy",
        COMPLEXIPY,
        "install: uv tool install complexipy",
        &[SignalKind::Complexity],
        false,
    ),
    managed(
        "mutmut",
        MUTMUT,
        "install: add Python devDependency mutmut via package manager",
        &[SignalKind::Mutation],
        true,
    ),
];

const fn managed(
    name: &'static str,
    key: &'static str,
    summary: &'static str,
    for_signals: &'static [SignalKind],
    opt_in: bool,
) -> CatalogEntry {
    CatalogEntry {
        name,
        check: None,
        installer: Installer::AdapterManaged { key, summary },
        for_signals,
        opt_in,
    }
}

#[cfg(test)]
mod tests {
    use super::{PYTHON_CATALOG, PYTHON_CATALOG_RUNTIME, RUNTIME_PROGRAMS, requirement};
    use crate::package_manager::PackageManager;
    use ayni_core::{
        CatalogOperation, CatalogOperationErrorKind, CatalogRuntime, ExecutionResolution,
    };
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    #[cfg(unix)]
    fn manager_install_and_status_matrix() {
        assert_eq!(RUNTIME_PROGRAMS, ["python3", "python"]);
        let dir = TempDir::new().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("workspace");
        let entry = PYTHON_CATALOG
            .iter()
            .find(|entry| entry.name == "pytest")
            .expect("pytest");
        for (manager, expected) in [
            (PackageManager::Uv, vec!["add", "--dev", "pytest"]),
            (
                PackageManager::Poetry,
                vec!["add", "--group", "dev", "pytest"],
            ),
            (PackageManager::Pdm, vec!["add", "--dev", "pytest"]),
            (PackageManager::Pipenv, vec!["install", "--dev", "pytest"]),
            (
                PackageManager::Hatch,
                vec!["-m", "pip", "install", "pytest"],
            ),
            (PackageManager::Pip, vec!["-m", "pip", "install", "pytest"]),
        ] {
            let runner = dir.path().join(manager.executable());
            fs::write(&runner, "#!/bin/sh\nprintf 'progress:%s\\n' \"$*\"\nprintf '%s|%s\\n' \"$PWD\" \"$*\" >> \"$0.log\"\nexit 0\n").expect("fake runner");
            let mut permissions = fs::metadata(&runner).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&runner, permissions).expect("executable");
            let execution = ExecutionResolution::direct(
                runner.to_string_lossy(),
                workspace.clone(),
                "test",
                100,
            );
            assert_eq!(
                PYTHON_CATALOG_RUNTIME
                    .status(entry, &execution, Duration::from_secs(2))
                    .expect("status"),
                ayni_core::ToolStatus::Current
            );
            let mut progress = Vec::new();
            PYTHON_CATALOG_RUNTIME
                .install(entry, &execution, Duration::from_secs(2), &mut |line| {
                    progress.push(line.to_string())
                })
                .expect("install");
            assert!(
                progress
                    .iter()
                    .any(|line| line == &format!("progress:{}", expected.join(" ")))
            );
            let log = fs::read_to_string(format!("{}.log", runner.display())).expect("log");
            assert!(log.lines().all(|line| {
                line.starts_with(
                    &workspace
                        .canonicalize()
                        .expect("canonical cwd")
                        .to_string_lossy()
                        .into_owned(),
                )
            }));
        }

        let uv_runner = dir.path().join("uv");
        fs::write(
            &uv_runner,
            "#!/bin/sh\nif [ \"$1 $2\" = \"tool list\" ]; then printf 'complexipy 4.0.0\\n'; fi\nprintf 'progress:%s\\n' \"$*\"\nexit 0\n",
        )
        .expect("fake uv");
        let mut permissions = fs::metadata(&uv_runner).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&uv_runner, permissions).expect("executable uv");
        let uv_execution =
            ExecutionResolution::direct(uv_runner.to_string_lossy(), workspace, "test", 100);
        let complexipy = PYTHON_CATALOG
            .iter()
            .find(|entry| entry.name == "complexipy")
            .expect("complexipy");
        assert_eq!(
            PYTHON_CATALOG_RUNTIME
                .status(complexipy, &uv_execution, Duration::from_secs(2))
                .expect("uv tool status"),
            ayni_core::ToolStatus::Current
        );
        let mut progress = Vec::new();
        PYTHON_CATALOG_RUNTIME
            .install(
                complexipy,
                &uv_execution,
                Duration::from_secs(2),
                &mut |line| progress.push(line.to_string()),
            )
            .expect("uv tool install");
        assert!(
            progress
                .iter()
                .any(|line| { line == "progress:tool install --force --upgrade complexipy" })
        );
        let error =
            requirement("python.unknown", CatalogOperation::Status).expect_err("unknown key");
        assert_eq!(error.kind, CatalogOperationErrorKind::Contract);
    }
}
