use crate::adapter::VerificationSelectorSupport;
use crate::language::Language;
use crate::signal::{
    ComplexityOffender, CoverageOffender, DepsOffender, MutationOffender, Offenders, SignalKind,
    SizeOffender, TestFailure,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Adapter-owned, signal-neutral hint from which the CLI can render an exact
/// `ayni verify` command. Core deliberately does not know CLI argument syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VerificationTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl VerificationTarget {
    /// Validate selector invariants without depending on CLI argument types.
    pub fn validate(&self) -> Result<(), FindingError> {
        validate_verification_target(self)
    }
}

/// Staged verification metadata. VFY-3.3 replaces this typed target with the
/// validated, shell-safe `command` in the public artifact wire representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<VerificationTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// Metadata common to every typed finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingMetadata {
    pub id: String,
    pub verification: VerificationMetadata,
}

impl FindingMetadata {
    fn validate(&self) -> Result<(), FindingError> {
        validate_finding_id(&self.id)?;
        match (&self.verification.target, &self.verification.command) {
            (Some(target), None) => target.validate(),
            (None, Some(command)) if command.starts_with("ayni verify ") => Ok(()),
            _ => Err(FindingError::InvalidVerificationTarget(
                "verification must contain exactly one validated target or command",
            )),
        }
    }
}

impl Serialize for FindingMetadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        #[derive(Serialize)]
        struct Wire<'a> {
            id: &'a str,
            verification: &'a VerificationMetadata,
        }
        Wire {
            id: &self.id,
            verification: &self.verification,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FindingMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            id: String,
            verification: VerificationMetadata,
        }
        let wire = Wire::deserialize(deserializer)?;
        let metadata = Self {
            id: wire.id,
            verification: wire.verification,
        };
        metadata.validate().map_err(serde::de::Error::custom)?;
        Ok(metadata)
    }
}

/// A staged typed finding. Metadata and offender fields are flattened so the
/// final wire shape can add `id` and `verification` without nesting the
/// historical offender payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding<T: Serialize> {
    #[serde(flatten)]
    pub metadata: FindingMetadata,
    #[serde(flatten)]
    pub offender: T,
}

/// Typed finding collections corresponding one-for-one with offender kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "items", rename_all = "snake_case")]
pub enum Findings {
    Test(Vec<Finding<TestFailure>>),
    Coverage(Vec<Finding<CoverageOffender>>),
    Size(Vec<Finding<SizeOffender>>),
    Complexity(Vec<Finding<ComplexityOffender>>),
    Deps(Vec<Finding<DepsOffender>>),
    Mutation(Vec<Finding<MutationOffender>>),
}

