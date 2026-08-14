use ayni_adapters_go::GoAdapter;
use ayni_adapters_kotlin::KotlinAdapter;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;

mod agents;
mod application;
mod args;
mod artifact_compare;
mod completion;
mod contract;
mod discovery;
mod environment;
mod environment_backend;
mod environment_lock;
mod ui;
mod verification_command;
mod verify;

use agents::sync_impl;
use ayni_adapters_node::NodeAdapter;
use ayni_adapters_python::PythonAdapter;
use ayni_adapters_rust::RustAdapter;
use ayni_core::{
    AYNI_SIGNAL_SCHEMA_VERSION, AdapterRegistry, AyniPolicy, Budget, CommandFailure,
    CompletionIssue, CompletionScope, CompletionStage, CompletionState, ComplexityResult,
    ConcurrencyPolicy, CoverageResult, DepsResult, InvocationContext, Language, MutationResult,
    Offenders, OutputContext, RunArtifact, RunArtifactMetadata, RunCompletion, RunContext, Scope,
    SignalKind, SignalResult, SignalRow, SizeResult, TestResult,
};
use clap::Parser;

const ARTIFACTS_DIR: &str = ".ayni/last";
const SIGNALS_ARTIFACT: &str = ".ayni/last/signals.json";
const VERIFY_ARTIFACTS_DIR: &str = ".ayni/verify/last";
const VERIFY_SIGNALS_ARTIFACT: &str = ".ayni/verify/last/signals.json";

fn main() -> ExitCode {
    dispatch(args::Cli::parse().into_operation())
}

fn dispatch(operation: application::Operation) -> ExitCode {
    use application::Operation;

    match operation {
        operation @ (Operation::Check(_) | Operation::Verify(_)) => dispatch_analysis(operation),
        operation @ (Operation::EnvShow(_)
        | Operation::EnvLock(_)
        | Operation::EnvDoctor(_)
        | Operation::EnvBuild(_)
        | Operation::EnvShell(_)
        | Operation::EnvRun(_)) => dispatch_environment(operation),
        operation @ (Operation::ContractShow(_) | Operation::ContractValidate(_)) => {
            dispatch_contract(operation)
        }
        Operation::AgentsSync(operation) => agents_sync(&operation.repo_root),
        Operation::ResultsCompare(operation) => artifact_compare::run(
            &operation.baseline,
            &operation.candidate,
            operation.output == application::OutputFormat::Json,
        ),
        Operation::GenerateDocs => {
            print!("{}", clap_markdown::help_markdown::<args::Cli>());
            ExitCode::SUCCESS
        }
        operation => not_implemented(&operation),
    }
}

fn dispatch_analysis(operation: application::Operation) -> ExitCode {
    use application::{ExecutionMode, Operation};

    match operation {
        Operation::Check(operation) if operation.execution_mode == ExecutionMode::Host => analyze(
            operation.config.to_string_lossy().as_ref(),
            AnalyzeOptions {
                output_mode: output_arg(operation.output),
                debug: operation.debug,
            },
        ),
        Operation::Verify(operation) if operation.execution_mode == ExecutionMode::Host => {
            run_verify_operation(operation)
        }
        Operation::Check(operation) => environment_backend::check(operation, &build_registry()),
        Operation::Verify(_) => environment_unavailable(),
        _ => unreachable!("dispatch_analysis received a non-analysis operation"),
    }
}

fn dispatch_environment(operation: application::Operation) -> ExitCode {
    use application::Operation;

    let registry = build_registry();
    match operation {
        Operation::EnvShow(operation) => environment::show(operation, &registry),
        Operation::EnvLock(operation) => environment_lock::run(operation, &registry),
        Operation::EnvDoctor(operation) => environment_backend::doctor(operation, &registry),
        Operation::EnvBuild(operation) => environment_backend::build(operation, &registry),
        Operation::EnvShell(operation) => environment_backend::shell(operation, &registry),
        Operation::EnvRun(operation) => environment_backend::run(operation, &registry),
        _ => unreachable!("dispatch_environment received a non-environment operation"),
    }
}

fn dispatch_contract(operation: application::Operation) -> ExitCode {
    use application::{Operation, OutputFormat};

    match operation {
        Operation::ContractShow(operation) => contract_display(
            operation.config.to_string_lossy().as_ref(),
            operation.output == OutputFormat::Json,
        ),
        Operation::ContractValidate(operation) => contract_validate(&operation.config),
        _ => unreachable!("dispatch_contract received a non-contract operation"),
    }
}

