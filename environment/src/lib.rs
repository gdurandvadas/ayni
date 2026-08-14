//! Lock-driven OCI image planning and launching for Ayni managed environments.
//!
//! This layer consumes validated core lock contracts. Language adapters remain
//! responsible for ecosystem semantics; the CLI owns user intent and rendering.

mod image;
mod lock;
mod preparation;
mod runtime;

pub use image::{ImagePlan, image_plan};
pub use lock::{
    BASE_MISE_VERSION, BASE_VARIANT, LOCK_FILE, plan_matches_lock, read_lock,
    resolve_provisioning_base,
};
pub use runtime::{
    Engine, TargetSelection, build, build_prepared, detect_engine, doctor, doctor_prepared, launch,
    launch_prepared, launch_repository, launch_repository_prepared,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub code: u8,
    pub message: String,
}

impl BackendError {
    pub(crate) fn input(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    pub(crate) fn environment(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }

    pub(crate) fn execution(message: impl Into<String>) -> Self {
        Self {
            code: 4,
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
