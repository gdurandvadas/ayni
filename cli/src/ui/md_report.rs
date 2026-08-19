use ayni_core::{
    AggregateStatus, CommandFailure, CompletionScope, FailureSummary, Level, Offenders,
    RunArtifact, SignalResult, SignalRow,
};

use crate::ui::report_view::{
    ReportStatus, ReportView, completion_scope_label, completion_stage_label,
    completion_state_label, signal_kind_label,
};

fn push_heading(out: &mut String, scope: CompletionScope) {
    match scope {
        CompletionScope::Repository => out.push_str("# ayni check\n\n"),
        CompletionScope::Requested => out.push_str(
            "# ayni verify\n\n> Focused evidence only; run `ayni check` for repository completion.\n\n",
        ),
    }
}

pub fn build_markdown(artifact: &RunArtifact, offenders_limit: usize) -> String {
    let view = ReportView::new(artifact);
    let mut out = String::new();
    let aggregate = match artifact.aggregate().status {
        AggregateStatus::Pass => "pass",
        AggregateStatus::Fail => "fail",
    };
    push_heading(&mut out, artifact.completion.scope);
    out.push_str(&format!(
        "**{}** / **{}** checks passing · aggregate **{}** · schema `{}`\n\n",
        view.passing, view.total, aggregate, artifact.schema_version
    ));
    out.push_str(&format!(
        "**Completion:** scope `{}` · state **{}** · targets **{}** / **{}** completed · **{}** detected · **{}** skipped\n\n",
        completion_scope_label(artifact.completion.scope),
        completion_state_label(artifact.completion.state),
        artifact.completion.completed_targets,
        artifact.completion.expected_targets,
        artifact.completion.detected_targets,
        artifact.completion.skipped_targets,
    ));
    if !artifact.completion.issues.is_empty() {
        out.push_str("## Completion issues\n\n");
        for issue in &artifact.completion.issues {
            out.push_str(&format!(
                "- `{}` `{}` `{}` — {}\n",
                issue.language.as_str(),
                issue.configured_root,
                completion_stage_label(issue.stage),
                issue.message,
            ));
        }
        out.push('\n');
    }

    for group in &view.groups {
        let root_label = if group.root == "." {
            "workspace"
        } else {
            group.root.as_str()
        };
        out.push_str(&format!(
            "## {} ({}) — {}/{} passing\n\n",
            group.language.as_str(),
            root_label,
            group.passing,
            group.rows.len()
        ));

        out.push_str("| # | Signal | Summary | Status |\n");
        out.push_str("|---|--------|---------|--------|\n");
        for (index, row) in group.rows.iter().enumerate() {
            out.push_str(&format!(
                "| **{}** | **{}** | `{}` | {} |\n",
                index + 1,
                signal_kind_label(row.kind),
                summarize_row(row),
                row_status_badge(row),
            ));
        }
        out.push('\n');

        let offenders: Vec<(&SignalRow, Vec<String>)> = group
            .rows
            .iter()
            .map(|row| (*row, offender_lines(row, offenders_limit)))
            .filter(|(_, lines)| !lines.is_empty())
            .collect();
        if !offenders.is_empty() {
            out.push_str("<details>\n<summary>Offenders</summary>\n\n");
            for (row, lines) in offenders {
                out.push_str(&format!("{}\n", signal_kind_label(row.kind)));
                for line in lines {
                    out.push_str(&format!("- {line}\n"));
                }
                out.push('\n');
            }
            out.push_str("</details>\n\n");
        }
    }
    if !view.commands.is_empty() {
        out.push_str("## Verification commands\n\n");
        out.push_str("Run the exact command supplied by each finding:\n\n");
        for command in view.commands {
            out.push_str(&markdown_code_block_with_language(command, "sh"));
            out.push_str("\n\n");
        }
    }
    render_failures(&mut out, artifact.failure_summaries());
    out
}

fn render_failures(out: &mut String, failures: Option<Vec<FailureSummary>>) {
    let Some(failures) = failures else {
        return;
    };

    out.push_str("## Failures\n\n");
    for failure in failures {
        out.push_str(&format!(
            "### {} ({})\n\n",
            signal_kind_label(failure.kind),
            failure.language.as_str(),
        ));
        markdown_failure_field(out, "Category", &failure.category);
        markdown_failure_field(out, "Classification", &failure.classification);
        markdown_failure_field(out, "Command", &failure.command);
        markdown_failure_field(out, "Working directory", &failure.cwd);
        if let Some(exit_code) = failure.exit_code {
            markdown_failure_field(out, "Exit code", &exit_code.to_string());
        }
        markdown_failure_field(out, "Message", &failure.message);
    }
}

