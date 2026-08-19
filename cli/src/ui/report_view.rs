use std::collections::{BTreeMap, BTreeSet};

use ayni_core::{
    CompletionScope, CompletionStage, CompletionState, Findings, Language, Level, Offenders,
    RunArtifact, SignalKind, SignalResult, SignalRow,
};

pub(crate) struct ReportView<'a> {
    pub(crate) groups: Vec<ReportGroup<'a>>,
    pub(crate) commands: Vec<&'a str>,
    pub(crate) total: usize,
    pub(crate) passing: usize,
}

pub(crate) struct ReportGroup<'a> {
    pub(crate) language: Language,
    pub(crate) root: String,
    pub(crate) rows: Vec<&'a SignalRow>,
    pub(crate) passing: usize,
}

impl<'a> ReportView<'a> {
    pub(crate) fn new(artifact: &'a RunArtifact) -> Self {
        let mut grouped = BTreeMap::<(Language, String), Vec<&SignalRow>>::new();
        for row in &artifact.rows {
            grouped
                .entry((
                    row.language,
                    row.scope.path.clone().unwrap_or_else(|| String::from(".")),
                ))
                .or_default()
                .push(row);
        }
        let groups = grouped
            .into_iter()
            .map(|((language, root), rows)| {
                let passing = rows.iter().filter(|row| row.pass).count();
                ReportGroup {
                    language,
                    root,
                    rows,
                    passing,
                }
            })
            .collect();

        Self {
            groups,
            commands: verification_commands(&artifact.findings),
            total: artifact.rows.len(),
            passing: artifact.rows.iter().filter(|row| row.pass).count(),
        }
    }
}

fn verification_commands(findings: &[Findings]) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    findings
        .iter()
        .flat_map(Findings::commands)
        .filter(|command| seen.insert(*command))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReportStatus {
    Pass,
    Warn,
    Fail,
}

impl ReportStatus {
    pub(crate) fn for_row(row: &SignalRow) -> Self {
        if !row.pass {
            return Self::Fail;
        }
        match &row.result {
            SignalResult::Size(result) if result.warn_count > 0 => Self::Warn,
            SignalResult::Complexity(result) if result.warn_count > 0 => Self::Warn,
            SignalResult::Mutation(result) if result.timeout > 0 => Self::Warn,
            _ if has_warn_offenders(&row.offenders) => Self::Warn,
            _ => Self::Pass,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
        }
    }
}

fn has_warn_offenders(offenders: &Offenders) -> bool {
    match offenders {
        Offenders::Coverage(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Size(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Complexity(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Deps(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Mutation(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Test(_) => false,
    }
}

pub(crate) fn signal_kind_label(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

pub(crate) fn completion_scope_label(scope: CompletionScope) -> &'static str {
    match scope {
        CompletionScope::Repository => "repository",
        CompletionScope::Requested => "requested",
    }
}

pub(crate) fn completion_state_label(state: CompletionState) -> &'static str {
    match state {
        CompletionState::Complete => "complete",
        CompletionState::Incomplete => "incomplete",
    }
}

pub(crate) fn completion_stage_label(stage: CompletionStage) -> &'static str {
    match stage {
        CompletionStage::Detection => "detection",
        CompletionStage::Resolution => "resolution",
        CompletionStage::Selection => "selection",
        CompletionStage::Scheduling => "scheduling",
        CompletionStage::Collection => "collection",
    }
}

#[cfg(test)]
mod tests {
    use super::ReportView;
    use ayni_core::{Finding, FindingMetadata, Findings, SizeOffender, VerificationMetadata};

    fn finding(id_character: char, command: &str) -> Finding<SizeOffender> {
        Finding {
            metadata: FindingMetadata {
                id: format!(
                    "ayni:finding:v1:sha256:{}",
                    id_character.to_string().repeat(64)
                ),
                verification: VerificationMetadata {
                    target: None,
                    command: Some(command.to_string()),
                },
            },
            offender: SizeOffender {
                file: String::from("src/lib.rs"),
                value: 10,
                warn: 5,
                fail: 9,
                level: ayni_core::Level::Fail,
            },
        }
    }

    #[test]
    fn report_view_deduplicates_commands_in_first_seen_order() {
        let artifact = ayni_core::RunArtifact {
            findings: vec![Findings::Size(vec![
                finding('a', "ayni verify size --file 'src/lib.rs'"),
                finding('b', "ayni verify size --file 'src/lib.rs'"),
                finding('c', "ayni verify size --file 'src/main.rs'"),
            ])],
            ..ayni_core::RunArtifact::default()
        };

        assert_eq!(
            ReportView::new(&artifact).commands,
            [
                "ayni verify size --file 'src/lib.rs'",
                "ayni verify size --file 'src/main.rs'",
            ]
        );
    }
}
