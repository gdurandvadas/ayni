use crate::signal::SignalKind;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonPackageManager {
    Uv,
    Poetry,
    Pdm,
    Pipenv,
    Hatch,
    Pip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonResolutionKind {
    DirectRoot,
    WorkspaceAncestor,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonPackageManagerResolution {
    pub manager: PythonPackageManager,
    pub resolved_from: PathBuf,
    pub kind: PythonResolutionKind,
    pub ambiguous: bool,
}

impl PythonPackageManagerResolution {
    #[must_use]
    pub fn manager_label(&self) -> &'static str {
        self.manager.executable()
    }

    #[must_use]
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            PythonResolutionKind::DirectRoot => "direct_root",
            PythonResolutionKind::WorkspaceAncestor => "workspace_ancestor",
            PythonResolutionKind::Fallback => "fallback",
        }
    }
}

impl PythonPackageManager {
    #[must_use]
    pub fn executable(self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Poetry => "poetry",
            Self::Pdm => "pdm",
            Self::Pipenv => "pipenv",
            Self::Hatch => "hatch",
            Self::Pip => "python",
        }
    }

    #[must_use]
    pub fn run_command(self, module: &str, args: &[&str]) -> (String, Vec<String>) {
        match self {
            Self::Uv => python_prefixed("uv", &["run"], module, args),
            Self::Poetry => python_prefixed("poetry", &["run"], module, args),
            Self::Pdm => python_prefixed("pdm", &["run"], module, args),
            Self::Pipenv => python_prefixed("pipenv", &["run"], module, args),
            Self::Hatch => python_prefixed("hatch", &["run"], module, args),
            Self::Pip => {
                let mut command = vec![String::from("-m"), module_name_for_python_m(module)];
                command.extend(args.iter().map(|value| (*value).to_string()));
                (String::from("python"), command)
            }
        }
    }

    #[must_use]
    pub fn add_dependency_args(self, package: &str, dev: bool) -> Vec<String> {
        match self {
            Self::Uv => {
                let mut args = vec![String::from("add")];
                if dev {
                    args.push(String::from("--dev"));
                }
                args.push(package.to_string());
                args
            }
            Self::Poetry => {
                let mut args = vec![String::from("add")];
                if dev {
                    args.push(String::from("--group"));
                    args.push(String::from("dev"));
                }
                args.push(package.to_string());
                args
            }
            Self::Pdm => {
                let mut args = vec![String::from("add")];
                if dev {
                    args.push(String::from("--dev"));
                }
                args.push(package.to_string());
                args
            }
            Self::Pipenv => {
                let mut args = vec![String::from("install")];
                if dev {
                    args.push(String::from("--dev"));
                }
                args.push(package.to_string());
                args
            }
            Self::Hatch | Self::Pip => {
                vec![
                    String::from("-m"),
                    String::from("pip"),
                    String::from("install"),
                    package.to_string(),
                ]
            }
        }
    }
}

fn python_prefixed(
    program: &str,
    prefix: &[&str],
    module: &str,
    args: &[&str],
) -> (String, Vec<String>) {
    let mut command = prefix
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    command.push(module.to_string());
    command.extend(args.iter().map(|value| (*value).to_string()));
    (program.to_string(), command)
}