fn markdown_failure_field(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!(
        "**{label}:**\n\n{}\n\n",
        markdown_code_block(value)
    ));
}

fn markdown_code_block(value: &str) -> String {
    markdown_code_block_with_language(value, "text")
}

fn markdown_code_block_with_language(value: &str, language: &str) -> String {
    let fence = "`"
        .repeat(longest_backtick_run(value) + 1)
        .max("```".to_string());
    format!("{fence}{language}\n{value}\n{fence}")
}

fn longest_backtick_run(value: &str) -> usize {
    value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn row_status_badge(row: &SignalRow) -> String {
    let label = ReportStatus::for_row(row).label().to_ascii_uppercase();
    format!("**{label}**")
}

fn summarize_row(row: &SignalRow) -> String {
    match &row.result {
        SignalResult::Test(result) => format!(
            "total={} passed={} failed={}",
            result.total_tests, result.passed, result.failed
        ),
        SignalResult::Coverage(result) => format!(
            "percent={} status={}",
            result
                .headline_percent()
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| String::from("—")),
            result.status
        ),
        SignalResult::Size(result) => format!(
            "max_lines={} files={} fail_count={}",
            result.max_lines, result.total_files, result.fail_count
        ),
        SignalResult::Complexity(result) => format!(
            "functions={} max_cyclo={:.1} fail_count={}",
            result.measured_functions, result.max_fn_cyclomatic, result.fail_count
        ),
        SignalResult::Deps(result) => format!(
            "crates={} edges={} violations={}",
            result.crate_count, result.edge_count, result.violation_count
        ),
        SignalResult::Mutation(result) => format!(
            "killed={} survived={} score={}",
            result.killed,
            result.survived,
            result
                .score
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| String::from("—"))
        ),
    }
}

