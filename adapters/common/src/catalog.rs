//! Catalog execution engine: checks tool status and runs installers.
//!
//! `ayni-core` owns the catalog *contract* (`CatalogEntry`, `Installer`,
//! `VersionCheck`, `ToolStatus`, `InstallContext`); this module owns the
//! process execution behind it, keeping tool invocation out of core.

use ayni_core::{CatalogEntry, InstallContext, Installer, PythonPackageManager, ToolStatus};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Determines whether a catalog tool is missing, outdated, or current in the
/// given install context.
pub fn tool_status(entry: &CatalogEntry, ctx: InstallContext<'_>) -> ToolStatus {
    let Some(check) = &entry.check else {
        return match entry.installer {
            Installer::NodePackage {
                package, version, ..
            } => node_package_status(ctx.cwd, package, version),
            Installer::Rustup { component } => rustup_component_status(component),
            Installer::PythonPackage {
                import_name,
                version,
                ..
            } => python_package_status(ctx, import_name, version),
            Installer::UvTool { package, version } => uv_tool_status(package, version),
            Installer::GradleTask { task } => gradle_task_status(ctx, task),
            Installer::GradleTaskAny { tasks } => gradle_task_any_status(ctx, tasks),
            Installer::PythonRuntime => python_runtime_status(),
            _ => ToolStatus::Missing,
        };
    };
    let mut command = Command::new(check.command);
    command
        .args(check.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(cwd) = ctx.cwd {
        command.current_dir(cwd);
    }
    let Ok(output) = command.output() else {
        return ToolStatus::Missing;
    };

    if !output.status.success() {
        return ToolStatus::Missing;
    }

    let Some(required) = check.contains else {
        return ToolStatus::Current;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains(required) {
        ToolStatus::Current
    } else {
        ToolStatus::Outdated
    }
}

/// Installs a catalog tool with its declared installer.
pub fn install_tool(entry: &CatalogEntry, ctx: InstallContext<'_>) -> Result<(), String> {
    install_with(&entry.installer, entry.name, ctx)
}

fn install_with(
    installer: &Installer,
    tool_name: &str,
    ctx: InstallContext<'_>,
) -> Result<(), String> {
    match installer {
        Installer::Bundled | Installer::PythonRuntime => Ok(()),
        Installer::Cargo {
            crate_name,
            version,
        } => install_cargo(crate_name, *version, tool_name),
        Installer::Rustup { component } => {
            run_cmd("rustup", &["component", "add", component], tool_name)
        }
        Installer::GoInstall { module, version } => install_go(module, *version, tool_name),
        Installer::NpmGlobal { package, version } => {
            install_npm_global(package, *version, tool_name)
        }
        Installer::NodePackage {
            package,
            version,
            dev,
        } => install_node_package(ctx, package, *version, *dev, tool_name),
        Installer::PythonPackage {
            package,
            version,
            dev,
            ..
        } => install_python_package(ctx, package, *version, *dev, tool_name),
        Installer::UvTool { package, version } => install_uv_tool(package, *version, tool_name),
        Installer::GradleTask { .. } | Installer::GradleTaskAny { .. } => Ok(()),
        Installer::Custom { program, args } => run_cmd_in(program, args, tool_name, ctx.cwd),
    }
}

fn install_cargo(crate_name: &str, version: Option<&str>, tool_name: &str) -> Result<(), String> {
    let mut args = vec!["install", "--locked", crate_name];
    if let Some(version) = version {
        args.push("--version");
        args.push(version);
    }
    run_cmd("cargo", &args, tool_name)
}

fn install_go(module: &str, version: Option<&str>, tool_name: &str) -> Result<(), String> {
    let target = format!("{}@{}", module, version.unwrap_or("latest"));
    run_cmd("go", &["install", target.as_str()], tool_name)
}

fn install_npm_global(package: &str, version: Option<&str>, tool_name: &str) -> Result<(), String> {
    let target = version.map_or_else(
        || package.to_owned(),
        |version| format!("{package}@{version}"),
    );
    run_cmd("npm", &["install", "-g", target.as_str()], tool_name)
}

fn install_node_package(
    ctx: InstallContext<'_>,
    package: &str,
    version: Option<&str>,
    dev: bool,
    tool_name: &str,
) -> Result<(), String> {
    let cwd = ctx
        .cwd
        .ok_or_else(|| format!("missing install root for local node package {tool_name}"))?;
    let manager = ctx
        .node_package_manager
        .ok_or_else(|| format!("missing package manager for local node package {tool_name}"))?;
    let target = version.map_or_else(
        || package.to_string(),
        |version| format!("{package}@{version}"),
    );
    let args = manager.add_dependency_args(&target, dev);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_cmd_in(manager.executable(), &arg_refs, tool_name, Some(cwd))
}

fn install_python_package(
    ctx: InstallContext<'_>,
    package: &str,
    version: Option<&str>,
    dev: bool,
    tool_name: &str,
) -> Result<(), String> {
    let cwd = ctx
        .cwd
        .ok_or_else(|| format!("missing install root for local python package {tool_name}"))?;
    let manager = ctx
        .python_package_manager
        .unwrap_or(PythonPackageManager::Pip);
    let target = version.map_or_else(
        || package.to_string(),
        |version| format!("{package}=={version}"),
    );
    let args = manager.add_dependency_args(&target, dev);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_cmd_in(manager.executable(), &arg_refs, tool_name, Some(cwd))
}

fn install_uv_tool(package: &str, version: Option<&str>, tool_name: &str) -> Result<(), String> {
    let target = version.map_or_else(
        || package.to_string(),
        |version| format!("{package}=={version}"),
    );
    run_cmd(
        "uv",
        &["tool", "install", "--force", "--upgrade", target.as_str()],
        tool_name,
    )
}

fn run_cmd(program: &str, args: &[&str], tool_name: &str) -> Result<(), String> {
    run_cmd_in(program, args, tool_name, None)
}

fn run_cmd_in(
    program: &str,
    args: &[&str],
    tool_name: &str,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let status = command
        .status()
        .map_err(|error| format!("failed to run {program} for {tool_name}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} failed for {tool_name} (exit {})",
            status.code().unwrap_or(-1)
        ))
    }
}

/// Whether `rustup component list --installed` contains this component.
///
/// Catalog entries use names accepted by `rustup component add` (for example
/// `llvm-tools-preview`); the installed list uses shorter names such as
/// `llvm-tools-aarch64-apple-darwin`, so we match on stable prefixes and strip
/// the common `-preview` suffix when needed.
fn rustup_component_status(component: &str) -> ToolStatus {
    let mut command = Command::new("rustup");
    command
        .args(["component", "list", "--installed"])
        .stdout(std::process::Stdio::piped());
    let Ok(output) = command.output() else {
        return ToolStatus::Missing;
    };
    if !output.status.success() {
        return ToolStatus::Missing;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if rustup_installed_lines_contain_component(&stdout, component) {
        ToolStatus::Current
    } else {
        ToolStatus::Missing
    }
}

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

fn python_runtime_status() -> ToolStatus {
    for program in ["python3", "python"] {
        let Ok(output) = Command::new(program)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
        else {
            continue;
        };
        if output.status.success() {
            return ToolStatus::Current;
        }
    }
    ToolStatus::Missing
}

fn python_package_status(
    ctx: InstallContext<'_>,
    import_name: &str,
    _version: Option<&str>,
) -> ToolStatus {
    let manager = ctx
        .python_package_manager
        .unwrap_or(PythonPackageManager::Pip);
    let script = format!(
        "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec('{import_name}') else 1)"
    );
    let (program, args) = if manager == PythonPackageManager::Pip {
        (String::from("python"), vec![String::from("-c"), script])
    } else {
        let script_ref = script.as_str();
        manager.run_command("python", &["-c", script_ref])
    };
    let mut command = Command::new(program);
    command
        .args(args.iter().map(String::as_str))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(cwd) = ctx.cwd {
        command.current_dir(cwd);
    }
    let Ok(output) = command.output() else {
        return ToolStatus::Missing;
    };
    if output.status.success() {
        ToolStatus::Current
    } else {
        ToolStatus::Missing
    }
}

fn uv_tool_status(package: &str, version: Option<&str>) -> ToolStatus {
    let output = Command::new("uv")
        .args(["tool", "list"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let Ok(output) = output else {
        return ToolStatus::Missing;
    };
    if !output.status.success() {
        return ToolStatus::Missing;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some(package))
    else {
        return ToolStatus::Missing;
    };
    if let Some(required) = version
        && !line.contains(required)
    {
        return ToolStatus::Outdated;
    }
    if uv_tool_command_runs(package) {
        ToolStatus::Current
    } else {
        ToolStatus::Missing
    }
}

fn uv_tool_command_runs(package: &str) -> bool {
    Command::new("uv")
        .args(["tool", "run", package, "--help"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn gradle_task_status(ctx: InstallContext<'_>, task: &str) -> ToolStatus {
    gradle_task_any_status(ctx, &[task])
}

fn gradle_task_any_status(ctx: InstallContext<'_>, tasks: &[&str]) -> ToolStatus {
    let Some(cwd) = ctx.cwd else {
        return ToolStatus::Missing;
    };
    let runner = ctx.gradle_runner.unwrap_or("gradle");
    let output = Command::new(runner)
        .args(["tasks", "--all", "--quiet"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let Ok(output) = output else {
        return ToolStatus::Missing;
    };
    if !output.status.success() {
        return ToolStatus::Missing;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if tasks
        .iter()
        .any(|task| gradle_task_list_contains(&stdout, task))
    {
        ToolStatus::Current
    } else {
        ToolStatus::Missing
    }
}

fn gradle_task_list_contains(stdout: &str, task: &str) -> bool {
    let suffix = format!(":{task}");
    stdout.lines().any(|line| {
        let first = line.split_whitespace().next().unwrap_or("");
        first == task || first.ends_with(&suffix)
    })
}

fn node_package_status(cwd: Option<&Path>, package: &str, version: Option<&str>) -> ToolStatus {
    let Some(cwd) = cwd else {
        return ToolStatus::Missing;
    };
    let manifest_path = cwd.join("package.json");
    let Ok(content) = fs::read_to_string(&manifest_path) else {
        return ToolStatus::Missing;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return ToolStatus::Missing;
    };
    let found = [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ]
    .iter()
    .find_map(|section| {
        value
            .get(*section)
            .and_then(serde_json::Value::as_object)
            .and_then(|deps| deps.get(package))
            .and_then(serde_json::Value::as_str)
    });
    let Some(found) = found else {
        return ToolStatus::Missing;
    };
    if !node_dependency_installed(cwd, package) {
        return ToolStatus::Missing;
    }
    match version {
        Some(required) if !found.contains(required) => ToolStatus::Outdated,
        _ => ToolStatus::Current,
    }
}

fn node_dependency_installed(cwd: &Path, package: &str) -> bool {
    let mut path = cwd.join("node_modules");
    for part in package.split('/') {
        path.push(part);
    }
    path.join("package.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::{node_package_status, rustup_installed_lines_contain_component, tool_status};
    use ayni_core::{CatalogEntry, InstallContext, Installer, ToolStatus};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

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
    fn node_package_status_requires_installed_dependency() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"fixture","devDependencies":{"vitest":"^2.1.8"}}"#,
        )
        .expect("package json");
        assert_eq!(
            node_package_status(Some(dir.path()), "vitest", None),
            ToolStatus::Missing
        );

        fs::create_dir_all(dir.path().join("node_modules").join("vitest")).expect("node_modules");
        fs::write(
            dir.path()
                .join("node_modules")
                .join("vitest")
                .join("package.json"),
            r#"{"name":"vitest","version":"2.1.8"}"#,
        )
        .expect("vitest package");
        assert_eq!(
            node_package_status(Some(dir.path()), "vitest", Some("2.1.8")),
            ToolStatus::Current
        );
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
            tool_status(
                &entry,
                InstallContext {
                    cwd: Some(dir.path()),
                    node_package_manager: None,
                    python_package_manager: None,
                    gradle_runner: Some("./gradlew"),
                }
            ),
            ToolStatus::Current
        );
    }
}
