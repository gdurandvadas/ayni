use crate::image::{
    IMAGE_AYNI_LABEL, IMAGE_BASE_LABEL, IMAGE_LOCK_LABEL, IMAGE_MISE_LABEL, IMAGE_PLATFORM_LABEL,
    IMAGE_SCHEMA_LABEL, IMAGE_SCHEMA_VERSION, ImagePlan, image_plan,
};
use crate::{BackendError, concise_output, read_lock};
use ayni_adapters_common::exec::{DEFAULT_TOOL_TIMEOUT, run_command};
use ayni_core::{EnvironmentLock, Language, LockedTargetEnvironment, TargetIdentity};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const WORKSPACE: &str = "/workspace";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Docker,
    Podman,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetSelection {
    pub language: Option<Language>,
    pub root: Option<String>,
}

pub fn detect_engine() -> Result<Engine, BackendError> {
    if let Ok(requested) = env::var("AYNI_OCI_RUNTIME") {
        return match requested.as_str() {
            "docker" if engine_usable(Engine::Docker) => Ok(Engine::Docker),
            "podman" if engine_usable(Engine::Podman) => Ok(Engine::Podman),
            "docker" | "podman" => Err(BackendError::environment(format!(
                "requested OCI runtime {requested} is not usable"
            ))),
            _ => Err(BackendError::input(
                "AYNI_OCI_RUNTIME must be `docker` or `podman`",
            )),
        };
    }
    if engine_usable(Engine::Docker) {
        return Ok(Engine::Docker);
    }
    if engine_usable(Engine::Podman) {
        return Ok(Engine::Podman);
    }
    Err(BackendError::environment(
        "no compatible OCI runtime found (tried Docker, then Podman)",
    ))
}

fn engine_usable(engine: Engine) -> bool {
    let args = match engine {
        Engine::Docker => vec![
            "version".to_owned(),
            "--format".to_owned(),
            "{{.Server.Version}}".to_owned(),
        ],
        Engine::Podman => vec!["info".to_owned(), "--format".to_owned(), "json".to_owned()],
    };
    env::current_dir()
        .ok()
        .and_then(|cwd| run_command(&cwd, engine_name(engine), &args, COMMAND_TIMEOUT).ok())
        .is_some_and(|output| output.status.success())
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Docker => "docker",
        Engine::Podman => "podman",
    }
}

pub fn doctor(repo_root: &Path) -> Result<String, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan(&lock)?;
    let engine = detect_engine()?;
    validate_image(engine, &plan, &lock)?;
    Ok(format!(
        "environment ready: {} ({})",
        plan.tag,
        engine_name(engine)
    ))
}

pub fn build(repo_root: &Path) -> Result<String, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan(&lock)?;
    let engine = detect_engine()?;
    if validate_image(engine, &plan, &lock).is_ok() {
        return Ok(format!("current {}", plan.tag));
    }
    let input = BuildInput::create(&plan)?;
    let args = vec![
        "build".to_owned(),
        "--tag".to_owned(),
        plan.tag.clone(),
        "--platform".to_owned(),
        plan.platform.clone(),
        "--file".to_owned(),
        input.path.join("Dockerfile").to_string_lossy().into_owned(),
        input.path.to_string_lossy().into_owned(),
    ];
    let output = run_command(
        &input.path,
        engine_name(engine),
        &args,
        DEFAULT_TOOL_TIMEOUT,
    )
    .map_err(|error| {
        BackendError::execution(format!(
            "failed to run {} build: {error}",
            engine_name(engine)
        ))
    })?;
    if !output.status.success() {
        return Err(BackendError::execution(format!(
            "{} build failed: {}",
            engine_name(engine),
            concise_output(&output.stderr)
        )));
    }
    validate_image(engine, &plan, &lock)?;
    Ok(format!("built {}", plan.tag))
}

struct BuildInput {
    path: PathBuf,
}

