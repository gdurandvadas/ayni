//! Catalog *contract* types: which tools an adapter needs, how to check
//! their versions, and how they are installed. Execution of these checks and
//! installers lives in `ayni-adapters-common`, keeping core free of tool
//! invocation.

use crate::signal::SignalKind;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};

mod python_resolution;

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
    pub fn from_executable(value: &str) -> Option<Self> {
        match value {
            "npm" => Some(Self::Npm),
            "pnpm" => Some(Self::Pnpm),
            "yarn" => Some(Self::Yarn),
            "bun" => Some(Self::Bun),
            _ => None,
        }
    }

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

pub use python_resolution::resolve_python_package_manager;

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
    pub fn from_executable(value: &str) -> Option<Self> {
        match value {
            "uv" => Some(Self::Uv),
            "poetry" => Some(Self::Poetry),
            "pdm" => Some(Self::Pdm),
            "pipenv" => Some(Self::Pipenv),
            "hatch" => Some(Self::Hatch),
            "python" | "python3" => Some(Self::Pip),
            _ => None,
        }
    }

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
    pub gradle_runner: Option<&'a str>,
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
    GradleTask {
        task: &'static str,
    },
    GradleTaskAny {
        tasks: &'static [&'static str],
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
        detect_python_package_manager, resolve_python_package_manager,
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
