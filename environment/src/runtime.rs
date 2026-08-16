use crate::image::image_plan_with_preparation;
use crate::{BackendError, read_lock};
use ayni_core::{
    DependencyPreparationPlan, EnvironmentLock, LockedTargetEnvironment, TargetIdentity,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const WORKSPACE: &str = "/workspace";

mod engine;
pub use engine::{
    Engine, TargetSelection, build, build_prepared, detect_engine, doctor, doctor_prepared,
};
use engine::{engine_name, validate_image};

pub fn select_target<'a>(
    lock: &'a EnvironmentLock,
    selection: &TargetSelection,
) -> Result<&'a LockedTargetEnvironment, BackendError> {
    let normalized_root = selection
        .root
        .as_deref()
        .map(|root| {
            let language = selection.language.ok_or_else(|| {
                BackendError::input("--root requires --language for environment target selection")
            })?;
            TargetIdentity::new(language, root)
                .map(|target| target.root)
                .map_err(|error| BackendError::input(error.to_string()))
        })
        .transpose()?;
    let matches = lock
        .targets()
        .iter()
        .filter(|target| {
            selection
                .language
                .is_none_or(|language| target.target.language == language)
                && normalized_root
                    .as_ref()
                    .is_none_or(|root| &target.target.root == root)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(*target),
        [] => Err(BackendError::input(
            "environment target selection did not match a locked target",
        )),
        _ => Err(BackendError::input(
            "environment has multiple targets; pass --language and --root",
        )),
    }
}

/// Launch the repository-scoped Ayni entrypoint without selecting one global
/// target environment. Managed quality execution activates each locked target
/// separately inside the container.
pub fn launch_repository(repo_root: &Path, command: &[String]) -> Result<i32, BackendError> {
    launch_repository_prepared(repo_root, &[], command)
}

pub fn launch_repository_prepared(
    repo_root: &Path,
    preparations: &[DependencyPreparationPlan],
    command: &[String],
) -> Result<i32, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan_with_preparation(&lock, preparations)?;
    let engine = detect_engine()?;
    validate_image(engine, &plan, &lock)?;
    let state_home = execution_state(&root, lock.fingerprint())?;
    let mounts = materialize_outputs(&root, engine, &lock, &plan, preparations)?;
    let managed_environments =
        crate::preparation::managed_environments(&lock, preparations, &state_home)?;
    let args = repository_launch_args(
        &root,
        engine,
        &state_home,
        &plan.tag,
        command,
        &mounts,
        Some(&managed_environments),
    );
    execute_launch(engine, &args)
}

pub fn launch(
    repo_root: &Path,
    selection: &TargetSelection,
    command: &[String],
    shell: bool,
) -> Result<i32, BackendError> {
    launch_prepared(repo_root, selection, command, shell, &[])
}

pub fn launch_prepared(
    repo_root: &Path,
    selection: &TargetSelection,
    command: &[String],
    shell: bool,
    preparations: &[DependencyPreparationPlan],
) -> Result<i32, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan_with_preparation(&lock, preparations)?;
    let engine = detect_engine()?;
    validate_image(engine, &plan, &lock)?;
    let target = select_target(&lock, selection)?;
    let state_home = execution_state(&root, lock.fingerprint())?;
    let mounts = materialize_outputs(&root, engine, &lock, &plan, preparations)?;
    let execution_environment = preparations
        .iter()
        .find(|preparation| preparation.target == target.target)
        .map(|preparation| {
            crate::preparation::resolved_execution_environment(preparation, &state_home)
        })
        .unwrap_or_default();
    let args = launch_args(TargetLaunch {
        root: &root,
        engine,
        target,
        state_home: &state_home,
        image_tag: &plan.tag,
        command,
        shell,
        mounts: &mounts,
        execution_environment: &execution_environment,
    })?;
    execute_launch(engine, &args)
}

mod materialization;
use materialization::materialize_outputs;

