use crate::image::image_plan_with_preparation;
use crate::{BackendError, read_lock};
use ayni_core::{
    ArtifactToolVersion, DependencyPreparationPlan, DockerAccess, EnvironmentCapabilities,
    EnvironmentLock, LockedTargetEnvironment, NetworkAccess, TargetIdentity,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const WORKSPACE: &str = "/workspace";
const CHECKOUT_SOURCE: &str = "/opt/ayni/checkout";
const MANAGED_WORKSPACE_INIT: &str = concat!(
    "set -eu\n",
    "archive_dir=$(mktemp -d /tmp/ayni-workspace.XXXXXX)\n",
    "archive_fifo=$archive_dir/archive\n",
    "mkfifo \"$archive_fifo\"\n",
    "trap 'rm -rf \"$archive_dir\"' EXIT HUP INT TERM\n",
    "(cd /opt/ayni/checkout; set --; ",
    "for entry in ./* ./.[!.]* ./..?*; do ",
    "if [ -e \"$entry\" ] || [ -L \"$entry\" ]; then set -- \"$@\" \"$entry\"; fi; ",
    "done; ",
    "if [ \"$#\" -eq 0 ]; then tar -cf - --files-from /dev/null; ",
    "else tar --exclude='./.ayni' --exclude='./.git' ",
    "--exclude='./node_modules' --exclude='*/node_modules' ",
    "--exclude='./target' --exclude='*/target' ",
    "--exclude='./.venv' --exclude='*/.venv' ",
    "--exclude='./build' --exclude='*/build' ",
    "--exclude='./coverage' --exclude='*/coverage' ",
    "--exclude='./.gradle' --exclude='*/.gradle' ",
    "--exclude='./.svelte-kit' --exclude='*/.svelte-kit' ",
    "--exclude='./__pycache__' --exclude='*/__pycache__' -cf - \"$@\"; fi) ",
    "> \"$archive_fifo\" &\n",
    "producer=$!\n",
    "consumer_status=0\n",
    "(cd /workspace && tar -xf - --no-same-owner --no-same-permissions --no-overwrite-dir ",
    "< \"$archive_fifo\") || consumer_status=$?\n",
    "producer_status=0\n",
    "wait \"$producer\" || producer_status=$?\n",
    "rm -rf \"$archive_dir\"\n",
    "trap - EXIT HUP INT TERM\n",
    "[ \"$producer_status\" -eq 0 ]\n",
    "[ \"$consumer_status\" -eq 0 ]\n",
    "mount_count=$1\n",
    "shift\n",
    "mount_index=0\n",
    "while [ \"$mount_index\" -lt \"$mount_count\" ]; do\n",
    "destination=$1\n",
    "shift\n",
    "case \"$destination\" in /workspace/*) ;; *) exit 64;; esac\n",
    "if [ -e \"$destination\" ] || [ -L \"$destination\" ]; then exit 64; fi\n",
    "mkdir -p \"$(dirname \"$destination\")\"\n",
    "source=/opt/ayni/prepared-$mount_index/${destination#/workspace/}\n",
    "ln -s \"$source\" \"$destination\"\n",
    "mount_index=$((mount_index + 1))\n",
    "done\n",
    "[ \"$1\" = -- ]\n",
    "shift\n",
    "exec /usr/local/bin/ayni \"$@\"",
);

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
    let tool_versions = managed_tool_versions(&lock)?;
    let args = repository_launch_args(RepositoryLaunch {
        root: &root,
        engine,
        state_home: &state_home,
        image_tag: &plan.tag,
        command,
        mounts: &mounts,
        managed_environments: Some(&managed_environments),
        capabilities: lock.capabilities(),
        lock_fingerprint: lock.fingerprint(),
        tool_versions: &tool_versions,
    })?;
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
        capabilities: lock.capabilities(),
    })?;
    execute_launch(engine, &args)
}

mod materialization;
use materialization::materialize_outputs;

struct RepositoryLaunch<'a> {
    root: &'a Path,
    engine: Engine,
    state_home: &'a str,
    image_tag: &'a str,
    command: &'a [String],
    mounts: &'a [(PathBuf, String)],
    managed_environments: Option<&'a str>,
    capabilities: EnvironmentCapabilities,
    lock_fingerprint: &'a str,
    tool_versions: &'a str,
}

