use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context_structured,
};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::xml;
use ayni_core::{
    Budget, Language, Level, MutationBudget, MutationOffender, MutationResult, Offenders,
    RunContext, SignalKind, SignalResult, SignalRow,
};
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

pub fn collect(context: &RunContext) -> CollectorResult {
    if !context.policy.checks.mutation {
        return Ok(SignalRow {
            kind: SignalKind::Mutation,
            language: Language::Python,
            scope: context.scope.clone(),
            pass: true,
            result: SignalResult::Mutation(MutationResult {
                engine: String::from("mutmut"),
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
                failure: None,
            }),
            budget: Budget::Mutation(MutationBudget {
                enabled: Some(false),
            }),
            offenders: Offenders::Mutation(Vec::new()),
        });
    }

    let (program, args) =
        command_for_override_or_default(context, SignalKind::Mutation, "mutmut", &["run"]);
    let run_output = run_command_for_context_structured(context, &program, &args)?;
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

    let junit_path =
        prepare_report_path(context, "mutmut-junit.xml").map_err(CollectorError::Adapter)?;
    let (junit_program, mut junit_args) =
        command_for_override_or_default(context, SignalKind::Mutation, "mutmut", &["junitxml"]);
    junit_args.push(String::from("--suspicious-policy=failure"));
    junit_args.push(String::from("--untested-policy=failure"));
    let junit_output = run_command_for_context_structured(context, &junit_program, &junit_args)?;
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
    fs::write(&junit_path, &junit_output.stdout).map_err(|error| {
        CollectorError::Adapter(format!("failed to write {}: {error}", junit_path.display()))
    })?;
    let report = parse_junit_report(&junit_path).map_err(CollectorError::Adapter)?;
    if report.tests == 0 {
        return Ok(error_row(
            context,
            String::from("mutmut"),
            ayni_adapters_common::failure::setup_failure(
                context,
                format_command(&junit_program, &junit_args),
                "mutmut evaluated zero mutants",
            ),
        ));
    }
    let survived = report.failures + report.errors;
    let killed = report
        .tests
        .saturating_sub(survived)
        .saturating_sub(report.skipped);
    let score = Some((killed as f64 / report.tests as f64) * 100.0);

    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: Language::Python,
        scope: context.scope.clone(),
        pass: survived == 0,
        result: SignalResult::Mutation(MutationResult {
            engine: String::from("mutmut"),
            killed,
            survived,
            timeout: 0,
            score,
            failure: None,
        }),
        budget: Budget::Mutation(MutationBudget {
            enabled: Some(true),
        }),
        offenders: Offenders::Mutation(report.offenders),
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
        scope: context.scope.clone(),
        pass: false,
        result: SignalResult::Mutation(MutationResult {
            engine,
            killed: 0,
            survived: 0,
            timeout: 0,
            score: None,
            failure: Some(failure),
        }),
        budget: Budget::Mutation(MutationBudget {
            enabled: Some(true),
        }),
        offenders: Offenders::Mutation(Vec::new()),
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
    static TESTSUITE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<testsuite\b([^>]*?)/?>"#).expect("valid testsuite regex"));
    static TESTCASE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<testcase\b([^>]*)>(.*?)</testcase>"#).expect("valid testcase regex")
    });
    static FAILURE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<(failure|error)\b([^>]*)>(.*?)</(failure|error)>"#)
            .expect("valid failure regex")
    });
    static SKIPPED_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<skipped\b"#).expect("valid skipped regex"));

    let mut report = JunitReport::default();
    let mut saw_testsuite = false;
    for caps in TESTSUITE_RE.captures_iter(content) {
        saw_testsuite = true;
        if let Some(attrs) = caps.get(1).map(|value| value.as_str()) {
            let attrs = xml::Attributes::parse(attrs)?;
            report.tests += attrs.u64("tests").unwrap_or(0);
            report.failures += attrs.u64("failures").unwrap_or(0);
            report.errors += attrs.u64("errors").unwrap_or(0);
            report.skipped += attrs.u64("skipped").unwrap_or(0);
        }
    }

    for caps in TESTCASE_RE.captures_iter(content) {
        let attrs = xml::Attributes::parse(caps.get(1).map(|value| value.as_str()).unwrap_or(""))?;
        let body = caps.get(2).map(|value| value.as_str()).unwrap_or("");
        report
            .offenders
            .extend(testcase_offenders(&attrs, body, &FAILURE_RE));
        if !saw_testsuite && SKIPPED_RE.is_match(body) {
            report.skipped += 1;
        }
    }

    if report.tests == 0 {
        report.tests = TESTCASE_RE.captures_iter(content).count() as u64;
    }
    if report.failures + report.errors == 0 && !report.offenders.is_empty() {
        report.failures = report.offenders.len() as u64;
    }
    Ok(report)
}