fn module_name_for_python_m(module: &str) -> String {
    match module {
        "pytest" => String::from("pytest"),
        "coverage" => String::from("coverage"),
        "complexipy" => String::from("complexipy"),
        "mutmut" => String::from("mutmut"),
        value => value.replace('-', "_"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstallContext<'a> {
    pub cwd: Option<&'a Path>,
    pub node_package_manager: Option<NodePackageManager>,
    pub python_package_manager: Option<PythonPackageManager>,
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

#[must_use]
pub fn detect_python_package_manager(root: &Path) -> Option<PythonPackageManager> {
    if root.join("uv.lock").is_file() {
        return Some(PythonPackageManager::Uv);
    }
    if root.join("poetry.lock").is_file() {
        return Some(PythonPackageManager::Poetry);
    }
    if root.join("pdm.lock").is_file() {
        return Some(PythonPackageManager::Pdm);
    }
    if root.join("Pipfile.lock").is_file() {
        return Some(PythonPackageManager::Pipenv);
    }
    if root.join("hatch.toml").is_file() {
        return Some(PythonPackageManager::Hatch);
    }
    if root.join("pyproject.toml").is_file() || root.join("requirements.txt").is_file() {
        return Some(PythonPackageManager::Pip);
    }
    None
}

#[must_use]
pub fn resolve_python_package_manager(
    repo_root: &Path,
    workdir: &Path,
) -> Option<PythonPackageManagerResolution> {
    let direct =
        detect_python_package_manager(workdir).map(|manager| PythonPackageManagerResolution {
            manager,
            resolved_from: workdir.to_path_buf(),
            kind: PythonResolutionKind::DirectRoot,
            ambiguous: false,
        });
    let has_manifest = workdir.join("pyproject.toml").is_file()
        || workdir.join("requirements.txt").is_file()
        || workdir.join("Pipfile").is_file();
    let ancestor = find_workspace_ancestor_resolution(repo_root, workdir);
    match (direct, ancestor) {
        (Some(direct), Some(ancestor))
            if direct.manager == PythonPackageManager::Pip
                && ancestor.manager == PythonPackageManager::Uv =>
        {
            Some(PythonPackageManagerResolution {
                ambiguous: true,
                ..ancestor
            })
        }
        (Some(direct), Some(mut ancestor))
            if ancestor.manager != direct.manager
                && ancestor.kind == PythonResolutionKind::WorkspaceAncestor =>
        {
            ancestor.ambiguous = true;
            Some(direct)
        }
        (Some(direct), _) => Some(direct),
        (None, Some(ancestor)) => Some(ancestor),
        (None, None) if has_manifest => Some(PythonPackageManagerResolution {
            manager: PythonPackageManager::Pip,
            resolved_from: workdir.to_path_buf(),
            kind: PythonResolutionKind::Fallback,
            ambiguous: false,
        }),
        (None, None) => None,
    }
}

fn find_workspace_ancestor_resolution(
    repo_root: &Path,
    workdir: &Path,
) -> Option<PythonPackageManagerResolution> {
    let mut current = workdir.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        if path.join("uv.lock").is_file() || pyproject_has_uv_workspace(path) {
            return Some(PythonPackageManagerResolution {
                manager: PythonPackageManager::Uv,
                resolved_from: path.to_path_buf(),
                kind: PythonResolutionKind::WorkspaceAncestor,
                ambiguous: false,
            });
        }
        current = path.parent();
    }
    None
}

fn pyproject_has_uv_workspace(root: &Path) -> bool {
    let pyproject_path = root.join("pyproject.toml");
    let Ok(content) = fs::read_to_string(pyproject_path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return false;
    };
    value
        .get("tool")
        .and_then(|value| value.get("uv"))
        .and_then(|value| value.get("workspace"))
        .is_some()
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
    PythonPackage {
        package: &'static str,
        import_name: &'static str,
        version: Option<&'static str>,
        dev: bool,
    },
    UvTool {
        package: &'static str,
        version: Option<&'static str>,
    },
    PythonRuntime,
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
                Installer::PythonPackage {
                    import_name,
                    version,
                    ..
                } => python_package_status(ctx, import_name, version),
                Installer::UvTool { package, version } => uv_tool_status(package, version),
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

    pub fn install(&self) -> Result<(), String> {
        self.install_in(InstallContext::default())
    }

    pub fn install_in(&self, ctx: InstallContext<'_>) -> Result<(), String> {
        install_with(&self.installer, self.name, ctx)
    }
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
        NodePackageManager, PythonPackageManager, detect_node_package_manager,
        detect_python_package_manager, node_package_status, resolve_python_package_manager,
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

    #[test]
    fn detects_python_manager_from_lockfile_precedence() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("pyproject.toml"), "").expect("pyproject");
        fs::write(dir.path().join("poetry.lock"), "").expect("poetry lock");
        assert_eq!(
            detect_python_package_manager(dir.path()),
            Some(PythonPackageManager::Poetry)
        );

        fs::write(dir.path().join("uv.lock"), "").expect("uv lock");
        assert_eq!(
            detect_python_package_manager(dir.path()),
            Some(PythonPackageManager::Uv)
        );
    }

    #[test]
    fn python_manager_builds_run_commands() {
        assert_eq!(
            PythonPackageManager::Uv.run_command("pytest", &["-q"]),
            (
                String::from("uv"),
                vec![
                    String::from("run"),
                    String::from("pytest"),
                    String::from("-q")
                ]
            )
        );
        assert_eq!(
            PythonPackageManager::Pip.run_command("pytest", &["-q"]),
            (
                String::from("python"),
                vec![
                    String::from("-m"),
                    String::from("pytest"),
                    String::from("-q")
                ]
            )
        );
    }

    #[test]
    fn resolves_python_manager_from_workspace_ancestor_uv_lock() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
        )
        .expect("root pyproject");
        fs::write(dir.path().join("uv.lock"), "").expect("uv lock");
        let member = dir.path().join("packages/config");
        fs::create_dir_all(&member).expect("member dir");
        fs::write(member.join("pyproject.toml"), "[project]\nname='config'\n").expect("member");

        let resolution =
            resolve_python_package_manager(dir.path(), &member).expect("python resolution");
        assert_eq!(resolution.manager, PythonPackageManager::Uv);
        assert_eq!(resolution.kind_label(), "workspace_ancestor");
        assert!(resolution.ambiguous);
    }

    #[test]
    fn resolves_python_manager_fallback_without_lockfile() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname='fixture'\n",
        )
        .expect("pyproject");

        let resolution =
            resolve_python_package_manager(dir.path(), dir.path()).expect("python resolution");
        assert_eq!(resolution.manager, PythonPackageManager::Pip);
        assert_eq!(resolution.kind_label(), "direct_root");
    }
}
