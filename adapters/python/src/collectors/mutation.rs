use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context,
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
            language: Language::Python,
            scope: Scope {
                workspace_root: context.scope.workspace_root.clone(),
                path: context.scope.path.clone(),
                package: context.scope.package.clone(),
                file: context.scope.file.clone(),
            },
            pass: true,
            result: SignalResult::Mutation(MutationResult {
                engine: String::from("mutmut"),
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

    let (program, args) =
        command_for_override_or_default(context, SignalKind::Mutation, "mutmut", &["run"]);
    let run_output = run_command_for_context(context, &program, &args)?;
    if !run_output.status.success() {
        return Ok(error_row(
            context,
            format_command(&program, &args),
            command_failure_from_output(
                context,
                SignalKind::Mutation,
                &program,
                &args,
                &run_output,
            ),
        ));
    }

    let junit_path = prepare_report_path(context, "mutmut-junit.xml")?;
    let (junit_program, mut junit_args) =
        command_for_override_or_default(context, SignalKind::Mutation, "mutmut", &["junitxml"]);
    junit_args.push(String::from("--suspicious-policy=failure"));
    junit_args.push(String::from("--untested-policy=failure"));
    let junit_output = run_command_for_context(context, &junit_program, &junit_args)?;
    if !junit_output.status.success() {
        return Ok(error_row(
            context,
            format_command(&junit_program, &junit_args),
            command_failure_from_output(
                context,
                SignalKind::Mutation,
                &junit_program,
                &junit_args,
                &junit_output,
            ),
        ));
    }
    fs::write(&junit_path, &junit_output.stdout)
        .map_err(|error| format!("failed to write {}: {error}", junit_path.display()))?;
    let report = parse_junit_report(&junit_path)?;
    let survived = report.failures + report.errors;
    let killed = report
        .tests
        .saturating_sub(survived)
        .saturating_sub(report.skipped);
    let score = if report.tests == 0 {
        None
    } else {
        Some((killed as f64 / report.tests as f64) * 100.0)
    };

    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: survived == 0,
        result: SignalResult::Mutation(MutationResult {
            engine: String::from("mutmut"),
            killed,
            survived,
            timeout: 0,
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
        language: Language::Python,
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

#[derive(Debug, Default)]
struct JunitReport {
    tests: u64,
    failures: u64,
    errors: u64,
    skipped: u64,
    offenders: Vec<MutationOffender>,
}

fn parse_junit_report(path: &Path) -> Result<JunitReport, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse_junit_xml(&content)
}

fn parse_junit_xml(content: &str) -> Result<JunitReport, String> {
    let testsuite_re = Regex::new(r#"<testsuite\b([^>]*)>"#)
        .map_err(|error| format!("failed to compile testsuite regex: {error}"))?;
    let testcase_re = Regex::new(r#"(?s)<testcase\b([^>]*)>(.*?)</testcase>"#)
        .map_err(|error| format!("failed to compile testcase regex: {error}"))?;
    let failure_re = Regex::new(r#"(?s)<(failure|error)\b([^>]*)>(.*?)</(failure|error)>"#)
        .map_err(|error| format!("failed to compile failure regex: {error}"))?;
    let skipped_re = Regex::new(r#"<skipped\b"#)
        .map_err(|error| format!("failed to compile skipped regex: {error}"))?;

    let mut report = JunitReport::default();
    for caps in testsuite_re.captures_iter(content) {
        if let Some(attrs) = caps.get(1).map(|value| value.as_str()) {
            report.tests += attr_u64(attrs, "tests");
            report.failures += attr_u64(attrs, "failures");
            report.errors += attr_u64(attrs, "errors");
            report.skipped += attr_u64(attrs, "skipped");
        }
    }

    for caps in testcase_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let body = caps.get(2).map(|value| value.as_str()).unwrap_or("");
        let name = attr_string(attrs, "name").unwrap_or_else(|| String::from("mutant"));
        for failure in failure_re.captures_iter(body) {
            let kind = failure
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or("failure");
            let message = failure
                .get(3)
                .map(|value| decode_xml(value.as_str().trim()))
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    attr_string(
                        failure.get(2).map(|value| value.as_str()).unwrap_or(""),
                        "message",
                    )
                })
                .unwrap_or_else(|| format!("mutmut {kind}: {name}"));
            report.offenders.push(MutationOffender {
                file: attr_string(attrs, "file")
                    .or_else(|| attr_string(attrs, "classname"))
                    .filter(|value| value.ends_with(".py")),
                line: attr_u64_option(attrs, "line"),
                mutation_kind: kind.to_string(),
                message,
                level: Level::Fail,
            });
        }
        if skipped_re.is_match(body) {
            report.skipped += 1;
        }
    }

    if report.tests == 0 {
        report.tests = testcase_re.captures_iter(content).count() as u64;
    }
    if report.failures + report.errors == 0 && !report.offenders.is_empty() {
        report.failures = report.offenders.len() as u64;
    }
    Ok(report)
}

fn attr_u64(attrs: &str, name: &str) -> u64 {
    attr_u64_option(attrs, name).unwrap_or(0)
}

fn attr_u64_option(attrs: &str, name: &str) -> Option<u64> {
    attr_string(attrs, name).and_then(|value| value.parse::<u64>().ok())
}

fn attr_string(attrs: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"{name}="([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    re.captures(attrs)
        .and_then(|caps| caps.get(1))
        .map(|value| decode_xml(value.as_str()))
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::parse_junit_xml;

    #[test]
    fn parses_mutmut_junit_failures() {
        let report = parse_junit_xml(
            r#"<testsuite tests="2" failures="1" errors="0" skipped="0">
<testcase classname="src/app.py" name="mutant 1"><failure message="survived">diff</failure></testcase>
<testcase classname="src/app.py" name="mutant 2"></testcase>
</testsuite>"#,
        )
        .expect("report");
        assert_eq!(report.tests, 2);
        assert_eq!(report.failures, 1);
        assert_eq!(report.offenders.len(), 1);
    }
}