fn testcase_offenders(
    attrs: &xml::Attributes,
    body: &str,
    failure_re: &Regex,
) -> Vec<MutationOffender> {
    let mut offenders = Vec::new();
    let name = attrs
        .string("name")
        .unwrap_or_else(|| String::from("mutant"));
    for failure in failure_re.captures_iter(body) {
        let kind = failure
            .get(1)
            .map(|value| value.as_str())
            .unwrap_or("failure");
        let message = failure
            .get(3)
            .map(|value| xml::decode_xml(value.as_str().trim()))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                xml::attr_string(
                    failure.get(2).map(|value| value.as_str()).unwrap_or(""),
                    "message",
                )
            })
            .unwrap_or_else(|| format!("mutmut {kind}: {name}"));
        offenders.push(MutationOffender {
            file: attrs
                .string("file")
                .or_else(|| attrs.string("classname"))
                .filter(|value| value.ends_with(".py")),
            line: attrs.u64("line"),
            mutation_kind: kind.to_string(),
            message,
            level: Level::Fail,
        });
    }
    offenders
}

#[cfg(test)]
mod tests {
    use super::parse_junit_xml;

    #[test]
    fn parses_real_mutmut_2_5_1_report() {
        let report = parse_junit_xml(include_str!(
            "../../tests/fixtures/tooling/mutmut/junit.xml"
        ))
        .unwrap();
        assert_eq!(report.tests, 2);
        assert_eq!(report.failures + report.errors + report.skipped, 0);
        assert!(report.offenders.is_empty());
    }

    #[test]
    fn accepts_self_closing_suite_summary() {
        let report =
            parse_junit_xml("<testsuite tests='1' failures='0' errors='0' skipped='1'/>").unwrap();
        assert_eq!(report.tests, 1);
        assert_eq!(report.skipped, 1);
        assert!(report.offenders.is_empty());
    }

    #[test]
    fn rejects_duplicate_and_unquoted_report_attributes() {
        for content in [
            r#"<testsuite tests="1" tests="2"></testsuite>"#,
            r#"<testcase name=mutant><failure>diff</failure></testcase>"#,
        ] {
            assert!(parse_junit_xml(content).is_err());
        }
    }

    #[test]
    fn parses_single_quoted_testcase_failure_attributes() {
        let report = parse_junit_xml(
            "<testcase name='mutant' file='src/app.py' line='7'><failure message='survived'></failure></testcase>"
        ).unwrap();
        let offender = &report.offenders[0];
        assert_eq!(offender.file.as_deref(), Some("src/app.py"));
        assert_eq!(offender.line, Some(7));
        assert_eq!(offender.message, "survived");
    }

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

    #[test]
    fn suite_skipped_count_is_not_double_counted_from_testcases() {
        let report = parse_junit_xml(
            r#"<testsuite tests="2" failures="0" errors="0" skipped="1">
<testcase name="mutant 1"><skipped/></testcase>
<testcase name="mutant 2"></testcase>
</testsuite>"#,
        )
        .expect("report");
        assert_eq!(report.tests, 2);
        assert_eq!(report.skipped, 1);
    }
}