fn repository_launch_args(request: RepositoryLaunch<'_>) -> Result<Vec<String>, BackendError> {
    let mut args = base_launch_args(request.engine, request.capabilities)?;
    append_managed_workspace_state_args(&mut args, request.root, WORKSPACE, request.state_home);
    let workspace_mounts = append_managed_prepared_mounts(&mut args, request.mounts);
    if let Some(value) = request.managed_environments {
        args.extend([
            "--env".into(),
            format!("AYNI_MANAGED_TARGET_ENVIRONMENTS={value}"),
        ]);
    }
    args.extend([
        "--env".into(),
        format!("AYNI_MANAGED_LOCK_FINGERPRINT={}", request.lock_fingerprint),
        "--env".into(),
        format!("AYNI_MANAGED_TOOL_VERSIONS={}", request.tool_versions),
    ]);
    args.extend([
        "--entrypoint".into(),
        "/bin/sh".into(),
        request.image_tag.to_owned(),
        "-c".into(),
        MANAGED_WORKSPACE_INIT.into(),
        "ayni-managed-workspace".into(),
        workspace_mounts.len().to_string(),
    ]);
    args.extend(workspace_mounts);
    args.push("--".into());
    args.extend(request.command.iter().cloned());
    Ok(args)
}

fn managed_tool_versions(lock: &EnvironmentLock) -> Result<String, BackendError> {
    let mut versions = vec![ArtifactToolVersion {
        tool: String::from("mise"),
        version: lock.mise_version().to_owned(),
    }];
    for target in lock.targets() {
        let prefix = format!("{}:{}", target.target.language.as_str(), target.target.root);
        versions.extend(target.runtimes.iter().map(|runtime| ArtifactToolVersion {
            tool: format!("{prefix}:runtime:{}", runtime.runtime),
            version: runtime.version.clone(),
        }));
        if let Some(manager) = &target.package_manager {
            versions.push(ArtifactToolVersion {
                tool: format!("{prefix}:package_manager:{}", manager.family),
                version: manager.version.clone(),
            });
        }
        versions.extend(target.signal_tools.iter().map(|tool| ArtifactToolVersion {
            tool: format!("{prefix}:signal_tool:{}", tool.tool),
            version: tool.version.clone(),
        }));
    }
    versions.extend(lock.tools().iter().map(|tool| ArtifactToolVersion {
        tool: format!("repository_tool:{}", tool.tool),
        version: tool.version.clone(),
    }));
    versions.sort();
    versions.dedup();
    serde_json::to_string(&versions).map_err(|error| {
        BackendError::environment(format!(
            "failed to serialize managed tool provenance: {error}"
        ))
    })
}

fn append_managed_prepared_mounts(
    args: &mut Vec<String>,
    mounts: &[(PathBuf, String)],
) -> Vec<String> {
    let mut workspace_mounts = Vec::new();
    for (source, destination) in mounts {
        let target = if let Some(relative) = destination.strip_prefix(&format!("{WORKSPACE}/")) {
            let target = format!("/opt/ayni/prepared-{}/{relative}", workspace_mounts.len());
            workspace_mounts.push(destination.clone());
            target
        } else {
            destination.clone()
        };
        args.extend([
            "--mount".into(),
            format!("type=bind,source={},target={target}", source.display()),
        ]);
    }
    workspace_mounts
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
    capabilities: EnvironmentCapabilities,
}

fn launch_args(request: TargetLaunch<'_>) -> Result<Vec<String>, BackendError> {
    let mut args = base_launch_args(request.engine, request.capabilities)?;
    append_workspace_args(&mut args, request.root, request.target, request.state_home);
    append_prepared_mounts(&mut args, request.mounts);
    append_target_environment(&mut args, request.target)?;
    for (name, value) in request.execution_environment {
        args.extend(["--env".into(), format!("{name}={value}")]);
    }
    append_command(&mut args, request.image_tag, request.command, request.shell)?;
    Ok(args)
}

fn base_launch_args(
    engine: Engine,
    capabilities: EnvironmentCapabilities,
) -> Result<Vec<String>, BackendError> {
    let network = match capabilities.network {
        NetworkAccess::None => "none",
        NetworkAccess::Bridge => "bridge",
    };
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "--network".into(),
        network.into(),
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
    append_docker_socket_args(&mut args, engine, capabilities.docker)?;
    Ok(args)
}

