//! Typed, adapter-owned native dependency preparation contracts.
//!
//! These contracts describe deterministic argv invocations only. They do not
//! select OCI runtimes, execute commands, or permit checkout mutation.

use crate::{AdapterError, Language, TargetEnvironment, TargetIdentity};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Digest-tracked repository input required to prepare native dependencies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct PreparationInput {
    pub path: String,
    pub digest: String,
    pub owner_root: String,
}

/// One structured native dependency command. `program` and `args` are never a
/// shell fragment; callers execute them directly with the supplied cwd and
/// explicit environment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct PreparationCommand {
    pub program: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
}

impl PreparationCommand {
    pub fn new(
        language: Language,
        program: impl Into<String>,
        args: Vec<String>,
        cwd: impl AsRef<str>,
        environment: BTreeMap<String, String>,
    ) -> Result<Self, AdapterError> {
        let program = program.into();
        if program.is_empty()
            || program.contains(char::is_whitespace)
            || program.contains('/')
            || program.contains('\\')
        {
            return Err(AdapterError::new(
                language,
                "preparation program must be a bare executable name",
            ));
        }
        if args.iter().any(|arg| arg.contains('\u{0}')) {
            return Err(AdapterError::new(
                language,
                "preparation arguments must not contain NUL",
            ));
        }
        for (key, value) in &environment {
            if !is_environment_key(key) || value.contains('\u{0}') {
                return Err(AdapterError::new(
                    language,
                    "preparation environment contains an unsafe entry",
                ));
            }
        }
        Ok(Self {
            program,
            args,
            cwd: normalize_relative("preparation cwd", cwd.as_ref())
                .map_err(|message| AdapterError::new(language, message))?,
            environment,
        })
    }
}

/// Minimal generated file required to make a digest-tracked manifest usable in
/// an isolated preparation context. Repository source is never copied.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct PreparationScaffold {
    pub path: String,
    pub content: String,
}

/// How an output directory is initialized before offline materialization.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PreparationOutputMode {
    /// Copy the prepared image tree before running materialization commands.
    #[default]
    Seeded,
    /// Start from an empty directory. Use this for non-relocatable environments.
    Fresh,
}

/// One generated directory produced while preparing dependencies and mounted
/// over its repository location during managed execution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct PreparationOutput {
    pub path: String,
    pub mount_path: String,
    #[serde(default)]
    pub mode: PreparationOutputMode,
}

/// Adapter-provided plan for preparing one target's native dependencies in an
/// isolated workspace. Backends must stage these inputs rather than run these
/// commands against the checkout.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DependencyPreparationPlan {
    pub target: TargetIdentity,
    pub inputs: Vec<PreparationInput>,
    pub commands: Vec<PreparationCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scaffolds: Vec<PreparationScaffold>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materialization_commands: Vec<PreparationCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<PreparationOutput>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub execution_environment: BTreeMap<String, String>,
}

impl DependencyPreparationPlan {
    pub fn new(
        target: TargetIdentity,
        mut inputs: Vec<PreparationInput>,
        commands: Vec<PreparationCommand>,
        mut scaffolds: Vec<PreparationScaffold>,
        materialization_commands: Vec<PreparationCommand>,
        mut outputs: Vec<PreparationOutput>,
        execution_environment: BTreeMap<String, String>,
    ) -> Result<Self, AdapterError> {
        if inputs.is_empty() || commands.is_empty() {
            return Err(AdapterError::new(
                target.language,
                "dependency preparation needs tracked inputs and at least one command",
            ));
        }
        normalize_inputs(target.language, &mut inputs)?;
        normalize_scaffolds(target.language, &mut scaffolds)?;
        normalize_outputs(target.language, &mut outputs)?;
        validate_execution_environment(target.language, &execution_environment)?;
        Ok(Self {
            target,
            inputs,
            commands,
            scaffolds,
            materialization_commands,
            outputs,
            execution_environment,
        })
    }
}

fn normalize_inputs(
    language: Language,
    inputs: &mut Vec<PreparationInput>,
) -> Result<(), AdapterError> {
    for input in inputs.iter_mut() {
        input.path = normalize_relative("preparation input", &input.path)
            .map_err(|message| AdapterError::new(language, message))?;
        input.owner_root = normalize_relative("preparation owner root", &input.owner_root)
            .map_err(|message| AdapterError::new(language, message))?;
        validate_digest(&input.digest).map_err(|message| AdapterError::new(language, message))?;
    }
    inputs.sort();
    inputs.dedup();
    Ok(())
}

