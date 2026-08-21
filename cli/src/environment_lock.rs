use crate::application::{EnvLockOperation, EnvShowOperation, OutputFormat};
use crate::environment;
use ayni_core::{
    AdapterRegistry, ENVIRONMENT_LOCK_SCHEMA_VERSION, EnvironmentLock, EnvironmentPlan,
    EnvironmentResolutionRequest,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const LOCK_FILE: &str = ".ayni.lock";

type LockError = crate::application_error::ApplicationError;

pub(crate) fn run(operation: EnvLockOperation, registry: &AdapterRegistry) -> ExitCode {
    match lock(&operation, registry) {
        Ok((path, lock, changed)) => {
            println!(
                "{} {}",
                if changed { "wrote" } else { "current" },
                path.display()
            );
            println!("fingerprint: {}", lock.fingerprint());
            println!("targets: {}", lock.targets().len());
            println!(
                "provisioning base: {}@{}",
                lock.provisioning_base().reference,
                lock.provisioning_base().digest
            );
            ExitCode::SUCCESS
        }
        Err(error) => crate::application_error::render_error(error),
    }
}

fn lock(
    operation: &EnvLockOperation,
    registry: &AdapterRegistry,
) -> Result<(PathBuf, EnvironmentLock, bool), LockError> {
    let prepared = prepare_lock(operation, registry)?;
    let resolved_plan = resolve_plan(&prepared.plan, &prepared.repo_root, registry)?;
    let mise_version = mise_version(&prepared.repo_root)?;
    ensure_prepared_snapshot(&prepared, registry)?;
    let source_digests = merge_source_digests(
        &resolved_plan,
        &prepared.repo_root,
        prepared.source_snapshot.clone(),
    )?;
    let lock = create_lock(
        operation,
        &prepared.repo_root,
        &resolved_plan,
        mise_version,
        &source_digests,
    )?;
    let serialized = lock
        .canonical_json()
        .map_err(|error| LockError::execution(error.to_string()))?;
    ensure_prepared_snapshot(&prepared, registry)?;
    let changed = persist_if_changed(
        &prepared.destination,
        prepared.existing.as_deref(),
        &serialized,
    )?;
    Ok((prepared.destination, lock, changed))
}

struct PreparedLock {
    show: EnvShowOperation,
    plan: EnvironmentPlan,
    repo_root: PathBuf,
    destination: PathBuf,
    existing: Option<String>,
    source_snapshot: BTreeMap<String, String>,
}

fn prepare_lock(
    operation: &EnvLockOperation,
    registry: &AdapterRegistry,
) -> Result<PreparedLock, LockError> {
    let show = EnvShowOperation {
        config: operation.config.clone(),
        repo_root: operation.repo_root.clone(),
        output: OutputFormat::Json,
    };
    let plan = environment::build_plan(&show, registry)?;
    ensure_no_conflicts(&plan)?;
    let repo_root = canonical_repo_root(operation)?;
    let destination = repo_root.join(LOCK_FILE);
    let existing = validate_existing_lock(&destination)?;
    let source_snapshot = capture_source_snapshot(&show, registry, &plan, &repo_root)?;
    Ok(PreparedLock {
        show,
        plan,
        repo_root,
        destination,
        existing,
        source_snapshot,
    })
}

fn ensure_prepared_snapshot(
    prepared: &PreparedLock,
    registry: &AdapterRegistry,
) -> Result<(), LockError> {
    ensure_plan_snapshot(
        &prepared.show,
        registry,
        &prepared.plan,
        &prepared.repo_root,
        &prepared.source_snapshot,
    )
}

fn create_lock(
    operation: &EnvLockOperation,
    repo_root: &Path,
    resolved_plan: &ayni_core::ResolvedEnvironmentPlan,
    mise_version: String,
    source_digests: &BTreeMap<String, String>,
) -> Result<EnvironmentLock, LockError> {
    let provisioning_base = ayni_environment::resolve_provisioning_base(
        env!("CARGO_PKG_VERSION"),
        operation.base.as_deref(),
    )
    .map_err(LockError::from)?;
    let contract_path = contract_path(operation, repo_root)?;
    EnvironmentLock::from_resolved_plan(
        resolved_plan,
        env!("CARGO_PKG_VERSION"),
        mise_version,
        provisioning_base,
        contract_path,
        source_digests,
    )
    .map_err(|error| LockError::environment(error.to_string()))
}

fn ensure_no_conflicts(plan: &EnvironmentPlan) -> Result<(), LockError> {
    if plan.conflicts().is_empty() {
        Ok(())
    } else {
        Err(LockError::environment(format!(
            "environment plan has {} blocking conflict(s); run `ayni env show` for details",
            plan.conflicts().len()
        )))
    }
}

fn canonical_repo_root(operation: &EnvLockOperation) -> Result<PathBuf, LockError> {
    operation.repo_root.canonicalize().map_err(|error| {
        LockError::input(format!(
            "failed to establish repository root {}: {error}",
            operation.repo_root.display()
        ))
    })
}

fn contract_path(operation: &EnvLockOperation, repo_root: &Path) -> Result<String, LockError> {
    let candidate = if operation.config.is_absolute() {
        operation.config.clone()
    } else {
        repo_root.join(&operation.config)
    };
    let canonical = candidate.canonicalize().map_err(|error| {
        LockError::input(format!(
            "failed to resolve environment contract {}: {error}",
            candidate.display()
        ))
    })?;
    let relative = canonical.strip_prefix(repo_root).map_err(|_| {
        LockError::input(format!(
            "environment contract {} escapes repository root {}",
            canonical.display(),
            repo_root.display()
        ))
    })?;
    let value = relative.to_string_lossy().replace('\\', "/");
    Ok(if value.is_empty() {
        ".".to_owned()
    } else {
        value
    })
}

fn resolve_plan(
    plan: &EnvironmentPlan,
    repo_root: &Path,
    registry: &AdapterRegistry,
) -> Result<ayni_core::ResolvedEnvironmentPlan, LockError> {
    let targets = plan
        .targets()
        .iter()
        .map(|target| resolve_target(target, repo_root, registry))
        .collect::<Result<Vec<_>, _>>()?;
    EnvironmentPlan::new(
        plan.repository().clone(),
        plan.platforms().to_vec(),
        targets,
        Vec::new(),
        Vec::new(),
    )
    .and_then(|resolved| resolved.with_tools(plan.tools().to_vec()))
    .and_then(|resolved| resolved.with_debian_packages(plan.debian_packages().to_vec()))
    .and_then(|resolved| resolved.with_capabilities(plan.capabilities()))
    .and_then(|resolved| resolved.with_resource_limits(plan.resource_limits()))
    .map_err(|error| LockError::environment(error.to_string()))?
    .resolve()
    .map_err(|error| LockError::environment(error.to_string()))
}

fn resolve_target(
    target: &ayni_core::TargetEnvironment,
    repo_root: &Path,
    registry: &AdapterRegistry,
) -> Result<ayni_core::TargetEnvironment, LockError> {
    let adapter = registry
        .adapters()
        .iter()
        .find(|adapter| adapter.language() == target.target.language)
        .ok_or_else(|| {
            LockError::environment(format!(
                "{} adapter is not registered",
                target.target.language
            ))
        })?;
    let request = EnvironmentResolutionRequest::new(repo_root.to_path_buf(), target.clone())
        .map_err(|error| LockError::environment(error.to_string()))?;
    adapter
        .resolve_environment(&request)
        .map_err(resolution_error)
}

fn merge_source_digests(
    plan: &ayni_core::ResolvedEnvironmentPlan,
    repo_root: &Path,
    mut digests: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, LockError> {
    for (path, digest) in source_digests(plan.plan(), repo_root)? {
        if digests
            .insert(path.clone(), digest.clone())
            .is_some_and(|existing| existing != digest)
        {
            return Err(LockError::environment(format!(
                "environment source changed during locking: {path}"
            )));
        }
    }
    Ok(digests)
}

fn persist_if_changed(
    destination: &Path,
    existing: Option<&str>,
    serialized: &str,
) -> Result<bool, LockError> {
    let changed = existing != Some(serialized);
    if changed {
        atomic_write(destination, serialized.as_bytes())?;
    }
    Ok(changed)
}

fn source_digests(
    plan: &EnvironmentPlan,
    repo_root: &Path,
) -> Result<BTreeMap<String, String>, LockError> {
    let mut paths = BTreeSet::new();
    paths.extend(plan.tools().iter().map(|tool| tool.source.path.clone()));
    paths.extend(
        plan.debian_packages()
            .iter()
            .map(|package| package.source.path.clone()),
    );
    for target in plan.targets() {
        paths.extend(target.runtimes.iter().map(|item| item.source.path.clone()));
        if let Some(manager) = &target.package_manager {
            paths.insert(manager.source.path.clone());
        }
        paths.extend(
            target
                .signal_tools
                .iter()
                .map(|item| item.source.path.clone()),
        );
        paths.extend(
            target
                .system_requirements
                .iter()
                .map(|item| item.source.path.clone()),
        );
        paths.extend(
            target
                .dependency_locks
                .iter()
                .map(|item| item.source.path.clone()),
        );
    }
    let mut digests = BTreeMap::new();
    for path in paths {
        let candidate = repo_root.join(&path);
        let metadata = fs::metadata(&candidate).map_err(|error| {
            LockError::environment(format!(
                "failed to inspect environment source {}: {error}",
                candidate.display()
            ))
        })?;
        if !metadata.is_file() {
            continue;
        }
        let bytes =
            ayni_adapters_common::repository::read_optional_contained_bytes(repo_root, &candidate)
                .map_err(LockError::environment)?
                .ok_or_else(|| {
                    LockError::environment(format!(
                        "environment source disappeared while locking: {}",
                        candidate.display()
                    ))
                })?;
        digests.insert(path, format!("sha256:{:x}", Sha256::digest(bytes)));
    }
    for target in plan.targets() {
        for dependency in &target.dependency_locks {
            if digests.get(&dependency.path) != Some(&dependency.digest) {
                return Err(LockError::environment(format!(
                    "dependency lock changed during locking: {}",
                    dependency.path
                )));
            }
        }
    }
    Ok(digests)
}

fn capture_source_snapshot(
    show: &EnvShowOperation,
    registry: &AdapterRegistry,
    plan: &EnvironmentPlan,
    repo_root: &Path,
) -> Result<BTreeMap<String, String>, LockError> {
    let snapshot = source_digests(plan, repo_root)?;
    ensure_plan_snapshot(show, registry, plan, repo_root, &snapshot)?;
    Ok(snapshot)
}

fn ensure_plan_snapshot(
    show: &EnvShowOperation,
    registry: &AdapterRegistry,
    expected_plan: &EnvironmentPlan,
    repo_root: &Path,
    expected_digests: &BTreeMap<String, String>,
) -> Result<(), LockError> {
    let current_plan = environment::build_plan(show, registry).map_err(|error| {
        LockError::environment(format!(
            "environment inputs changed during locking: {}",
            error.message
        ))
    })?;
    if current_plan != *expected_plan {
        return Err(LockError::environment(
            "environment inputs changed during locking; rerun the command",
        ));
    }
    ensure_source_snapshot(&current_plan, repo_root, expected_digests)
}

fn ensure_source_snapshot(
    plan: &EnvironmentPlan,
    repo_root: &Path,
    expected: &BTreeMap<String, String>,
) -> Result<(), LockError> {
    if source_digests(plan, repo_root)? == *expected {
        Ok(())
    } else {
        Err(LockError::environment(
            "environment inputs changed during locking; rerun the command",
        ))
    }
}

fn resolution_error(error: ayni_core::AdapterError) -> LockError {
    let message = error.to_string();
    match error.kind {
        ayni_core::AdapterErrorKind::Environment => LockError::environment(message),
        ayni_core::AdapterErrorKind::Execution => LockError::execution(message),
    }
}

fn mise_version(repo_root: &Path) -> Result<String, LockError> {
    let args = vec![
        "--no-config".to_owned(),
        "--no-env".to_owned(),
        "--no-hooks".to_owned(),
        "version".to_owned(),
    ];
    let output = ayni_adapters_common::exec::run_command(
        repo_root,
        "mise",
        &args,
        std::time::Duration::from_secs(30),
    )
    .map_err(|error| LockError::execution(format!("failed to run mise --version: {error}")))?;
    if !output.status.success() {
        return Err(LockError::execution("mise --version failed"));
    }
    let output = String::from_utf8(output.stdout).map_err(|error| {
        LockError::execution(format!("mise --version returned non-UTF-8 output: {error}"))
    })?;
    output
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| LockError::execution("mise --version returned no version"))
}

fn validate_existing_lock(path: &Path) -> Result<Option<String>, LockError> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(LockError::input(format!(
            "lock destination must be a regular file: {}",
            path.display()
        )));
    }
    let content = fs::read_to_string(path).map_err(|error| {
        LockError::input(format!(
            "failed to read existing lock {}: {error}",
            path.display()
        ))
    })?;
    let document: serde_json::Value = serde_json::from_str(&content).map_err(|error| {
        LockError::input(format!(
            "failed to validate existing lock {}: {error}",
            path.display()
        ))
    })?;
    let schema = document
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            LockError::input(format!(
                "failed to validate existing lock {}: missing schema_version",
                path.display()
            ))
        })?;
    if schema == ENVIRONMENT_LOCK_SCHEMA_VERSION {
        serde_json::from_value::<EnvironmentLock>(document).map_err(|error| {
            LockError::input(format!(
                "failed to validate existing lock {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(Some(content))
}

fn atomic_write(destination: &Path, bytes: &[u8]) -> Result<(), LockError> {
    let parent = destination
        .parent()
        .ok_or_else(|| LockError::execution("lock path has no parent"))?;
    let (mut file, temporary) = create_temporary_lock(parent)?;
    let result = (|| {
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                LockError::execution(format!("failed to write temporary lock: {error}"))
            })?;
        fs::rename(&temporary, destination).map_err(|error| {
            LockError::execution(format!(
                "failed to atomically replace {}: {error}",
                destination.display()
            ))
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary_lock(parent: &Path) -> Result<(File, PathBuf), LockError> {
    for attempt in 0..100 {
        let path = parent.join(format!(".ayni.lock.tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(LockError::execution(format!(
                    "failed to create temporary lock: {error}"
                )));
            }
        }
    }
    Err(LockError::execution(
        "failed to allocate a unique temporary lock file",
    ))
}
