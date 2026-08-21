use crate::image::{
    IMAGE_AYNI_LABEL, IMAGE_BASE_LABEL, IMAGE_LOCK_LABEL, IMAGE_MISE_LABEL, IMAGE_OWNER_LABEL,
    IMAGE_OWNER_VALUE, IMAGE_PLATFORM_LABEL, IMAGE_PREPARATION_LABEL, IMAGE_SCHEMA_LABEL,
    IMAGE_SCHEMA_VERSION, ImagePlan, image_plan_with_preparation,
};
use crate::{BackendError, concise_output, read_lock};
use ayni_adapters_common::exec::run_command;
use ayni_core::{DependencyPreparationPlan, EnvironmentLock};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const STATE_ROOT: &str = ".ayni/environment";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageImageOwnership {
    Managed,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageImagePruneScope {
    /// A managed image can be referenced by any repository using the selected
    /// engine. Ayni cannot prove that deleting it is repository-local.
    EngineWideAcrossRepositories,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageImage {
    pub id: String,
    pub tags: Vec<String>,
    /// Cumulative image size reported by the engine. Shared layers mean this
    /// is not an estimate of incremental or reclaimable host storage.
    pub cumulative_size_bytes: u64,
    pub lock_fingerprint: Option<String>,
    pub preparation_digest: Option<String>,
    pub schema_version: Option<String>,
    pub ownership: StorageImageOwnership,
    pub current: bool,
    pub prune_candidate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageStateGeneration {
    pub path: String,
    pub logical_size_bytes: u64,
    pub current: bool,
    pub prune_candidate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageReport {
    pub engine: String,
    /// Tag the current lock and preparation would use for a managed launch.
    /// The image may not have been built, or may no longer be present.
    pub expected_image_tag: String,
    /// Whether an inspected Ayni-managed image has the complete current label
    /// identity. This is independent of the expected tag being printable.
    pub current_image_present: bool,
    pub images: Vec<StorageImage>,
    /// Sum of the engine-reported cumulative size for each image. This can
    /// double-count shared layers and is neither unique nor reclaimable size.
    pub image_cumulative_size_bytes: u64,
    pub image_prune_scope: StorageImagePruneScope,
    pub state_root: String,
    pub state_generations: Vec<StorageStateGeneration>,
    /// Logical bytes below the complete state root, including unclassified
    /// files and directories that Ayni will never prune.
    pub state_root_logical_size_bytes: u64,
    /// Logical bytes belonging to the classified entries in
    /// `state_generations`.
    pub classified_state_logical_size_bytes: u64,
    /// Unclassified subset of `state_root_logical_size_bytes`. Ayni reports
    /// these bytes but never selects them for pruning.
    pub unclassified_state_logical_size_bytes: u64,
    /// Ayni currently uses the engine's shared default builder. Its cache
    /// cannot be attributed safely to one product or repository, so storage
    /// reporting and pruning deliberately exclude it.
    pub build_cache_included: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoragePruneFailure {
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoragePruneResult {
    pub applied: bool,
    /// Whether the caller explicitly acknowledged that image deletion is
    /// engine-wide across repositories. Repository-local state does not need
    /// this acknowledgement.
    pub images_requested: bool,
    pub report: StorageReport,
    pub removed_images: Vec<String>,
    pub removed_state_generations: Vec<String>,
    pub failures: Vec<StoragePruneFailure>,
}

impl StoragePruneResult {
    #[must_use]
    pub fn complete(&self) -> bool {
        self.failures.is_empty()
    }
}

pub fn storage_report(repo_root: &Path) -> Result<StorageReport, BackendError> {
    storage_report_prepared(repo_root, &[])
}

pub fn storage_report_prepared(
    repo_root: &Path,
    preparations: &[DependencyPreparationPlan],
) -> Result<StorageReport, BackendError> {
    let (root, engine, lock, plan) = storage_context(repo_root, preparations)?;
    report_for_context(&root, engine, &lock, &plan)
}

pub fn prune_storage(
    repo_root: &Path,
    apply: bool,
    images: bool,
) -> Result<StoragePruneResult, BackendError> {
    prune_storage_prepared(repo_root, &[], apply, images)
}

pub fn prune_storage_prepared(
    repo_root: &Path,
    preparations: &[DependencyPreparationPlan],
    apply: bool,
    images: bool,
) -> Result<StoragePruneResult, BackendError> {
    let (root, engine, lock, plan) = storage_context(repo_root, preparations)?;
    let report = report_for_context(&root, engine, &lock, &plan)?;
    let mut result = StoragePruneResult {
        applied: apply,
        images_requested: images,
        report,
        removed_images: Vec::new(),
        removed_state_generations: Vec::new(),
        failures: Vec::new(),
    };
    if !apply {
        return Ok(result);
    }

    if images {
        for image in result
            .report
            .images
            .iter()
            .filter(|image| image.prune_candidate)
        {
            let args = [String::from("image"), String::from("rm"), image.id.clone()];
            match run_engine(&root, engine, &args) {
                Ok(output) if output.status.success() => {
                    result.removed_images.push(image.id.clone());
                }
                Ok(output) => result.failures.push(StoragePruneFailure {
                    target: image.id.clone(),
                    message: concise_output(&output.stderr),
                }),
                Err(error) => result.failures.push(StoragePruneFailure {
                    target: image.id.clone(),
                    message: error.message,
                }),
            }
        }
    }

    for generation in result
        .report
        .state_generations
        .iter()
        .filter(|generation| generation.prune_candidate)
    {
        match remove_state_generation(&root, &generation.path) {
            Ok(()) => result
                .removed_state_generations
                .push(generation.path.clone()),
            Err(error) => result.failures.push(StoragePruneFailure {
                target: generation.path.clone(),
                message: error.message,
            }),
        }
    }

    Ok(result)
}

fn storage_context(
    repo_root: &Path,
    preparations: &[DependencyPreparationPlan],
) -> Result<(PathBuf, crate::Engine, EnvironmentLock, ImagePlan), BackendError> {
    let root = repo_root.canonicalize().map_err(|error| {
        BackendError::input(format!(
            "failed to establish repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(BackendError::input(format!(
            "repository root is not a directory: {}",
            root.display()
        )));
    }
    let lock = read_lock(&root)?;
    let plan = image_plan_with_preparation(&lock, preparations)?;
    let engine = crate::detect_engine()?;
    Ok((root, engine, lock, plan))
}

fn report_for_context(
    root: &Path,
    engine: crate::Engine,
    lock: &EnvironmentLock,
    plan: &ImagePlan,
) -> Result<StorageReport, BackendError> {
    let images = inspect_ayni_images(root, engine, lock, plan)?;
    let image_cumulative_size_bytes = images.iter().fold(0_u64, |total, image| {
        total.saturating_add(image.cumulative_size_bytes)
    });
    let fingerprint = fingerprint_segment(lock.fingerprint());
    let preparation = fingerprint_segment(&plan.preparation_digest);
    let (state_generations, state_root_logical_size_bytes, classified_state_bytes) =
        inspect_state(root, fingerprint, preparation)?;
    let current_image_present = images.iter().any(|image| image.current);

    Ok(StorageReport {
        engine: engine_name(engine).into(),
        expected_image_tag: plan.tag.clone(),
        current_image_present,
        images,
        image_cumulative_size_bytes,
        image_prune_scope: StorageImagePruneScope::EngineWideAcrossRepositories,
        state_root: STATE_ROOT.into(),
        state_generations,
        state_root_logical_size_bytes,
        classified_state_logical_size_bytes: classified_state_bytes,
        unclassified_state_logical_size_bytes: state_root_logical_size_bytes
            .saturating_sub(classified_state_bytes),
        build_cache_included: false,
    })
}

fn inspect_ayni_images(
    root: &Path,
    engine: crate::Engine,
    lock: &EnvironmentLock,
    plan: &ImagePlan,
) -> Result<Vec<StorageImage>, BackendError> {
    let filter = format!("label={IMAGE_SCHEMA_LABEL}");
    let args = [
        String::from("image"),
        String::from("ls"),
        String::from("--all"),
        String::from("--no-trunc"),
        String::from("--quiet"),
        String::from("--filter"),
        filter,
    ];
    let output = run_engine(root, engine, &args)?;
    if !output.status.success() {
        return Err(BackendError::execution(format!(
            "failed to list Ayni-managed images: {}",
            concise_output(&output.stderr)
        )));
    }
    let ids = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut images = Vec::with_capacity(ids.len());
    for id in ids {
        let args = [String::from("image"), String::from("inspect"), id.clone()];
        let output = run_engine(root, engine, &args)?;
        if !output.status.success() {
            return Err(BackendError::execution(format!(
                "failed to inspect Ayni-managed image {id}: {}",
                concise_output(&output.stderr)
            )));
        }
        let parsed = parse_inspected_image(&id, &output.stdout)?;
        let ownership = if parsed
            .labels
            .get(IMAGE_OWNER_LABEL)
            .is_some_and(|value| value == IMAGE_OWNER_VALUE)
        {
            StorageImageOwnership::Managed
        } else {
            StorageImageOwnership::Legacy
        };
        let current = image_labels_are_current(&parsed.labels, lock, plan);
        images.push(StorageImage {
            id: parsed.id,
            tags: parsed.tags,
            cumulative_size_bytes: parsed.size,
            lock_fingerprint: parsed.labels.get(IMAGE_LOCK_LABEL).cloned(),
            preparation_digest: parsed.labels.get(IMAGE_PREPARATION_LABEL).cloned(),
            schema_version: parsed.labels.get(IMAGE_SCHEMA_LABEL).cloned(),
            ownership,
            current,
            prune_candidate: ownership == StorageImageOwnership::Managed && !current,
        });
    }
    images.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(images)
}

struct ParsedImage {
    id: String,
    tags: Vec<String>,
    size: u64,
    labels: BTreeMap<String, String>,
}

fn parse_inspected_image(fallback_id: &str, bytes: &[u8]) -> Result<ParsedImage, BackendError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        BackendError::execution(format!("engine returned invalid image metadata: {error}"))
    })?;
    let image = value
        .as_array()
        .and_then(|images| images.first())
        .unwrap_or(&value);
    let id = image
        .get("Id")
        .or_else(|| image.get("ID"))
        .and_then(Value::as_str)
        .unwrap_or(fallback_id)
        .to_owned();
    let mut tags = image
        .get("RepoTags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    let size = image
        .get("Size")
        .or_else(|| image.get("VirtualSize"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let labels = image
        .pointer("/Config/Labels")
        .or_else(|| image.get("Labels"))
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| value.as_str().map(|value| (name.clone(), value.into())))
        .collect();
    Ok(ParsedImage {
        id,
        tags,
        size,
        labels,
    })
}

fn image_labels_are_current(
    labels: &BTreeMap<String, String>,
    lock: &EnvironmentLock,
    plan: &ImagePlan,
) -> bool {
    labels
        .get(IMAGE_OWNER_LABEL)
        .is_some_and(|value| value == IMAGE_OWNER_VALUE)
        && labels
            .get(IMAGE_SCHEMA_LABEL)
            .is_some_and(|value| value == IMAGE_SCHEMA_VERSION)
        && labels
            .get(IMAGE_LOCK_LABEL)
            .is_some_and(|value| value == lock.fingerprint())
        && labels
            .get(IMAGE_BASE_LABEL)
            .is_some_and(|value| value == &lock.provisioning_base().digest)
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
            .is_some_and(|value| value == &plan.preparation_digest)
}

fn inspect_state(
    root: &Path,
    current_fingerprint: &str,
    current_preparation: &str,
) -> Result<(Vec<StorageStateGeneration>, u64, u64), BackendError> {
    let Some(canonical_state) = validated_state_root(root)? else {
        return Ok((Vec::new(), 0, 0));
    };
    let total = logical_tree_size(&canonical_state)?;
    let mut generations =
        collect_state_generations(&canonical_state, current_fingerprint, current_preparation)?;
    generations.sort_by(|left, right| left.path.cmp(&right.path));
    let classified = generations.iter().fold(0_u64, |sum, generation| {
        sum.saturating_add(generation.logical_size_bytes)
    });
    Ok((generations, total, classified))
}

fn validated_state_root(root: &Path) -> Result<Option<PathBuf>, BackendError> {
    let state_root = root.join(STATE_ROOT);
    let metadata = match fs::symlink_metadata(&state_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BackendError::execution(format!(
                "failed to inspect managed environment state {}: {error}",
                state_root.display()
            )));
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BackendError::execution(format!(
            "managed environment state must be a directory, not a symlink: {}",
            state_root.display()
        )));
    }
    let canonical_state = state_root.canonicalize().map_err(|error| {
        BackendError::execution(format!(
            "failed to validate managed environment state {}: {error}",
            state_root.display()
        ))
    })?;
    if !canonical_state.starts_with(root) {
        return Err(BackendError::execution(format!(
            "managed environment state escapes repository root: {}",
            state_root.display()
        )));
    }
    Ok(Some(canonical_state))
}

fn collect_state_generations(
    state_root: &Path,
    current_fingerprint: &str,
    current_preparation: &str,
) -> Result<Vec<StorageStateGeneration>, BackendError> {
    let mut generations = Vec::new();
    for fingerprint in sorted_directory_entries(state_root)? {
        let Some(fingerprint_name) = generation_segment(&fingerprint) else {
            continue;
        };
        for state_path in sorted_directory_entries(&fingerprint)? {
            let Some(state_name) = state_path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let current = if state_name == "home" {
                fingerprint_name == current_fingerprint
            } else if generation_name(state_name) {
                fingerprint_name == current_fingerprint && state_name == current_preparation
            } else {
                continue;
            };
            generations.push(StorageStateGeneration {
                path: format!("{STATE_ROOT}/{fingerprint_name}/{state_name}"),
                logical_size_bytes: logical_tree_size(&state_path)?,
                current,
                prune_candidate: !current,
            });
        }
    }
    Ok(generations)
}

fn sorted_directory_entries(path: &Path) -> Result<Vec<PathBuf>, BackendError> {
    let entries = fs::read_dir(path).map_err(|error| {
        BackendError::execution(format!(
            "failed to read managed environment state {}: {error}",
            path.display()
        ))
    })?;
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            BackendError::execution(format!(
                "failed to read managed environment state {}: {error}",
                path.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            BackendError::execution(format!(
                "failed to inspect managed environment state {}: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() && !file_type.is_symlink() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn generation_segment(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    generation_name(name).then_some(name)
}

fn generation_name(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn logical_tree_size(path: &Path) -> Result<u64, BackendError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        BackendError::execution(format!(
            "failed to inspect managed environment state {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0_u64;
    let entries = fs::read_dir(path).map_err(|error| {
        BackendError::execution(format!(
            "failed to read managed environment state {}: {error}",
            path.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            BackendError::execution(format!(
                "failed to read managed environment state {}: {error}",
                path.display()
            ))
        })?;
        total = total.saturating_add(logical_tree_size(&entry.path())?);
    }
    Ok(total)
}

fn remove_state_generation(root: &Path, relative: &str) -> Result<(), BackendError> {
    let relative = Path::new(relative);
    validate_state_candidate_shape(relative)?;
    let expected_root = Path::new(STATE_ROOT);
    let state_root = root.join(expected_root).canonicalize().map_err(|error| {
        BackendError::execution(format!(
            "failed to validate managed state root {}: {error}",
            root.join(expected_root).display()
        ))
    })?;
    let target = root.join(relative);
    let canonical = validated_state_candidate(&state_root, &target)?;
    fs::remove_dir_all(&canonical).map_err(|error| {
        BackendError::execution(format!(
            "failed to remove managed state {}: {error}",
            target.display()
        ))
    })?;
    if let Some(parent) = canonical.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(())
}

fn validate_state_candidate_shape(relative: &Path) -> Result<(), BackendError> {
    let suffix = relative.strip_prefix(STATE_ROOT).map_err(|_| {
        BackendError::execution(format!(
            "refusing to remove state outside {STATE_ROOT}: {}",
            relative.display()
        ))
    })?;
    let components = suffix.components().collect::<Vec<_>>();
    let valid = match components.as_slice() {
        [Component::Normal(fingerprint), Component::Normal(state)] => {
            fingerprint.to_str().is_some_and(generation_name)
                && state
                    .to_str()
                    .is_some_and(|state| state == "home" || generation_name(state))
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(BackendError::execution(format!(
            "refusing to remove malformed managed state path: {}",
            relative.display()
        )))
    }
}

fn validated_state_candidate(state_root: &Path, target: &Path) -> Result<PathBuf, BackendError> {
    let metadata = fs::symlink_metadata(target).map_err(|error| {
        BackendError::execution(format!(
            "failed to inspect managed state candidate {}: {error}",
            target.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BackendError::execution(format!(
            "refusing to remove non-directory managed state: {}",
            target.display()
        )));
    }
    let canonical = target.canonicalize().map_err(|error| {
        BackendError::execution(format!(
            "failed to validate managed state candidate {}: {error}",
            target.display()
        ))
    })?;
    if canonical == state_root || !canonical.starts_with(state_root) {
        return Err(BackendError::execution(format!(
            "refusing to remove managed state outside {}: {}",
            state_root.display(),
            target.display()
        )));
    }
    Ok(canonical)
}

fn fingerprint_segment(value: &str) -> &str {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    &value[..16.min(value.len())]
}

fn engine_name(engine: crate::Engine) -> &'static str {
    match engine {
        crate::Engine::Docker => "docker",
        crate::Engine::Podman => "podman",
    }
}

fn run_engine(
    root: &Path,
    engine: crate::Engine,
    args: &[String],
) -> Result<std::process::Output, BackendError> {
    run_command(root, engine_name(engine), args, COMMAND_TIMEOUT).map_err(|error| {
        BackendError::execution(format!(
            "failed to run {} storage command: {error}",
            engine_name(engine)
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_image_metadata_without_assuming_tags_exist() {
        let bytes = br#"[{
            "Id":"sha256:abc",
            "RepoTags":null,
            "Size":1234,
            "Config":{"Labels":{
                "dev.ayni.environment.owner":"ayni",
                "dev.ayni.environment.schema":"0.5.0"
            }}
        }]"#;
        let image = parse_inspected_image("fallback", bytes).expect("metadata");
        assert_eq!(image.id, "sha256:abc");
        assert!(image.tags.is_empty());
        assert_eq!(image.size, 1234);
        assert_eq!(
            image.labels.get(IMAGE_OWNER_LABEL).map(String::as_str),
            Some(IMAGE_OWNER_VALUE)
        );
    }

    #[test]
    fn state_report_classifies_preparation_generations_and_runtime_homes() {
        let root = temporary_root("storage-report");
        let current = root
            .join(STATE_ROOT)
            .join("aaaaaaaaaaaaaaaa")
            .join("bbbbbbbbbbbbbbbb");
        let stale = root
            .join(STATE_ROOT)
            .join("cccccccccccccccc")
            .join("dddddddddddddddd");
        fs::create_dir_all(&current).expect("current state");
        fs::create_dir_all(&stale).expect("stale state");
        fs::write(current.join("current"), b"1234").expect("current bytes");
        fs::write(stale.join("stale"), b"123456").expect("stale bytes");
        let current_home = root.join(STATE_ROOT).join("aaaaaaaaaaaaaaaa").join("home");
        let stale_home = root.join(STATE_ROOT).join("cccccccccccccccc").join("home");
        fs::create_dir_all(&current_home).expect("current home");
        fs::create_dir_all(&stale_home).expect("stale home");
        fs::write(current_home.join("cache"), b"123").expect("current home bytes");
        fs::write(stale_home.join("cache"), b"12345").expect("stale home bytes");
        fs::create_dir_all(root.join(STATE_ROOT).join("unclassified")).expect("unclassified");
        fs::write(root.join(STATE_ROOT).join("unclassified/file"), b"12345678")
            .expect("unclassified bytes");

        let (generations, total, classified) =
            inspect_state(&root, "aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb").expect("state");
        assert_eq!(generations.len(), 4);
        assert_eq!(generations.iter().filter(|entry| entry.current).count(), 2);
        assert_eq!(
            generations
                .iter()
                .filter(|entry| entry.prune_candidate)
                .count(),
            2
        );
        assert!(
            generations
                .iter()
                .any(|entry| entry.current && entry.path.ends_with("/home"))
        );
        assert!(
            generations
                .iter()
                .any(|entry| entry.prune_candidate && entry.path.ends_with("/home"))
        );
        assert_eq!(total, 26);
        assert_eq!(classified, 18);
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn state_removal_rejects_paths_outside_exact_generation_shape() {
        let root = temporary_root("storage-remove");
        fs::create_dir_all(root.join(STATE_ROOT)).expect("state root");
        assert!(remove_state_generation(&root, ".ayni/environment").is_err());
        assert!(remove_state_generation(&root, ".ayni/environment/../../outside").is_err());
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn state_removal_accepts_only_fingerprinted_runtime_home() {
        let root = temporary_root("storage-remove-home");
        let home = root.join(STATE_ROOT).join("aaaaaaaaaaaaaaaa").join("home");
        fs::create_dir_all(&home).expect("runtime home");
        fs::write(home.join("cache"), b"state").expect("runtime state");

        remove_state_generation(&root, ".ayni/environment/aaaaaaaaaaaaaaaa/home")
            .expect("fingerprinted runtime home is removable");
        assert!(!home.exists());
        assert!(
            remove_state_generation(&root, ".ayni/environment/aaaaaaaaaaaaaaaa/not-home").is_err()
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn temporary_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ayni-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("temporary root");
        root.canonicalize().expect("canonical temporary root")
    }
}