fn output_arg(output: application::OutputFormat) -> OutputArg {
    match output {
        application::OutputFormat::Human => OutputArg::Stdout,
        application::OutputFormat::Json => OutputArg::Json,
        application::OutputFormat::Markdown => OutputArg::Md,
    }
}

fn run_verify_operation(operation: application::VerifyOperation) -> ExitCode {
    let request = verify::Request {
        kind: operation.signal,
        config_path: operation.config,
        file: operation.file,
        package: operation.package,
        name: operation.name,
        language: operation.language,
        root: operation.root,
        output_mode: output_arg(operation.output),
        debug: operation.debug,
    };
    match verify::run(request) {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::from(1),
        Err(error) => {
            eprintln!("{error}");
            if error.starts_with("failed to read ") || error.starts_with("failed to parse ") {
                ExitCode::from(2)
            } else {
                ExitCode::from(4)
            }
        }
    }
}

fn contract_validate(config_path: &Path) -> ExitCode {
    let adapter_facts = build_registry()
        .adapters()
        .iter()
        .map(|adapter| adapter.policy_effectiveness_facts())
        .collect::<Vec<_>>();
    match contract::display(config_path, &adapter_facts, false) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn contract_display(config_path: &str, json: bool) -> ExitCode {
    let adapter_facts = build_registry()
        .adapters()
        .iter()
        .map(|adapter| adapter.policy_effectiveness_facts())
        .collect::<Vec<_>>();
    match contract::display(Path::new(config_path), &adapter_facts, json) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn agents_sync(repo_root: &Path) -> ExitCode {
    match sync_impl(repo_root.to_string_lossy().as_ref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(4)
        }
    }
}

fn environment_unavailable() -> ExitCode {
    eprintln!("managed environment execution is not implemented yet; rerun with --host");
    ExitCode::from(3)
}

fn not_implemented(operation: &application::Operation) -> ExitCode {
    eprintln!("{operation:?} is part of the greenfield command model but is not implemented yet");
    ExitCode::from(4)
}

fn build_registry() -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(GoAdapter::new()));
    registry.register(Arc::new(RustAdapter::new()));
    registry.register(Arc::new(NodeAdapter::new()));
    registry.register(Arc::new(PythonAdapter::new()));
    registry.register(Arc::new(KotlinAdapter::new()));
    registry
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputArg {
    Stdout,
    Md,
    Json,
}

impl OutputArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Md => "md",
            Self::Json => "json",
        }
    }
}

fn enabled_signal_kinds(policy: &AyniPolicy) -> Vec<SignalKind> {
    let mut kinds = Vec::new();
    if policy.checks.test {
        kinds.push(SignalKind::Test);
    }
    if policy.checks.coverage {
        kinds.push(SignalKind::Coverage);
    }
    if policy.checks.size {
        kinds.push(SignalKind::Size);
    }
    if policy.checks.complexity {
        kinds.push(SignalKind::Complexity);
    }
    if policy.checks.deps {
        kinds.push(SignalKind::Deps);
    }
    if policy.checks.mutation {
        kinds.push(SignalKind::Mutation);
    }
    kinds
}

