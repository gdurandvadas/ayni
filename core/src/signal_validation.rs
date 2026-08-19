use crate::{
    Budget, ComplexityResult, CoverageResult, DepsResult, Level, MutationResult, Offenders,
    SignalResult, SignalRow, SizeResult, TestResult,
};
use serde::Deserialize;
use std::collections::BTreeMap;

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
        Budget::Test(value) => parse_budget::<EmptyBudget>("test", value),
        Budget::Coverage(value) => parse_budget::<CoverageBudget>("coverage", value),
        Budget::Size(value) => parse_budget::<SizeBudget>("size", value),
        Budget::Complexity(value) => parse_budget::<ComplexityBudget>("complexity", value),
        Budget::Deps(value) => parse_budget::<DepsBudget>("deps", value),
        Budget::Mutation(value) => parse_budget::<MutationBudget>("mutation", value),
    }
}

fn parse_budget<T: serde::de::DeserializeOwned>(
    kind: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|error| format!("{kind} budget is invalid: {error}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyBudget {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageBudget {
    #[allow(dead_code)]
    line_percent_warn: Option<f64>,
    #[allow(dead_code)]
    line_percent_fail: Option<f64>,
    #[allow(dead_code)]
    branch_percent_warn: Option<f64>,
    #[allow(dead_code)]
    branch_percent_fail: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeBudget {
    #[allow(dead_code)]
    rules: Option<Vec<SizeBudgetRule>>,
    #[allow(dead_code)]
    warn: Option<u64>,
    #[allow(dead_code)]
    fail: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeBudgetRule {
    #[allow(dead_code)]
    glob: String,
    #[allow(dead_code)]
    warn: u64,
    #[allow(dead_code)]
    fail: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComplexityBudget {
    #[allow(dead_code)]
    fn_cyclomatic: Option<FloatThresholdBudget>,
    #[allow(dead_code)]
    fn_cognitive: Option<FloatThresholdBudget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FloatThresholdBudget {
    #[allow(dead_code)]
    warn: f64,
    #[allow(dead_code)]
    fail: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DepsBudget {
    #[allow(dead_code)]
    forbidden: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationBudget {
    #[allow(dead_code)]
    enabled: Option<bool>,
}
