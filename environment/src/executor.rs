//! Exact executor identity and repository-local execution build state.
use crate::image::ImagePlan;
use crate::{BackendError, Engine, concise_output};
use ayni_adapters_common::exec::run_command;
use ayni_core::{EnvironmentLock, sha256_fingerprint};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const RECIPE_VERSION: &str = ayni_core::ENVIRONMENT_LOCK_RECIPE_VERSION;
const RECORD_SCHEMA: &str = "1";
pub(crate) const EXECUTOR_LABEL: &str = "dev.ayni.environment.executor";
pub(crate) const RECIPE_LABEL: &str = "dev.ayni.environment.recipe";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutorIdentity {
    pub reference: String,
    pub oci_digest: String,
    pub executable_digest: String,
    pub source_revision: String,
    pub ayni_version: String,
    pub platform: String,
}

impl ExecutorIdentity {
    pub fn fingerprint(&self) -> String {
        sha256_fingerprint(serde_json::to_vec(self).expect("executor identity serializes"))
    }

    fn validate(&self, platform: &str) -> Result<(), BackendError> {
        crate::lock::parse_exact_base(&self.image())?;
        if !valid_digest(&self.executable_digest)
            || self.source_revision.len() != 40
            || !self
                .source_revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.ayni_version != env!("CARGO_PKG_VERSION")
            || self.platform != platform
        {
            return Err(rebuild(
                "executor metadata is incompatible with this CLI or platform",
            ));
        }
        Ok(())
    }