fn signal_kind_slug(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

fn tool_id(language: Language, root: &str, kind: SignalKind) -> String {
    format!("{}:{}:{}", language.as_str(), root, signal_kind_slug(kind))
}

#[derive(Clone, Debug)]
struct AnalyzeTarget {
    language: Language,
    root: String,
    run_context: RunContext,
}

#[derive(Clone, Debug)]
struct AnalyzePlanning {
    targets: Vec<AnalyzeTarget>,
    expected_targets: u64,
    detected_targets: u64,
    issues: Vec<CompletionIssue>,
}

impl AnalyzePlanning {
    fn completion(
        &self,
        scope: CompletionScope,
        completed_targets: u64,
        mut additional_issues: Vec<CompletionIssue>,
    ) -> RunCompletion {
        let mut issues = self.issues.clone();
        issues.append(&mut additional_issues);
        let skipped_targets = self.expected_targets - completed_targets;
        RunCompletion {
            scope,
            state: if skipped_targets == 0 {
                CompletionState::Complete
            } else {
                CompletionState::Incomplete
            },
            expected_targets: self.expected_targets,
            detected_targets: self.detected_targets,
            completed_targets,
            skipped_targets,
            issues,
        }
    }

    fn runnable_failure_issues(
        &self,
        stage: CompletionStage,
        message: &str,
    ) -> Vec<CompletionIssue> {
        self.targets
            .iter()
            .map(|target| CompletionIssue {
                language: target.language,
                configured_root: target.root.clone(),
                stage,
                message: message.to_string(),
            })
            .collect()
    }
}

type TargetCollectResult = Result<Vec<SignalRow>, String>;
type TargetResultSlots = Arc<Mutex<Vec<Option<TargetCollectResult>>>>;

#[derive(Clone, Debug)]
struct AnalyzeOptions {
    output_mode: OutputArg,
    debug: bool,
}

impl AnalyzeTarget {
    fn root_label(&self) -> String {
        if self.root == "." {
            String::from("workspace")
        } else {
            self.root.clone()
        }
    }
}

fn build_analyze_plan(targets: &[AnalyzeTarget]) -> ui::runner::Plan {
    let mut tools = Vec::new();
    for target in targets {
        for kind in enabled_signal_kinds(&target.run_context.policy) {
            tools.push(ui::runner::PlanTool {
                id: tool_id(target.language, &target.root, kind),
                language: format!("{}:{}", target.language.as_str(), target.root_label()),
                name: signal_kind_slug(kind).to_string(),
            });
        }
    }
    ui::runner::Plan { tools }
}

fn run_collect_with_ui(
    ctx: &ui::runner::ExecContext,
    planning: &AnalyzePlanning,
    scope: CompletionScope,
) -> Result<RunArtifact, String> {
    let concurrency = planning
        .targets
        .first()
        .map(|target| target.run_context.policy.concurrency.clone())
        .unwrap_or_default();
    let rows = collect_targets_with_ui(ctx, &planning.targets, &concurrency)?;
    let (completion, rows) = completion::reconcile(planning, scope, None, rows);
    Ok(RunArtifact {
        schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
        metadata: Default::default(),
        completion,
        findings: Vec::new(),
        rows,
    })
}

fn collect_targets_with_ui(
    ctx: &ui::runner::ExecContext,
    targets: &[AnalyzeTarget],
    concurrency: &ConcurrencyPolicy,
) -> Result<Vec<SignalRow>, String> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    if targets.len() == 1 || concurrency.amount <= 1 {
        return collect_targets_serial(ctx, targets);
    }

    let indexed_targets = targets
        .iter()
        .cloned()
        .enumerate()
        .collect::<Vec<(usize, AnalyzeTarget)>>();
    let mut result_slots = Vec::with_capacity(indexed_targets.len());
    result_slots.resize_with(indexed_targets.len(), || None);
    let result_slots = Arc::new(Mutex::new(result_slots));

    if concurrency.per_language {
        let mut by_language = BTreeMap::<Language, Vec<(usize, AnalyzeTarget)>>::new();
        for (index, target) in indexed_targets {
            by_language
                .entry(target.language)
                .or_default()
                .push((index, target));
        }
        let registry = build_registry();
        let mut group_handles = Vec::new();
        for (language, jobs) in by_language {
            let ctx = ctx.clone();
            let result_slots = Arc::clone(&result_slots);
            let adapter_cap = registry
                .adapters()
                .iter()
                .find(|adapter| adapter.language() == language)
                .and_then(|adapter| adapter.max_target_concurrency());
            let worker_limit = adapter_cap.map_or(concurrency.amount, |cap| {
                cap.clamp(1, concurrency.amount.max(1))
            });
            group_handles.push(thread::spawn(move || {
                run_target_jobs(&ctx, jobs, worker_limit, result_slots)
            }));
        }
        for handle in group_handles {
            handle
                .join()
                .map_err(|_| String::from("analyze scheduler panicked"))??;
        }
    } else {
        run_target_jobs(
            ctx,
            indexed_targets,
            concurrency.amount,
            Arc::clone(&result_slots),
        )?;
    }

    flatten_target_results(result_slots, ctx.is_aborted())
}

fn collect_targets_serial(
    ctx: &ui::runner::ExecContext,
    targets: &[AnalyzeTarget],
) -> Result<Vec<SignalRow>, String> {
    let mut rows = Vec::new();
    for target in targets {
        rows.extend(collect_target_with_ui(ctx, target)?);
    }
    Ok(rows)
}

