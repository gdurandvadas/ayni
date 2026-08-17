use super::engine::write_new_file;
use super::{
    Engine, WORKSPACE, base_launch_args, create_contained_directory_tree, engine_name,
    target_environment,
};
use crate::image::ImagePlan;
use crate::{BackendError, concise_output};
use ayni_adapters_common::exec::{DEFAULT_TOOL_TIMEOUT, run_command};
use ayni_core::{DependencyPreparationPlan, EnvironmentLock, PreparationOutputMode};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub(super) fn materialize_outputs(
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
    validate_output_ownership(preparations)?;
    let cache_destination = materialize_cache(root, engine, image_plan, &state_root)?;
    let outputs = crate::preparation::unique_outputs(preparations);
    let mut destinations = BTreeMap::new();
    let mut handled = std::collections::BTreeSet::new();

    for plan in preparations {
        let mut plan_outputs = outputs
            .iter()
            .filter(|output| {
                plan.outputs.contains(output)
                    && handled.insert(crate::preparation::output_key(output))
            })
            .collect::<Vec<_>>();
        plan_outputs.sort_by(|left, right| left.path.cmp(&right.path));
        for (key, destination) in materialize_preparation_outputs(
            root,
            engine,
            lock,
            image_plan,
            plan,
            &plan_outputs,
            &state_root,
            &cache_destination,
        )? {
            destinations.insert(key, destination);
        }
    }

    if handled.len() != outputs.len() {
        return Err(BackendError::environment(
            "dependency preparation output has no owning plan",
        ));
    }
    let mut mounts = vec![(cache_destination, String::from("/home/ayni/.cache"))];
    for output in outputs {
        let key = crate::preparation::output_key(&output);
        let destination = destinations.remove(&key).ok_or_else(|| {
            BackendError::environment("dependency preparation output was not materialized")
        })?;
        mounts.push((destination, crate::preparation::workspace_mount(&output)));
    }
    Ok(mounts)
}

fn validate_output_ownership(
    preparations: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let mut owners = BTreeMap::new();
    for plan in preparations {
        for output in &plan.outputs {
            if let Some(previous) = owners.insert(output.mount_path.clone(), &plan.target) {
                return Err(BackendError::environment(format!(
                    "dependency output {} is claimed by both {}:{} and {}:{}",
                    output.mount_path,
                    previous.language,
                    previous.root,
                    plan.target.language,
                    plan.target.root
                )));
            }
        }
    }
    Ok(())
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

struct PendingOutput<'a> {
    key: String,
    output: &'a ayni_core::PreparationOutput,
    destination: PathBuf,
    marker: PathBuf,
    current: bool,
    staging: Option<StagingDirectory>,
}

