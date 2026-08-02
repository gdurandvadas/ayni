use crate::package_manager::PackageManager;
use ayni_adapters_common::catalog::GENERIC_CATALOG_RUNTIME;
use ayni_adapters_common::exec::{format_command, run_command_streaming_structured};
use ayni_adapters_common::failure::{catalog_error_from_execution_error, concise_failure_message};
use ayni_core::{
    CatalogEntry, CatalogOperation, CatalogOperationError, CatalogOperationErrorKind,
    CatalogRuntime, ExecutionResolution, Installer, SignalKind, ToolStatus, VersionCheck,
};
use std::fs;
use std::path::Path;
use std::time::Duration;

const VITEST: &str = "node.requirement.vitest";
const COVERAGE_V8: &str = "node.requirement.vitest-coverage-v8";
const ESLINT: &str = "node.requirement.eslint";
const STYLISTIC: &str = "node.requirement.stylistic-eslint";
const STRYKER: &str = "node.requirement.stryker";

#[derive(Debug, Clone, Copy)]
struct Requirement {
    package: &'static str,
    version: Option<&'static str>,
    dev: bool,
}

pub(crate) struct NodeCatalogRuntime;
pub(crate) static NODE_CATALOG_RUNTIME: NodeCatalogRuntime = NodeCatalogRuntime;

impl CatalogRuntime for NodeCatalogRuntime {
    fn status(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
    ) -> Result<ToolStatus, CatalogOperationError> {
        match &entry.installer {
            Installer::AdapterManaged { key, .. } => {
                let requirement = requirement(key, CatalogOperation::Status)?;
                Ok(local_requirement_status(
                    &execution.install_cwd,
                    requirement,
                ))
            }
            _ => GENERIC_CATALOG_RUNTIME.status(entry, execution, timeout),
        }
    }

    fn prepare(
        &self,
        execution: &ExecutionResolution,
        timeout: Duration,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError> {
        manager(execution, CatalogOperation::Prepare)?;
        run_manager_command(
            CatalogOperation::Prepare,
            "Node dependency preparation",
            execution,
            vec![String::from("install")],
            timeout,
            on_line,
        )
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
        let requirement = requirement(key, CatalogOperation::Install)?;
        let manager = manager(execution, CatalogOperation::Install)?;
        let target = requirement.version.map_or_else(
            || requirement.package.to_string(),
            |version| format!("{}@{version}", requirement.package),
        );
        run_manager_command(
            CatalogOperation::Install,
            entry.name,
            execution,
            manager.add_dependency_args(&target, requirement.dev),
            timeout,
            on_line,
        )
    }
}

fn requirement(
    key: &str,
    operation: CatalogOperation,
) -> Result<Requirement, CatalogOperationError> {
    match key {
        VITEST => Ok(Requirement {
            package: "vitest",
            version: Some("3.2.4"),
            dev: true,
        }),
        COVERAGE_V8 => Ok(Requirement {
            package: "@vitest/coverage-v8",
            version: Some("3.2.4"),
            dev: true,
        }),
        ESLINT => Ok(Requirement {
            package: "eslint",
            version: None,
            dev: true,
        }),
        STYLISTIC => Ok(Requirement {
            package: "@stylistic/eslint-plugin",
            version: None,
            dev: true,
        }),
        STRYKER => Ok(Requirement {
            package: "@stryker-mutator/core",
            version: None,
            dev: true,
        }),
        _ => Err(CatalogOperationError::contract(
            operation,
            format!("unknown Node adapter-managed catalog key `{key}`"),
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
                "Node execution runner `{}` is not a supported package manager",
                execution.runner
            ),
        )
    })
}

