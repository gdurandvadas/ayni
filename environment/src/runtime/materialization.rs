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
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(super) fn materialize_outputs(
    root: &Path,
    engine: Engine,
    lock: &EnvironmentLock,
    image_plan: &ImagePlan,
    preparations: &[DependencyPreparationPlan],
) -> Result<Vec<(PathBuf, String)>, BackendError> {
    let fingerprint = image_plan.installation_digest.trim_start_matches("sha256:");
    let preparation = image_plan
        .preparation_digest
        .strip_prefix("sha256:")
        .unwrap_or(&image_plan.preparation_digest);
    let state_root = PathBuf::from(".ayni/environment")
        .join(&fingerprint[..16.min(fingerprint.len())])
        .join(&preparation[..16.min(preparation.len())]);
    validate_output_ownership(lock, preparations)?;
    let cache_destination = materialize_cache(root, engine, image_plan, &state_root)?;
    let outputs = crate::preparation::unique_outputs(preparations);
    let mut destinations = BTreeMap::new();
    let mut handled = std::collections::BTreeSet::new();

    for group in crate::preparation_groups::groups(preparations)? {
        let plan = combined_materialization_plan(lock, &group.plans)?;
        let mut plan_outputs = outputs
            .iter()
            .filter(|output| {
                plan.outputs.contains(output)
                    && handled.insert(crate::preparation::output_key(output))
            })
            .collect::<Vec<_>>();
        plan_outputs.sort_by(|left, right| left.path.cmp(&right.path));
        let mut group_plan = image_plan.clone();
        group_plan.preparation_digest = image_plan
            .preparation_groups
            .get(&plan.target)
            .ok_or_else(|| BackendError::environment("preparation group identity is missing"))?
            .clone();
        let group_digest = group_plan.preparation_digest.trim_start_matches("sha256:");
        let group_root = PathBuf::from(".ayni/environment")
            .join(&fingerprint[..16])
            .join(&group_digest[..16]);
        for (key, destination) in materialize_preparation_outputs(
            root,
            engine,
            lock,
            &group_plan,
            &plan,
            &plan_outputs,
            &group_root,
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
    lock: &EnvironmentLock,
    preparations: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let mut owners = BTreeMap::new();
    for plan in preparations {
        let target = lock
            .targets()
            .iter()
            .find(|target| target.target == plan.target)
            .ok_or_else(|| BackendError::environment("preparation target is absent from lock"))?;
        let activation = target_environment(target)?;
        for output in &plan.outputs {
            if let Some((previous, previous_activation)) =
                owners.insert(output.mount_path.clone(), (output, activation.clone()))
                && (previous != output || previous_activation != activation)
            {
                return Err(BackendError::environment(format!(
                    "dependency output {} has incompatible ownership or activation requirements",
                    output.mount_path
                )));
            }
        }
    }
    Ok(())
}

fn combined_materialization_plan(
    lock: &EnvironmentLock,
    plans: &[ayni_core::DependencyPreparationPlan],
) -> Result<ayni_core::DependencyPreparationPlan, BackendError> {
    let mut combined = plans
        .first()
        .ok_or_else(|| BackendError::environment("empty preparation group"))?
        .clone();
    combined.materialization_commands.clear();
    for plan in plans {
        for input in &plan.inputs {
            if !combined.inputs.contains(input) {
                combined.inputs.push(input.clone());
            }
        }
        for scaffold in &plan.scaffolds {
            if !combined.scaffolds.contains(scaffold) {
                combined.scaffolds.push(scaffold.clone());
            }
        }
        let target = lock
            .targets()
            .iter()
            .find(|target| target.target == plan.target)
            .ok_or_else(|| BackendError::environment("preparation target is absent from lock"))?;
        for command in &plan.materialization_commands {
            let mut command = command.clone();
            let mut environment = target_environment(target)?
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            environment.extend(command.environment);
            command.environment = environment;
            if !combined.materialization_commands.contains(&command) {
                combined.materialization_commands.push(command);
            }
        }
    }
    combined.outputs = crate::preparation::unique_outputs(plans);
    Ok(combined)
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
    copy_image_tree(ImageTreeCopy {
        root,
        engine,
        image_tag: &image_plan.tag,
        source: &format!("{}/.", crate::preparation::CACHE_SEED_ROOT),
        destination: staging.path(),
        description: "prepared tool cache",
    })?;
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
    let mut pending = create_pending_outputs(root, outputs, state_root)?;
    refresh_materialization_state(root, image_plan, &mut pending)?;
    if all_outputs_current(&pending) {
        return Ok(pending_destinations(pending));
    }

    // Acquire every output lock in stable path order. A workspace package
    // manager may update several nested outputs in one operation.
    let _locks = acquire_output_locks(root, image_plan, &pending)?;
    refresh_materialization_state(root, image_plan, &mut pending)?;
    if all_outputs_current(&pending) {
        return Ok(pending_destinations(pending));
    }

    stage_pending_outputs(root, engine, image_plan, &mut pending)?;
    run_preparation_for_outputs(
        root,
        engine,
        lock,
        image_plan,
        preparation,
        state_root,
        cache_state,
        &pending,
    )?;
    relocate_pending_output_links(&pending)?;
    publish_pending_outputs(root, image_plan, &mut pending)?;
    Ok(pending_destinations(pending))
}

fn create_pending_outputs<'a>(
    root: &Path,
    outputs: &[&'a ayni_core::PreparationOutput],
    state_root: &Path,
) -> Result<Vec<PendingOutput<'a>>, BackendError> {
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
    Ok(pending)
}

fn all_outputs_current(outputs: &[PendingOutput<'_>]) -> bool {
    outputs.iter().all(|output| output.current)
}

fn pending_destinations(outputs: Vec<PendingOutput<'_>>) -> Vec<(String, PathBuf)> {
    outputs
        .into_iter()
        .map(|output| (output.key, output.destination))
        .collect()
}

fn acquire_output_locks(
    root: &Path,
    image_plan: &ImagePlan,
    outputs: &[PendingOutput<'_>],
) -> Result<Vec<MaterializationLock>, BackendError> {
    outputs
        .iter()
        .map(|output| {
            MaterializationLock::acquire(
                root,
                &output.marker.with_extension("lock"),
                &output.marker,
                &image_plan.preparation_digest,
            )
        })
        .collect()
}

fn stage_pending_outputs(
    root: &Path,
    engine: Engine,
    image_plan: &ImagePlan,
    outputs: &mut [PendingOutput<'_>],
) -> Result<(), BackendError> {
    for output in outputs {
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
            let source = format!("{}/{}/.", crate::preparation::SEED_ROOT, output.key);
            copy_image_tree(ImageTreeCopy {
                root,
                engine,
                image_tag: &image_plan.tag,
                source: &source,
                destination: staging.path(),
                description: "locked dependencies",
            })?;
        }
        output.staging = Some(staging);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_preparation_for_outputs(
    root: &Path,
    engine: Engine,
    lock: &EnvironmentLock,
    image_plan: &ImagePlan,
    preparation: &DependencyPreparationPlan,
    state_root: &Path,
    cache_state: &Path,
    outputs: &[PendingOutput<'_>],
) -> Result<(), BackendError> {
    let workspace_parent = state_root.join("workspaces");
    create_contained_directory_tree(root, &workspace_parent)?;
    let workspace = StagingDirectory::create(&root.join(&workspace_parent))?;
    crate::preparation::stage_workspace(
        root,
        &workspace.path().join("repository"),
        std::slice::from_ref(preparation),
    )?;
    let output_states = outputs
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
    })
}

fn relocate_pending_output_links(outputs: &[PendingOutput<'_>]) -> Result<(), BackendError> {
    for output in outputs.iter().filter(|output| !output.current) {
        relocate_output_links(
            output.output,
            output.staging.as_ref().expect("staged output").path(),
        )?;
    }
    Ok(())
}

// Managed launch stores each prepared output under a separate /opt tree. Keep
// links within an output relative, but anchor links that cross an output
// boundary at their logical workspace path so that isolation preserves the
// topology in which the package manager created them.
fn relocate_output_links(
    output: &ayni_core::PreparationOutput,
    state_root: &Path,
) -> Result<(), BackendError> {
    let logical_root = crate::preparation::workspace_mount(output);
    let mut directories = vec![(state_root.to_path_buf(), logical_root.clone())];

    while let Some((physical_directory, logical_directory)) = directories.pop() {
        for entry in dependency_output_entries(&physical_directory)? {
            if let Some(directory) =
                relocate_output_entry(entry, &logical_directory, &logical_root)?
            {
                directories.push(directory);
            }
        }
    }
    Ok(())
}

fn dependency_output_entries(directory: &Path) -> Result<Vec<fs::DirEntry>, BackendError> {
    let inspect_error = |error| {
        BackendError::execution(format!(
            "failed to inspect managed dependency output {}: {error}",
            directory.display()
        ))
    };
    let mut entries = fs::read_dir(directory)
        .map_err(inspect_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(inspect_error)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn relocate_output_entry(
    entry: fs::DirEntry,
    logical_directory: &str,
    logical_root: &str,
) -> Result<Option<(PathBuf, String)>, BackendError> {
    let physical_path = entry.path();
    let name = entry.file_name().into_string().map_err(|_| {
        BackendError::execution(format!(
            "managed dependency output contains a non-UTF-8 path: {}",
            physical_path.display()
        ))
    })?;
    let logical_path = format!("{logical_directory}/{name}");
    let file_type = entry.file_type().map_err(|error| {
        BackendError::execution(format!(
            "failed to inspect managed dependency output {}: {error}",
            physical_path.display()
        ))
    })?;
    if file_type.is_dir() {
        return Ok(Some((physical_path, logical_path)));
    }
    if !file_type.is_symlink() {
        return Ok(None);
    }

    let target = fs::read_link(&physical_path).map_err(|error| {
        BackendError::execution(format!(
            "failed to read managed dependency symbolic link {logical_path}: {error}"
        ))
    })?;
    let target = target.to_str().ok_or_else(|| {
        BackendError::execution(format!(
            "managed dependency symbolic link {logical_path} has a non-UTF-8 target"
        ))
    })?;
    if target.starts_with('/') {
        return Ok(None);
    }

    let resolved = resolve_workspace_link(&logical_path, target)?;
    if !logical_path_within(&resolved, logical_root) {
        replace_symbolic_link(&physical_path, &logical_path, &resolved)?;
    }
    Ok(None)
}

fn resolve_workspace_link(logical_link: &str, target: &str) -> Result<String, BackendError> {
    let parent = logical_link
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    if !logical_path_within(parent, WORKSPACE) {
        return Err(BackendError::execution(format!(
            "managed dependency symbolic link is outside {WORKSPACE}: {logical_link}"
        )));
    }

    let mut components = parent
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.len() <= 1 {
                    return Err(BackendError::execution(format!(
                        "managed dependency symbolic link {logical_link} escapes {WORKSPACE}: {target}"
                    )));
                }
                components.pop();
            }
            component => components.push(component.to_owned()),
        }
    }
    let resolved = format!("/{}", components.join("/"));
    if logical_path_within(&resolved, WORKSPACE) {
        Ok(resolved)
    } else {
        Err(BackendError::execution(format!(
            "managed dependency symbolic link {logical_link} escapes {WORKSPACE}: {target}"
        )))
    }
}

fn logical_path_within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|relative| relative.starts_with('/'))
}

#[cfg(unix)]
fn replace_symbolic_link(
    physical_path: &Path,
    logical_path: &str,
    target: &str,
) -> Result<(), BackendError> {
    fs::remove_file(physical_path).map_err(|error| {
        BackendError::execution(format!(
            "failed to relocate managed dependency symbolic link {logical_path}: {error}"
        ))
    })?;
    std::os::unix::fs::symlink(target, physical_path).map_err(|error| {
        BackendError::execution(format!(
            "failed to relocate managed dependency symbolic link {logical_path}: {error}"
        ))
    })
}

#[cfg(not(unix))]
fn replace_symbolic_link(
    _physical_path: &Path,
    logical_path: &str,
    _target: &str,
) -> Result<(), BackendError> {
    Err(BackendError::execution(format!(
        "managed dependency symbolic link {logical_path} crosses prepared outputs, but safe symbolic-link relocation is unsupported on this host"
    )))
}

fn publish_pending_outputs(
    root: &Path,
    image_plan: &ImagePlan,
    outputs: &mut [PendingOutput<'_>],
) -> Result<(), BackendError> {
    for output in outputs {
        let staging = output.staging.take().expect("staged output");
        if output.current {
            drop(staging);
            continue;
        }
        staging.publish(&output.destination)?;
        write_completion_marker(root, &output.marker, &image_plan.preparation_digest)?;
    }
    Ok(())
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

struct ImageTreeCopy<'a> {
    root: &'a Path,
    engine: Engine,
    image_tag: &'a str,
    source: &'a str,
    destination: &'a Path,
    description: &'a str,
}

fn copy_image_tree(request: ImageTreeCopy<'_>) -> Result<(), BackendError> {
    let ImageTreeCopy {
        root,
        engine,
        image_tag,
        source,
        destination,
        description,
    } = request;
    let container = create_materialization_container(root, engine, image_tag, description)?;
    let copied = copy_container_archive(root, engine, &container, source, destination, description);
    let removed = remove_materialization_container(root, engine, &container);
    match copied {
        Ok(()) => removed,
        Err(error) => Err(error),
    }
}

fn copy_container_archive(
    root: &Path,
    engine: Engine,
    container: &str,
    source: &str,
    destination: &Path,
    description: &str,
) -> Result<(), BackendError> {
    let mut copied = Command::new(engine_name(engine))
        .current_dir(root)
        .args(["cp", &format!("{container}:{source}"), "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            BackendError::execution(format!("failed to materialize {description}: {error}"))
        })?;
    let stdout = copied.stdout.take().ok_or_else(|| {
        BackendError::execution(format!(
            "failed to materialize {description}: engine copy stream is unavailable"
        ))
    })?;
    let mut stderr = copied.stderr.take().ok_or_else(|| {
        BackendError::execution(format!(
            "failed to materialize {description}: engine diagnostics stream is unavailable"
        ))
    })?;
    let extraction_destination = destination.to_path_buf();
    let extracted =
        std::thread::spawn(move || unpack_container_archive(stdout, &extraction_destination));
    let diagnostics = std::thread::spawn(move || {
        let mut diagnostics = Vec::new();
        stderr.read_to_end(&mut diagnostics).map(|_| diagnostics)
    });
    let status = wait_for_container_archive(&mut copied, DEFAULT_TOOL_TIMEOUT, description)?;
    let extracted = extracted.join().map_err(|_| {
        BackendError::execution(format!(
            "failed to materialize {description}: archive reader panicked"
        ))
    })?;
    let diagnostics = diagnostics
        .join()
        .map_err(|_| {
            BackendError::execution(format!(
                "failed to materialize {description}: diagnostics reader panicked"
            ))
        })?
        .map_err(|error| {
            BackendError::execution(format!("failed to materialize {description}: {error}"))
        })?;
    if !status.success() {
        return Err(BackendError::execution(format!(
            "{description} materialization failed: {}",
            concise_output(&diagnostics)
        )));
    }
    extracted.map_err(|error| {
        BackendError::execution(format!("failed to materialize {description}: {error}"))
    })
}

fn wait_for_container_archive(
    child: &mut Child,
    timeout: Duration,
    description: &str,
) -> Result<ExitStatus, BackendError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BackendError::execution(format!(
                    "{description} materialization timed out after {} seconds",
                    timeout.as_secs()
                )));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BackendError::execution(format!(
                    "failed to materialize {description}: {error}"
                )));
            }
        }
    }
}