fn offender_lines(row: &SignalRow, offenders_limit: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let command_failure = command_failure_for_row(row);
    if let Some(failure) = command_failure {
        lines.push(format!(
            "**FAIL** command failure category={} classification={} command=`{}` cwd=`{}`: {}",
            failure.category,
            failure.classification,
            failure.command,
            failure.cwd,
            failure.message.replace('\n', " ")
        ));
    }
    match &row.offenders {
        Offenders::Test(items) => {
            for item in items.iter().take(offenders_limit) {
                let location = item
                    .file
                    .as_deref()
                    .map(|file| format!("`{file}`"))
                    .unwrap_or_else(|| String::from("`<unnamed-test>`"));
                lines.push(format!(
                    "**FAIL** {} {} {}",
                    location,
                    item.test_name.as_deref().unwrap_or("<unnamed-test>"),
                    item.message
                ));
            }
        }
        Offenders::Coverage(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}` {:.1}%",
                    level_label(item.level),
                    item.file,
                    item.value
                ));
            }
        }
        Offenders::Size(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}` lines={} warn={} fail={}",
                    level_label(item.level),
                    item.file,
                    item.value,
                    item.warn,
                    item.fail
                ));
            }
        }
        Offenders::Complexity(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}:{}` {} cyclo={:.1}",
                    level_label(item.level),
                    item.file,
                    item.line,
                    item.function,
                    item.cyclomatic
                ));
            }
        }
        Offenders::Deps(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}` -> {} (rule={})",
                    level_label(item.level),
                    item.from,
                    item.to,
                    item.rule
                ));
            }
        }
        Offenders::Mutation(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}` {}",
                    level_label(item.level),
                    item.mutation_kind,
                    item.message
                ));
            }
        }
    }
    lines
}

fn command_failure_for_row(row: &SignalRow) -> Option<&CommandFailure> {
    match &row.result {
        SignalResult::Test(result) => result.failure.as_ref(),
        SignalResult::Coverage(result) => result.failure.as_ref(),
        SignalResult::Size(result) => result.failure.as_ref(),
        SignalResult::Complexity(result) => result.failure.as_ref(),
        SignalResult::Deps(result) => result.failure.as_ref(),
        SignalResult::Mutation(result) => result.failure.as_ref(),
    }
}

fn level_label(level: Level) -> &'static str {
    match level {
        Level::Warn => "WARN",
        Level::Fail => "FAIL",
    }
}

#[cfg(test)]
mod tests {
    use super::build_markdown;
    use ayni_core::{
        AYNI_SIGNAL_SCHEMA_VERSION, Budget, CommandFailure, CompletionIssue, CompletionScope,
        CompletionStage, CompletionState, CoverageBudget, CoverageOffender, CoverageResult,
        DepsBudget, DepsResult, Finding, FindingMetadata, Findings, Language, Level, Offenders,
        RunArtifact, RunCompletion, Scope, SignalKind, SignalResult, SignalRow, SizeBudget,
        SizeOffender, SizeResult, TestBudget, TestFailure, TestResult, VerificationMetadata,
    };

    #[test]
    fn build_markdown_renders_grouped_table() {
        let artifact = RunArtifact {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: Default::default(),
            completion: Default::default(),
            findings: Vec::new(),
            rows: vec![SignalRow {
                kind: SignalKind::Coverage,
                language: Language::Rust,
                scope: Scope::default(),
                pass: false,
                result: SignalResult::Coverage(CoverageResult {
                    percent: Some(41.0),
                    line_percent: Some(41.0),
                    branch_percent: None,
                    engine: String::from("cargo-llvm-cov"),
                    status: String::from("ok"),
                    failure: None,
                }),
                budget: Budget::Coverage(CoverageBudget {
                    line_percent_fail: Some(50.0),
                    ..CoverageBudget::default()
                }),
                offenders: Offenders::Coverage(vec![CoverageOffender {
                    file: String::from("src/lib.rs"),
                    line: Some(10),
                    value: 41.0,
                    level: Level::Fail,
                }]),
            }],
        };

        let text = build_markdown(&artifact, 3);
        assert!(text.contains("# ayni check"));
        assert!(text.contains("## rust (workspace)"));
        assert!(text.contains("| # | Signal | Summary | Status |"));
        assert!(text.contains("| **1** | **coverage** | `percent=41.0% status=ok` | **FAIL** |"));
        assert!(!text.contains("raw.githubusercontent.com"));
        assert!(text.contains("<details>\n<summary>Offenders</summary>\n\n"));
        assert!(text.contains("\ncoverage\n- "));
        assert!(text.contains("**FAIL** `src/lib.rs` 41.0%"));
        assert!(!text.contains("## Failures"));
    }

    #[test]
    fn build_markdown_distinguishes_incomplete_requested_evidence() {
        let artifact = RunArtifact {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: Default::default(),
            findings: Vec::new(),
            completion: RunCompletion {
                scope: CompletionScope::Requested,
                state: CompletionState::Incomplete,
                expected_targets: 1,
                detected_targets: 1,
                completed_targets: 0,
                skipped_targets: 1,
                issues: vec![CompletionIssue {
                    language: Language::Rust,
                    configured_root: String::from("crates/api"),
                    stage: CompletionStage::Collection,
                    message: String::from("collector stopped"),
                }],
            },
            rows: Vec::new(),
        };

        let text = build_markdown(&artifact, 3);
        assert!(text.starts_with("# ayni verify\n"));
        assert!(text.contains("Focused evidence only; run `ayni check`"));
        assert!(text.contains("aggregate **fail**"));
        assert!(text.contains("scope `requested` · state **incomplete**"));
        assert!(text.contains("## Completion issues"));
        assert!(text.contains("`rust` `crates/api` `collection` — collector stopped"));
    }

    #[test]
    fn build_markdown_surfaces_stable_deduplicated_verification_commands() {
        let finding = |id_character: char, command: &str| Finding {
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
                level: Level::Fail,
            },
        };
        let command = "ayni verify size --file 'src/lib.rs'";
        let artifact = RunArtifact {
            findings: vec![Findings::Size(vec![
                finding('a', command),
                finding('b', command),
            ])],
            ..RunArtifact::default()
        };

        let text = build_markdown(&artifact, 2);

        assert!(text.contains("## Verification commands\n\n"));
        assert!(text.contains(&format!("```sh\n{command}\n```")));
        assert_eq!(text.matches(command).count(), 1);
        assert!(!text.contains("raw.githubusercontent.com"));
    }

    #[test]
    fn build_markdown_renders_all_failures_without_truncating_them() {
        let artifact = RunArtifact {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: Default::default(),
            completion: Default::default(),
            findings: Vec::new(),
            rows: vec![
                SignalRow {
                    kind: SignalKind::Test,
                    language: Language::Rust,
                    scope: Scope::default(),
                    pass: false,
                    result: SignalResult::Test(TestResult {
                        total_tests: 2,
                        passed: 0,
                        failed: 2,
                        duration_ms: None,
                        runner: String::from("cargo test"),
                        failure: Some(CommandFailure {
                            category: String::from("tool"),
                            classification: String::from("command_error"),
                            command: String::from("cargo test `weird`"),
                            cwd: String::from("/tmp/a[yni]"),
                            exit_code: Some(101),
                            message: String::from("failed *badly*\n```"),
                        }),
                    }),
                    budget: Budget::Test(TestBudget::default()),
                    offenders: Offenders::Test(vec![
                        TestFailure {
                            file: None,
                            line: None,
                            message: String::from("first"),
                            test_name: Some(String::from("first_failure")),
                        },
                        TestFailure {
                            file: None,
                            line: None,
                            message: String::from("second"),
                            test_name: Some(String::from("second_failure")),
                        },
                    ]),
                },
                SignalRow {
                    kind: SignalKind::Coverage,
                    language: Language::Rust,
                    scope: Scope::default(),
                    pass: false,
                    result: SignalResult::Coverage(CoverageResult {
                        percent: None,
                        line_percent: None,
                        branch_percent: None,
                        engine: String::from("coverage"),
                        status: String::from("failed"),
                        failure: Some(CommandFailure {
                            category: String::from("tool"),
                            classification: String::from("timeout"),
                            command: String::from("coverage run"),
                            cwd: String::from("/tmp/ayni"),
                            exit_code: None,
                            message: String::from("timed out"),
                        }),
                    }),
                    budget: Budget::Coverage(CoverageBudget::default()),
                    offenders: Offenders::Coverage(Vec::new()),
                },
            ],
        };

        let text = build_markdown(&artifact, 1);
        assert!(text.contains("first_failure"));
        assert!(!text.contains("second_failure"));
        assert!(text.contains("## Failures"));
        assert!(text.contains("### test (rust)"));
        assert!(text.contains("**Category:**\n\n```text\ntool"));
        assert!(text.contains("**Classification:**\n\n```text\ncommand_error"));
        assert!(text.contains("**Command:**\n\n```text\ncargo test `weird`"));
        assert!(text.contains("**Working directory:**\n\n```text\n/tmp/a[yni]"));
        assert!(text.contains("**Exit code:**\n\n```text\n101"));
        assert!(text.contains("**Message:**\n\n````text\nfailed *badly*\n```\n````"));
        let test_failure = text.find("### test (rust)").expect("test failure");
        let coverage_failure = text.find("### coverage (rust)").expect("coverage failure");
        assert!(test_failure < coverage_failure);
        assert!(
            text.contains(
                "coverage\n- **FAIL** command failure category=tool classification=timeout"
            )
        );
        let coverage_section = &text[coverage_failure..];
        assert!(!coverage_section.contains("**Exit code:**"));
    }

    #[test]
    fn build_markdown_renders_complete_size_and_deps_failures() {
        let failure = |kind: &str, exit_code| CommandFailure {
            category: format!("{kind}_category"),
            classification: format!("{kind}_classification"),
            command: format!("{kind} command"),
            cwd: format!("/{kind}"),
            exit_code,
            message: format!("{kind} message"),
        };
        let artifact = RunArtifact {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: Default::default(),
            completion: Default::default(),
            findings: Vec::new(),
            rows: vec![
                SignalRow {
                    kind: SignalKind::Size,
                    language: Language::Rust,
                    scope: Scope::default(),
                    pass: false,
                    result: SignalResult::Size(SizeResult {
                        max_lines: 0,
                        total_files: 0,
                        warn_count: 0,
                        fail_count: 1,
                        failure: Some(failure("size", Some(17))),
                    }),
                    budget: Budget::Size(SizeBudget::default()),
                    offenders: Offenders::Size(Vec::new()),
                },
                SignalRow {
                    kind: SignalKind::Deps,
                    language: Language::Rust,
                    scope: Scope::default(),
                    pass: false,
                    result: SignalResult::Deps(DepsResult {
                        crate_count: 0,
                        edge_count: 0,
                        violation_count: 1,
                        failure: Some(failure("deps", None)),
                    }),
                    budget: Budget::Deps(DepsBudget::default()),
                    offenders: Offenders::Deps(Vec::new()),
                },
            ],
        };

        let text = build_markdown(&artifact, 1);
        for (kind, exit_code) in [("size", Some(17)), ("deps", None)] {
            assert!(text.contains(&format!("### {kind} (rust)")));
            assert!(text.contains(&format!("{}{}_category", "```text\n", kind)));
            assert!(text.contains(&format!("{}{}_classification", "```text\n", kind)));
            assert!(text.contains(&format!("{}{} command", "```text\n", kind)));
            assert!(text.contains(&format!("{}{}", "```text\n/", kind)));
            assert!(text.contains(&format!("{}{} message", "```text\n", kind)));
            match exit_code {
                Some(code) => assert!(text.contains(&format!("**Exit code:**\n\n```text\n{code}"))),
                None => assert!(!text.contains("**Exit code:**\n\n```text\nnone")),
            }
        }
    }
}
