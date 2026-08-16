use super::util::{find_reports, gradle_command, prepare_gradle_execution, report_root};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_adapters_common::failure::{command_failure_from_output, setup_failure};
use ayni_adapters_common::paths::{
    canonicalize_relative_posix, resolve_repo_path, to_repo_relative_path,
};
use ayni_adapters_common::xml::{attr_string, attr_u64};
use ayni_core::{
    Budget, ComplexityOffender, ComplexityResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow, classify_maximum,
};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn collect(context: &RunContext) -> CollectorResult {
    let config = context.policy.kotlin.complexity.as_ref().ok_or_else(|| {
        CollectorError::Adapter(String::from("missing [kotlin.complexity] policy"))
    })?;
    let cyclomatic = config.fn_cyclomatic.ok_or_else(|| {
        CollectorError::Adapter(String::from("missing kotlin.complexity.fn_cyclomatic"))
    })?;
    prepare_gradle_execution(context, SignalKind::Complexity).map_err(CollectorError::Adapter)?;
    let (program, args) = gradle_command(context, SignalKind::Complexity, "detekt");
    let engine = format_command(&program, &args);
    let output = run_command_for_context_structured(context, &program, &args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            engine,
            command_failure_from_output(context, SignalKind::Complexity, &program, &args, &output),
        ));
    }
    let report_paths = find_reports(
        &report_root(context, SignalKind::Complexity),
        &["build", "reports", "detekt"],
        "xml",
    );
    if report_paths.is_empty() {
        return Ok(error_row(
            context,
            engine,
            setup_failure(
                context,
                format_command(&program, &args),
                "detekt did not produce an XML report under build/reports/detekt",
            ),
        ));
    }
    let mut offenders = Vec::new();
    for report_path in &report_paths {
        offenders.extend(
            parse_checkstyle_xml(report_path, context, cyclomatic.fail + 1.0)
                .map_err(CollectorError::Adapter)?,
        );
    }
    let mut max_fn_cyclomatic = 0.0_f64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    offenders.retain_mut(|offender| {
        max_fn_cyclomatic = max_fn_cyclomatic.max(offender.cyclomatic);
        let Some(level) = classify_complexity(
            offender.cyclomatic,
            cyclomatic.warn,
            cyclomatic.fail,
            &mut warn_count,
            &mut fail_count,
        ) else {
            return false;
        };
        offender.level = level;
        true
    });
    offenders.sort_by(|left, right| {
        right
            .level
            .cmp(&left.level)
            .then_with(|| right.cyclomatic.total_cmp(&left.cyclomatic))
            .then_with(|| left.file.cmp(&right.file))
    });

    Ok(SignalRow {
        kind: SignalKind::Complexity,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: fail_count == 0,
        result: SignalResult::Complexity(ComplexityResult {
            engine,
            method: String::from("detekt_complexity"),
            measured_functions: offenders.len() as u64,
            max_fn_cyclomatic,
            max_fn_cognitive: None,
            warn_count,
            fail_count,
            failure: None,
        }),
        budget: Budget::Complexity(json!({
            "fn_cyclomatic": {"warn": cyclomatic.warn, "fail": cyclomatic.fail}
        })),
        offenders: Offenders::Complexity(offenders),
    })
}

fn classify_complexity(
    value: f64,
    warn: f64,
    fail: f64,
    warn_count: &mut u64,
    fail_count: &mut u64,
) -> Option<Level> {
    let level = classify_maximum(value, warn, fail);
    match level {
        Some(Level::Warn) => *warn_count += 1,
        Some(Level::Fail) => *fail_count += 1,
        None => {}
    }
    level
}

fn error_row(
    context: &RunContext,
    engine: String,
    failure: ayni_core::CommandFailure,
) -> SignalRow {
    SignalRow {
        kind: SignalKind::Complexity,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: false,
        result: SignalResult::Complexity(ComplexityResult {
            engine,
            method: String::from("detekt_complexity"),
            measured_functions: 0,
            max_fn_cyclomatic: 0.0,
            max_fn_cognitive: None,
            warn_count: 0,
            fail_count: 1,
            failure: Some(failure),
        }),
        budget: Budget::Complexity(json!({})),
        offenders: Offenders::Complexity(Vec::new()),
    }
}

fn parse_checkstyle_xml(
    path: &Path,
    context: &RunContext,
    fallback_complexity: f64,
) -> Result<Vec<ComplexityOffender>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse_checkstyle_content(&content, context, fallback_complexity)
}

