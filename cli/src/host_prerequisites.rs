use ayni_core::{AyniPolicy, Language, RunContext, SignalCollector, SignalKind};
use std::collections::BTreeSet;
use std::env;
use std::path::Path;

pub(crate) struct SelectedCheck<'a> {
    pub(crate) language: Language,
    pub(crate) signal: SignalKind,
    pub(crate) context: &'a RunContext,
    pub(crate) collector: &'a dyn SignalCollector,
}

/// Validate every executable entry point required by the selected host
/// execution topology, plus unqualified repository-wide Mise tools.
pub(crate) fn validate<'a>(
    policy: &AyniPolicy,
    selected_checks: impl IntoIterator<Item = SelectedCheck<'a>>,
) -> Result<(), String> {
    if crate::analysis::managed_execution_active() {
        return Ok(());
    }

    let mut missing = BTreeSet::new();
    for tool in policy.environment_tools().keys() {
        // A qualified Mise coordinate (for example `ubi:owner/tool`) names a
        // package source, not necessarily its installed executable.
        if is_unqualified_tool_name(tool) && !executable_on_path(tool) {
            missing.insert(format!("environment.tools.{tool} (`{tool}`)"));
        }
    }

    for check in selected_checks {
        for command in check
            .collector
            .required_host_executables(check.signal, check.context)
        {
            let command = command.trim();
            if command.is_empty()
                || executable_available(command, &check.context.execution.exec_cwd)
            {
                continue;
            }
            let root = check.context.scope.path.as_deref().unwrap_or(".");
            missing.insert(format!(
                "`{command}` ({}:{root} {})",
                check.language.as_str(),
                signal_name(check.signal)
            ));
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    let missing = missing.into_iter().collect::<Vec<_>>().join(", ");
    Err(format!(
        "host execution is missing required executable(s): {missing}. `--host` runs collectors using the host filesystem and PATH; install the required executable(s), or rerun without `--host` to use the locked managed environment"
    ))
}

fn is_unqualified_tool_name(tool: &str) -> bool {
    !tool.contains(':') && !tool.contains('/') && !tool.contains('\\')
}

fn executable_available(command: &str, cwd: &Path) -> bool {
    let path = Path::new(command);
    if path.is_absolute() {
        executable_at_path(path)
    } else if command.contains('/') || command.contains('\\') {
        executable_at_path(&cwd.join(path))
    } else {
        executable_on_path(command)
    }
}

fn executable_on_path(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| executable_at_path(&directory.join(name)))
}

fn executable_at_path(path: &Path) -> bool {
    if is_executable(path) {
        return true;
    }
    #[cfg(windows)]
    {
        if path.extension().is_none()
            && let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        {
            let name = name.to_string_lossy();
            let extensions = env::var_os("PATHEXT")
                .and_then(|value| value.into_string().ok())
                .unwrap_or_else(|| String::from(".COM;.EXE;.BAT;.CMD"));
            return extensions
                .split(';')
                .filter(|extension| !extension.is_empty())
                .any(|extension| is_executable(&parent.join(format!("{name}{extension}"))));
        }
    }
    false
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn signal_name(signal: SignalKind) -> &'static str {
    match signal {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

#[cfg(test)]
mod tests {
    use super::is_unqualified_tool_name;

    #[test]
    fn qualified_mise_coordinates_are_not_treated_as_executable_names() {
        assert!(is_unqualified_tool_name("protoc"));
        assert!(!is_unqualified_tool_name("ubi:protocolbuffers/protobuf"));
        assert!(!is_unqualified_tool_name("npm:prettier"));
    }

    #[cfg(unix)]
    #[test]
    fn relative_commands_are_resolved_from_the_execution_cwd() {
        use super::executable_available;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        let root = TempDir::new().expect("tempdir");
        fs::create_dir(root.path().join("tools")).expect("tools dir");
        let command = root.path().join("tools/runner");
        fs::write(&command, "#!/bin/sh\nexit 0\n").expect("command");
        let mut permissions = fs::metadata(&command).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&command, permissions).expect("executable");

        assert!(executable_available("tools/runner", root.path()));
        assert!(!executable_available("tools/missing", root.path()));
    }
}
