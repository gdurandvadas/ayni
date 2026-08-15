use crate::image::{
    IMAGE_AYNI_LABEL, IMAGE_BASE_LABEL, IMAGE_LOCK_LABEL, IMAGE_MISE_LABEL, IMAGE_PLATFORM_LABEL,
    IMAGE_PREPARATION_LABEL, IMAGE_SCHEMA_LABEL, IMAGE_SCHEMA_VERSION, ImagePlan,
    image_plan_with_preparation,
};
use crate::{BackendError, concise_output, read_lock};
use ayni_adapters_common::exec::{DEFAULT_TOOL_TIMEOUT, run_command, run_command_streaming};
use ayni_core::{
    DependencyPreparationPlan, EnvironmentLock, Language, LockedTargetEnvironment,
    PreparationOutputMode, TargetIdentity,
};
use std::collections::BTreeMap;
use std::env;
#[cfg(unix)]
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
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
    doctor_prepared(repo_root, &[])
}

pub fn doctor_prepared(
    repo_root: &Path,
    preparations: &[DependencyPreparationPlan],
) -> Result<String, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan_with_preparation(&lock, preparations)?;
    let engine = detect_engine()?;
    validate_image(engine, &plan, &lock)?;
    Ok(format!(
        "environment ready: {} ({})",
        plan.tag,
        engine_name(engine)
    ))
}

pub fn build(repo_root: &Path) -> Result<String, BackendError> {
    build_prepared(repo_root, &[])
}

