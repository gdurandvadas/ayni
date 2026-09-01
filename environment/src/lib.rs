//! Lock-driven OCI image planning and launching for Ayni managed environments.
//!
//! This layer consumes validated core lock contracts. Language adapters remain
//! responsible for ecosystem semantics; the CLI owns user intent and rendering.

use ayni_core::{DockerAccess, EnvironmentCapabilities};

mod image;
mod lock;
mod preparation;
mod runtime;
mod storage;

pub use image::{ImagePlan, image_plan, signal_tool_coordinate};
pub use lock::{
    BASE_MISE_VERSION, BASE_VARIANT, LOCK_FILE, plan_matches_lock, read_lock,
    resolve_provisioning_base,
};
pub use runtime::{
    CapturedLaunch, Engine, LaunchAuthorization, ReadOnlyInput, TargetSelection, build,
    build_prepared, detect_engine, doctor, doctor_prepared, launch, launch_prepared,
    launch_repository, launch_repository_prepared, launch_repository_prepared_with_inputs,
    launch_repository_prepared_with_inputs_captured,
};
pub use storage::{
    StorageImage, StorageImageOwnership, StorageImagePruneScope, StoragePruneFailure,
    StoragePruneResult, StorageReport, StorageStateGeneration, prune_storage,
    prune_storage_prepared, storage_report, storage_report_prepared,
};

/// Merge backend packages required by declared execution capabilities into the
/// repository's explicitly configured Debian packages.
#[must_use]
pub fn resolve_debian_packages(
    configured: &[String],
    capabilities: EnvironmentCapabilities,
) -> Vec<String> {
    let mut resolved = configured.to_vec();
    if capabilities.docker == DockerAccess::Socket
        && !resolved
            .iter()
            .any(|package| package == "docker.io" || package.starts_with("docker.io="))
    {
        resolved.push(String::from("docker.io"));
    }
    resolved
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorKind {
    Input,
    Environment,
    Execution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub kind: BackendErrorKind,
    pub message: String,
}

impl BackendError {
    pub fn input(message: impl Into<String>) -> Self {
        Self {
            kind: BackendErrorKind::Input,
            message: message.into(),
        }
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self {
            kind: BackendErrorKind::Environment,
            message: message.into(),
        }
    }

    pub fn execution(message: impl Into<String>) -> Self {
        Self {
            kind: BackendErrorKind::Execution,
            message: message.into(),
        }
    }
}

pub(crate) fn concise_output(bytes: &[u8]) -> String {
    let output = String::from_utf8_lossy(bytes);
    let mut lines = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(8)
        .collect::<Vec<_>>();
    lines.reverse();
    if lines.is_empty() {
        String::from("command failed without diagnostics")
    } else {
        let value = lines.join("\n");
        value.chars().take(4000).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_debian_packages;
    use ayni_core::{DockerAccess, EnvironmentCapabilities, NetworkAccess};

    #[test]
    fn docker_socket_capability_owns_client_package_injection() {
        let capabilities = EnvironmentCapabilities {
            docker: DockerAccess::Socket,
            network: NetworkAccess::None,
        };
        assert_eq!(
            resolve_debian_packages(&[String::from("libssl-dev")], capabilities),
            ["libssl-dev", "docker.io"]
        );
        assert_eq!(
            resolve_debian_packages(&[String::from("docker.io=1.2.3")], capabilities),
            ["docker.io=1.2.3"]
        );
    }
}
