use crate::signal::SignalKind;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Missing,
    Outdated,
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodePackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl NodePackageManager {
    #[must_use]
    pub fn executable(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    #[must_use]
    pub fn exec_command(self, tool: &str, args: &[&str]) -> (String, Vec<String>) {
        match self {
            Self::Npm => {
                let mut command =
                    vec![String::from("exec"), String::from("--"), String::from(tool)];
                command.extend(args.iter().map(|value| (*value).to_string()));
                (String::from("npm"), command)
            }
            Self::Pnpm => {
                let mut command = vec![String::from("exec"), String::from(tool)];
                command.extend(args.iter().map(|value| (*value).to_string()));
                (String::from("pnpm"), command)
            }
            Self::Yarn => {
                let mut command = vec![String::from("exec"), String::from(tool)];
                command.extend(args.iter().map(|value| (*value).to_string()));
                (String::from("yarn"), command)
            }
            Self::Bun => {
                let mut command = vec![String::from("x"), String::from(tool)];
                command.extend(args.iter().map(|value| (*value).to_string()));
                (String::from("bun"), command)
            }
        }
    }

    #[must_use]
    pub fn add_dependency_args(self, package: &str, dev: bool) -> Vec<String> {
        match self {
            Self::Npm => {
                let mut args = vec![String::from("install")];
                if dev {
                    args.push(String::from("--save-dev"));
                }
                args.push(String::from(package));
                args
            }
            Self::Pnpm => {
                let mut args = vec![String::from("add")];
                if dev {
                    args.push(String::from("-D"));
                }
                args.push(String::from(package));
                args
            }
            Self::Yarn => {
                let mut args = vec![String::from("add")];
                if dev {
                    args.push(String::from("--dev"));
                }
                args.push(String::from(package));
                args
            }
            Self::Bun => {
                let mut args = vec![String::from("add")];
                if dev {
                    args.push(String::from("-d"));
                }
                args.push(String::from(package));
                args
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstallContext<'a> {
    pub cwd: Option<&'a Path>,
    pub node_package_manager: Option<NodePackageManager>,
}

#[must_use]
pub fn detect_node_package_manager(root: &Path) -> Option<NodePackageManager> {
    if root.join("pnpm-lock.yaml").is_file() {
        return Some(NodePackageManager::Pnpm);
    }
    if root.join("yarn.lock").is_file() {
        return Some(NodePackageManager::Yarn);
    }
    if root.join("package-lock.json").is_file() {
        return Some(NodePackageManager::Npm);
    }
    if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
        return Some(NodePackageManager::Bun);
    }
    parse_package_manager_from_manifest(&root.join("package.json"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionCheck {
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub contains: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installer {
    Cargo {
        crate_name: &'static str,
        version: Option<&'static str>,
    },
    Rustup {
        component: &'static str,
    },
    GoInstall {
        module: &'static str,
        version: Option<&'static str>,
    },
    NpmGlobal {
        package: &'static str,
        version: Option<&'static str>,
    },
    NodePackage {
        package: &'static str,
        version: Option<&'static str>,
        dev: bool,
    },
    Bundled,
    Custom {
        program: &'static str,
        args: &'static [&'static str],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub check: Option<VersionCheck>,
    pub installer: Installer,
    pub for_signals: &'static [SignalKind],
    pub opt_in: bool,
}

impl CatalogEntry {
    pub fn status(&self) -> ToolStatus {
        self.status_in(InstallContext::default())
    }

    pub fn status_in(&self, ctx: InstallContext<'_>) -> ToolStatus {
        let Some(check) = &self.check else {
            return match self.installer {
                Installer::NodePackage {
                    package, version, ..
                } => node_package_status(ctx.cwd, package, version),
                Installer::Rustup { component } => rustup_component_status(component),
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

    pub fn install(&self) -> Result<(), String> {
        self.install_in(InstallContext::default())
    }

    pub fn install_in(&self, ctx: InstallContext<'_>) -> Result<(), String> {
        match self.installer {
            Installer::Bundled => Ok(()),
            Installer::Cargo {
                crate_name,
                version,
            } => {
                let mut args = vec!["install", "--locked", crate_name];
                if let Some(version) = version {
                    args.push("--version");
                    args.push(version);
                }
                run_cmd("cargo", &args, self.name)
            }
            Installer::Rustup { component } => {
                run_cmd("rustup", &["component", "add", component], self.name)
            }
            Installer::GoInstall { module, version } => {
                let target = if let Some(version) = version {
                    format!("{module}@{version}")
                } else {
                    format!("{module}@latest")
                };
                run_cmd("go", &["install", target.as_str()], self.name)
            }
            Installer::NpmGlobal { package, version } => {
                let target = if let Some(version) = version {
                    format!("{package}@{version}")
                } else {
                    package.to_owned()
                };
                run_cmd("npm", &["install", "-g", target.as_str()], self.name)
            }
            Installer::NodePackage {
                package,
                version,
                dev,
            } => {
                let cwd = ctx.cwd.ok_or_else(|| {
                    format!("missing install root for local node package {}", self.name)
                })?;
                let manager = ctx.node_package_manager.ok_or_else(|| {
                    format!(
                        "missing package manager for local node package {}",
                        self.name
                    )
                })?;
                let target = if let Some(version) = version {
                    format!("{package}@{version}")
                } else {
                    package.to_string()
                };
                let args = manager.add_dependency_args(&target, dev);
                let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
                run_cmd_in(manager.executable(), &arg_refs, self.name, Some(cwd))
            }
            Installer::Custom { program, args } => run_cmd_in(program, args, self.name, ctx.cwd),
        }
    }
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

fn node_package_status(cwd: Option<&Path>, package: &str, version: Option<&str>) -> ToolStatus {
    let Some(cwd) = cwd else {
        return ToolStatus::Missing;
    };
    let manifest_path = cwd.join("package.json");
    let Ok(content) = fs::read_to_string(&manifest_path) else {
        return ToolStatus::Missing;
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(&content) else {
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
            .and_then(JsonValue::as_object)
            .and_then(|deps| deps.get(package))
            .and_then(JsonValue::as_str)
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

fn parse_package_manager_from_manifest(manifest_path: &Path) -> Option<NodePackageManager> {
    let content = fs::read_to_string(manifest_path).ok()?;
    let value = serde_json::from_str::<JsonValue>(&content).ok()?;
    let raw = value.get("packageManager")?.as_str()?.to_ascii_lowercase();
    if raw.starts_with("pnpm@") {
        Some(NodePackageManager::Pnpm)
    } else if raw.starts_with("yarn@") {
        Some(NodePackageManager::Yarn)
    } else if raw.starts_with("bun@") {
        Some(NodePackageManager::Bun)
    } else if raw.starts_with("npm@") {
        Some(NodePackageManager::Npm)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NodePackageManager, detect_node_package_manager, node_package_status,
        rustup_installed_lines_contain_component,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn detects_manager_from_lockfile_precedence() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("package.json"), "{}").expect("package json");
        fs::write(dir.path().join("yarn.lock"), "").expect("yarn lock");
        assert_eq!(
            detect_node_package_manager(dir.path()),
            Some(NodePackageManager::Yarn)
        );

        fs::write(dir.path().join("pnpm-lock.yaml"), "").expect("pnpm lock");
        assert_eq!(
            detect_node_package_manager(dir.path()),
            Some(NodePackageManager::Pnpm)
        );
    }

    #[test]
    fn falls_back_to_package_manager_field_without_lockfile() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"fixture","packageManager":"bun@1.1.0"}"#,
        )
        .expect("package json");
        assert_eq!(
            detect_node_package_manager(dir.path()),
            Some(NodePackageManager::Bun)
        );
    }

    #[test]
    fn detects_bun_from_text_lockfile() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("package.json"), "{}").expect("package json");
        fs::write(dir.path().join("bun.lock"), "").expect("bun.lock");
        assert_eq!(
            detect_node_package_manager(dir.path()),
            Some(NodePackageManager::Bun)
        );
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
    fn node_package_status_requires_installed_dependency() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"fixture","devDependencies":{"vitest":"^2.1.8"}}"#,
        )
        .expect("package json");
        assert_eq!(
            node_package_status(Some(dir.path()), "vitest", None),
            super::ToolStatus::Missing
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
            super::ToolStatus::Current
        );
    }
}