fn unpack_container_archive(stream: impl Read, destination: &Path) -> std::io::Result<()> {
    let mut archive = tar::Archive::new(stream);
    archive.set_preserve_permissions(false);
    archive.set_preserve_ownerships(false);
    archive.unpack(destination)
}

fn create_materialization_container(
    root: &Path,
    engine: Engine,
    image_tag: &str,
    description: &str,
) -> Result<String, BackendError> {
    let created = run_command(
        root,
        engine_name(engine),
        &["create".into(), image_tag.into()],
        DEFAULT_TOOL_TIMEOUT,
    )
    .map_err(|error| {
        BackendError::execution(format!("failed to prepare {description} copy: {error}"))
    })?;
    if !created.status.success() {
        return Err(BackendError::execution(format!(
            "failed to prepare {description} copy: {}",
            concise_output(&created.stderr)
        )));
    }
    parse_materialization_container_id(&created.stdout).map_err(|()| {
        BackendError::execution(format!(
            "{} engine create returned an invalid container identifier",
            engine_name(engine)
        ))
    })
}

fn parse_materialization_container_id(output: &[u8]) -> Result<String, ()> {
    let container = String::from_utf8_lossy(output).trim().to_owned();
    if container.is_empty()
        || !container
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        Err(())
    } else {
        Ok(container)
    }
}