fn parse_checkstyle_content(
    content: &str,
    context: &RunContext,
    fallback_complexity: f64,
) -> Result<Vec<ComplexityOffender>, String> {
    let file_re = Regex::new(r#"(?s)<file\b([^>]*)>(.*?)</file>"#)
        .map_err(|error| format!("failed to compile file regex: {error}"))?;
    let error_re = Regex::new(r#"<error\b([^>]*)/?>"#)
        .map_err(|error| format!("failed to compile error regex: {error}"))?;
    let number_re = Regex::new(r"(\d+(?:\.\d+)?)")
        .map_err(|error| format!("failed to compile number regex: {error}"))?;
    let mut offenders = Vec::new();
    let selected_file = selected_file(context);
    for file_caps in file_re.captures_iter(content) {
        let file_attrs = file_caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let file_name =
            attr_string(file_attrs, "name").unwrap_or_else(|| String::from("<unknown>"));
        let normalized_file = canonicalize_relative_posix(&to_repo_relative_path(
            &context.repo_root,
            Path::new(&file_name),
        ));
        if selected_file
            .as_ref()
            .is_some_and(|selected| selected != &normalized_file)
        {
            continue;
        }
        let body = file_caps.get(2).map(|value| value.as_str()).unwrap_or("");
        for error_caps in error_re.captures_iter(body) {
            let attrs = error_caps.get(1).map(|value| value.as_str()).unwrap_or("");
            let source = attr_string(attrs, "source").unwrap_or_default();
            let message = attr_string(attrs, "message").unwrap_or_default();
            if !source.to_ascii_lowercase().contains("complex")
                && !message.to_ascii_lowercase().contains("complex")
            {
                continue;
            }
            let cyclomatic = number_re
                .captures_iter(&message)
                .last()
                .and_then(|caps| caps.get(1))
                .and_then(|value| value.as_str().parse::<f64>().ok())
                .unwrap_or(fallback_complexity);
            offenders.push(ComplexityOffender {
                file: normalized_file.clone(),
                line: attr_u64(attrs, "line").unwrap_or(1),
                function: attr_string(attrs, "source").unwrap_or_else(|| String::from("detekt")),
                cyclomatic,
                cognitive: None,
                level: Level::Fail,
            });
        }
    }
    Ok(offenders)
}

fn selected_file(context: &RunContext) -> Option<String> {
    context.scope.file.as_ref().map(|file| {
        let resolved = resolve_repo_path(&context.repo_root, file);
        canonicalize_relative_posix(&to_repo_relative_path(&context.repo_root, &resolved))
    })
}

#[cfg(test)]
mod tests {
    use super::{classify_complexity, parse_checkstyle_content};
    use ayni_core::{AyniPolicy, ExecutionResolution, Level, RunContext, Scope};
    use std::path::PathBuf;

    #[test]
    fn parses_detekt_complexity_errors() {
        let context = RunContext {
            repo_root: PathBuf::from("/repo"),
            target_root: PathBuf::from("/repo"),
            workdir: PathBuf::from("/repo"),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("gradle", PathBuf::from("/repo"), "test", 100),
            debug: false,
        };
        let offenders = parse_checkstyle_content(
            r#"<checkstyle><file name="/repo/src/App.kt"><error line="7" source="ComplexMethod" message="complexity is 22"/></file></checkstyle>"#,
            &context,
            21.0,
        )
        .expect("detekt");

        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0].file, "src/App.kt");
        assert_eq!(offenders[0].cyclomatic, 22.0);
    }

    #[test]
    fn file_scope_filters_detekt_report_before_metrics_are_calculated() {
        let context = RunContext {
            repo_root: PathBuf::from("/repo"),
            target_root: PathBuf::from("/repo"),
            workdir: PathBuf::from("/repo"),
            policy: AyniPolicy::default(),
            scope: Scope {
                file: Some(String::from("src/Selected.kt")),
                ..Scope::default()
            },
            execution: ExecutionResolution::direct("gradle", PathBuf::from("/repo"), "test", 100),
            debug: false,
        };
        let offenders = parse_checkstyle_content(
            r#"<checkstyle>
                <file name="/repo/src/Selected.kt"><error line="7" source="ComplexMethod" message="complexity is 12"/></file>
                <file name="/repo/src/Other.kt"><error line="9" source="ComplexMethod" message="complexity is 30"/></file>
            </checkstyle>"#,
            &context,
            21.0,
        )
        .expect("detekt");

        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0].file, "src/Selected.kt");
        assert_eq!(offenders[0].cyclomatic, 12.0);
    }

    #[test]
    fn maximum_threshold_equality() {
        let mut warn_count = 0;
        let mut fail_count = 0;
        assert_eq!(
            classify_complexity(9.0, 10.0, 15.0, &mut warn_count, &mut fail_count),
            None
        );
        assert_eq!(
            classify_complexity(10.0, 10.0, 15.0, &mut warn_count, &mut fail_count),
            Some(Level::Warn)
        );
        assert_eq!(
            classify_complexity(15.0, 10.0, 15.0, &mut warn_count, &mut fail_count),
            Some(Level::Fail)
        );
        assert_eq!((warn_count, fail_count), (1, 1));
    }
}
