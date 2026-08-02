//! Catalog *contract* types: which tools an adapter needs, how to check
//! their versions, and how they are installed. Execution of these checks and
//! installers lives in `ayni-adapters-common`, keeping core free of tool
//! invocation.

use crate::signal::SignalKind;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Missing,
    Outdated,
    Current,
}

/// Catalog operation being attempted when a runtime failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogOperation {
    Status,
    Prepare,
    Install,
}

impl fmt::Display for CatalogOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Status => "status",
            Self::Prepare => "prepare",
            Self::Install => "install",
        })
    }
}

/// Language-neutral classification of a catalog runtime failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogOperationErrorKind {
    Spawn,
    Wait,
    Timeout,
    NonZeroExit,
    Contract,
}

/// Structured failure returned by a [`CatalogRuntime`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogOperationError {
    pub operation: CatalogOperation,
    pub kind: CatalogOperationErrorKind,
    pub command: Option<String>,
    pub cwd: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub message: String,
}

impl CatalogOperationError {
    #[must_use]
    pub fn new(
        operation: CatalogOperation,
        kind: CatalogOperationErrorKind,
        command: Option<String>,
        cwd: Option<PathBuf>,
        exit_code: Option<i32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            kind,
            command,
            cwd,
            exit_code,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn contract(operation: CatalogOperation, message: impl Into<String>) -> Self {
        Self::new(
            operation,
            CatalogOperationErrorKind::Contract,
            None,
            None,
            None,
            message,
        )
    }
}

impl fmt::Display for CatalogOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "catalog {} failed: {}",
            self.operation, self.message
        )
    }
}

impl std::error::Error for CatalogOperationError {}

/// Object-safe behavior boundary for an adapter's declarative catalog.
pub trait CatalogRuntime: Send + Sync {
    fn status(
        &self,
        entry: &CatalogEntry,
        execution: &crate::runtime::ExecutionResolution,
        timeout: Duration,
    ) -> Result<ToolStatus, CatalogOperationError>;

    /// Apply-only target preparation. Most adapters need no preparation.
    fn prepare(
        &self,
        execution: &crate::runtime::ExecutionResolution,
        timeout: Duration,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError> {
        let _ = (execution, timeout, on_line);
        Ok(())
    }

    fn install(
        &self,
        entry: &CatalogEntry,
        execution: &crate::runtime::ExecutionResolution,
        timeout: Duration,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionCheck {
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub contains: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installer {
    Cargo {
        crate_name: &'static str,
        version: Option<&'static str>,
    },
    Rustup {
        component: &'static str,
    },
    GoInstall {
        module: &'static str,
        version: Option<&'static str>,
    },
    GradleTask {
        task: &'static str,
    },
    GradleTaskAny {
        tasks: &'static [&'static str],
    },
    Bundled,
    Custom {
        program: &'static str,
        args: &'static [&'static str],
    },
    /// Behavior known only to the adapter that owns this catalog entry.
    AdapterManaged {
        key: &'static str,
        summary: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub check: Option<VersionCheck>,
    pub installer: Installer,
    pub for_signals: &'static [SignalKind],
    pub opt_in: bool,
}

#[cfg(test)]
mod tests {
    use super::Installer;

    #[test]
    fn adapter_managed_entries_are_opaque() {
        let installer = Installer::AdapterManaged {
            key: "private-requirement-key",
            summary: "install: managed by the language adapter",
        };
        assert_eq!(
            installer,
            Installer::AdapterManaged {
                key: "private-requirement-key",
                summary: "install: managed by the language adapter",
            }
        );
    }
}
