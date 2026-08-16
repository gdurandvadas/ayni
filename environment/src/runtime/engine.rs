use super::canonical_root;
use crate::image::{
    IMAGE_AYNI_LABEL, IMAGE_BASE_LABEL, IMAGE_LOCK_LABEL, IMAGE_MISE_LABEL, IMAGE_PLATFORM_LABEL,
    IMAGE_PREPARATION_LABEL, IMAGE_SCHEMA_LABEL, IMAGE_SCHEMA_VERSION, ImagePlan,
    image_plan_with_preparation,
};
use crate::{BackendError, concise_output, read_lock};
use ayni_adapters_common::exec::{DEFAULT_TOOL_TIMEOUT, run_command, run_command_streaming};
use ayni_core::{DependencyPreparationPlan, EnvironmentLock, Language};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

pub(super) fn engine_name(engine: Engine) -> &'static str {
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

pub(super) fn write_new_file(path: &Path, content: &str) -> std::io::Result<()> {
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

pub(super) fn validate_image(
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