fn repository_launch_args(
    root: &Path,
    engine: Engine,
    state_home: &str,
    image_tag: &str,
    command: &[String],
    mounts: &[(PathBuf, String)],
    managed_environments: Option<&str>,
) -> Vec<String> {
    let mut args = base_launch_args(engine);
    append_managed_workspace_state_args(&mut args, root, WORKSPACE, state_home);
    append_prepared_mounts(&mut args, mounts);
    if let Some(value) = managed_environments {
        args.extend([
            "--env".into(),
            format!("AYNI_MANAGED_TARGET_ENVIRONMENTS={value}"),
        ]);
    }
    args.extend([image_tag.to_owned()]);
    args.extend(command.iter().cloned());
    args
}

fn append_prepared_mounts(args: &mut Vec<String>, mounts: &[(PathBuf, String)]) {
    for (source, destination) in mounts {
        args.extend([
            "--mount".into(),
            format!("type=bind,source={},target={destination}", source.display()),
        ]);
    }
}

struct TargetLaunch<'a> {
    root: &'a Path,
    engine: Engine,
    target: &'a LockedTargetEnvironment,
    state_home: &'a str,
    image_tag: &'a str,
    command: &'a [String],
    shell: bool,
    mounts: &'a [(PathBuf, String)],
    execution_environment: &'a BTreeMap<String, String>,
}

fn launch_args(request: TargetLaunch<'_>) -> Result<Vec<String>, BackendError> {
    let mut args = base_launch_args(request.engine);
    append_workspace_args(&mut args, request.root, request.target, request.state_home);
    append_prepared_mounts(&mut args, request.mounts);
    append_target_environment(&mut args, request.target)?;
    for (name, value) in request.execution_environment {
        args.extend(["--env".into(), format!("{name}={value}")]);
    }
    append_command(&mut args, request.image_tag, request.command, request.shell)?;
    Ok(args)
}

fn base_launch_args(engine: Engine) -> Vec<String> {
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "--network".into(),
        "none".into(),
        "--read-only".into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--security-opt".into(),
        "no-new-privileges".into(),
        "--tmpfs".into(),
        "/tmp:rw,exec,nosuid,size=1g".into(),
    ];
    match engine {
        Engine::Docker => args.extend(["--user".into(), host_identity()]),
        Engine::Podman => args.extend(["--userns".into(), "keep-id".into()]),
    }
    args
}

fn append_workspace_args(
    args: &mut Vec<String>,
    root: &Path,
    target: &LockedTargetEnvironment,
    state_home: &str,
) {
    let target_workdir = if target.target.root == "." {
        WORKSPACE.to_owned()
    } else {
        format!("{WORKSPACE}/{}", target.target.root)
    };
    append_workspace_state_args(args, root, &target_workdir, state_home);
}

fn append_workspace_state_args(
    args: &mut Vec<String>,
    root: &Path,
    workdir: &str,
    state_home: &str,
) {
    args.extend([
        "--volume".into(),
        format!("{}:{WORKSPACE}:rw", root.display()),
    ]);
    append_workspace_execution_settings(args, workdir, state_home);
}

/// Managed quality commands must not modify checkout source files. The nested
/// `.ayni` bind mount deliberately remains writable for signal artifacts,
/// environment state, and tool caches.
fn append_managed_workspace_state_args(
    args: &mut Vec<String>,
    root: &Path,
    workdir: &str,
    state_home: &str,
) {
    args.extend([
        "--mount".into(),
        format!(
            "type=bind,source={},target={WORKSPACE},readonly",
            root.display()
        ),
        "--mount".into(),
        format!(
            "type=bind,source={},target={WORKSPACE}/.ayni",
            root.join(".ayni").display()
        ),
    ]);
    append_workspace_execution_settings(args, workdir, state_home);
}