fn collect_target_with_ui(
    ctx: &ui::runner::ExecContext,
    target: &AnalyzeTarget,
) -> Result<Vec<SignalRow>, String> {
    let registry = build_registry();
    let Some(adapter) = registry
        .adapters()
        .iter()
        .find(|candidate| candidate.language() == target.language)
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for kind in enabled_signal_kinds(&target.run_context.policy) {
        if ctx.is_aborted() {
            return Err(String::from("operation aborted"));
        }
        let tool = ctx.tool(&tool_id(target.language, &target.root, kind))?;
        tool.started();
        let row_result = adapter
            .collector()
            .collect_streaming(kind, &target.run_context, &mut |line| {
                tool.line(line);
            })
            .map_err(|e| e.to_string());
        match row_result {
            Ok(row) => {
                tool.line(signal_outcome_line(kind, &row));
                tool.finished(if row.pass {
                    ui::runner::ToolState::Done
                } else {
                    ui::runner::ToolState::Failed
                });
                rows.push(row);
            }
            Err(error) => {
                tool.line(error.clone());
                let row = failed_signal_row(target.language, kind, &target.run_context, error);
                tool.finished(ui::runner::ToolState::Failed);
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn failed_signal_row(
    language: Language,
    kind: SignalKind,
    context: &RunContext,
    message: String,
) -> SignalRow {
    let failure = CommandFailure {
        category: failure_category_for_signal(kind).to_string(),
        classification: String::from("adapter_error"),
        command: signal_kind_slug(kind).to_string(),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: None,
        message,
    };
    let scope = Scope {
        workspace_root: context.scope.workspace_root.clone(),
        path: context.scope.path.clone(),
        package: context.scope.package.clone(),
        file: context.scope.file.clone(),
    };
    let (result, budget, offenders) = match kind {
        SignalKind::Test => (
            SignalResult::Test(TestResult {
                total_tests: 0,
                passed: 0,
                failed: 1,
                duration_ms: None,
                runner: String::from("test"),
                failure: Some(failure),
            }),
            Budget::Test(serde_json::json!({})),
            Offenders::Test(Vec::new()),
        ),
        SignalKind::Coverage => (
            SignalResult::Coverage(CoverageResult {
                percent: None,
                line_percent: None,
                branch_percent: None,
                engine: String::from("coverage"),
                status: String::from("error"),
                failure: Some(failure),
            }),
            Budget::Coverage(serde_json::json!({})),
            Offenders::Coverage(Vec::new()),
        ),
        SignalKind::Size => (
            SignalResult::Size(SizeResult {
                max_lines: 0,
                total_files: 0,
                warn_count: 0,
                fail_count: 1,
                failure: Some(failure),
            }),
            Budget::Size(serde_json::json!({})),
            Offenders::Size(Vec::new()),
        ),
        SignalKind::Complexity => (
            SignalResult::Complexity(ComplexityResult {
                engine: String::from("complexity"),
                method: String::from("unknown"),
                measured_functions: 0,
                max_fn_cyclomatic: 0.0,
                max_fn_cognitive: None,
                warn_count: 0,
                fail_count: 1,
                failure: Some(failure),
            }),
            Budget::Complexity(serde_json::json!({})),
            Offenders::Complexity(Vec::new()),
        ),
        SignalKind::Deps => (
            SignalResult::Deps(DepsResult {
                crate_count: 0,
                edge_count: 0,
                violation_count: 1,
                failure: Some(failure),
            }),
            Budget::Deps(serde_json::json!({})),
            Offenders::Deps(Vec::new()),
        ),
        SignalKind::Mutation => (
            SignalResult::Mutation(MutationResult {
                engine: String::from("mutation"),
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
                failure: Some(failure),
            }),
            Budget::Mutation(serde_json::json!({})),
            Offenders::Mutation(Vec::new()),
        ),
    };
    SignalRow {
        kind,
        language,
        scope,
        pass: false,
        result,
        budget,
        offenders,
    }
}

fn failure_category_for_signal(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test | SignalKind::Coverage | SignalKind::Mutation => "repo_code_issue",
        SignalKind::Complexity => "repo_setup_issue",
        SignalKind::Size | SignalKind::Deps => "ayni_internal_issue",
    }
}

fn signal_outcome_line(kind: SignalKind, row: &SignalRow) -> String {
    let status = if row.pass { "ok" } else { "fail" };
    let metrics = signal_metrics(row);
    if metrics.is_empty() {
        format!("{} {status}", signal_kind_slug(kind))
    } else {
        format!("{} {status} {metrics}", signal_kind_slug(kind))
    }
}

fn signal_metrics(row: &SignalRow) -> String {
    match &row.result {
        SignalResult::Test(value) => format!(
            "(total:{}, pass:{}, fail:{})",
            value.total_tests, value.passed, value.failed
        ),
        SignalResult::Coverage(value) => {
            let budget = match &row.budget {
                Budget::Coverage(value) => Some(value),
                _ => None,
            };
            let measured = value.headline_percent();
            let warn = budget
                .and_then(|value| value.get("line_percent_warn"))
                .and_then(|value| value.as_f64());
            let fail = budget
                .and_then(|value| value.get("line_percent_fail"))
                .and_then(|value| value.as_f64());
            let delta_warn = measured.zip(warn).map(|(m, w)| m - w);
            let delta_fail = measured.zip(fail).map(|(m, f)| m - f);
            format!(
                "(pct:{}, warn:{}, fail:{}, Δw:{}, Δf:{})",
                fmt_opt_percent(measured),
                fmt_opt_percent(warn),
                fmt_opt_percent(fail),
                fmt_opt_signed(delta_warn),
                fmt_opt_signed(delta_fail)
            )
        }
        SignalResult::Size(value) => format!(
            "(max_lines:{}, files:{}, fail_count:{})",
            value.max_lines, value.total_files, value.fail_count
        ),
        SignalResult::Complexity(value) => {
            let budget = match &row.budget {
                Budget::Complexity(value) => Some(value),
                _ => None,
            };
            let cyclo_warn =
                budget.and_then(|value| nested_budget_number(value, "fn_cyclomatic", "warn"));
            let cyclo_fail =
                budget.and_then(|value| nested_budget_number(value, "fn_cyclomatic", "fail"));
            format!(
                "(max_cyclo:{}, warn:{}, fail:{}, funcs:{})",
                fmt_number(value.max_fn_cyclomatic),
                fmt_opt_number(cyclo_warn),
                fmt_opt_number(cyclo_fail),
                value.measured_functions
            )
        }
        SignalResult::Deps(value) => format!(
            "(violations:{}, edges:{}, crates:{})",
            value.violation_count, value.edge_count, value.crate_count
        ),
        SignalResult::Mutation(value) => format!(
            "(score:{}, survived:{}, killed:{})",
            fmt_opt_percent(value.score),
            value.survived,
            value.killed
        ),
    }
}

fn nested_budget_number(value: &serde_json::Value, key: &str, nested: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(|value| value.get(nested))
        .and_then(|value| value.as_f64())
}

fn fmt_number(value: f64) -> String {
    format!("{value:.1}")
}

fn fmt_opt_number(value: Option<f64>) -> String {
    value.map(fmt_number).unwrap_or_else(|| String::from("—"))
}

fn fmt_opt_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| String::from("—"))
}

fn fmt_opt_signed(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.1}"))
        .unwrap_or_else(|| String::from("—"))
}