#[allow(clippy::too_many_arguments)]
fn materialize_preparation_outputs(
    root: &Path,
    engine: Engine,
    lock: &EnvironmentLock,
    image_plan: &ImagePlan,
    preparation: &DependencyPreparationPlan,
    outputs: &[&ayni_core::PreparationOutput],
    state_root: &Path,
    cache_state: &Path,
) -> Result<Vec<(String, PathBuf)>, BackendError> {
    if outputs.is_empty() {
        return Ok(Vec::new());
    }
    let mut pending = Vec::with_capacity(outputs.len());
    for output in outputs {
        let key = crate::preparation::output_key(output);
        let parent_relative = state_root.join("dependencies").join(&key);
        create_contained_directory_tree(root, &parent_relative)?;
        pending.push(PendingOutput {
            destination: root.join(&parent_relative).join("content"),
            marker: root
                .join(state_root)
                .join("dependencies")
                .join(format!("{key}.complete")),
            key,
            output,
            current: false,
            staging: None,
        });
    }

    refresh_materialization_state(root, image_plan, &mut pending)?;
    if pending.iter().all(|output| output.current) {
        return Ok(pending
            .into_iter()
            .map(|output| (output.key, output.destination))
            .collect());
    }

    // Acquire every output lock in stable path order. A workspace package
    // manager may update several nested outputs in one operation.
    let mut locks = Vec::with_capacity(pending.len());
    for output in &pending {
        locks.push(MaterializationLock::acquire(
            root,
            &output.marker.with_extension("lock"),
            &output.marker,
            &image_plan.preparation_digest,
        )?);
    }
    refresh_materialization_state(root, image_plan, &mut pending)?;
    if pending.iter().all(|output| output.current) {
        return Ok(pending
            .into_iter()
            .map(|output| (output.key, output.destination))
            .collect());
    }

    for output in &mut pending {
        if !output.current {
            reject_partial_materialization(&output.destination)?;
        }
        let staging = StagingDirectory::create(
            output
                .destination
                .parent()
                .expect("dependency output parent"),
        )?;
        if output.output.mode == PreparationOutputMode::Seeded {
            copy_image_tree(
                root,
                engine,
                &image_plan.tag,
                &format!("{}/{}/.", crate::preparation::SEED_ROOT, output.key),
                staging.path(),
                "/tmp/ayni/dependencies",
                "locked dependencies",
            )?;
        }
        output.staging = Some(staging);
    }

    let workspace_parent = state_root.join("workspaces");
    create_contained_directory_tree(root, &workspace_parent)?;
    let workspace = StagingDirectory::create(&root.join(&workspace_parent))?;
    crate::preparation::stage_inputs(root, workspace.path(), std::slice::from_ref(preparation))?;
    let output_states = pending
        .iter()
        .map(|output| {
            (
                output.output,
                output.staging.as_ref().expect("staged output").path(),
            )
        })
        .collect::<Vec<_>>();
    run_materialization_commands(MaterializationRequest {
        root,
        engine,
        lock,
        image_tag: &image_plan.tag,
        preparation,
        cache_state,
        workspace_state: &workspace.path().join("repository"),
        output_states: &output_states,
    })?;

    for output in &mut pending {
        let staging = output.staging.take().expect("staged output");
        if output.current {
            drop(staging);
            continue;
        }
        staging.publish(&output.destination)?;
        write_completion_marker(root, &output.marker, &image_plan.preparation_digest)?;
    }
    drop(locks);
    Ok(pending
        .into_iter()
        .map(|output| (output.key, output.destination))
        .collect())
}

fn refresh_materialization_state(
    root: &Path,
    image_plan: &ImagePlan,
    outputs: &mut [PendingOutput<'_>],
) -> Result<(), BackendError> {
    for output in outputs {
        output.current =
            materialization_marker_current(root, &output.marker, &image_plan.preparation_digest)?;
        if output.current {
            validate_materialized_directory(root, &output.destination)?;
        }
    }
    Ok(())
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
    let mut args = base_launch_args(engine, ayni_core::EnvironmentCapabilities::default())?;
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
    workspace_state: &'a Path,
    output_states: &'a [(&'a ayni_core::PreparationOutput, &'a Path)],
}

fn run_materialization_commands(request: MaterializationRequest<'_>) -> Result<(), BackendError> {
    let MaterializationRequest {
        root,
        engine,
        lock,
        image_tag,
        preparation,
        cache_state,
        workspace_state,
        output_states,
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
    // Materialization containers have a read-only root filesystem. Mise may
    // track version files discovered in the read-only checkout, so direct its
    // ephemeral state into the writable /tmp tmpfs.
    activation.insert(String::from("HOME"), String::from("/tmp/ayni/home"));
    activation.insert(
        String::from("XDG_DATA_HOME"),
        String::from("/tmp/ayni/xdg-data"),
    );
    activation.insert(
        String::from("XDG_STATE_HOME"),
        String::from("/tmp/ayni/xdg-state"),
    );
    activation.extend(preparation.execution_environment.clone());
    let cwd = root.to_path_buf();
    for command in &preparation.materialization_commands {
        let workdir = if command.cwd == "." {
            WORKSPACE.to_owned()
        } else {
            format!("{WORKSPACE}/{}", command.cwd)
        };
        let mut args = base_launch_args(engine, ayni_core::EnvironmentCapabilities::default())?;
        args.extend([
            "--mount".into(),
            format!(
                "type=bind,source={},target={WORKSPACE}",
                workspace_state.display()
            ),
        ]);
        for (output, state) in output_states {
            args.extend([
                "--mount".into(),
                format!(
                    "type=bind,source={},target={}",
                    state.display(),
                    crate::preparation::workspace_mount(output)
                ),
            ]);
        }
        args.extend([
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
            let stderr = concise_output(&result.stderr);
            let diagnostics = if stderr == "command failed without diagnostics" {
                concise_output(&result.stdout)
            } else {
                stderr
            };
            return Err(BackendError::execution(format!(
                "offline dependency materialization command {} failed with {}: {diagnostics}",
                command.program, result.status
            )));
        }
    }
    Ok(())
}