fn append_workspace_execution_settings(args: &mut Vec<String>, workdir: &str, state_home: &str) {
    args.extend([
        "--workdir".into(),
        workdir.to_owned(),
        "--env".into(),
        format!("HOME={state_home}"),
        "--env".into(),
        "RUSTUP_HOME=/home/ayni/.rustup".into(),
        "--env".into(),
        "MISE_AUTO_INSTALL=0".into(),
    ]);
}

fn append_target_environment(
    args: &mut Vec<String>,
    target: &LockedTargetEnvironment,
) -> Result<(), BackendError> {
    for (name, version) in target_environment(target)? {
        args.extend(["--env".into(), format!("{name}={version}")]);
    }
    Ok(())
}

fn append_command(
    args: &mut Vec<String>,
    image_tag: &str,
    command: &[String],
    shell: bool,
) -> Result<(), BackendError> {
    if shell {
        args.extend(["--interactive".into(), "--tty".into()]);
        args.extend(["--entrypoint".into(), "/bin/sh".into(), image_tag.into()]);
        return Ok(());
    }
    let entrypoint = command
        .first()
        .ok_or_else(|| BackendError::input("env run requires a command after `--`"))?;
    args.extend([
        "--entrypoint".into(),
        entrypoint.to_owned(),
        image_tag.into(),
    ]);
    args.extend(command.iter().skip(1).cloned());
    Ok(())
}

fn execute_launch(engine: Engine, args: &[String]) -> Result<i32, BackendError> {
    let status = Command::new(engine_name(engine))
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            BackendError::execution(format!("failed to start {}: {error}", engine_name(engine)))
        })?;
    Ok(status.code().unwrap_or(4))
}

pub(crate) fn target_environment(
    target: &LockedTargetEnvironment,
) -> Result<Vec<(String, String)>, BackendError> {
    let mut variables = BTreeMap::new();
    for runtime in &target.runtimes {
        variables.insert(
            mise_version_variable(&runtime.runtime)?,
            runtime.version.clone(),
        );
        if runtime.runtime == "java" {
            validate_mise_install_version("java", &runtime.version)?;
            variables.insert(
                String::from("JAVA_HOME"),
                format!("/opt/ayni/mise/installs/java/{}", runtime.version),
            );
        }
    }
    if let Some(manager) = &target.package_manager {
        variables.insert(
            mise_version_variable(&manager.family)?,
            manager.version.clone(),
        );
    }
    Ok(variables.into_iter().collect())
}

fn mise_version_variable(tool: &str) -> Result<String, BackendError> {
    if tool.is_empty()
        || !tool
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(BackendError::environment(format!(
            "locked mise tool name cannot be activated safely: {tool}"
        )));
    }
    Ok(format!(
        "MISE_{}_VERSION",
        tool.to_ascii_uppercase().replace('-', "_")
    ))
}

fn validate_mise_install_version(tool: &str, version: &str) -> Result<(), BackendError> {
    let safe = !version.is_empty()
        && version != "."
        && version != ".."
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'));
    if safe {
        Ok(())
    } else {
        Err(BackendError::environment(format!(
            "locked {tool} version cannot form a safe mise install path: {version}"
        )))
    }
}

fn execution_state(repo_root: &Path, fingerprint: &str) -> Result<String, BackendError> {
    let fingerprint = fingerprint.strip_prefix("sha256:").unwrap_or(fingerprint);
    let relative = PathBuf::from(".ayni/environment")
        .join(&fingerprint[..16.min(fingerprint.len())])
        .join("home");
    create_contained_directory_tree(repo_root, &relative)?;
    Ok(format!(
        "{WORKSPACE}/{}",
        relative.to_string_lossy().replace('\\', "/")
    ))
}

fn create_contained_directory_tree(repo_root: &Path, relative: &Path) -> Result<(), BackendError> {
    let mut current = repo_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(BackendError::execution(
                "managed environment state path is not repository-relative",
            ));
        };
        current.push(name);
        ensure_managed_directory(&current)?;
    }
    let canonical = current.canonicalize().map_err(|error| {
        BackendError::execution(format!(
            "failed to validate managed environment state {}: {error}",
            current.display()
        ))
    })?;
    if canonical.starts_with(repo_root) {
        Ok(())
    } else {
        Err(BackendError::execution(format!(
            "managed environment state escapes the repository: {}",
            current.display()
        )))
    }
}