fn normalize_scaffolds(
    language: Language,
    scaffolds: &mut Vec<PreparationScaffold>,
) -> Result<(), AdapterError> {
    for scaffold in scaffolds.iter_mut() {
        scaffold.path = normalize_relative("preparation scaffold", &scaffold.path)
            .map_err(|message| AdapterError::new(language, message))?;
        if scaffold.content.contains('\u{0}') {
            return Err(AdapterError::new(
                language,
                "preparation scaffold must not contain NUL",
            ));
        }
    }
    scaffolds.sort();
    scaffolds.dedup();
    Ok(())
}

fn normalize_outputs(
    language: Language,
    outputs: &mut Vec<PreparationOutput>,
) -> Result<(), AdapterError> {
    for output in outputs.iter_mut() {
        output.path = normalize_relative("preparation output", &output.path)
            .map_err(|message| AdapterError::new(language, message))?;
        output.mount_path = normalize_relative("preparation mount path", &output.mount_path)
            .map_err(|message| AdapterError::new(language, message))?;
    }
    outputs.sort();
    outputs.dedup();
    Ok(())
}

fn validate_execution_environment(
    language: Language,
    environment: &BTreeMap<String, String>,
) -> Result<(), AdapterError> {
    if environment
        .iter()
        .any(|(key, value)| !is_environment_key(key) || value.contains('\u{0}'))
    {
        Err(AdapterError::new(
            language,
            "execution environment contains an unsafe entry",
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DependencyPreparationRequest {
    repo_root: PathBuf,
    target: TargetEnvironment,
}

impl DependencyPreparationRequest {
    pub fn new(repo_root: PathBuf, target: TargetEnvironment) -> Result<Self, AdapterError> {
        let language = target.target.language;
        if !repo_root.is_absolute() {
            return Err(AdapterError::new(
                language,
                "dependency preparation repository root must be absolute",
            ));
        }
        let repo_root = repo_root.canonicalize().map_err(|error| {
            AdapterError::new(
                language,
                format!(
                    "failed to establish dependency preparation repository root {}: {error}",
                    repo_root.display()
                ),
            )
        })?;
        Ok(Self { repo_root, target })
    }
    #[must_use]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
    #[must_use]
    pub const fn target(&self) -> &TargetEnvironment {
        &self.target
    }
}

/// Optional adapter capability for deterministic native dependency preparation.
pub trait DependencyPreparationCapability: Send + Sync {
    fn language(&self) -> Language;
    fn prepare(
        &self,
        request: &DependencyPreparationRequest,
    ) -> Result<DependencyPreparationPlan, AdapterError>;
}

fn normalize_relative(field: &str, value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    let path = Path::new(value);
    if path.is_absolute() || value.contains('\\') || value.contains(':') {
        return Err(format!("{field} must be a portable relative path"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => return Err(format!("{field} must be a portable relative path")),
        }
    }
    Ok(if parts.is_empty() {
        String::from(".")
    } else {
        parts.join("/")
    })
}

fn validate_digest(value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(String::from("preparation input digest must use sha256"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(String::from(
            "preparation input digest must be a SHA-256 digest",
        ));
    }
    Ok(())
}

fn is_environment_key(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || (byte.is_ascii_alphanumeric() && (index != 0 || !byte.is_ascii_digit()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_are_argv_with_portable_cwd_and_explicit_environment() {
        let command = PreparationCommand::new(
            Language::Rust,
            "cargo",
            vec!["fetch".into(), "--locked".into()],
            ".",
            BTreeMap::from([(String::from("CARGO_NET_OFFLINE"), String::from("true"))]),
        )
        .expect("command");
        assert_eq!(command.cwd, ".");
        assert!(
            PreparationCommand::new(Language::Node, "sh -c", vec![], ".", BTreeMap::new()).is_err()
        );
        assert!(
            PreparationCommand::new(
                Language::Rust,
                "cargo",
                vec![],
                "../outside",
                BTreeMap::new()
            )
            .is_err()
        );
    }
}
