use ayni_core::ExecutionResolution;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageManager {
    Uv,
    Poetry,
    Pdm,
    Pipenv,
    Hatch,
    Pip,
}

impl PackageManager {
    pub(crate) fn from_executable(value: &str) -> Option<Self> {
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

    pub(crate) fn from_runner(value: &str) -> Option<Self> {
        Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(Self::from_executable)
    }

    pub(crate) const fn executable(self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Poetry => "poetry",
            Self::Pdm => "pdm",
            Self::Pipenv => "pipenv",
            Self::Hatch => "hatch",
            Self::Pip => "python",
        }
    }

    pub(crate) fn run_command(self, module: &str, args: &[&str]) -> (String, Vec<String>) {
        if self == Self::Pip {
            let mut argv = vec![String::from("-m"), module_name(module)];
            argv.extend(args.iter().map(|arg| (*arg).to_string()));
            return (String::from("python"), argv);
        }
        let mut argv = vec![String::from("run"), module.to_string()];
        argv.extend(args.iter().map(|arg| (*arg).to_string()));
        (self.executable().to_string(), argv)
    }

    pub(crate) fn add_dependency_args(self, package: &str, dev: bool) -> Vec<String> {
        let mut args = match self {
            Self::Uv => vec![String::from("add")],
            Self::Poetry | Self::Pdm => vec![String::from("add")],
            Self::Pipenv => vec![String::from("install")],
            Self::Hatch | Self::Pip => vec![
                String::from("-m"),
                String::from("pip"),
                String::from("install"),
            ],
        };
        if dev {
            match self {
                Self::Uv | Self::Pdm => args.push(String::from("--dev")),
                Self::Poetry => args.extend([String::from("--group"), String::from("dev")]),
                Self::Pipenv => args.push(String::from("--dev")),
                Self::Hatch | Self::Pip => {}
            }
        }
        args.push(package.to_string());
        args
    }
}

pub(crate) fn detect(root: &Path) -> Option<PackageManager> {
    [
        ("uv.lock", PackageManager::Uv),
        ("poetry.lock", PackageManager::Poetry),
        ("pdm.lock", PackageManager::Pdm),
        ("Pipfile.lock", PackageManager::Pipenv),
        ("hatch.toml", PackageManager::Hatch),
    ]
    .into_iter()
    .find_map(|(marker, manager)| root.join(marker).is_file().then_some(manager))
    .or_else(|| {
        (root.join("pyproject.toml").is_file() || root.join("requirements.txt").is_file())
            .then_some(PackageManager::Pip)
    })
}

pub(crate) fn resolve(repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
    let direct = detect(root).map(|manager| resolution(manager, root, "direct_root", 100, root));
    let ancestor = workspace_ancestor(repo_root, root);
    let mut resolved = match (direct, ancestor) {
        (Some(direct), Some(mut ancestor))
            if direct.runner == PackageManager::Pip.executable()
                && ancestor.runner == PackageManager::Uv.executable() =>
        {
            ancestor.ambiguous = true;
            Some(ancestor)
        }
        (Some(mut direct), Some(ancestor)) if direct.runner != ancestor.runner => {
            direct.ambiguous = true;
            Some(direct)
        }
        (Some(direct), _) => Some(direct),
        (None, Some(ancestor)) => Some(ancestor),
        (None, None) if has_manifest(root) => {
            Some(resolution(PackageManager::Pip, root, "fallback", 100, root))
        }
        (None, None) => None,
    }?;
    if resolved.ambiguous {
        resolved.confidence = 80;
    }
    Some(resolved)
}

fn resolution(
    manager: PackageManager,
    resolved_from: &Path,
    kind: &str,
    confidence: u8,
    exec_root: &Path,
) -> ExecutionResolution {
    ExecutionResolution {
        runner: manager.executable().to_string(),
        resolved_from: resolved_from.to_path_buf(),
        kind: kind.to_string(),
        source: String::from("python package manager"),
        confidence,
        ambiguous: false,
        install_cwd: if kind == "workspace_ancestor" {
            resolved_from.to_path_buf()
        } else {
            exec_root.to_path_buf()
        },
        exec_cwd: exec_root.to_path_buf(),
    }
}

fn workspace_ancestor(repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
    let mut current = root.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        if path.join("uv.lock").is_file() || pyproject_has_uv_workspace(path) {
            return Some(resolution(
                PackageManager::Uv,
                path,
                "workspace_ancestor",
                100,
                root,
            ));
        }
        current = path.parent();
    }
    None
}

fn has_manifest(root: &Path) -> bool {
    root.join("pyproject.toml").is_file()
        || root.join("requirements.txt").is_file()
        || root.join("Pipfile").is_file()
}

fn pyproject_has_uv_workspace(root: &Path) -> bool {
    fs::read_to_string(root.join("pyproject.toml"))
        .ok()
        .and_then(|content| toml::from_str::<toml::Value>(&content).ok())
        .and_then(|value| value.get("tool")?.get("uv")?.get("workspace").cloned())
        .is_some()
}

fn module_name(module: &str) -> String {
    module.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::{PackageManager, detect};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn manager_commands_are_characterized() {
        let cases = [
            (
                PackageManager::Uv,
                "uv",
                &["run"][..],
                &["add", "--dev"][..],
            ),
            (
                PackageManager::Poetry,
                "poetry",
                &["run"][..],
                &["add", "--group", "dev"][..],
            ),
            (
                PackageManager::Pdm,
                "pdm",
                &["run"][..],
                &["add", "--dev"][..],
            ),
            (
                PackageManager::Pipenv,
                "pipenv",
                &["run"][..],
                &["install", "--dev"][..],
            ),
            (
                PackageManager::Hatch,
                "hatch",
                &["run"][..],
                &["-m", "pip", "install"][..],
            ),
            (
                PackageManager::Pip,
                "python",
                &["-m"][..],
                &["-m", "pip", "install"][..],
            ),
        ];
        for (manager, executable, run_prefix, add_prefix) in cases {
            assert_eq!(PackageManager::from_executable(executable), Some(manager));
            let (program, argv) = manager.run_command("pytest-json-report", &["-q"]);
            assert_eq!(program, executable);
            let mut expected = run_prefix
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            expected.extend([String::from("pytest_json_report"), String::from("-q")]);
            if manager != PackageManager::Pip {
                expected[1] = String::from("pytest-json-report");
            }
            assert_eq!(argv, expected);
            let mut expected = add_prefix
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            expected.push(String::from("pytest"));
            assert_eq!(manager.add_dependency_args("pytest", true), expected);
        }
    }

    #[test]
    fn marker_precedence_is_characterized() {
        let dir = TempDir::new().expect("tempdir");
        for marker in [
            "requirements.txt",
            "hatch.toml",
            "Pipfile.lock",
            "pdm.lock",
            "poetry.lock",
            "uv.lock",
        ] {
            fs::write(dir.path().join(marker), "").expect("marker");
        }
        for (marker, manager) in [
            ("uv.lock", PackageManager::Uv),
            ("poetry.lock", PackageManager::Poetry),
            ("pdm.lock", PackageManager::Pdm),
            ("Pipfile.lock", PackageManager::Pipenv),
            ("hatch.toml", PackageManager::Hatch),
            ("requirements.txt", PackageManager::Pip),
        ] {
            assert_eq!(detect(dir.path()), Some(manager));
            fs::remove_file(dir.path().join(marker)).expect("remove marker");
        }
    }
}