fn ensure_managed_directory(path: &Path) -> Result<(), BackendError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_managed_directory(path, &metadata),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(managed_directory_error(path, error)),
            }
            let metadata =
                fs::symlink_metadata(path).map_err(|error| managed_directory_error(path, error))?;
            validate_managed_directory(path, &metadata)
        }
        Err(error) => Err(managed_directory_error(path, error)),
    }
}

fn validate_managed_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), BackendError> {
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(BackendError::execution(format!(
            "managed environment state must not contain symlinks or non-directories: {}",
            path.display()
        )))
    }
}

fn managed_directory_error(path: &Path, error: std::io::Error) -> BackendError {
    BackendError::execution(format!(
        "failed to create managed environment state {}: {error}",
        path.display()
    ))
}

fn canonical_root(path: &Path) -> Result<PathBuf, BackendError> {
    let root = path.canonicalize().map_err(|error| {
        BackendError::input(format!(
            "failed to establish repository root {}: {error}",
            path.display()
        ))
    })?;
    if root.is_dir() {
        Ok(root)
    } else {
        Err(BackendError::input(format!(
            "repository root is not a directory: {}",
            root.display()
        )))
    }
}

fn host_identity() -> String {
    #[cfg(unix)]
    {
        // SAFETY: libc identity getters have no preconditions.
        unsafe { format!("{}:{}", libc::getuid(), libc::getgid()) }
    }
    #[cfg(not(unix))]
    {
        "1000:1000".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_target_activation_sets_mise_selection_and_java_home() {
        use ayni_core::{
            Language, LockedRequirementSource, LockedRuntime, RequirementConfidence, TargetIdentity,
        };
        let target = LockedTargetEnvironment {
            target: TargetIdentity::new(Language::Kotlin, ".").expect("target"),
            runtimes: vec![LockedRuntime {
                runtime: "java".into(),
                version: "temurin-21.0.6+7".into(),
                components: Vec::new(),
                targets: Vec::new(),
                source: LockedRequirementSource {
                    kind: "test".into(),
                    path: ".java-version".into(),
                    digest: None,
                    confidence: RequirementConfidence::Exact,
                },
            }],
            package_manager: None,
            signal_tools: Vec::new(),
            dependency_locks: Vec::new(),
        };
        let environment = target_environment(&target).expect("activation");
        assert!(environment.contains(&("MISE_JAVA_VERSION".into(), "temurin-21.0.6+7".into())));
        assert!(environment.contains(&(
            "JAVA_HOME".into(),
            "/opt/ayni/mise/installs/java/temurin-21.0.6+7".into()
        )));
    }

    #[test]
    fn repository_launch_keeps_target_activation_inside_ayni() {
        let args = repository_launch_args(
            Path::new("/checkout"),
            Engine::Docker,
            "/workspace/.ayni/environment/state/home",
            "ayni-env:test",
            &["check".into(), "--host".into()],
            &[],
            None,
        );
        assert!(args.windows(2).any(|pair| pair == ["--network", "none"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--workdir", "/workspace"])
        );
        assert!(
            args.iter()
                .any(|arg| { arg == "type=bind,source=/checkout,target=/workspace,readonly" })
        );
        assert!(
            args.iter()
                .any(|arg| { arg == "type=bind,source=/checkout/.ayni,target=/workspace/.ayni" })
        );
        assert!(!args.iter().any(|arg| arg == "/checkout:/workspace:rw"));
        assert!(!args.iter().any(|arg| arg == "--entrypoint"));
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("MISE_") && arg.ends_with("_VERSION"))
        );
        assert!(args.ends_with(&["ayni-env:test".into(), "check".into(), "--host".into()]));
    }
}