impl Findings {
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        macro_rules! ids {
            ($items:expr) => {
                $items
                    .iter()
                    .map(|finding| finding.metadata.id.as_str())
                    .collect()
            };
        }
        match self {
            Self::Test(items) => ids!(items),
            Self::Coverage(items) => ids!(items),
            Self::Size(items) => ids!(items),
            Self::Complexity(items) => ids!(items),
            Self::Deps(items) => ids!(items),
            Self::Mutation(items) => ids!(items),
        }
    }

    #[must_use]
    pub fn commands(&self) -> Vec<&str> {
        macro_rules! commands {
            ($items:expr) => {
                $items
                    .iter()
                    .filter_map(|finding| finding.metadata.verification.command.as_deref())
                    .collect()
            };
        }
        match self {
            Self::Test(items) => commands!(items),
            Self::Coverage(items) => commands!(items),
            Self::Size(items) => commands!(items),
            Self::Complexity(items) => commands!(items),
            Self::Deps(items) => commands!(items),
            Self::Mutation(items) => commands!(items),
        }
    }

    pub(crate) fn validate_wire(&self) -> Result<(), FindingError> {
        let mut ids = HashSet::new();
        macro_rules! validate_items {
            ($items:expr) => {
                for finding in $items {
                    finding.metadata.validate()?;
                    if finding.metadata.verification.target.is_some()
                        || finding.metadata.verification.command.is_none()
                    {
                        return Err(FindingError::InvalidVerificationTarget(
                            "serialized findings must contain a rendered verification command",
                        ));
                    }
                    if !ids.insert(finding.metadata.id.as_str()) {
                        return Err(FindingError::InvalidIdentity(finding.metadata.id.clone()));
                    }
                }
            };
        }
        match self {
            Self::Test(items) => validate_items!(items),
            Self::Coverage(items) => validate_items!(items),
            Self::Size(items) => validate_items!(items),
            Self::Complexity(items) => validate_items!(items),
            Self::Deps(items) => validate_items!(items),
            Self::Mutation(items) => validate_items!(items),
        }
        Ok(())
    }

    pub(crate) fn matches_offenders(&self, offenders: &Offenders) -> bool {
        match (self, offenders) {
            (Self::Test(findings), Offenders::Test(offenders)) => findings
                .iter()
                .map(|finding| &finding.offender)
                .eq(offenders),
            (Self::Coverage(findings), Offenders::Coverage(offenders)) => findings
                .iter()
                .map(|finding| &finding.offender)
                .eq(offenders),
            (Self::Size(findings), Offenders::Size(offenders)) => findings
                .iter()
                .map(|finding| &finding.offender)
                .eq(offenders),
            (Self::Complexity(findings), Offenders::Complexity(offenders)) => findings
                .iter()
                .map(|finding| &finding.offender)
                .eq(offenders),
            (Self::Deps(findings), Offenders::Deps(offenders)) => findings
                .iter()
                .map(|finding| &finding.offender)
                .eq(offenders),
            (Self::Mutation(findings), Offenders::Mutation(offenders)) => findings
                .iter()
                .map(|finding| &finding.offender)
                .eq(offenders),
            _ => false,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Test(items) => items.is_empty(),
            Self::Coverage(items) => items.is_empty(),
            Self::Size(items) => items.is_empty(),
            Self::Complexity(items) => items.is_empty(),
            Self::Deps(items) => items.is_empty(),
            Self::Mutation(items) => items.is_empty(),
        }
    }

    /// Replace staged adapter targets with final CLI-rendered commands before
    /// writing an artifact. The target is consumed so it cannot leak onto the
    /// public artifact wire shape.
    pub fn render_commands<F>(&mut self, mut render: F) -> Result<(), FindingError>
    where
        F: FnMut(&VerificationTarget) -> Result<String, FindingError>,
    {
        macro_rules! render_items {
            ($items:expr) => {
                for finding in $items {
                    let target = finding.metadata.verification.target.as_ref().ok_or(
                        FindingError::InvalidVerificationTarget(
                            "finding command was already rendered",
                        ),
                    )?;
                    let command = render(target)?;
                    finding.metadata.verification.target = None;
                    finding.metadata.verification.command = Some(command);
                    finding.metadata.validate()?;
                }
            };
        }
        match self {
            Self::Test(items) => render_items!(items),
            Self::Coverage(items) => render_items!(items),
            Self::Size(items) => render_items!(items),
            Self::Complexity(items) => render_items!(items),
            Self::Deps(items) => render_items!(items),
            Self::Mutation(items) => render_items!(items),
        }
        Ok(())
    }
    /// Validate every mapped target against the adapter's public selector
    /// declaration. This keeps capability and finding mapping one contract.
    pub fn validate_targets(
        &self,
        kind: SignalKind,
        support: VerificationSelectorSupport,
    ) -> Result<(), FindingError> {
        if self.kind() != kind {
            return Err(FindingError::SignalKindMismatch);
        }
        macro_rules! validate {
            ($items:expr) => {
                for finding in $items {
                    let target = finding.metadata.verification.target.as_ref().ok_or(
                        FindingError::InvalidVerificationTarget("finding target is unavailable"),
                    )?;
                    support.validate_target(kind, target)?;
                }
            };
        }
        match self {
            Self::Test(items) => validate!(items),
            Self::Coverage(items) => validate!(items),
            Self::Size(items) => validate!(items),
            Self::Complexity(items) => validate!(items),
            Self::Deps(items) => validate!(items),
            Self::Mutation(items) => validate!(items),
        }
        Ok(())
    }

    pub(crate) fn kind(&self) -> SignalKind {
        match self {
            Self::Test(_) => SignalKind::Test,
            Self::Coverage(_) => SignalKind::Coverage,
            Self::Size(_) => SignalKind::Size,
            Self::Complexity(_) => SignalKind::Complexity,
            Self::Deps(_) => SignalKind::Deps,
            Self::Mutation(_) => SignalKind::Mutation,
        }
    }
}