impl BuildInput {
    fn create(plan: &ImagePlan) -> Result<Self, BackendError> {
        for attempt in 0..100 {
            let path = env::temp_dir().join(format!(
                "ayni-env-{}-{}-{attempt}",
                std::process::id(),
                plan.tag.strip_prefix("ayni-env:").unwrap_or("build")
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    if let Err(error) = restrict_directory(&path)
                        .and_then(|()| write_new_file(&path.join("Dockerfile"), &plan.dockerfile))
                        .and_then(|()| write_new_file(&path.join("mise.toml"), &plan.mise_toml))
                    {
                        let _ = fs::remove_dir_all(&path);
                        return Err(BackendError::execution(format!(
                            "failed to write generated build input: {error}"
                        )));
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(BackendError::execution(format!(
                        "failed to create generated build input: {error}"
                    )));
                }
            }
        }
        Err(BackendError::execution(
            "failed to allocate a unique generated build input",
        ))
    }
}

impl Drop for BuildInput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn restrict_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn write_new_file(path: &Path, content: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()
}

fn validate_image(
    engine: Engine,
    plan: &ImagePlan,
    lock: &EnvironmentLock,
) -> Result<(), BackendError> {
    let args = vec![
        "image".to_owned(),
        "inspect".to_owned(),
        "--format".to_owned(),
        "{{json .Config.Labels}}".to_owned(),
        plan.tag.clone(),
    ];
    let cwd = env::current_dir().map_err(|error| {
        BackendError::execution(format!("failed to establish current directory: {error}"))
    })?;
    let output = run_command(&cwd, engine_name(engine), &args, COMMAND_TIMEOUT).map_err(|_| {
        BackendError::environment(format!(
            "environment image {} is missing; run `ayni env build`",
            plan.tag
        ))
    })?;
    if !output.status.success() {
        return Err(BackendError::environment(format!(
            "environment image {} is missing; run `ayni env build`",
            plan.tag
        )));
    }
    let labels: BTreeMap<String, String> =
        serde_json::from_slice(&output.stdout).map_err(|error| {
            BackendError::environment(format!(
                "environment image {} has invalid labels: {error}; run `ayni env build`",
                plan.tag
            ))
        })?;
    let current = labels
        .get(IMAGE_LOCK_LABEL)
        .is_some_and(|value| value == lock.fingerprint())
        && labels
            .get(IMAGE_BASE_LABEL)
            .is_some_and(|value| value == &lock.provisioning_base().digest)
        && labels
            .get(IMAGE_SCHEMA_LABEL)
            .is_some_and(|value| value == IMAGE_SCHEMA_VERSION)
        && labels
            .get(IMAGE_AYNI_LABEL)
            .is_some_and(|value| value == lock.ayni_version())
        && labels
            .get(IMAGE_MISE_LABEL)
            .is_some_and(|value| value == &lock.provisioning_base().mise_version)
        && labels
            .get(IMAGE_PLATFORM_LABEL)
            .is_some_and(|value| value == &plan.platform);
    if current {
        Ok(())
    } else {
        Err(BackendError::environment(format!(
            "environment image {} is stale; run `ayni env build`",
            plan.tag
        )))
    }
}

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

pub fn launch(
    repo_root: &Path,
    selection: &TargetSelection,
    command: &[String],
    shell: bool,
) -> Result<i32, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan(&lock)?;
    let engine = detect_engine()?;
    validate_image(engine, &plan, &lock)?;
    let target = select_target(&lock, selection)?;
    let state_home = execution_state(&root, lock.fingerprint())?;
    let args = launch_args(
        &root,
        engine,
        target,
        &state_home,
        &plan.tag,
        command,
        shell,
    )?;
    execute_launch(engine, &args)
}

fn launch_args(
    root: &Path,
    engine: Engine,
    target: &LockedTargetEnvironment,
    state_home: &str,
    image_tag: &str,
    command: &[String],
    shell: bool,
) -> Result<Vec<String>, BackendError> {
    let mut args = base_launch_args(engine);
    append_workspace_args(&mut args, root, target, state_home);
    append_target_environment(&mut args, target)?;
    append_command(&mut args, image_tag, command, shell)?;
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
    args.extend([
        "--volume".into(),
        format!("{}:{WORKSPACE}:rw", root.display()),
        "--workdir".into(),
        target_workdir,
        "--env".into(),
        format!("HOME={state_home}"),
        "--env".into(),
        format!("XDG_CACHE_HOME={state_home}/.cache"),
        "--env".into(),
        format!("CARGO_HOME={state_home}/.cache/cargo"),
        "--env".into(),
        format!("MISE_CACHE_DIR={state_home}/.cache/mise"),
        "--env".into(),
        "RUSTUP_HOME=/home/ayni/.rustup".into(),
        "--env".into(),
        format!("npm_config_cache={state_home}/.cache/npm"),
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

fn target_environment(
    target: &LockedTargetEnvironment,
) -> Result<Vec<(String, String)>, BackendError> {
    let mut variables = BTreeMap::new();
    for runtime in &target.runtimes {
        variables.insert(
            mise_version_variable(&runtime.runtime)?,
            runtime.version.clone(),
        );
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
