use std::collections::BTreeMap;

use ayni_core::{
    CompletionScope, CompletionStage, CompletionState, Language, Level, Offenders, RunArtifact,
    SignalKind, SignalResult, SignalRow,
};

pub(crate) struct ReportView<'a> {
    pub(crate) groups: Vec<ReportGroup<'a>>,
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
            total: artifact.rows.len(),
            passing: artifact.rows.iter().filter(|row| row.pass).count(),
        }
    }
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