fn run_target_jobs(
    ctx: &ui::runner::ExecContext,
    jobs: Vec<(usize, AnalyzeTarget)>,
    worker_limit: usize,
    result_slots: TargetResultSlots,
) -> Result<(), String> {
    if jobs.is_empty() {
        return Ok(());
    }
    let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let worker_count = worker_limit.max(1).min(
        queue
            .lock()
            .map_err(|_| String::from("analyze queue mutex poisoned"))?
            .len(),
    );
    let mut handles = Vec::new();
    for _ in 0..worker_count {
        let ctx = ctx.clone();
        let queue = Arc::clone(&queue);
        let result_slots = Arc::clone(&result_slots);
        handles.push(thread::spawn(move || -> Result<(), String> {
            loop {
                if ctx.is_aborted() {
                    break;
                }
                let next_job = {
                    let mut guard = queue
                        .lock()
                        .map_err(|_| String::from("analyze queue mutex poisoned"))?;
                    guard.pop_front()
                };
                let Some((index, target)) = next_job else {
                    break;
                };
                let result = collect_target_with_ui(&ctx, &target);
                if result.is_err() {
                    ctx.abort();
                }
                let mut guard = result_slots
                    .lock()
                    .map_err(|_| String::from("analyze result mutex poisoned"))?;
                guard[index] = Some(result);
            }
            Ok(())
        }));
    }
    for handle in handles {
        handle
            .join()
            .map_err(|_| String::from("analyze worker panicked"))??;
    }
    Ok(())
}