pub fn build_prepared(
    repo_root: &Path,
    preparations: &[DependencyPreparationPlan],
) -> Result<String, BackendError> {
    let root = canonical_root(repo_root)?;
    let lock = read_lock(&root)?;
    let plan = image_plan_with_preparation(&lock, preparations)?;
    let engine = detect_engine()?;
    if validate_image(engine, &plan, &lock).is_ok() {
        return Ok(format!("current {}", plan.tag));
    }
    let input = BuildInput::create(&root, &plan, preparations)?;
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
    let output = run_command_streaming(
        &input.path,
        engine_name(engine),
        &args,
        DEFAULT_TOOL_TIMEOUT,
        |line| eprintln!("{line}"),
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
    fn create(
        repo_root: &Path,
        plan: &ImagePlan,
        preparations: &[DependencyPreparationPlan],
    ) -> Result<Self, BackendError> {
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
                    if !preparations.is_empty()
                        && let Err(error) =
                            crate::preparation::stage_inputs(repo_root, &path, preparations)
                    {
                        let _ = fs::remove_dir_all(&path);
                        return Err(error);
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
            .is_some_and(|value| value == &plan.platform)
        && labels
            .get(IMAGE_PREPARATION_LABEL)
            .is_some_and(|value| value == &plan.preparation_digest);
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

fn materialize_outputs(
    root: &Path,
    engine: Engine,
    lock: &EnvironmentLock,
    image_plan: &ImagePlan,
    preparations: &[DependencyPreparationPlan],
) -> Result<Vec<(PathBuf, String)>, BackendError> {
    let fingerprint = lock
        .fingerprint()
        .strip_prefix("sha256:")
        .unwrap_or(lock.fingerprint());
    let preparation = image_plan
        .preparation_digest
        .strip_prefix("sha256:")
        .unwrap_or(&image_plan.preparation_digest);
    let state_root = PathBuf::from(".ayni/environment")
        .join(&fingerprint[..16.min(fingerprint.len())])
        .join(&preparation[..16.min(preparation.len())]);
    let cache_destination = materialize_cache(root, engine, image_plan, &state_root)?;
    let mut mounts = vec![(cache_destination.clone(), String::from("/home/ayni/.cache"))];
    for output in crate::preparation::unique_outputs(preparations) {
        let key = crate::preparation::output_key(&output);
        let destination = materialize_output(OutputMaterialization {
            root,
            engine,
            lock,
            image_plan,
            preparations,
            state_root: &state_root,
            cache_state: &cache_destination,
            key: &key,
            output: &output,
        })?;
        mounts.push((destination, crate::preparation::workspace_mount(&output)));
    }
    Ok(mounts)
}

fn materialize_cache(
    root: &Path,
    engine: Engine,
    image_plan: &ImagePlan,
    state_root: &Path,
) -> Result<PathBuf, BackendError> {
    let parent_relative = state_root.join("cache");
    create_contained_directory_tree(root, &parent_relative)?;
    let destination = root.join(&parent_relative).join("content");
    let marker = root.join(state_root).join("cache.complete");
    if materialization_marker_current(root, &marker, &image_plan.preparation_digest)? {
        validate_materialized_directory(root, &destination)?;
        return Ok(destination);
    }
    let _lock = MaterializationLock::acquire(
        root,
        &marker.with_extension("lock"),
        &marker,
        &image_plan.preparation_digest,
    )?;
    if materialization_marker_current(root, &marker, &image_plan.preparation_digest)? {
        validate_materialized_directory(root, &destination)?;
        return Ok(destination);
    }
    reject_partial_materialization(&destination)?;
    let staging = StagingDirectory::create(destination.parent().expect("cache parent"))?;
    copy_image_tree(
        root,
        engine,
        &image_plan.tag,
        "/home/ayni/.cache/.",
        staging.path(),
        "/tmp/ayni/cache",
        "prepared tool cache",
    )?;
    staging.publish(&destination)?;
    write_completion_marker(root, &marker, &image_plan.preparation_digest)?;
    Ok(destination)
}

struct OutputMaterialization<'a> {
    root: &'a Path,
    engine: Engine,
    lock: &'a EnvironmentLock,
    image_plan: &'a ImagePlan,
    preparations: &'a [DependencyPreparationPlan],
    state_root: &'a Path,
    cache_state: &'a Path,
    key: &'a str,
    output: &'a ayni_core::PreparationOutput,
}

fn materialize_output(request: OutputMaterialization<'_>) -> Result<PathBuf, BackendError> {
    let parent_relative = request.state_root.join("dependencies").join(request.key);
    create_contained_directory_tree(request.root, &parent_relative)?;
    let destination = request.root.join(&parent_relative).join("content");
    let marker = request
        .root
        .join(request.state_root)
        .join("dependencies")
        .join(format!("{}.complete", request.key));
    if reuse_materialization(
        request.root,
        &marker,
        &request.image_plan.preparation_digest,
        &destination,
    )? {
        return Ok(destination);
    }
    let _lock = MaterializationLock::acquire(
        request.root,
        &marker.with_extension("lock"),
        &marker,
        &request.image_plan.preparation_digest,
    )?;
    if reuse_materialization(
        request.root,
        &marker,
        &request.image_plan.preparation_digest,
        &destination,
    )? {
        return Ok(destination);
    }
    let staging = stage_dependency_output(&request, &destination)?;
    staging.publish(&destination)?;
    write_completion_marker(
        request.root,
        &marker,
        &request.image_plan.preparation_digest,
    )?;
    Ok(destination)
}

fn reuse_materialization(
    root: &Path,
    marker: &Path,
    expected: &str,
    destination: &Path,
) -> Result<bool, BackendError> {
    if !materialization_marker_current(root, marker, expected)? {
        return Ok(false);
    }
    validate_materialized_directory(root, destination)?;
    Ok(true)
}

fn stage_dependency_output(
    request: &OutputMaterialization<'_>,
    destination: &Path,
) -> Result<StagingDirectory, BackendError> {
    reject_partial_materialization(destination)?;
    let staging = StagingDirectory::create(destination.parent().expect("dependency parent"))?;
    if request.output.mode == PreparationOutputMode::Seeded {
        copy_image_tree(
            request.root,
            request.engine,
            &request.image_plan.tag,
            &format!("{}/{}/.", crate::preparation::SEED_ROOT, request.key),
            staging.path(),
            "/tmp/ayni/dependencies",
            "locked dependencies",
        )?;
    }
    rebuild_dependency_output(request, staging.path())?;
    Ok(staging)
}

fn rebuild_dependency_output(
    request: &OutputMaterialization<'_>,
    output_state: &Path,
) -> Result<(), BackendError> {
    let Some(preparation) = request
        .preparations
        .iter()
        .find(|preparation| preparation.outputs.contains(request.output))
    else {
        return Ok(());
    };
    run_materialization_commands(MaterializationRequest {
        root: request.root,
        engine: request.engine,
        lock: request.lock,
        image_tag: &request.image_plan.tag,
        preparation,
        cache_state: request.cache_state,
        output_state,
        output: request.output,
    })
}

fn reject_partial_materialization(destination: &Path) -> Result<(), BackendError> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(BackendError::execution(format!(
            "incomplete managed dependency state must be removed before retrying: {}",
            destination.display()
        ))),
        Err(error) => Err(BackendError::execution(format!(
            "failed to inspect managed dependency state {}: {error}",
            destination.display()
        ))),
    }
}

fn validate_materialized_directory(root: &Path, destination: &Path) -> Result<(), BackendError> {
    let metadata = fs::symlink_metadata(destination).map_err(|error| {
        BackendError::execution(format!(
            "failed to inspect managed dependency state {}: {error}",
            destination.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BackendError::execution(format!(
            "managed dependency state must be a directory: {}",
            destination.display()
        )));
    }
    let canonical = destination.canonicalize().map_err(|error| {
        BackendError::execution(format!(
            "failed to validate managed dependency state {}: {error}",
            destination.display()
        ))
    })?;
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err(BackendError::execution(format!(
            "managed dependency state escapes the repository: {}",
            destination.display()
        )))
    }
}

struct StagingDirectory {
    path: Option<PathBuf>,
}

impl StagingDirectory {
    fn create(parent: &Path) -> Result<Self, BackendError> {
        for attempt in 0..100 {
            let path = parent.join(format!(".materializing-{}-{attempt}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path: Some(path) }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(BackendError::execution(format!(
                        "failed to create dependency staging directory: {error}"
                    )));
                }
            }
        }
        Err(BackendError::execution(
            "failed to allocate dependency staging directory",
        ))
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("unpublished staging path")
    }

    fn publish(mut self, destination: &Path) -> Result<(), BackendError> {
        let source = self.path.take().expect("unpublished staging path");
        if let Err(error) = fs::rename(&source, destination) {
            let _ = fs::remove_dir_all(&source);
            return Err(BackendError::execution(format!(
                "failed to publish dependency materialization: {error}"
            )));
        }
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_dir_all(path);
        }
    }
}