fn remove_materialization_container(
    root: &Path,
    engine: Engine,
    container: &str,
) -> Result<(), BackendError> {
    let removed = run_command(
        root,
        engine_name(engine),
        &["rm".into(), container.into()],
        DEFAULT_TOOL_TIMEOUT,
    )
    .map_err(|error| {
        BackendError::execution(format!(
            "failed to remove temporary materialization container: {error}"
        ))
    })?;
    if removed.status.success() {
        Ok(())
    } else {
        Err(BackendError::execution(format!(
            "failed to remove temporary materialization container: {}",
            concise_output(&removed.stderr)
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
        let mut args = base_launch_args(
            engine,
            ayni_core::EnvironmentCapabilities::default(),
            lock.resource_limits(),
        )?;
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

#[cfg(all(test, unix))]
mod tests {
    use super::{
        parse_materialization_container_id, relocate_output_links, unpack_container_archive,
    };
    use ayni_core::{PreparationOutput, PreparationOutputMode};
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    fn test_directory(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ayni-materialization-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("test directory");
        root
    }

    fn web_node_modules_output() -> PreparationOutput {
        PreparationOutput {
            path: String::from("frontend/apps/web/node_modules"),
            mount_path: String::from("frontend/apps/web/node_modules"),
            mode: PreparationOutputMode::Seeded,
        }
    }

    fn root_node_modules_output() -> PreparationOutput {
        PreparationOutput {
            path: String::from("frontend/node_modules"),
            mount_path: String::from("frontend/node_modules"),
            mode: PreparationOutputMode::Seeded,
        }
    }

    #[test]
    fn unpacks_cross_workspace_node_symlink_without_following_it() {
        let root = test_directory("node-workspace-archive");
        let state = root.join("state");
        let mut archive = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_mode(0o777);
        header.set_size(0);
        archive
            .append_link(&mut header, "@example/greeting", "../../libs/greeting")
            .expect("archive symlink");
        let archive = archive.into_inner().expect("archive bytes");

        unpack_container_archive(&archive[..], &state).expect("unpack archive");

        assert_eq!(
            fs::read_link(state.join("@example/greeting")).expect("workspace symlink"),
            Path::new("../../libs/greeting")
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn relocates_child_link_to_root_node_modules_store() {
        let root = test_directory("root-node-modules-link");
        let state = root.join("state");
        let plugin_parent = state.join("@sveltejs");
        let bin = state.join(".bin");
        fs::create_dir_all(&plugin_parent).expect("plugin scope");
        fs::create_dir_all(&bin).expect("binary links");
        let plugin = plugin_parent.join("vite-plugin-svelte");
        let plugin_target = "../../../../node_modules/.pnpm/@sveltejs+vite-plugin-svelte@1/node_modules/@sveltejs/vite-plugin-svelte";
        symlink(plugin_target, &plugin).expect("plugin link");
        let internal = bin.join("vite");
        symlink("../vite/bin/vite.js", &internal).expect("internal link");
        let absolute = state.join("python");
        symlink("/opt/ayni/python", &absolute).expect("absolute link");

        relocate_output_links(&web_node_modules_output(), &state).expect("relocate links");

        assert_eq!(
            fs::read_link(plugin).expect("relocated plugin link"),
            Path::new(
                "/workspace/frontend/node_modules/.pnpm/@sveltejs+vite-plugin-svelte@1/node_modules/@sveltejs/vite-plugin-svelte"
            )
        );
        assert_eq!(
            fs::read_link(internal).expect("internal link"),
            Path::new("../vite/bin/vite.js")
        );
        assert_eq!(
            fs::read_link(absolute).expect("absolute link"),
            Path::new("/opt/ayni/python")
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn relocates_root_and_child_links_to_workspace_source() {
        let root = test_directory("sibling-workspace-link");
        let child_state = root.join("child-state");
        let scope = child_state.join("@guita");
        fs::create_dir_all(&scope).expect("workspace scope");
        let child_workspace = scope.join("ui");
        symlink("../../../../packages/ui", &child_workspace).expect("child workspace link");

        relocate_output_links(&web_node_modules_output(), &child_state)
            .expect("relocate child links");

        assert_eq!(
            fs::read_link(child_workspace).expect("relocated child workspace link"),
            Path::new("/workspace/frontend/packages/ui")
        );

        let root_state = root.join("root-state");
        let root_scope = root_state.join(".pnpm/node_modules/@guita");
        fs::create_dir_all(&root_scope).expect("root workspace scope");
        let root_workspace = root_scope.join("ui");
        symlink("../../../../packages/ui", &root_workspace).expect("root workspace link");

        relocate_output_links(&root_node_modules_output(), &root_state)
            .expect("relocate root links");

        assert_eq!(
            fs::read_link(root_workspace).expect("relocated root workspace link"),
            Path::new("/workspace/frontend/packages/ui")
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn rejects_child_link_that_escapes_the_workspace() {
        let root = test_directory("escaping-link");
        let state = root.join("state");
        fs::create_dir(&state).expect("output state");
        let escape = state.join("escape");
        symlink("../../../../../../etc/passwd", &escape).expect("escaping link");

        let error = relocate_output_links(&web_node_modules_output(), &state)
            .expect_err("escaping link must fail");

        assert!(error.message.contains("escapes /workspace"));
        assert_eq!(
            fs::read_link(escape).expect("original escaping link"),
            Path::new("../../../../../../etc/passwd")
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn materialization_container_identifier_must_be_a_single_hex_value() {
        assert_eq!(
            parse_materialization_container_id(b"a1b2c3\n").expect("container identifier"),
            "a1b2c3"
        );
        for invalid in [b"\n".as_slice(), b"a1b2\nextra\n", b"container-name\n"] {
            assert!(parse_materialization_container_id(invalid).is_err());
        }
    }
}