fn flatten_target_results(
    result_slots: TargetResultSlots,
    aborted: bool,
) -> Result<Vec<SignalRow>, String> {
    let mut guard = result_slots
        .lock()
        .map_err(|_| String::from("analyze result mutex poisoned"))?;
    let mut rows = Vec::new();
    let mut first_error = None;
    for slot in guard.iter_mut() {
        match slot.take() {
            Some(Ok(target_rows)) => rows.extend(target_rows),
            Some(Err(error)) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
            None => {
                if first_error.is_none() && aborted {
                    first_error = Some(String::from("operation aborted"));
                }
            }
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(rows)
    }
}

fn analyze(config_path: &str, options: AnalyzeOptions) -> ExitCode {
    match analyze_impl(config_path, options) {
        Ok(AnalyzeOutcome::Completed { has_failures }) => {
            if has_failures {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Ok(AnalyzeOutcome::Aborted) => {
            eprintln!("check aborted");
            ExitCode::from(4)
        }
        Err(AnalyzeError::InvalidContract(error)) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
        Err(AnalyzeError::Incomplete(error)) => {
            eprintln!("{error}");
            ExitCode::from(4)
        }
    }
}

enum AnalyzeOutcome {
    Completed { has_failures: bool },
    Aborted,
}

enum AnalyzeError {
    InvalidContract(String),
    Incomplete(String),
}

impl From<String> for AnalyzeError {
    fn from(error: String) -> Self {
        Self::Incomplete(error)
    }
}

fn analyze_impl(
    config_path: &str,
    options: AnalyzeOptions,
) -> Result<AnalyzeOutcome, AnalyzeError> {
    let config_path = PathBuf::from(config_path);
    let workspace_root = workspace_root_from_config_path(&config_path);
    let policy = AyniPolicy::load_from_path(&config_path).map_err(AnalyzeError::InvalidContract)?;
    ensure_analyze_directories(&workspace_root)?;

    let AnalyzeOptions { output_mode, debug } = options;

    let planning = build_analyze_targets(&workspace_root, &policy, None, None, None, debug)?;
    let plan = build_analyze_plan(&planning.targets);
    let metadata = build_artifact_metadata(&config_path, &workspace_root, &planning, output_mode)?;
    let artifact_slot = Arc::new(Mutex::new(None));
    let aborted = execute_analyze_plan_or_persist_failure(
        &workspace_root,
        &planning,
        &metadata,
        output_mode,
        debug,
        plan,
        Arc::clone(&artifact_slot),
    )?;
    if persist_aborted_analysis(&workspace_root, &planning, &metadata, aborted)? {
        return Ok(AnalyzeOutcome::Aborted);
    }

    let mut artifact = take_collected_artifact_or_persist_failure(
        &workspace_root,
        &planning,
        &metadata,
        artifact_slot,
    )?;
    artifact.metadata = metadata;
    verification_command::materialize_finding_commands(&mut artifact, &build_registry())?;
    let serialized = serialize_artifact(&artifact)?;
    persist_artifact_at(&workspace_root, SIGNALS_ARTIFACT, &serialized)?;
    emit_analyze_outputs(output_mode, &policy, &artifact, &serialized)?;

    Ok(AnalyzeOutcome::Completed {
        has_failures: artifact.completion.state == CompletionState::Incomplete
            || artifact.rows.iter().any(|row| !row.pass),
    })
}

fn execute_analyze_plan_or_persist_failure(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    output_mode: OutputArg,
    debug: bool,
    plan: ui::runner::Plan,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<bool, String> {
    match execute_analyze_plan(output_mode, debug, plan, planning.clone(), artifact_slot) {
        Ok(aborted) => Ok(aborted),
        Err(error) => {
            persist_incomplete_execution_artifact(
                workspace_root,
                metadata.clone(),
                planning,
                CompletionStage::Scheduling,
                &error,
            )?;
            Err(error)
        }
    }
}

fn persist_aborted_analysis(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    aborted: bool,
) -> Result<bool, String> {
    if aborted {
        persist_incomplete_execution_artifact(
            workspace_root,
            metadata.clone(),
            planning,
            CompletionStage::Collection,
            "analysis was interrupted before every target completed",
        )?;
    }
    Ok(aborted)
}

fn take_collected_artifact_or_persist_failure(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<RunArtifact, String> {
    match take_collected_artifact(artifact_slot) {
        Ok(artifact) => Ok(artifact),
        Err(error) => {
            persist_incomplete_execution_artifact(
                workspace_root,
                metadata.clone(),
                planning,
                CompletionStage::Collection,
                &error,
            )?;
            Err(error)
        }
    }
}

fn persist_incomplete_execution_artifact(
    workspace_root: &Path,
    metadata: RunArtifactMetadata,
    planning: &AnalyzePlanning,
    stage: CompletionStage,
    message: &str,
) -> Result<(), String> {
    let artifact = RunArtifact::new(
        metadata,
        planning.completion(
            CompletionScope::Repository,
            0,
            planning.runnable_failure_issues(stage, message),
        ),
        Vec::new(),
    );
    let serialized = serialize_artifact(&artifact)?;
    persist_artifact_at(workspace_root, SIGNALS_ARTIFACT, &serialized)
}

fn ensure_analyze_directories(workspace_root: &Path) -> Result<(), String> {
    fs::create_dir_all(workspace_root.join(ARTIFACTS_DIR)).map_err(|error| error.to_string())?;
    Ok(())
}

fn execute_analyze_plan(
    output_mode: OutputArg,
    debug: bool,
    plan: ui::runner::Plan,
    planning: AnalyzePlanning,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<bool, String> {
    let execution = build_analyze_execution(planning, artifact_slot);
    if debug {
        return ui::runner::run_plain(plan, execution, debug_progress_event)
            .map(|outcome| outcome.aborted);
    }
    match output_mode {
        OutputArg::Md | OutputArg::Json => {
            ui::runner::run_plain(plan, execution, ui::progress_log::log_started_check)
                .map(|outcome| outcome.aborted)
        }
        OutputArg::Stdout => run_stdout_plan(plan, execution),
    }
}

fn debug_progress_event(event: ui::runner::ProgressEvent) {
    match event {
        ui::runner::ProgressEvent::Started { language, name } => {
            eprintln!("[{language}] {name} started");
        }
        ui::runner::ProgressEvent::Line {
            language,
            name,
            line,
        } => {
            eprintln!("[{language}] {name}: {line}");
        }
        ui::runner::ProgressEvent::Finished {
            language,
            name,
            state,
            elapsed,
        } => {
            eprintln!(
                "[{language}] {name} {state:?} {:.1}s",
                elapsed.as_secs_f64()
            );
        }
    }
}

fn build_analyze_execution(
    planning: AnalyzePlanning,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> impl FnOnce(ui::runner::ExecContext) -> Result<(), String> {
    move |exec_ctx: ui::runner::ExecContext| {
        let artifact = run_collect_with_ui(&exec_ctx, &planning, CompletionScope::Repository)?;
        let mut slot = artifact_slot
            .lock()
            .map_err(|_| String::from("artifact mutex poisoned"))?;
        *slot = Some(artifact);
        Ok(())
    }
}

fn run_stdout_plan(
    plan: ui::runner::Plan,
    execution: impl FnOnce(ui::runner::ExecContext) -> Result<(), String> + Send + 'static,
) -> Result<bool, String> {
    if ui::is_interactive_stdout() {
        ui::runner::run(plan, execution).map(|outcome| outcome.aborted)
    } else {
        ui::fallback::run(&plan, execution)?;
        Ok(false)
    }
}

fn take_collected_artifact(
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<RunArtifact, String> {
    let artifact = artifact_slot
        .lock()
        .map_err(|_| String::from("artifact mutex poisoned"))?
        .take();
    artifact.ok_or_else(|| String::from("analyze produced no artifact"))
}

fn build_artifact_metadata(
    config_path: &Path,
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    output_mode: OutputArg,
) -> Result<RunArtifactMetadata, String> {
    build_artifact_metadata_for_command(config_path, workspace_root, planning, output_mode, "check")
}

fn build_artifact_metadata_for_command(
    config_path: &Path,
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    output_mode: OutputArg,
    command: &str,
) -> Result<RunArtifactMetadata, String> {
    let generated_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| format!("failed to format analysis timestamp: {error}"))?;
    let languages = planning
        .targets
        .iter()
        .map(|target| target.language)
        .chain(planning.issues.iter().map(|issue| issue.language))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let scope = planning
        .targets
        .first()
        .map(|target| target.run_context.scope.clone());
    Ok(RunArtifactMetadata {
        generated_at,
        ayni_version: String::from(env!("CARGO_PKG_VERSION")),
        invocation: InvocationContext {
            command: command.to_string(),
            languages,
            scope,
        },
        output: OutputContext {
            format: output_mode.as_str().to_string(),
            destination: String::from("stdout"),
        },
        config_path: config_path.to_string_lossy().into_owned(),
        repository_root: workspace_root.to_string_lossy().into_owned(),
    })
}

fn serialize_artifact(artifact: &RunArtifact) -> Result<String, String> {
    serde_json::to_string_pretty(artifact)
        .map(|serialized| format!("{serialized}\n"))
        .map_err(|error| format!("failed to serialize artifact: {error}"))
}

fn persist_artifact_at(
    repo_root: &Path,
    relative_path: &str,
    serialized: &str,
) -> Result<(), String> {
    let destination = repo_root.join(relative_path);
    let parent = destination
        .parent()
        .ok_or_else(|| format!("artifact path {relative_path} has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!("failed to create artifact directory for {relative_path}: {error}")
    })?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("artifact path {relative_path} has no file name"))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temporary, serialized).map_err(|error| {
        format!("failed to write temporary artifact for {relative_path}: {error}")
    })?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "failed to atomically replace {relative_path}: {error}"
        ));
    }
    Ok(())
}

fn emit_analyze_outputs(
    output_mode: OutputArg,
    policy: &AyniPolicy,
    artifact: &RunArtifact,
    serialized: &str,
) -> Result<(), String> {
    match output_mode {
        OutputArg::Stdout => {
            ui::report::print_from_artifact(artifact, policy.report.offenders_limit);
        }
        OutputArg::Md => {
            ui::progress_log::log_command_failures(artifact);
            let summary = ui::md_report::build_markdown(artifact, policy.report.offenders_limit);
            println!("{summary}");
        }
        OutputArg::Json => {
            print!("{serialized}");
        }
    }
    Ok(())
}

fn workspace_root_from_config_path(config_path: &Path) -> PathBuf {
    let Some(parent) = config_path.parent() else {
        return PathBuf::from(".");
    };
    if parent.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        parent.to_path_buf()
    }
}

fn build_analyze_targets(
    repo_root: &Path,
    policy: &AyniPolicy,
    package: Option<String>,
    file: Option<String>,
    language_filter: Option<Language>,
    debug: bool,
) -> Result<AnalyzePlanning, String> {
    let file = file.map(|value| canonicalize_relative_posix(&value));
    let enabled_languages = policy.enabled_languages()?;
    if let Some(language) = language_filter
        && !enabled_languages.contains(&language)
    {
        return Err(format!(
            "requested language {language} is not enabled in the configured policy"
        ));
    }
    let registry = build_registry();
    let configured =
        discovery::plan_configured_targets(repo_root, policy, language_filter, &registry)?;
    let expected_targets = configured.len() as u64;
    let detected_targets = configured.iter().filter(|target| target.detected).count() as u64;
    let issues = configured
        .iter()
        .filter_map(|target| target.issue.clone())
        .collect();
    let mut targets = Vec::new();
    for configured_target in configured {
        let language = configured_target.language;
        let root = configured_target.configured_root;
        if let Some(mut execution) = configured_target.execution {
            if let Some(environment) = managed_target_environment(language, &root)? {
                execution.environment.extend(environment);
            }
            let workdir = repo_root.join(&root);
            let scope = Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: if root == "." {
                    None
                } else {
                    Some(root.clone())
                },
                package: package.clone(),
                file: file.clone(),
            };
            let run_context = RunContext {
                repo_root: repo_root.to_path_buf(),
                target_root: workdir.clone(),
                workdir: workdir.clone(),
                policy: policy.clone(),
                scope,
                execution,
                debug,
            };
            targets.push(AnalyzeTarget {
                language,
                root,
                run_context,
            });
        }
    }
    Ok(AnalyzePlanning {
        targets,
        expected_targets,
        detected_targets,
        issues,
    })
}

fn managed_target_environment(
    language: Language,
    root: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(serialized) = std::env::var_os("AYNI_MANAGED_TARGET_ENVIRONMENTS") else {
        return Ok(None);
    };
    let environments: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_str(&serialized.to_string_lossy())
            .map_err(|error| format!("managed target environments are invalid: {error}"))?;
    let key = format!("{language}:{root}");
    environments
        .get(&key)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("managed environment has no locked activation for target {key}"))
}

fn canonicalize_relative_posix(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        String::from(".")
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests;