struct MaterializationLock {
    path: PathBuf,
}

impl MaterializationLock {
    fn acquire(
        root: &Path,
        path: &Path,
        marker: &Path,
        expected: &str,
    ) -> Result<Self, BackendError> {
        for _ in 0..100 {
            if materialization_marker_current(root, marker, expected)? {
                return Ok(Self {
                    path: PathBuf::new(),
                });
            }
            match write_new_file(path, "") {
                Ok(()) => {
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    return Err(BackendError::execution(format!(
                        "failed to acquire dependency materialization lock {}: {error}",
                        path.display()
                    )));
                }
            }
        }
        Err(BackendError::execution(format!(
            "dependency materialization is already running or left a stale lock: {}",
            path.display()
        )))
    }
}

impl Drop for MaterializationLock {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn copy_image_tree(
    root: &Path,
    engine: Engine,
    image_tag: &str,
    source: &str,
    destination: &Path,
    container_destination: &str,
    description: &str,
) -> Result<(), BackendError> {
    let mut args = base_launch_args(engine);
    args.extend([
        "--mount".into(),
        format!(
            "type=bind,source={},target={container_destination}",
            destination.display()
        ),
        "--entrypoint".into(),
        "cp".into(),
        image_tag.into(),
        "-R".into(),
        source.into(),
        format!("{container_destination}/"),
    ]);
    let copied =
        run_command(root, engine_name(engine), &args, DEFAULT_TOOL_TIMEOUT).map_err(|error| {
            BackendError::execution(format!("failed to materialize {description}: {error}"))
        })?;
    if copied.status.success() {
        Ok(())
    } else {
        Err(BackendError::execution(format!(
            "{description} materialization failed: {}",
            concise_output(&copied.stderr)
        )))
    }
}

fn write_completion_marker(root: &Path, marker: &Path, content: &str) -> Result<(), BackendError> {
    #[cfg(unix)]
    {
        write_completion_marker_unix(root, marker, content)
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        for attempt in 0..100 {
            let temporary = marker.with_extension(format!("tmp-{}-{attempt}", std::process::id()));
            match write_new_file(&temporary, content) {
                Ok(()) => {
                    fs::rename(&temporary, marker).map_err(|error| {
                        BackendError::execution(format!(
                            "failed to publish dependency materialization marker {}: {error}",
                            marker.display()
                        ))
                    })?;
                    return Ok(());
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(BackendError::execution(format!(
                        "failed to write dependency materialization marker {}: {error}",
                        marker.display()
                    )));
                }
            }
        }
        Err(BackendError::execution(format!(
            "failed to allocate dependency materialization marker for {}",
            marker.display()
        )))
    }
}

#[cfg(unix)]
fn write_completion_marker_unix(
    root: &Path,
    marker: &Path,
    content: &str,
) -> Result<(), BackendError> {
    let (parent, marker_name) = open_managed_parent(root, marker)?;
    for attempt in 0..100 {
        let temporary_name = CString::new(format!(".ayni-marker-{}-{attempt}", std::process::id()))
            .expect("generated marker name");
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::AlreadyExists {
                continue;
            }
            return Err(BackendError::execution(format!(
                "failed to write dependency materialization marker {}: {error}",
                marker.display()
            )));
        }
        let mut file = unsafe { fs::File::from_raw_fd(descriptor) };
        if let Err(error) = file
            .write_all(content.as_bytes())
            .and_then(|()| file.sync_all())
        {
            unsafe {
                libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
            }
            return Err(BackendError::execution(format!(
                "failed to write dependency materialization marker {}: {error}",
                marker.display()
            )));
        }
        drop(file);
        let renamed = unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                parent.as_raw_fd(),
                marker_name.as_ptr(),
            )
        };
        if renamed == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
        return Err(BackendError::execution(format!(
            "failed to publish dependency materialization marker {}: {error}",
            marker.display()
        )));
    }
    Err(BackendError::execution(format!(
        "failed to allocate dependency materialization marker for {}",
        marker.display()
    )))
}