fn validate_runtime_capabilities(
    engine: Engine,
    capabilities: EnvironmentCapabilities,
) -> Result<(), BackendError> {
    if capabilities.docker == DockerAccess::Socket {
        if engine != Engine::Docker {
            return Err(BackendError::environment(
                "Docker socket access currently requires the Docker runtime",
            ));
        }
        docker_socket_path()?;
    }
    Ok(())
}

fn append_docker_socket_args(
    args: &mut Vec<String>,
    engine: Engine,
    access: DockerAccess,
) -> Result<(), BackendError> {
    if access == DockerAccess::None {
        return Ok(());
    }
    if engine != Engine::Docker {
        return Err(BackendError::environment(
            "Docker socket access currently requires the Docker runtime",
        ));
    }
    let socket = docker_socket_path()?;
    // Docker Desktop projects its host socket into the Linux VM as root:root,
    // regardless of the macOS socket's owner/group metadata. Native Unix
    // engines preserve the socket GID and must use that group instead.
    #[cfg(target_os = "macos")]
    let gid = Some(0);
    #[cfg(all(unix, not(target_os = "macos")))]
    let gid = {
        use std::os::unix::fs::MetadataExt;
        Some(
            fs::metadata(&socket)
                .map_err(|error| {
                    BackendError::environment(format!(
                        "failed to inspect Docker socket {}: {error}",
                        socket.display()
                    ))
                })?
                .gid(),
        )
    };
    #[cfg(not(unix))]
    let gid = None;
    append_known_docker_socket_args(args, &socket, gid);
    Ok(())
}

fn append_known_docker_socket_args(args: &mut Vec<String>, socket: &Path, gid: Option<u32>) {
    args.extend([
        "--mount".into(),
        format!(
            "type=bind,source={},target=/var/run/docker.sock",
            socket.display()
        ),
        "--env".into(),
        "DOCKER_HOST=unix:///var/run/docker.sock".into(),
        "--env".into(),
        "TESTCONTAINERS_HOST_OVERRIDE=host.docker.internal".into(),
        "--env".into(),
        "TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock".into(),
        "--add-host".into(),
        "host.docker.internal:host-gateway".into(),
    ]);
    if let Some(gid) = gid {
        args.extend(["--group-add".into(), gid.to_string()]);
    }
}

fn active_docker_context_host() -> Option<String> {
    let output = Command::new("docker")
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let host = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!host.is_empty()).then_some(host)
}

fn docker_unix_socket_path(host: Option<&str>) -> Result<PathBuf, BackendError> {
    match host {
        Some(host) if host.starts_with("unix://") => Ok(PathBuf::from(&host[7..])),
        Some(host) => Err(BackendError::environment(format!(
            "Docker socket access requires a unix Docker context, found {host}"
        ))),
        None => Ok(PathBuf::from("/var/run/docker.sock")),
    }
}