/// Borrowed semantic offender supplied to adapter-owned target mapping.
#[derive(Debug, Clone, Copy)]
pub enum OffenderIdentity<'a> {
    Test(&'a TestFailure),
    Coverage(&'a CoverageOffender),
    Size(&'a SizeOffender),
    Complexity(&'a ComplexityOffender),
    Deps(&'a DepsOffender),
    Mutation(&'a MutationOffender),
}

/// Fail-closed finding construction errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingError {
    IdentityCollision(String),
    InvalidIdentity(String),
    InvalidVerificationTarget(&'static str),
    UnsupportedVerificationSelector(&'static str),
    SignalKindMismatch,
}

impl std::fmt::Display for FindingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdentityCollision(id) => write!(formatter, "finding identity collision: {id}"),
            Self::InvalidIdentity(id) => write!(formatter, "invalid finding identity: {id}"),
            Self::InvalidVerificationTarget(message) => formatter.write_str(message),
            Self::UnsupportedVerificationSelector(selector) => write!(
                formatter,
                "verification target uses unsupported {selector} selector"
            ),
            Self::SignalKindMismatch => {
                formatter.write_str("finding kind does not match the collected signal row")
            }
        }
    }
}

impl std::error::Error for FindingError {}

impl Offenders {
    /// Deduplicate semantic offenders and assign stable identities before
    /// adapter-owned verification targets are attached. First occurrence order
    /// is retained, but does not participate in identity generation.
    pub fn into_findings<F>(
        self,
        language: Language,
        checkout_root: &str,
        mut target_for: F,
    ) -> Result<Findings, FindingError>
    where
        F: FnMut(OffenderIdentity<'_>) -> VerificationTarget,
    {
        macro_rules! findings {
            ($items:expr, $kind:expr, $variant:ident) => {{
                let mut keys = HashSet::new();
                let mut ids = HashMap::<String, Vec<u8>>::new();
                let mut findings = Vec::new();
                for offender in $items {
                    let identity = OffenderIdentity::$variant(&offender);
                    let key = canonical_finding_key(language, $kind, identity, checkout_root);
                    if !keys.insert(key.clone()) {
                        continue;
                    }
                    let id = finding_id(&key);
                    if ids
                        .insert(id.clone(), key.clone())
                        .is_some_and(|old| old != key)
                    {
                        return Err(FindingError::IdentityCollision(id));
                    }
                    let target = target_for(identity);
                    validate_verification_target(&target)?;
                    findings.push(Finding {
                        metadata: FindingMetadata {
                            id,
                            verification: VerificationMetadata {
                                target: Some(target),
                                command: None,
                            },
                        },
                        offender,
                    });
                }
                Findings::$variant(findings)
            }};
        }

        Ok(match self {
            Self::Test(items) => findings!(items, SignalKind::Test, Test),
            Self::Coverage(items) => findings!(items, SignalKind::Coverage, Coverage),
            Self::Size(items) => findings!(items, SignalKind::Size, Size),
            Self::Complexity(items) => findings!(items, SignalKind::Complexity, Complexity),
            Self::Deps(items) => findings!(items, SignalKind::Deps, Deps),
            Self::Mutation(items) => findings!(items, SignalKind::Mutation, Mutation),
        })
    }
}

fn validate_verification_target(target: &VerificationTarget) -> Result<(), FindingError> {
    if target.file.is_some() && target.package.is_some() {
        return Err(FindingError::InvalidVerificationTarget(
            "verification target cannot combine file and package selectors",
        ));
    }
    if target
        .file
        .iter()
        .chain(target.package.iter())
        .chain(target.name.iter())
        .any(|value| value.is_empty())
    {
        return Err(FindingError::InvalidVerificationTarget(
            "verification target selectors cannot be empty",
        ));
    }
    Ok(())
}

fn validate_finding_id(id: &str) -> Result<(), FindingError> {
    const PREFIX: &str = "ayni:finding:v1:sha256:";
    let digest = id
        .strip_prefix(PREFIX)
        .filter(|digest| digest.len() == 64)
        .filter(|digest| {
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if digest.is_none() {
        return Err(FindingError::InvalidIdentity(id.to_owned()));
    }
    Ok(())
}

fn canonical_finding_key(
    language: Language,
    kind: SignalKind,
    offender: OffenderIdentity<'_>,
    checkout_root: &str,
) -> Vec<u8> {
    let mut fields = Vec::<String>::new();
    fields.push(language.as_str().to_owned());
    fields.push(signal_name(kind).to_owned());
    match offender {
        OffenderIdentity::Test(value) => {
            fields.push(canonical_optional_path(
                value.file.as_deref(),
                checkout_root,
            ));
            fields.push(optional_u64(value.line));
            fields.push(value.test_name.clone().unwrap_or_default());
            // Tool-only or synthetic failures may have no location/name. Their
            // diagnostic is then the only stable semantic identity available.
            if value.file.is_none() && value.line.is_none() && value.test_name.is_none() {
                fields.push(value.message.clone());
            }
        }
        OffenderIdentity::Coverage(value) => {
            fields.push(canonical_path(&value.file, checkout_root));
            fields.push(optional_u64(value.line));
        }
        OffenderIdentity::Size(value) => fields.push(canonical_path(&value.file, checkout_root)),
        OffenderIdentity::Complexity(value) => {
            fields.push(canonical_path(&value.file, checkout_root));
            fields.push(value.line.to_string());
            fields.push(value.function.clone());
        }
        OffenderIdentity::Deps(value) => {
            fields.push(value.from.clone());
            fields.push(value.to.clone());
            fields.push(value.rule.clone());
        }
        OffenderIdentity::Mutation(value) => {
            fields.push(canonical_optional_path(
                value.file.as_deref(),
                checkout_root,
            ));
            fields.push(optional_u64(value.line));
            fields.push(value.mutation_kind.clone());
            fields.push(value.message.clone());
        }
    }

    let mut canonical = b"ayni-finding-identity\0v1\0".to_vec();
    for field in fields {
        canonical.extend_from_slice(field.len().to_string().as_bytes());
        canonical.push(b':');
        canonical.extend_from_slice(field.as_bytes());
    }
    canonical
}

fn finding_id(canonical: &[u8]) -> String {
    let encoded = crate::sha256_hex(canonical);
    format!("ayni:finding:v1:sha256:{encoded}")
}

fn signal_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

fn canonical_optional_path(path: Option<&str>, checkout_root: &str) -> String {
    path.map_or_else(String::new, |path| canonical_path(path, checkout_root))
}

fn canonical_path(path: &str, checkout_root: &str) -> String {
    let path = path.replace('\\', "/");
    let root = checkout_root.replace('\\', "/");
    let relative = path
        .strip_prefix(root.trim_end_matches('/'))
        .and_then(|suffix| suffix.strip_prefix('/'))
        .unwrap_or(&path);
    relative.strip_prefix("./").unwrap_or(relative).to_owned()
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

#[cfg(test)]
mod finding_tests {
    use super::*;
    use crate::signal::Level;
    use std::collections::HashSet;

    #[test]
    fn finding_ids_ignore_values_levels_order_and_checkout_root() {
        let offender = |root: &str, value: u64, level| SizeOffender {
            file: format!("{root}/src/lib.rs"),
            value,
            warn: value.saturating_sub(1),
            fail: value + 1,
            level,
        };
        let first = Offenders::Size(vec![
            offender("/one/checkout", 20, Level::Warn),
            SizeOffender {
                file: String::from("src/other.rs"),
                value: 40,
                warn: 30,
                fail: 50,
                level: Level::Warn,
            },
        ])
        .into_findings(Language::Rust, "/one/checkout", |_| {
            VerificationTarget::default()
        })
        .expect("findings");
        let second = Offenders::Size(vec![
            SizeOffender {
                file: String::from("src/other.rs"),
                value: 999,
                warn: 1,
                fail: 2,
                level: Level::Fail,
            },
            offender("/different/checkout", 800, Level::Fail),
        ])
        .into_findings(Language::Rust, "/different/checkout", |_| {
            VerificationTarget::default()
        })
        .expect("findings");

        let ids = |findings: Findings| match findings {
            Findings::Size(items) => items
                .into_iter()
                .map(|finding| finding.metadata.id)
                .collect::<HashSet<_>>(),
            _ => panic!("expected size findings"),
        };
        assert_eq!(ids(first), ids(second));
    }

    #[test]
    fn finding_assignment_deduplicates_semantic_duplicates() {
        let findings = Offenders::Coverage(vec![
            CoverageOffender {
                file: String::from("src/lib.rs"),
                line: Some(8),
                value: 10.0,
                level: Level::Warn,
            },
            CoverageOffender {
                file: String::from("src/lib.rs"),
                line: Some(8),
                value: 90.0,
                level: Level::Fail,
            },
        ])
        .into_findings(Language::Rust, ".", |_| VerificationTarget {
            file: Some(String::from("src/lib.rs")),
            ..VerificationTarget::default()
        })
        .expect("findings");

        let Findings::Coverage(items) = findings else {
            panic!("expected coverage findings");
        };
        assert_eq!(items.len(), 1);
        assert!(items[0].metadata.id.starts_with("ayni:finding:v1:sha256:"));
        assert_eq!(items[0].metadata.id.len(), 87);
        let serialized = serde_json::to_value(&items[0]).expect("serialize finding");
        assert!(serialized.get("id").is_some());
        assert_eq!(serialized["verification"]["target"]["file"], "src/lib.rs");
        assert!(serialized.get("offender").is_none());
    }

    #[test]
    fn synthetic_zero_test_finding_has_stable_identity() {
        let synthetic = || TestFailure {
            file: None,
            line: None,
            message: String::from("test runner completed successfully but discovered zero tests"),
            test_name: None,
        };
        let make = || {
            Offenders::Test(vec![synthetic(), synthetic()])
                .into_findings(Language::Python, "/checkout", |_| {
                    VerificationTarget::default()
                })
                .expect("findings")
        };
        let (Findings::Test(first), Findings::Test(second)) = (make(), make()) else {
            panic!("expected test findings");
        };
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].metadata.id, second[0].metadata.id);
    }

    #[test]
    fn finding_assignment_rejects_invalid_verification_target() {
        let error = Offenders::Size(vec![SizeOffender {
            file: String::from("src/lib.rs"),
            value: 10,
            warn: 5,
            fail: 8,
            level: Level::Fail,
        }])
        .into_findings(Language::Rust, ".", |_| VerificationTarget {
            file: Some(String::from("src/lib.rs")),
            package: Some(String::from("core")),
            name: None,
        })
        .expect_err("invalid target");
        assert!(error.to_string().contains("cannot combine"));
    }

    #[test]
    fn every_finding_variant_serializes_flat_rendered_metadata() {
        let cases = [
            Offenders::Test(vec![TestFailure {
                file: Some(String::from("tests/api.rs")),
                line: Some(4),
                message: String::from("failed"),
                test_name: Some(String::from("creates")),
            }]),
            Offenders::Coverage(vec![CoverageOffender {
                file: String::from("src/api.rs"),
                line: Some(4),
                value: 50.0,
                level: Level::Fail,
            }]),
            Offenders::Size(vec![SizeOffender {
                file: String::from("src/api.rs"),
                value: 20,
                warn: 10,
                fail: 15,
                level: Level::Fail,
            }]),
            Offenders::Complexity(vec![ComplexityOffender {
                file: String::from("src/api.rs"),
                line: 4,
                function: String::from("create"),
                cyclomatic: 20.0,
                cognitive: None,
                level: Level::Fail,
            }]),
            Offenders::Deps(vec![DepsOffender {
                from: String::from("api"),
                to: String::from("db"),
                rule: String::from("forbidden"),
                level: Level::Fail,
            }]),
            Offenders::Mutation(vec![MutationOffender {
                file: Some(String::from("src/api.rs")),
                line: Some(4),
                mutation_kind: String::from("replace"),
                message: String::from("survived"),
                level: Level::Fail,
            }]),
        ];

        for offenders in cases {
            let mut findings = offenders
                .into_findings(Language::Rust, "/checkout", |_| {
                    VerificationTarget::default()
                })
                .expect("findings");
            findings
                .render_commands(|_| Ok(String::from("ayni verify test --language rust")))
                .expect("rendered command");
            let serialized = serde_json::to_value(findings).expect("serialize findings");
            let finding = &serialized["items"][0];
            assert!(
                finding["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("ayni:finding:v1:"))
            );
            assert_eq!(
                finding["verification"]["command"],
                "ayni verify test --language rust"
            );
            assert!(finding["verification"].get("target").is_none());
            assert!(finding.get("offender").is_none());
        }
    }

    #[test]
    fn finding_metadata_serialization_rejects_invalid_identity() {
        let metadata = FindingMetadata {
            id: String::from("truncated"),
            verification: VerificationMetadata {
                target: Some(VerificationTarget::default()),
                command: None,
            },
        };
        let error = serde_json::to_value(metadata).expect_err("invalid metadata");
        assert!(error.to_string().contains("invalid finding identity"));
    }
}
