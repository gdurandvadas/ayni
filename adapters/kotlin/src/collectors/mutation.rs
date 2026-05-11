use super::util::{
    attr_string, command_failure_from_output, find_reports, format_command, gradle_command,
    run_command_for_context, setup_failure,
};
use ayni_core::{
    Budget, Language, Level, MutationOffender, MutationResult, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    if !context.policy.checks.mutation {
        return Ok(SignalRow {
            kind: SignalKind::Mutation,
            language: Language::Kotlin,
            scope: Scope {
                workspace_root: context.scope.workspace_root.clone(),
                path: context.scope.path.clone(),
                package: context.scope.package.clone(),
                file: context.scope.file.clone(),
            },
            pass: true,
            result: SignalResult::Mutation(MutationResult {
                engine: String::from("pitest"),
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
                failure: None,
            }),
            budget: Budget::Mutation(json!({"enabled": false})),
            offenders: Offenders::Mutation(Vec::new()),
            delta_vs_previous: None,
            delta_vs_baseline: None,
        });
    }

    let (program, args) = gradle_command(context, SignalKind::Mutation, "pitest");
    let engine = format_command(&program, &args);
    let output = run_command_for_context(context, &program, &args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            engine,
            command_failure_from_output(context, SignalKind::Mutation, &program, &args, &output),
        ));
    }
    let report_paths = find_reports(&context.workdir, &["build", "reports", "pitest"], "xml");
    if report_paths.is_empty() {
        return Ok(error_row(
            context,
            engine,
            setup_failure(
                context,
                format_command(&program, &args),
                "pitest did not produce mutations.xml under build/reports/pitest",
            ),
        ));
    }
    let mut report = PitestReport::default();
    for path in &report_paths {
        let next = parse_pitest_xml(path)?;
        report.killed += next.killed;
        report.survived += next.survived;
        report.timeout += next.timeout;
        report.offenders.extend(next.offenders);
    }
    let killed = report.killed;
    let survived = report.survived;
    let timeout = report.timeout;
    let total = killed + survived + timeout;
    let score = (total > 0).then_some((killed as f64 / total as f64) * 100.0);

    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: survived == 0,
        result: SignalResult::Mutation(MutationResult {
            engine,
            killed,
            survived,
            timeout,
            score,
            failure: None,
        }),
        budget: Budget::Mutation(json!({"enabled": true})),
        offenders: Offenders::Mutation(report.offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

fn error_row(
    context: &RunContext,
    engine: String,
    failure: ayni_core::CommandFailure,
) -> SignalRow {
    SignalRow {
        kind: SignalKind::Mutation,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: false,
        result: SignalResult::Mutation(MutationResult {
            engine,
            killed: 0,
            survived: 0,
            timeout: 0,
            score: None,
            failure: Some(failure),
        }),
        budget: Budget::Mutation(json!({"enabled": true})),
        offenders: Offenders::Mutation(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    }
}

#[derive(Default)]
struct PitestReport {
    killed: u64,
    survived: u64,
    timeout: u64,
    offenders: Vec<MutationOffender>,
}

fn parse_pitest_xml(path: &Path) -> Result<PitestReport, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse_pitest_content(&content)
}

fn parse_pitest_content(content: &str) -> Result<PitestReport, String> {
    let mutation_re = Regex::new(r#"(?s)<mutation\b([^>]*)>(.*?)</mutation>"#)
        .map_err(|error| format!("failed to compile mutation regex: {error}"))?;
    let tag_re = Regex::new(r#"(?s)<([A-Za-z0-9_]+)>(.*?)</[A-Za-z0-9_]+>"#)
        .map_err(|error| format!("failed to compile mutation tag regex: {error}"))?;
    let mut report = PitestReport::default();
    for caps in mutation_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let body = caps.get(2).map(|value| value.as_str()).unwrap_or("");
        let status = attr_string(attrs, "status").unwrap_or_default();
        let detected = attr_string(attrs, "detected").unwrap_or_default();
        let fields = tag_re
            .captures_iter(body)
            .filter_map(|tag| {
                Some((
                    tag.get(1)?.as_str().to_string(),
                    super::util::decode_xml(tag.get(2)?.as_str().trim()),
                ))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        match status.as_str() {
            "KILLED" => report.killed += 1,
            "TIMED_OUT" => report.timeout += 1,
            _ if detected == "true" => report.killed += 1,
            _ => {
                report.survived += 1;
                report.offenders.push(MutationOffender {
                    file: fields.get("sourceFile").cloned(),
                    line: fields
                        .get("lineNumber")
                        .and_then(|value| value.parse::<u64>().ok()),
                    mutation_kind: fields
                        .get("mutator")
                        .cloned()
                        .unwrap_or_else(|| status.clone()),
                    message: fields
                        .get("description")
                        .cloned()
                        .unwrap_or_else(|| String::from("PIT mutation survived")),
                    level: Level::Fail,
                });
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::parse_pitest_content;

    #[test]
    fn parses_pitest_mutations() {
        let report = parse_pitest_content(
            r#"<mutations>
<mutation detected="true" status="KILLED"><sourceFile>A.kt</sourceFile><lineNumber>1</lineNumber><mutator>x</mutator></mutation>
<mutation detected="false" status="SURVIVED"><sourceFile>B.kt</sourceFile><lineNumber>2</lineNumber><mutator>y</mutator><description>survived</description></mutation>
</mutations>"#,
        )
        .expect("pitest");

        assert_eq!(report.killed, 1);
        assert_eq!(report.survived, 1);
        assert_eq!(report.offenders[0].file.as_deref(), Some("B.kt"));
    }
}