fn materialization_marker_current(
    root: &Path,
    marker: &Path,
    expected: &str,
) -> Result<bool, BackendError> {
    #[cfg(unix)]
    let file = {
        let (parent, marker_name) = open_managed_parent(root, marker)?;
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                marker_name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(marker_open_error(marker, error));
        }
        unsafe { fs::File::from_raw_fd(descriptor) }
    };
    #[cfg(not(unix))]
    let file = match OpenOptions::new().read(true).open(marker) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(marker_open_error(marker, error)),
    };
    read_completion_marker(file, marker, expected)
}

fn marker_open_error(marker: &Path, error: std::io::Error) -> BackendError {
    BackendError::execution(format!(
        "failed to open dependency materialization marker {} without following symlinks: {error}",
        marker.display()
    ))
}

fn read_completion_marker(
    mut file: fs::File,
    marker: &Path,
    expected: &str,
) -> Result<bool, BackendError> {
    let metadata = file.metadata().map_err(|error| {
        BackendError::execution(format!(
            "failed to inspect dependency materialization marker {}: {error}",
            marker.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(BackendError::execution(format!(
            "managed environment marker must be a regular file: {}",
            marker.display()
        )));
    }
    let mut content = String::new();
    file.read_to_string(&mut content).map_err(|error| {
        BackendError::execution(format!(
            "failed to read dependency materialization marker {}: {error}",
            marker.display()
        ))
    })?;
    if content == expected {
        Ok(true)
    } else {
        Err(BackendError::execution(format!(
            "dependency materialization marker is stale or corrupt: {}",
            marker.display()
        )))
    }
}

#[cfg(unix)]
fn open_managed_parent(root: &Path, path: &Path) -> Result<(fs::File, CString), BackendError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        BackendError::execution(format!(
            "managed environment path escapes repository: {}",
            path.display()
        ))
    })?;
    let marker_name = relative
        .file_name()
        .ok_or_else(|| BackendError::execution("managed environment marker has no file name"))?;
    let mut directory = open_directory_nofollow(root).map_err(|error| {
        BackendError::execution(format!(
            "failed to open repository root without following symlinks: {error}"
        ))
    })?;
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            let Component::Normal(name) = component else {
                return Err(BackendError::execution(
                    "managed environment marker path is not repository-relative",
                ));
            };
            let name = CString::new(name.as_bytes())
                .map_err(|_| BackendError::execution("managed environment path contains NUL"))?;
            let descriptor = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if descriptor < 0 {
                return Err(BackendError::execution(format!(
                    "failed to open managed environment directory without following symlinks: {}",
                    std::io::Error::last_os_error()
                )));
            }
            directory = unsafe { fs::File::from_raw_fd(descriptor) };
        }
    }
    let marker_name = CString::new(marker_name.as_bytes())
        .map_err(|_| BackendError::execution("managed environment marker contains NUL"))?;
    Ok((directory, marker_name))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

