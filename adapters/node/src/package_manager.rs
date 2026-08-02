use ayni_core::ExecutionResolution;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub(crate) fn from_executable(value: &str) -> Option<Self> {
        match value {
            "npm" => Some(Self::Npm),
            "pnpm" => Some(Self::Pnpm),
            "yarn" => Some(Self::Yarn),
            "bun" => Some(Self::Bun),
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
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    pub(crate) fn exec_command(self, tool: &str, args: &[&str]) -> (String, Vec<String>) {
        let prefix: &[&str] = match self {
            Self::Npm => &["exec", "--"],
            Self::Pnpm | Self::Yarn => &["exec"],
            Self::Bun => &["x"],
        };
        let mut argv = prefix
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        argv.push(tool.to_string());
        argv.extend(args.iter().map(|arg| (*arg).to_string()));
        (self.executable().to_string(), argv)
    }

    pub(crate) fn add_dependency_args(self, package: &str, dev: bool) -> Vec<String> {
        let (command, dev_flag) = match self {
            Self::Npm => ("install", "--save-dev"),
            Self::Pnpm => ("add", "-D"),
            Self::Yarn => ("add", "--dev"),
            Self::Bun => ("add", "-d"),
        };
        let mut args = vec![command.to_string()];
        if dev {
            args.push(dev_flag.to_string());
        }
        args.push(package.to_string());
        args
    }
}

pub(crate) fn detect(root: &Path) -> Option<PackageManager> {
    [
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("yarn.lock", PackageManager::Yarn),
        ("package-lock.json", PackageManager::Npm),
        ("bun.lock", PackageManager::Bun),
        ("bun.lockb", PackageManager::Bun),
    ]
    .into_iter()
    .find_map(|(marker, manager)| root.join(marker).is_file().then_some(manager))
    .or_else(|| parse_manifest(&root.join("package.json")))
}

pub(crate) fn resolve(repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
    let direct = detect(root)
        .map(|manager| resolution(manager, root.to_path_buf(), "direct_root", 100, root));
    let ancestor = workspace_ancestor(repo_root, root);
    match (direct, ancestor) {
        (Some(mut direct), Some(ancestor)) if direct.runner != ancestor.runner => {
            direct.ambiguous = true;
            Some(direct)
        }
        (Some(direct), _) => Some(direct),
        (None, Some(ancestor)) => Some(ancestor),
        (None, None) if root.join("package.json").is_file() => Some(resolution(
            PackageManager::Npm,
            root.to_path_buf(),
            "fallback",
            60,
            root,
        )),
        (None, None) => None,
    }
}

fn resolution(
    manager: PackageManager,
    resolved_from: PathBuf,
    kind: &str,
    confidence: u8,
    exec_root: &Path,
) -> ExecutionResolution {
    ExecutionResolution {
        runner: manager.executable().to_string(),
        resolved_from: resolved_from.clone(),
        kind: kind.to_string(),
        source: String::from("node package manager"),
        confidence,
        ambiguous: false,
        install_cwd: resolved_from,
        exec_cwd: exec_root.to_path_buf(),
    }
}

fn workspace_ancestor(repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
    let mut current = root.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        let manifest = path.join("package.json");
        if manifest.is_file() && manifest_has_workspaces(&manifest) {
            let manager = detect(path).unwrap_or(PackageManager::Npm);
            return Some(resolution(
                manager,
                path.to_path_buf(),
                "workspace_ancestor",
                90,
                root,
            ));
        }
        current = path.parent();
    }
    None
}

fn parse_manifest(path: &Path) -> Option<PackageManager> {
    let content = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<JsonValue>(&content).ok()?;
    let raw = value.get("packageManager")?.as_str()?.to_ascii_lowercase();
    [
        ("pnpm@", PackageManager::Pnpm),
        ("yarn@", PackageManager::Yarn),
        ("bun@", PackageManager::Bun),
        ("npm@", PackageManager::Npm),
    ]
    .into_iter()
    .find_map(|(prefix, manager)| raw.starts_with(prefix).then_some(manager))
}

fn manifest_has_workspaces(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<JsonValue>(&content).ok())
        .is_some_and(|value| value.get("workspaces").is_some())
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
                PackageManager::Npm,
                "npm",
                &["exec", "--"][..],
                &["install", "--save-dev"][..],
            ),
            (
                PackageManager::Pnpm,
                "pnpm",
                &["exec"][..],
                &["add", "-D"][..],
            ),
            (
                PackageManager::Yarn,
                "yarn",
                &["exec"][..],
                &["add", "--dev"][..],
            ),
            (PackageManager::Bun, "bun", &["x"][..], &["add", "-d"][..]),
        ];
        for (manager, executable, exec_prefix, add_prefix) in cases {
            assert_eq!(PackageManager::from_executable(executable), Some(manager));
            let (program, argv) = manager.exec_command("vitest", &["run"]);
            assert_eq!(program, executable);
            let mut expected = exec_prefix
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            expected.extend([String::from("vitest"), String::from("run")]);
            assert_eq!(argv, expected);
            let args = manager.add_dependency_args("vitest@3.2.4", true);
            let mut expected = add_prefix
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            expected.push(String::from("vitest@3.2.4"));
            assert_eq!(args, expected);
        }
        assert_eq!(
            PackageManager::Npm.add_dependency_args("left-pad", false),
            ["install", "left-pad"]
        );
    }

    #[test]
    fn marker_and_manifest_precedence_is_characterized() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"packageManager":"bun@1"}"#,
        )
        .expect("manifest");
        fs::write(dir.path().join("bun.lockb"), "").expect("bun lock");
        fs::write(dir.path().join("package-lock.json"), "").expect("npm lock");
        fs::write(dir.path().join("yarn.lock"), "").expect("yarn lock");
        fs::write(dir.path().join("pnpm-lock.yaml"), "").expect("pnpm lock");
        assert_eq!(detect(dir.path()), Some(PackageManager::Pnpm));
        fs::remove_file(dir.path().join("pnpm-lock.yaml")).expect("remove pnpm");
        assert_eq!(detect(dir.path()), Some(PackageManager::Yarn));
        fs::remove_file(dir.path().join("yarn.lock")).expect("remove yarn");
        assert_eq!(detect(dir.path()), Some(PackageManager::Npm));
        fs::remove_file(dir.path().join("package-lock.json")).expect("remove npm");
        assert_eq!(detect(dir.path()), Some(PackageManager::Bun));
        fs::remove_file(dir.path().join("bun.lockb")).expect("remove bun");
        assert_eq!(detect(dir.path()), Some(PackageManager::Bun));
    }
}