fn docker_socket_path() -> Result<PathBuf, BackendError> {
    let configured = std::env::var("DOCKER_HOST")
        .ok()
        .or_else(active_docker_context_host);
    let path = docker_unix_socket_path(configured.as_deref())?;
    if fs::metadata(&path).is_ok() {
        Ok(path)
    } else {
        Err(BackendError::environment(format!(
            "Docker socket is unavailable at {}",
            path.display()
        )))
    }
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

/// Managed quality commands must not modify checkout source files. The checkout
/// is mounted separately as read-only input and copied into an ephemeral tmpfs;
/// prepared dependency mounts can therefore create missing nested mountpoints
/// without writing empty directories into a clean checkout. The nested `.ayni`
/// bind mount deliberately remains writable for signal artifacts, environment
/// state, and tool caches.
fn append_managed_workspace_state_args(
    args: &mut Vec<String>,
    root: &Path,
    workdir: &str,
    _state_home: &str,
) {
    args.extend([
        "--mount".into(),
        format!(
            "type=bind,source={},target={CHECKOUT_SOURCE},readonly",
            root.display()
        ),
        "--tmpfs".into(),
        format!("{WORKSPACE}:rw,exec,nosuid,size=4g,mode=1777"),
        "--mount".into(),
        format!(
            "type=bind,source={},target={WORKSPACE}/.ayni",
            root.join(".ayni").display()
        ),
        "--tmpfs".into(),
        format!("{WORKSPACE}/.ayni/environment:rw,exec,nosuid,size=6g,mode=1777"),
    ]);
    append_workspace_execution_settings(args, workdir, "/tmp");
}

fn append_workspace_execution_settings(args: &mut Vec<String>, workdir: &str, state_home: &str) {
    args.extend([
        "--workdir".into(),
        workdir.to_owned(),
        "--env".into(),
        format!("HOME={state_home}"),
        "--env".into(),
        format!("XDG_STATE_HOME={state_home}/.local/state"),
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
        // npm and pnpm are installed into the selected Node runtime rather than
        // as independent Mise tools. The Node version therefore selects the
        // matching package-manager installation as well.
        if manager.family != "npm" && manager.family != "pnpm" && manager.family != "gradle" {
            variables.insert(
                mise_version_variable(&manager.family)?,
                manager.version.clone(),
            );
        }
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
            Language, LockedPackageManager, LockedRequirementSource, LockedRuntime,
            RequirementConfidence, TargetIdentity,
        };
        let source = LockedRequirementSource {
            kind: "test".into(),
            path: "gradle/wrapper/gradle-wrapper.properties".into(),
            digest: None,
            confidence: RequirementConfidence::Exact,
        };
        let target = LockedTargetEnvironment {
            target: TargetIdentity::new(Language::Kotlin, ".").expect("target"),
            runtimes: vec![LockedRuntime {
                runtime: "java".into(),
                version: "temurin-21.0.6+7".into(),
                components: Vec::new(),
                targets: Vec::new(),
                source: source.clone(),
            }],
            package_manager: Some(LockedPackageManager {
                family: "gradle".into(),
                version: "9.5.0".into(),
                ownership_root: ".".into(),
                source,
            }),
            signal_tools: Vec::new(),
            dependency_locks: Vec::new(),
        };
        let environment = target_environment(&target).expect("activation");
        assert!(environment.contains(&("MISE_JAVA_VERSION".into(), "temurin-21.0.6+7".into())));
        assert!(environment.contains(&(
            "JAVA_HOME".into(),
            "/opt/ayni/mise/installs/java/temurin-21.0.6+7".into()
        )));
        assert!(
            !environment
                .iter()
                .any(|(name, _)| name == "MISE_GRADLE_VERSION")
        );
    }

    #[test]
    fn node_package_manager_activation_follows_the_selected_node_runtime() {
        use ayni_core::{
            Language, LockedPackageManager, LockedRequirementSource, LockedRuntime,
            RequirementConfidence, TargetIdentity,
        };
        let source = LockedRequirementSource {
            kind: "test".into(),
            path: "package.json".into(),
            digest: None,
            confidence: RequirementConfidence::Exact,
        };
        let target = LockedTargetEnvironment {
            target: TargetIdentity::new(Language::Node, ".").expect("target"),
            runtimes: vec![LockedRuntime {
                runtime: "node".into(),
                version: "24.14.0".into(),
                components: Vec::new(),
                targets: Vec::new(),
                source: source.clone(),
            }],
            package_manager: Some(LockedPackageManager {
                family: "pnpm".into(),
                version: "11.15.1".into(),
                ownership_root: ".".into(),
                source,
            }),
            signal_tools: Vec::new(),
            dependency_locks: Vec::new(),
        };

        let environment = target_environment(&target).expect("activation");
        assert!(environment.contains(&("MISE_NODE_VERSION".into(), "24.14.0".into())));
        assert!(
            !environment
                .iter()
                .any(|(name, _)| name == "MISE_PNPM_VERSION")
        );
    }

    #[test]
    fn docker_context_host_selects_only_unix_sockets() {
        assert_eq!(
            docker_unix_socket_path(Some("unix:///home/user/.docker/run/docker.sock"))
                .expect("unix context"),
            PathBuf::from("/home/user/.docker/run/docker.sock")
        );
        assert!(docker_unix_socket_path(Some("tcp://remote:2376")).is_err());
        assert_eq!(
            docker_unix_socket_path(None).expect("default socket"),
            PathBuf::from("/var/run/docker.sock")
        );
    }

    #[test]
    fn docker_socket_args_are_explicit_and_testcontainers_aware() {
        let mut args = Vec::new();
        append_known_docker_socket_args(&mut args, Path::new("/host/docker.sock"), Some(123));
        assert!(args.iter().any(|arg| {
            arg == "type=bind,source=/host/docker.sock,target=/var/run/docker.sock"
        }));
        assert!(
            args.iter()
                .any(|arg| arg == "DOCKER_HOST=unix:///var/run/docker.sock")
        );
        assert!(
            args.iter()
                .any(|arg| arg == "TESTCONTAINERS_HOST_OVERRIDE=host.docker.internal")
        );
        assert!(
            args.iter()
                .any(|arg| arg == "TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock")
        );
        assert!(args.windows(2).any(|pair| pair == ["--group-add", "123"]));
    }

    #[test]
    fn bridge_network_is_explicit_while_socket_access_remains_opt_in() {
        let args = base_launch_args(
            Engine::Docker,
            EnvironmentCapabilities {
                docker: DockerAccess::None,
                network: NetworkAccess::Bridge,
            },
        )
        .expect("launch args");
        assert!(args.windows(2).any(|pair| pair == ["--network", "bridge"]));
        assert!(!args.iter().any(|arg| arg.contains("docker.sock")));
        assert!(args.windows(2).any(|pair| pair == ["--cap-drop", "ALL"]));
    }

    #[test]
    fn repository_launch_keeps_target_activation_inside_ayni() {
        let mounts = [
            (
                PathBuf::from("/state/cache"),
                String::from("/home/ayni/.cache"),
            ),
            (
                PathBuf::from("/state/dependencies"),
                String::from("/workspace/packages/member/node_modules"),
            ),
        ];
        let args = repository_launch_args(RepositoryLaunch {
            root: Path::new("/checkout"),
            engine: Engine::Docker,
            state_home: "/workspace/.ayni/environment/state/home",
            image_tag: "ayni-env:test",
            command: &["check".into(), "--host".into()],
            mounts: &mounts,
            managed_environments: None,
            capabilities: EnvironmentCapabilities::default(),
            lock_fingerprint: "sha256:test",
            tool_versions: "[]",
        })
        .expect("launch args");
        assert!(args.windows(2).any(|pair| pair == ["--network", "none"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--workdir", "/workspace"])
        );
        assert!(
            args.iter().any(|arg| {
                arg == "type=bind,source=/checkout,target=/opt/ayni/checkout,readonly"
            })
        );
        assert!(
            args.iter()
                .any(|arg| arg == "/workspace:rw,exec,nosuid,size=4g,mode=1777")
        );
        assert!(
            args.iter()
                .any(|arg| { arg == "type=bind,source=/checkout/.ayni,target=/workspace/.ayni" })
        );
        assert!(!args.iter().any(|arg| arg == "/checkout:/workspace:rw"));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--entrypoint", "/bin/sh"])
        );
        assert!(args.iter().any(|arg| arg == MANAGED_WORKSPACE_INIT));
        assert!(
            args.iter()
                .any(|arg| arg == "AYNI_MANAGED_LOCK_FINGERPRINT=sha256:test")
        );
        assert!(
            args.iter()
                .any(|arg| arg == "AYNI_MANAGED_TOOL_VERSIONS=[]")
        );
        assert!(
            args.iter()
                .any(|arg| { arg == "type=bind,source=/state/cache,target=/home/ayni/.cache" })
        );
        assert!(args.iter().any(|arg| {
            arg == "type=bind,source=/state/dependencies,target=/opt/ayni/prepared-0/packages/member/node_modules"
        }));
        assert!(
            !args
                .iter()
                .any(|arg| { arg.contains("target=/workspace/packages/member/node_modules") })
        );
        assert!(
            args.iter().any(|arg| {
                arg == "/workspace/.ayni/environment:rw,exec,nosuid,size=6g,mode=1777"
            })
        );
        assert!(args.iter().any(|arg| arg == "HOME=/tmp"));
        assert!(
            args.iter()
                .any(|arg| arg == "XDG_STATE_HOME=/tmp/.local/state")
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("MISE_") && arg.ends_with("_VERSION"))
        );
        assert!(args.ends_with(&[
            "ayni-env:test".into(),
            "-c".into(),
            MANAGED_WORKSPACE_INIT.into(),
            "ayni-managed-workspace".into(),
            "1".into(),
            "/workspace/packages/member/node_modules".into(),
            "--".into(),
            "check".into(),
            "--host".into(),
        ]));
    }
}