struct MaterializationRequest<'a> {
    root: &'a Path,
    engine: Engine,
    lock: &'a EnvironmentLock,
    image_tag: &'a str,
    preparation: &'a DependencyPreparationPlan,
    cache_state: &'a Path,
    output_state: &'a Path,
    output: &'a ayni_core::PreparationOutput,
}

fn run_materialization_commands(request: MaterializationRequest<'_>) -> Result<(), BackendError> {
    let MaterializationRequest {
        root,
        engine,
        lock,
        image_tag,
        preparation,
        cache_state,
        output_state,
        output,
    } = request;
    if preparation.materialization_commands.is_empty() {
        return Ok(());
    }
    let target = lock
        .targets()
        .iter()
        .find(|target| target.target == preparation.target)
        .ok_or_else(|| BackendError::environment("preparation target is absent from lock"))?;
    let mut activation = target_environment(target)?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    activation.extend(preparation.execution_environment.clone());
    let cwd = env::current_dir().map_err(|error| {
        BackendError::execution(format!("failed to establish current directory: {error}"))
    })?;
    for command in &preparation.materialization_commands {
        let workdir = if command.cwd == "." {
            WORKSPACE.to_owned()
        } else {
            format!("{WORKSPACE}/{}", command.cwd)
        };
        let mut args = base_launch_args(engine);
        args.extend([
            "--mount".into(),
            format!(
                "type=bind,source={},target={WORKSPACE},readonly",
                root.display()
            ),
            "--mount".into(),
            format!(
                "type=bind,source={},target={}",
                output_state.display(),
                crate::preparation::workspace_mount(output)
            ),
            "--mount".into(),
            format!(
                "type=bind,source={},target=/home/ayni/.cache",
                cache_state.display()
            ),
            "--workdir".into(),
            workdir,
            "--entrypoint".into(),
            "env".into(),
            image_tag.into(),
        ]);
        args.extend(
            activation
                .iter()
                .chain(command.environment.iter())
                .map(|(name, value)| format!("{name}={value}")),
        );
        args.push(command.program.clone());
        args.extend(command.args.clone());
        let result = run_command(&cwd, engine_name(engine), &args, DEFAULT_TOOL_TIMEOUT).map_err(
            |error| {
                BackendError::execution(format!(
                    "failed to run offline dependency materialization: {error}"
                ))
            },
        )?;
        if !result.status.success() {
            return Err(BackendError::execution(format!(
                "offline dependency materialization failed: {}",
                concise_output(&result.stderr)
            )));
        }
    }
    Ok(())
}

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
    append_workspace_state_args(&mut args, root, WORKSPACE, state_home);
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
            LockedRequirementSource, LockedRuntime, RequirementConfidence, TargetIdentity,
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
        assert!(!args.iter().any(|arg| arg == "--entrypoint"));
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("MISE_") && arg.ends_with("_VERSION"))
        );
        assert!(args.ends_with(&["ayni-env:test".into(), "check".into(), "--host".into()]));
    }
}
