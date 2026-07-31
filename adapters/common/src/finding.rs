//! Shared, command-neutral mapping from typed offenders to focused selectors.

use ayni_core::{
    OffenderIdentity, Scope, SignalKind, VerificationSelectorSupport, VerificationTarget,
};

/// Semantic shape of a dependency offender's `from` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencySource {
    File,
    Package,
    Unscoped,
}

/// Select the narrowest target supported by both the offender data and the
/// adapter's declared focused-verification capability.
#[must_use]
pub fn target_for_finding(
    _kind: SignalKind,
    support: VerificationSelectorSupport,
    scope: &Scope,
    offender: OffenderIdentity<'_>,
    dependency_source: DependencySource,
) -> VerificationTarget {
    match offender {
        OffenderIdentity::Test(failure) => test_target(support, scope, failure),
        OffenderIdentity::Coverage(offender) => file_target(support, &offender.file),
        OffenderIdentity::Size(offender) => file_target(support, &offender.file),
        OffenderIdentity::Complexity(offender) => file_target(support, &offender.file),
        OffenderIdentity::Deps(offender) => match dependency_source {
            DependencySource::File if support.file => VerificationTarget {
                file: Some(offender.from.clone()),
                ..VerificationTarget::default()
            },
            DependencySource::Package if support.package => VerificationTarget {
                package: Some(offender.from.clone()),
                ..VerificationTarget::default()
            },
            _ => VerificationTarget::default(),
        },
        OffenderIdentity::Mutation(offender) => offender
            .file
            .as_deref()
            .map_or_else(VerificationTarget::default, |file| {
                file_target(support, file)
            }),
    }
}

fn file_target(support: VerificationSelectorSupport, file: &str) -> VerificationTarget {
    if support.file {
        VerificationTarget {
            file: Some(file.to_owned()),
            ..VerificationTarget::default()
        }
    } else {
        VerificationTarget::default()
    }
}

fn test_target(
    support: VerificationSelectorSupport,
    scope: &Scope,
    failure: &ayni_core::TestFailure,
) -> VerificationTarget {
    let name = support.name.then(|| failure.test_name.clone()).flatten();
    if support.file
        && let Some(file) = &failure.file
    {
        return VerificationTarget {
            file: Some(file.clone()),
            package: None,
            name,
        };
    }
    if support.package
        && let Some(package) = &scope.package
    {
        return VerificationTarget {
            file: None,
            package: Some(package.clone()),
            name,
        };
    }
    if name.is_some() {
        return VerificationTarget {
            name,
            ..VerificationTarget::default()
        };
    }

    // Synthetic zero-test findings have no offender location. Reuse an
    // already-scoped run when that selector is genuinely supported; otherwise
    // an unscoped language verification remains an actionable fallback.
    if support.file
        && let Some(file) = &scope.file
    {
        return file_target(support, file);
    }
    if support.package
        && let Some(package) = &scope.package
    {
        return VerificationTarget {
            package: Some(package.clone()),
            ..VerificationTarget::default()
        };
    }
    VerificationTarget::default()
}

#[cfg(test)]
mod tests {
    use super::{DependencySource, target_for_finding};
    use ayni_core::{
        ComplexityOffender, CoverageOffender, DepsOffender, Level, MutationOffender,
        OffenderIdentity, Scope, SignalKind, SizeOffender, TestFailure,
        VerificationSelectorSupport, VerificationTarget,
    };

    fn scope() -> Scope {
        Scope {
            workspace_root: String::from("."),
            path: None,
            package: Some(String::from("api")),
            file: Some(String::from("tests/api.rs")),
        }
    }

    #[test]
    fn finding_test_target_combines_supported_location_and_name() {
        let failure = TestFailure {
            file: Some(String::from("tests/api.test.ts")),
            line: Some(4),
            message: String::from("failed"),
            test_name: Some(String::from("creates user")),
        };
        let target = target_for_finding(
            SignalKind::Test,
            VerificationSelectorSupport::new(true, true, true),
            &scope(),
            OffenderIdentity::Test(&failure),
            DependencySource::Unscoped,
        );
        assert_eq!(
            target,
            VerificationTarget {
                file: failure.file,
                package: None,
                name: failure.test_name,
            }
        );
    }

    #[test]
    fn finding_zero_test_target_reuses_supported_scope() {
        let failure = TestFailure {
            file: None,
            line: None,
            message: String::from("discovered zero tests"),
            test_name: None,
        };
        let target = target_for_finding(
            SignalKind::Test,
            VerificationSelectorSupport::new(false, true, true),
            &scope(),
            OffenderIdentity::Test(&failure),
            DependencySource::Unscoped,
        );
        assert_eq!(target.package.as_deref(), Some("api"));
    }

    #[test]
    fn finding_dependency_target_obeys_source_semantics_and_capability() {
        let offender = DepsOffender {
            from: String::from("crates/api"),
            to: String::from("crates/db"),
            rule: String::from("api -> db"),
            level: Level::Fail,
        };
        let target = target_for_finding(
            SignalKind::Deps,
            VerificationSelectorSupport::new(true, true, false),
            &scope(),
            OffenderIdentity::Deps(&offender),
            DependencySource::Package,
        );
        assert_eq!(target.package.as_deref(), Some("crates/api"));
        assert!(target.file.is_none());
    }

    #[test]
    fn finding_file_bearing_signals_use_file_only_when_supported() {
        let support = VerificationSelectorSupport::new(true, false, false);
        let coverage = CoverageOffender {
            file: String::from("src/api.rs"),
            line: Some(3),
            value: 50.0,
            level: Level::Fail,
        };
        let size = SizeOffender {
            file: String::from("src/api.rs"),
            value: 800,
            warn: 400,
            fail: 700,
            level: Level::Fail,
        };
        let complexity = ComplexityOffender {
            file: String::from("src/api.rs"),
            line: 8,
            function: String::from("create"),
            cyclomatic: 20.0,
            cognitive: None,
            level: Level::Fail,
        };
        let mutation = MutationOffender {
            file: Some(String::from("src/api.rs")),
            line: Some(9),
            mutation_kind: String::from("replace"),
            message: String::from("survived"),
            level: Level::Fail,
        };
        let cases = [
            (SignalKind::Coverage, OffenderIdentity::Coverage(&coverage)),
            (SignalKind::Size, OffenderIdentity::Size(&size)),
            (
                SignalKind::Complexity,
                OffenderIdentity::Complexity(&complexity),
            ),
            (SignalKind::Mutation, OffenderIdentity::Mutation(&mutation)),
        ];
        for (kind, offender) in cases {
            let target = target_for_finding(
                kind,
                support,
                &scope(),
                offender,
                DependencySource::Unscoped,
            );
            assert_eq!(target.file.as_deref(), Some("src/api.rs"));
        }

        let unscoped = target_for_finding(
            SignalKind::Coverage,
            VerificationSelectorSupport::NONE,
            &scope(),
            OffenderIdentity::Coverage(&coverage),
            DependencySource::Unscoped,
        );
        assert_eq!(unscoped, VerificationTarget::default());
    }
}