fn run_manager_command(
    operation: CatalogOperation,
    label: &str,
    execution: &ExecutionResolution,
    args: Vec<String>,
    timeout: Duration,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), CatalogOperationError> {
    let program = &execution.runner;
    let output =
        run_command_streaming_structured(&execution.install_cwd, program, &args, timeout, |line| {
            on_line(line)
        })
        .map_err(|error| catalog_error_from_execution_error(operation, &error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CatalogOperationError::new(
            operation,
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

fn local_requirement_status(cwd: &Path, requirement: Requirement) -> ToolStatus {
    let Ok(content) = fs::read_to_string(cwd.join("package.json")) else {
        return ToolStatus::Missing;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) else {
        return ToolStatus::Missing;
    };
    let declared = [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ]
    .iter()
    .find_map(|section| {
        manifest
            .get(*section)
            .and_then(serde_json::Value::as_object)
            .and_then(|dependencies| dependencies.get(requirement.package))
            .and_then(serde_json::Value::as_str)
    });
    let Some(declared) = declared else {
        return ToolStatus::Missing;
    };
    let mut installed = cwd.join("node_modules");
    for component in requirement.package.split('/') {
        installed.push(component);
    }
    if !installed.join("package.json").is_file() {
        return ToolStatus::Missing;
    }
    match requirement.version {
        Some(version) if !declared.contains(version) => ToolStatus::Outdated,
        _ => ToolStatus::Current,
    }
}

/// Ordered declarative catalog. Adapter-managed keys are intentionally opaque
/// outside this crate; all metadata and command interpretation lives above.
pub static NODE_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "node",
        check: Some(VersionCheck {
            command: "node",
            args: &["--version"],
            contains: None,
        }),
        installer: Installer::Bundled,
        for_signals: &[
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Size,
            SignalKind::Complexity,
            SignalKind::Deps,
            SignalKind::Mutation,
        ],
        opt_in: false,
    },
    managed(
        "vitest",
        VITEST,
        "install: add devDependency vitest@3.2.4 via package manager",
        &[SignalKind::Test, SignalKind::Coverage],
        false,
    ),
    managed(
        "@vitest/coverage-v8",
        COVERAGE_V8,
        "install: add devDependency @vitest/coverage-v8@3.2.4 via package manager",
        &[SignalKind::Coverage],
        false,
    ),
    managed(
        "eslint",
        ESLINT,
        "install: add devDependency eslint via package manager",
        &[SignalKind::Complexity],
        false,
    ),
    managed(
        "@stylistic/eslint-plugin",
        STYLISTIC,
        "install: add devDependency @stylistic/eslint-plugin via package manager",
        &[SignalKind::Complexity],
        false,
    ),
    managed(
        "@stryker-mutator/core",
        STRYKER,
        "install: add devDependency @stryker-mutator/core via package manager",
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
    use super::{
        NODE_CATALOG, NODE_CATALOG_RUNTIME, Requirement, local_requirement_status, requirement,
    };
    use crate::package_manager::PackageManager;
    use ayni_core::{
        CatalogOperation, CatalogOperationErrorKind, CatalogRuntime, ExecutionResolution,
        ToolStatus,
    };
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    #[cfg(unix)]
    fn manager_install_and_status_matrix() {
        let dir = TempDir::new().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("workspace");
        let entry = NODE_CATALOG
            .iter()
            .find(|entry| entry.name == "vitest")
            .expect("vitest entry");

        for (manager, expected) in [
            (
                PackageManager::Npm,
                vec!["install", "--save-dev", "vitest@3.2.4"],
            ),
            (PackageManager::Pnpm, vec!["add", "-D", "vitest@3.2.4"]),
            (PackageManager::Yarn, vec!["add", "--dev", "vitest@3.2.4"]),
            (PackageManager::Bun, vec!["add", "-d", "vitest@3.2.4"]),
        ] {
            assert_eq!(manager.add_dependency_args("vitest@3.2.4", true), expected);
            let runner = dir.path().join(manager.executable());
            fs::write(
                &runner,
                "#!/bin/sh\nprintf 'progress:%s\\n' \"$*\"\nprintf '%s|%s\\n' \"$PWD\" \"$*\" >> \"$0.log\"\n",
            )
            .expect("fake manager");
            let mut permissions = fs::metadata(&runner).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&runner, permissions).expect("executable manager");
            let execution = ExecutionResolution::direct(
                runner.to_string_lossy(),
                workspace.clone(),
                "test",
                100,
            );
            let mut progress = Vec::new();
            NODE_CATALOG_RUNTIME
                .prepare(&execution, Duration::from_secs(2), &mut |line| {
                    progress.push(line.to_string());
                })
                .expect("preparation");
            NODE_CATALOG_RUNTIME
                .install(entry, &execution, Duration::from_secs(2), &mut |line| {
                    progress.push(line.to_string())
                })
                .expect("install");
            assert!(progress.iter().any(|line| line == "progress:install"));
            assert!(
                progress
                    .iter()
                    .any(|line| line == &format!("progress:{}", expected.join(" ")))
            );
            let log = fs::read_to_string(format!("{}.log", runner.display())).expect("log");
            let lines = log.lines().collect::<Vec<_>>();
            let canonical_workspace = workspace.canonicalize().expect("canonical workspace");
            assert_eq!(
                lines[0],
                format!("{}|install", canonical_workspace.display())
            );
            assert_eq!(
                lines[1],
                format!("{}|{}", canonical_workspace.display(), expected.join(" "))
            );
        }

        let local_requirement = Requirement {
            package: "vitest",
            version: Some("3.2.4"),
            dev: true,
        };
        fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vitest":"^3.2.4"}}"#,
        )
        .expect("manifest");
        assert_eq!(
            local_requirement_status(dir.path(), local_requirement),
            ToolStatus::Missing
        );
        fs::create_dir_all(dir.path().join("node_modules/vitest")).expect("module dir");
        fs::write(dir.path().join("node_modules/vitest/package.json"), "{}")
            .expect("module manifest");
        assert_eq!(
            local_requirement_status(dir.path(), local_requirement),
            ToolStatus::Current
        );
        fs::write(
            dir.path().join("package.json"),
            r#"{"dependencies":{"vitest":"^2.0.0"}}"#,
        )
        .expect("manifest");
        assert_eq!(
            local_requirement_status(dir.path(), local_requirement),
            ToolStatus::Outdated
        );

        let error = requirement("node.requirement.unknown", CatalogOperation::Status)
            .expect_err("unknown key");
        assert_eq!(error.kind, CatalogOperationErrorKind::Contract);
    }
}
