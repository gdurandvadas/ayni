use crate::{
    Budget, ComplexityResult, CoverageResult, DepsResult, Level, MutationResult, Offenders,
    SignalResult, SignalRow, SizeResult, TestResult,
};

pub(crate) fn validate_signal_row(row: &SignalRow) -> Result<(), String> {
    validate_variant_alignment(row)?;
    validate_budget_shape(&row.budget)?;
    let failing_findings = failing_findings(&row.offenders);
    validate_common_pass_invariants(row, failing_findings)?;
    let evidence_passes = typed_evidence_passes(&row.result, failing_findings)?;
    if row.pass != evidence_passes {
        return Err(String::from(
            "signal row pass does not match its typed result and findings",
        ));
    }
    Ok(())
}

fn validate_common_pass_invariants(row: &SignalRow, failing_findings: u64) -> Result<(), String> {
    if row.pass && row.result.command_failure().is_some() {
        return Err(String::from(
            "passing signal row cannot contain a command failure",
        ));
    }
    if row.pass && failing_findings > 0 {
        return Err(String::from(
            "passing signal row cannot contain fail-level findings",
        ));
    }
    Ok(())
}

fn typed_evidence_passes(result: &SignalResult, failing_findings: u64) -> Result<bool, String> {
    match result {
        SignalResult::Test(result) => test_passes(result, failing_findings),
        SignalResult::Coverage(result) => coverage_passes(result, failing_findings),
        SignalResult::Size(result) => Ok(size_passes(result, failing_findings)),
        SignalResult::Complexity(result) => complexity_passes(result, failing_findings),
        SignalResult::Deps(result) => Ok(deps_passes(result, failing_findings)),
        SignalResult::Mutation(result) => mutation_passes(result, failing_findings),
    }
}

fn validate_variant_alignment(row: &SignalRow) -> Result<(), String> {
    if matches!(
        (row.kind, &row.result, &row.budget, &row.offenders),
        (
            crate::SignalKind::Test,
            SignalResult::Test(_),
            Budget::Test(_),
            Offenders::Test(_)
        ) | (
            crate::SignalKind::Coverage,
            SignalResult::Coverage(_),
            Budget::Coverage(_),
            Offenders::Coverage(_)
        ) | (
            crate::SignalKind::Size,
            SignalResult::Size(_),
            Budget::Size(_),
            Offenders::Size(_)
        ) | (
            crate::SignalKind::Complexity,
            SignalResult::Complexity(_),
            Budget::Complexity(_),
            Offenders::Complexity(_)
        ) | (
            crate::SignalKind::Deps,
            SignalResult::Deps(_),
            Budget::Deps(_),
            Offenders::Deps(_)
        ) | (
            crate::SignalKind::Mutation,
            SignalResult::Mutation(_),
            Budget::Mutation(_),
            Offenders::Mutation(_)
        )
    ) {
        Ok(())
    } else {
        Err(String::from(
            "artifact row kind does not match its typed payloads",
        ))
    }
}

fn test_passes(result: &TestResult, failing_findings: u64) -> Result<bool, String> {
    let accounted = result
        .passed
        .checked_add(result.failed)
        .ok_or_else(|| String::from("test result counts overflow"))?;
    if accounted > result.total_tests {
        return Err(String::from(
            "test result passed and failed counts exceed total_tests",
        ));
    }
    Ok(result.total_tests > 0
        && result.failed == 0
        && result.failure.is_none()
        && failing_findings == 0)
}

fn coverage_passes(result: &CoverageResult, failing_findings: u64) -> Result<bool, String> {
    validate_percentage("percent", result.percent)?;
    validate_percentage("line_percent", result.line_percent)?;
    validate_percentage("branch_percent", result.branch_percent)?;
    Ok(result.status == "ok" && result.failure.is_none() && failing_findings == 0)
}

fn validate_percentage(name: &str, value: Option<f64>) -> Result<(), String> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value)) {
        Err(format!(
            "coverage {name} must be finite and between 0 and 100"
        ))
    } else {
        Ok(())
    }
}

fn size_passes(result: &SizeResult, failing_findings: u64) -> bool {
    result.fail_count == 0 && result.failure.is_none() && failing_findings == 0
}

fn complexity_passes(result: &ComplexityResult, failing_findings: u64) -> Result<bool, String> {
    if !result.max_fn_cyclomatic.is_finite()
        || result.max_fn_cyclomatic < 0.0
        || result
            .max_fn_cognitive
            .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(String::from(
            "complexity metrics must be finite and non-negative",
        ));
    }
    Ok(result.fail_count == 0 && result.failure.is_none() && failing_findings == 0)
}

fn deps_passes(result: &DepsResult, failing_findings: u64) -> bool {
    result.violation_count == 0 && result.failure.is_none() && failing_findings == 0
}

