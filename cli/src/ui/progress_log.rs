use crate::ui::runner::ProgressEvent;
use ayni_core::RunArtifact;

pub fn log_started_check(event: ProgressEvent) {
    if let Some(line) = started_check_line(&event) {
        eprintln!("{line}");
    }
}

pub fn log_command_failures(artifact: &RunArtifact) {
    for line in command_failure_diagnostics(artifact) {
        eprintln!("{line}");
    }
}

pub fn command_failure_diagnostics(artifact: &RunArtifact) -> Vec<String> {
    artifact
        .failure_summaries()
        .unwrap_or_default()
        .into_iter()
        .map(|failure| {
            let scope = failure.scope.path.as_deref().unwrap_or("workspace");
            let exit_code = failure
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| String::from("none"));
            format!(
                "command failure kind={:?} language={} workspace={} category={} classification={}\n  command: {}\n  cwd: {}\n  exit_code: {}\n  message: {}",
                failure.kind,
                failure.language.as_str(),
                scope,
                failure.category,
                failure.classification,
                failure.command,
                failure.cwd,
                exit_code,
                failure.message,
            )
        })
        .collect()
}

fn started_check_line(event: &ProgressEvent) -> Option<String> {
    let ProgressEvent::Started { language, name } = event else {
        return None;
    };
    let (language, workspace) = split_target_label(language);
    Some(format!(
        "running language={language} workspace={workspace} signal={name}"
    ))
}

fn split_target_label(label: &str) -> (&str, &str) {
    label.split_once(':').unwrap_or((label, "workspace"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::runner::{ProgressEvent, ToolState};
    use ayni_core::{
        Budget, CommandFailure, Language, Offenders, RunArtifact, Scope, SignalKind, SignalResult,
        SignalRow, TestResult,
    };
    use std::time::Duration;

    #[test]
    fn started_check_line_formats_workspace_root() {
        let event = ProgressEvent::Started {
            language: String::from("rust:workspace"),
            name: String::from("test"),
        };

        assert_eq!(
            started_check_line(&event),
            Some(String::from(
                "running language=rust workspace=workspace signal=test"
            ))
        );
    }

    #[test]
    fn started_check_line_formats_non_root_workspace() {
        let event = ProgressEvent::Started {
            language: String::from("node:apps/web"),
            name: String::from("coverage"),
        };

        assert_eq!(
            started_check_line(&event),
            Some(String::from(
                "running language=node workspace=apps/web signal=coverage"
            ))
        );
    }

    #[test]
    fn started_check_line_ignores_non_started_events() {
        let event = ProgressEvent::Finished {
            language: String::from("rust:workspace"),
            name: String::from("test"),
            state: ToolState::Done,
            elapsed: Duration::from_secs(1),
        };

        assert_eq!(started_check_line(&event), None);
    }

    #[test]
    fn command_failure_diagnostics_include_complete_failure_context() {
        let artifact = RunArtifact {
            schema_version: String::from("0.2.0"),
            metadata: Default::default(),
            rows: vec![SignalRow {
                kind: SignalKind::Test,
                language: Language::Rust,
                scope: Scope::default(),
                pass: false,
                result: SignalResult::Test(TestResult {
                    total_tests: 0,
                    passed: 0,
                    failed: 1,
                    duration_ms: None,
                    runner: String::from("cargo test"),
                    failure: Some(CommandFailure {
                        category: String::from("tool"),
                        classification: String::from("command_error"),
                        command: String::from("cargo test"),
                        cwd: String::from("/tmp/ayni"),
                        exit_code: Some(101),
                        message: String::from("test command failed"),
                    }),
                }),
                budget: Budget::Test(serde_json::json!({})),
                offenders: Offenders::Test(Vec::new()),
                delta_vs_previous: None,
            }],
        };

        assert_eq!(
            command_failure_diagnostics(&artifact),
            [String::from(
                "command failure kind=Test language=rust workspace=workspace category=tool classification=command_error\n  command: cargo test\n  cwd: /tmp/ayni\n  exit_code: 101\n  message: test command failed"
            )]
        );
    }
}