    fn image(&self) -> String {
        format!("{}@{}", self.reference, self.oci_digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildRecord {
    pub schema_version: String,
    pub recipe_version: String,
    pub executor: ExecutorIdentity,
    pub environment_fingerprint: String,
    pub preparation_digest: String,
    pub image_tag: String,
    pub image_id: String,
}

pub(crate) fn rebuild(message: impl std::fmt::Display) -> BackendError {
    BackendError::environment(format!(
        "{message}; run `ayni env build` (use --executor-image for a checkout executor)"
    ))
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn state_path(root: &Path, create: bool) -> Result<PathBuf, BackendError> {
    let mut path = root.to_path_buf();
    for component in [".ayni", "environment"] {
        path.push(component);
        if create {
            crate::runtime::ensure_managed_directory(&path)?;
        } else {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(root.join(".ayni/environment/build.json"));
                }
                _ => {
                    return Err(BackendError::execution(
                        "managed environment state must not contain symlinks or non-directories",
                    ));
                }
            }
        }
    }
    Ok(path.join("build.json"))
}

pub(crate) fn read_record(root: &Path) -> Result<Option<BuildRecord>, BackendError> {
    let path = state_path(root, false)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(rebuild(error)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65536 {
        return Err(rebuild(
            "execution build record must be a bounded regular file",
        ));
    }
    let bytes = fs::read(path).map_err(rebuild)?;
    let record: BuildRecord = serde_json::from_slice(&bytes).map_err(rebuild)?;
    if record.schema_version != RECORD_SCHEMA
        || record.recipe_version != RECIPE_VERSION
        || !valid_digest(&record.image_id)
    {
        return Err(rebuild(
            "execution build record schema or recipe is incompatible",
        ));
    }
    Ok(Some(record))
}

pub(crate) fn bind(plan: &mut ImagePlan, executor: &ExecutorIdentity) {
    let identity = executor.fingerprint();
    plan.tag.push_str(&format!("-exec-{}", &identity[7..23]));
    plan.dockerfile = format!(
        "FROM {} AS ayni-executor\n{}\nCOPY --from=ayni-executor /usr/local/bin/ayni /usr/local/bin/ayni\nLABEL {EXECUTOR_LABEL}=\"{identity}\" {RECIPE_LABEL}=\"{RECIPE_VERSION}\"\nENTRYPOINT [\"ayni\"]\n",
        executor.image(),
        plan.dockerfile
    );
}

pub(crate) fn recorded_plan(
    root: &Path,
    lock: &EnvironmentLock,
    mut plan: ImagePlan,
) -> Result<(ImagePlan, BuildRecord), BackendError> {
    let record = read_record(root)?.ok_or_else(|| rebuild("execution build record is missing"))?;
    record.executor.validate(&plan.platform)?;
    bind(&mut plan, &record.executor);
    if record.environment_fingerprint != lock.fingerprint()
        || record.preparation_digest != plan.preparation_digest
        || record.image_tag != plan.tag
    {
        return Err(rebuild(
            "execution build record does not match the current environment",
        ));
    }
    Ok((plan, record))
}

pub(crate) fn engine_output(
    root: &Path,
    engine: Engine,
    args: &[String],
) -> Result<Vec<u8>, BackendError> {
    let name = match engine {
        Engine::Docker => "docker",
        Engine::Podman => "podman",
    };
    let output = run_command(root, name, args, Duration::from_secs(300)).map_err(rebuild)?;
    if !output.status.success() {
        return Err(rebuild(concise_output(&output.stderr)));
    }
    Ok(output.stdout)
}

pub(crate) fn inspect(
    root: &Path,
    engine: Engine,
    image: &str,
) -> Result<serde_json::Value, BackendError> {
    let output = engine_output(
        root,
        engine,
        &["image".into(), "inspect".into(), image.into()],
    )?;
    let values: Vec<serde_json::Value> = serde_json::from_slice(&output).map_err(rebuild)?;
    values
        .into_iter()
        .next()
        .ok_or_else(|| rebuild("image inspection returned no metadata"))
}

pub(crate) fn executable_digest(
    root: &Path,
    engine: Engine,
    image: &str,
) -> Result<String, BackendError> {
    let bytes = engine_output(
        root,
        engine,
        &[
            "run".into(),
            "--rm".into(),
            "--network".into(),
            "none".into(),
            "--entrypoint".into(),
            "sha256sum".into(),
            image.into(),
            "/usr/local/bin/ayni".into(),
        ],
    )?;
    let digest = format!(
        "sha256:{}",
        String::from_utf8_lossy(&bytes)
            .split_whitespace()
            .next()
            .unwrap_or("")
    );
    if !valid_digest(&digest) {
        return Err(rebuild("executor executable digest is invalid"));
    }
    Ok(digest)
}

pub(crate) fn resolve(
    root: &Path,
    engine: Engine,
    platform: &str,
    explicit: Option<&str>,
) -> Result<ExecutorIdentity, BackendError> {
    if explicit.is_none()
        && let Some(record) = read_record(root)?
    {
        record.executor.validate(platform)?;
        return Ok(record.executor);
    }
    let (reference, oci_digest) = match explicit {
        Some(image) => crate::lock::parse_exact_base(image)?,
        None => {
            let reference = format!(
                "ghcr.io/gdurandvadas/ayni-env:{}-debian",
                env!("CARGO_PKG_VERSION")
            );
            let digest = crate::lock::inspect_remote_digest(&reference)?;
            (reference, digest)
        }
    };
    inspect_executor(root, engine, platform, reference, oci_digest)
}

fn inspect_executor(
    root: &Path,
    engine: Engine,
    platform: &str,
    reference: String,
    oci_digest: String,
) -> Result<ExecutorIdentity, BackendError> {
    let image = format!("{reference}@{oci_digest}");
    engine_output(
        root,
        engine,
        &[
            "pull".into(),
            "--platform".into(),
            platform.into(),
            image.clone(),
        ],
    )?;
    let metadata = inspect(root, engine, &image)?;
    let labels = &metadata["Config"]["Labels"];
    if labels["dev.ayni.executor.lock-schema"].as_str()
        != Some(ayni_core::ENVIRONMENT_LOCK_SCHEMA_VERSION)
        || labels["dev.ayni.executor.recipe"].as_str() != Some(RECIPE_VERSION)
    {
        return Err(rebuild(
            "executor image does not support this lock schema and recipe",
        ));
    }
    let actual_platform = format!(
        "{}/{}",
        metadata["Os"].as_str().unwrap_or(""),
        metadata["Architecture"].as_str().unwrap_or("")
    );
    let version = engine_output(
        root,
        engine,
        &[
            "run".into(),
            "--rm".into(),
            "--network".into(),
            "none".into(),
            "--entrypoint".into(),
            "/usr/local/bin/ayni".into(),
            image.clone(),
            "--version".into(),
        ],
    )?;
    let expected_version = format!("ayni {}", env!("CARGO_PKG_VERSION"));
    if String::from_utf8_lossy(&version).trim() != expected_version {
        return Err(rebuild(
            "executor executable version differs from the host CLI",
        ));
    }
    let executor = ExecutorIdentity {
        reference,
        oci_digest,
        executable_digest: executable_digest(root, engine, &image)?,
        source_revision: metadata["Config"]["Labels"]["org.opencontainers.image.revision"]
            .as_str()
            .unwrap_or("")
            .into(),
        ayni_version: env!("CARGO_PKG_VERSION").into(),
        platform: actual_platform,
    };
    executor.validate(platform)?;
    Ok(executor)
}

pub(crate) fn persist(
    root: &Path,
    lock: &EnvironmentLock,
    plan: &ImagePlan,
    executor: ExecutorIdentity,
    image_id: String,
) -> Result<(), BackendError> {
    if !valid_digest(&image_id) {
        return Err(rebuild("built image has no immutable identity"));
    }
    let record = BuildRecord {
        schema_version: RECORD_SCHEMA.into(),
        recipe_version: RECIPE_VERSION.into(),
        executor,
        environment_fingerprint: lock.fingerprint().into(),
        preparation_digest: plan.preparation_digest.clone(),
        image_tag: plan.tag.clone(),
        image_id,
    };
    let path = state_path(root, true)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().expect("state parent")).map_err(rebuild)?;
    temporary
        .write_all(&serde_json::to_vec_pretty(&record).map_err(rebuild)?)
        .map_err(rebuild)?;
    temporary.as_file().sync_all().map_err(rebuild)?;
    temporary.persist(path).map_err(rebuild)?;
    Ok(())
}

pub(crate) fn validate_record_image(
    root: &Path,
    engine: Engine,
    plan: &ImagePlan,
    record: &BuildRecord,
) -> Result<(), BackendError> {
    let image = inspect(root, engine, &plan.tag)?;
    if image["Id"].as_str() != Some(record.image_id.as_str())
        || image["Config"]["Labels"][EXECUTOR_LABEL].as_str()
            != Some(record.executor.fingerprint().as_str())
        || image["Config"]["Labels"][RECIPE_LABEL].as_str() != Some(RECIPE_VERSION)
    {
        return Err(rebuild(
            "image identity or executor metadata differs from the build record",
        ));
    }
    Ok(())
}

/// Snapshot execution provenance before a quality launch; callers retain it
/// beside the resulting artifact rather than rereading mutable state afterwards.
pub fn execution_build_record(root: &Path) -> Result<serde_json::Value, BackendError> {
    let record = read_record(root)?.ok_or_else(|| rebuild("execution build record is missing"))?;
    serde_json::to_value(record).map_err(rebuild)
}

pub(crate) fn validate_substrate(
    root: &Path,
    engine: Engine,
    lock: &EnvironmentLock,
) -> Result<(), BackendError> {
    let base = lock.provisioning_base();
    let reference = format!("{}@{}", base.reference, base.digest);
    engine_output(root, engine, &["pull".into(), reference.clone()])?;
    let image = inspect(root, engine, &reference)?;
    let labels = &image["Config"]["Labels"];
    if labels["dev.ayni.provisioning.schema"].as_str() != Some("1")
        || labels["dev.ayni.environment.variant"].as_str() != Some(base.variant.as_str())
        || labels["dev.ayni.environment.mise-version"].as_str() != Some(base.mise_version.as_str())
    {
        return Err(BackendError::environment(
            "the lock requires a compatible language-neutral provisioning substrate; regenerate with `ayni env lock` and rebuild",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ExecutorIdentity {
        ExecutorIdentity {
            reference: "registry.example/ayni".into(),
            oci_digest: format!("sha256:{}", "a".repeat(64)),
            executable_digest: format!("sha256:{}", "b".repeat(64)),
            source_revision: "c".repeat(40),
            ayni_version: env!("CARGO_PKG_VERSION").into(),
            platform: "linux/amd64".into(),
        }
    }

    #[test]
    fn executor_requires_exact_platform_version_and_digest() {
        let executor = identity();
        assert!(executor.validate("linux/amd64").is_ok());
        assert!(executor.validate("linux/arm64").is_err());
        let mut wrong = executor.clone();
        wrong.ayni_version = "999.0.0".into();
        assert!(wrong.validate("linux/amd64").is_err());
        wrong = executor;
        wrong.executable_digest = "latest".into();
        assert!(wrong.validate("linux/amd64").is_err());
    }

    #[test]
    fn executor_assembly_follows_reusable_preparation() {
        let executor = identity();
        let mut plan = ImagePlan {
            tag: "ayni-env:lock-test".into(),
            dockerfile: "FROM substrate AS ayni-runtime\nRUN prepare-native-dependencies\n".into(),
            mise_toml: "[tools]\n".into(),
            runtime_mise_toml: "[tools]\n".into(),
            installation_digest: "sha256:installation".into(),
            preparation_groups: std::collections::BTreeMap::new(),
            platform: "linux/amd64".into(),
            preparation_digest: format!("sha256:{}", "d".repeat(64)),
        };
        let original = plan.clone();
        bind(&mut plan, &executor);
        assert!(
            plan.dockerfile
                .find("RUN prepare-native-dependencies")
                .unwrap()
                < plan.dockerfile.find("COPY --from=ayni-executor").unwrap()
        );
        assert_eq!(plan.mise_toml, original.mise_toml);
        assert_eq!(plan.preparation_digest, original.preparation_digest);
        let mut replacement = executor;
        replacement.executable_digest = format!("sha256:{}", "e".repeat(64));
        let mut replaced = original;
        bind(&mut replaced, &replacement);
        assert_ne!(plan.tag, replaced.tag);
    }

    #[test]
    fn incompatible_and_malformed_records_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_record(root.path()).unwrap().is_none());
        let path = state_path(root.path(), true).unwrap();
        fs::write(&path, "{}").unwrap();
        assert!(read_record(root.path()).is_err());
        let record = BuildRecord {
            schema_version: "old".into(),
            recipe_version: RECIPE_VERSION.into(),
            executor: identity(),
            environment_fingerprint: "unused".into(),
            preparation_digest: "unused".into(),
            image_tag: "unused".into(),
            image_id: format!("sha256:{}", "f".repeat(64)),
        };
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(
            read_record(root.path())
                .unwrap_err()
                .message
                .contains("schema or recipe")
        );
    }

    #[test]
    fn durable_substrate_does_not_follow_producer_release_tags() {
        let first = crate::resolve_provisioning_base("0.1.0", None).unwrap();
        let next = crate::resolve_provisioning_base("999.0.0", None).unwrap();
        assert_eq!(first, next);
        assert!(first.reference.ends_with("ayni-provisioning"));
        assert!(valid_digest(&first.digest));
        assert_eq!(first.mise_version, crate::BASE_MISE_VERSION);
    }

    #[cfg(unix)]
    #[test]
    fn execution_record_symlinks_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let path = state_path(root.path(), true).unwrap();
        std::os::unix::fs::symlink(external.path(), path).unwrap();
        assert!(read_record(root.path()).is_err());
    }
}