fn mutation_passes(result: &MutationResult, failing_findings: u64) -> Result<bool, String> {
    if result
        .score
        .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
    {
        return Err(String::from(
            "mutation score must be finite and between 0 and 100",
        ));
    }
    let total = result
        .killed
        .checked_add(result.survived)
        .and_then(|value| value.checked_add(result.timeout))
        .ok_or_else(|| String::from("mutation result counts overflow"))?;
    Ok(total > 0
        && result.survived == 0
        && result.timeout == 0
        && result.score.is_some()
        && result.failure.is_none()
        && failing_findings == 0)
}

fn failing_findings(offenders: &Offenders) -> u64 {
    match offenders {
        Offenders::Test(items) => items.len() as u64,
        Offenders::Coverage(items) => count_failures(items.iter().map(|item| item.level)),
        Offenders::Size(items) => count_failures(items.iter().map(|item| item.level)),
        Offenders::Complexity(items) => count_failures(items.iter().map(|item| item.level)),
        Offenders::Deps(items) => count_failures(items.iter().map(|item| item.level)),
        Offenders::Mutation(items) => count_failures(items.iter().map(|item| item.level)),
    }
}

fn count_failures(levels: impl Iterator<Item = Level>) -> u64 {
    levels.filter(|level| *level == Level::Fail).count() as u64
}

fn validate_budget_shape(budget: &Budget) -> Result<(), String> {
    match budget {
        Budget::Test(_) | Budget::Mutation(_) => Ok(()),
        Budget::Coverage(value) => validate_coverage_budget(value),
        Budget::Size(value) => validate_size_budget(value),
        Budget::Complexity(value) => validate_complexity_budget(value),
        Budget::Deps(value) => validate_deps_budget(value),
    }
}

fn validate_coverage_budget(value: &crate::CoverageBudget) -> Result<(), String> {
    validate_minimum_threshold(
        "coverage line_percent",
        value.line_percent_warn,
        value.line_percent_fail,
        Some(100.0),
    )?;
    validate_minimum_threshold(
        "coverage branch_percent",
        value.branch_percent_warn,
        value.branch_percent_fail,
        Some(100.0),
    )
}

fn validate_size_budget(value: &crate::SizeBudget) -> Result<(), String> {
    if !value.rules.is_empty() && (value.warn.is_some() || value.fail.is_some()) {
        return Err(String::from(
            "size budget cannot combine rules with focused warn/fail thresholds",
        ));
    }
    match (value.warn, value.fail) {
        (Some(warn), Some(fail)) if warn <= fail => {}
        (None, None) => {}
        (Some(_), Some(_)) => {
            return Err(String::from("size budget warn must not exceed fail"));
        }
        _ => {
            return Err(String::from(
                "size budget focused warn and fail must be supplied together",
            ));
        }
    }
    for rule in &value.rules {
        if rule.glob.trim().is_empty() {
            return Err(String::from("size budget rule glob cannot be empty"));
        }
        if rule.warn > rule.fail {
            return Err(String::from("size budget rule warn must not exceed fail"));
        }
    }
    Ok(())
}

fn validate_complexity_budget(value: &crate::ComplexityBudget) -> Result<(), String> {
    if let Some(threshold) = value.fn_cyclomatic {
        validate_maximum_threshold("complexity fn_cyclomatic", threshold.warn, threshold.fail)?;
    }
    if let Some(threshold) = value.fn_cognitive {
        validate_maximum_threshold("complexity fn_cognitive", threshold.warn, threshold.fail)?;
    }
    Ok(())
}

fn validate_deps_budget(value: &crate::DepsBudget) -> Result<(), String> {
    if value.forbidden.as_ref().is_some_and(|rules| {
        rules.iter().any(|(source, targets)| {
            source.trim().is_empty() || targets.iter().any(|target| target.trim().is_empty())
        })
    }) {
        Err(String::from("dependency budget patterns cannot be empty"))
    } else {
        Ok(())
    }
}

fn validate_minimum_threshold(
    name: &str,
    warn: Option<f64>,
    fail: Option<f64>,
    maximum: Option<f64>,
) -> Result<(), String> {
    match (warn, fail) {
        (None, None) => Ok(()),
        (Some(warn), Some(fail))
            if warn.is_finite()
                && fail.is_finite()
                && warn >= 0.0
                && fail >= 0.0
                && maximum.is_none_or(|maximum| warn <= maximum && fail <= maximum)
                && warn >= fail =>
        {
            Ok(())
        }
        (Some(_), Some(_)) => Err(format!(
            "{name} budget must be finite, in range, and have warn >= fail"
        )),
        _ => Err(format!(
            "{name} budget warn and fail must be supplied together"
        )),
    }
}

fn validate_maximum_threshold(name: &str, warn: f64, fail: f64) -> Result<(), String> {
    if warn.is_finite() && fail.is_finite() && warn >= 0.0 && warn <= fail {
        Ok(())
    } else {
        Err(format!(
            "{name} budget must be finite, non-negative, and have warn <= fail"
        ))
    }
}
