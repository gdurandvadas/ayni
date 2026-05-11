use ayni_adapters_go::GoAdapter;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;

mod delta;
mod discovery;
mod install;
mod ui;

use ayni_adapters_node::NodeAdapter;
use ayni_adapters_python::PythonAdapter;
use ayni_adapters_rust::RustAdapter;
use ayni_core::{
    AYNI_POLICY_FILE, AYNI_SIGNAL_SCHEMA_VERSION, AdapterRegistry, AyniPolicy, Budget,
    CommandFailure, ComplexityResult, ConcurrencyPolicy, CoverageResult, DepsResult, Language,
    MutationResult, Offenders, RunArtifact, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    SizeResult, TestResult,
};
use clap::{Parser, Subcommand, ValueEnum};
use delta::annotate_deltas_vs_previous;
use install::{enabled_signal_kinds, install_impl, persist_artifact};

const ARTIFACTS_DIR: &str = ".ayni/last";
const HISTORY_DIR: &str = ".ayni/history";
const SIGNALS_ARTIFACT: &str = ".ayni/last/signals.json";
const PREVIOUS_SIGNALS_SNAPSHOT: &str = ".ayni/history/previous-signals.jsonl";
const AGENTS_MANAGED_BEGIN: &str = "<!-- AYNI:BEGIN -->";
const AGENTS_MANAGED_END: &str = "<!-- AYNI:END -->";
const RUST_POLICY_TEMPLATE: &str = include_str!("../templates/policy/rust.toml");
const GO_POLICY_TEMPLATE: &str = include_str!("../templates/policy/go.toml");
const NODE_POLICY_TEMPLATE: &str = include_str!("../templates/policy/node.toml");
const PYTHON_POLICY_TEMPLATE: &str = include_str!("../templates/policy/python.toml");

#[derive(Parser, Debug)]
#[command(name = "ayni")]
#[command(version, about = "Open-source code quality signals for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Analyze the local repository and print a quality report.
    Analyze {
        #[arg(long, default_value = "./.ayni.toml")]
        config: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        package: Option<String>,
        #[arg(long, value_enum)]
        language: Option<LanguageArg>,
        /// Report format: `stdout` (default, coloured console) or `md` (markdown report).
        #[arg(long, value_enum, default_value = "stdout")]
        output: OutputArg,
        /// Print raw command diagnostics and disable the live dashboard.
        #[arg(long)]
        debug: bool,
    },
    /// Scaffold repo guidance and show required tools; use `--apply` to install them.
    Install {
        #[arg(long, default_value = ".")]
        repo_root: String,
        #[arg(long, value_enum)]
        language: Option<LanguageArg>,
        /// Install missing or outdated tools from adapter catalogs (cargo, rustup, go, npm, …).
        #[arg(long)]
        apply: bool,
    },
    /// Print the Ayni CLI version.
    Version,
    #[command(hide = true)]
    GenerateDocs,
}

#[derive(Clone, Debug, ValueEnum)]
enum LanguageArg {
    Rust,
    Go,
    Node,
    Python,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum OutputArg {
    /// Coloured console report (default).
    Stdout,
    /// Markdown report printed to stdout.
    Md,
}

impl LanguageArg {
    fn as_language(&self) -> Language {
        match self {
            Self::Rust => Language::Rust,
            Self::Go => Language::Go,
            Self::Node => Language::Node,
            Self::Python => Language::Python,
        }
    }
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Commands::Analyze {
            config,
            file,
            package,
            language,
            output,
            debug,
        } => analyze(
            &config,
            AnalyzeOptions {
                package,
                file,
                language_filter: language.map(|value| value.as_language()),
                output_mode: output,
                debug,
            },
        ),
        Commands::Install {
            repo_root,
            language,
            apply,
        } => install(&repo_root, language.map(|value| value.as_language()), apply),
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Commands::GenerateDocs => {
            println!("{}", clap_markdown::help_markdown::<Cli>());
            ExitCode::SUCCESS
        }
    }
}

fn build_registry() -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(GoAdapter::new()));
    registry.register(Arc::new(RustAdapter::new()));
    registry.register(Arc::new(NodeAdapter::new()));
    registry.register(Arc::new(PythonAdapter::new()));
    registry
}

fn install(repo_root: &str, language_filter: Option<Language>, apply: bool) -> ExitCode {
    match install_impl(repo_root, language_filter, apply) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failures) => {
            for failure in failures {
                eprintln!("{failure}");
            }
            ExitCode::FAILURE
        }
    }
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

type TargetCollectResult = Result<Vec<SignalRow>, String>;
type TargetResultSlots = Arc<Mutex<Vec<Option<TargetCollectResult>>>>;

#[derive(Clone, Debug)]
struct AnalyzeOptions {
    package: Option<String>,
    file: Option<String>,
    language_filter: Option<Language>,
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
    targets: &[AnalyzeTarget],
) -> Result<RunArtifact, String> {
    let concurrency = targets
        .first()
        .map(|target| target.run_context.policy.concurrency.clone())
        .unwrap_or_default();
    let rows = collect_targets_with_ui(ctx, targets, &concurrency)?;
    Ok(RunArtifact {
        schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
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
        let mut group_handles = Vec::new();
        for (language, jobs) in by_language {
            let ctx = ctx.clone();
            let result_slots = Arc::clone(&result_slots);
            let worker_limit = if language == Language::Rust {
                1
            } else {
                concurrency.amount
            };
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
        let row_result = match (target.language, kind) {
            (Language::Rust, SignalKind::Test) => {
                ayni_adapters_rust::collectors::test::collect_with_lines(
                    &target.run_context,
                    |line| {
                        tool.line(line);
                    },
                )
            }
            _ => adapter
                .collector()
                .collect(kind, &target.run_context)
                .map_err(|e| e.to_string()),
        };
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
        delta_vs_previous: None,
        delta_vs_baseline: None,
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
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Ok(AnalyzeOutcome::Aborted) => {
            eprintln!("analyze aborted");
            ExitCode::from(130)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

enum AnalyzeOutcome {
    Completed { has_failures: bool },
    Aborted,
}

fn analyze_impl(config_path: &str, options: AnalyzeOptions) -> Result<AnalyzeOutcome, String> {
    let config_path = PathBuf::from(config_path);
    let workspace_root = workspace_root_from_config_path(&config_path);
    let policy = AyniPolicy::load_from_path(&config_path)?;
    ensure_analyze_directories(&workspace_root)?;

    let AnalyzeOptions {
        package,
        file,
        language_filter,
        output_mode,
        debug,
    } = options;

    let targets = build_analyze_targets(
        &workspace_root,
        &policy,
        package,
        file,
        language_filter,
        debug,
    )?;
    let plan = build_analyze_plan(&targets);
    let artifact_slot = Arc::new(Mutex::new(None));
    let aborted = execute_analyze_plan(
        output_mode,
        debug,
        plan,
        targets,
        Arc::clone(&artifact_slot),
    )?;
    if aborted {
        return Ok(AnalyzeOutcome::Aborted);
    }

    let mut artifact = take_collected_artifact(artifact_slot)?;
    let previous_artifact = load_previous_artifact(&workspace_root);
    annotate_deltas_vs_previous(&mut artifact, previous_artifact.as_ref());
    persist_artifact(&workspace_root, &artifact)?;
    emit_analyze_outputs(output_mode, &policy, &artifact)?;

    Ok(AnalyzeOutcome::Completed {
        has_failures: artifact.rows.iter().any(|row| !row.pass),
    })
}

fn ensure_analyze_directories(workspace_root: &Path) -> Result<(), String> {
    fs::create_dir_all(workspace_root.join(ARTIFACTS_DIR)).map_err(|error| error.to_string())?;
    fs::create_dir_all(workspace_root.join(HISTORY_DIR)).map_err(|error| error.to_string())?;
    Ok(())
}

fn execute_analyze_plan(
    output_mode: OutputArg,
    debug: bool,
    plan: ui::runner::Plan,
    targets: Vec<AnalyzeTarget>,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<bool, String> {
    let execution = build_analyze_execution(targets, artifact_slot);
    if debug {
        return ui::runner::run_plain(plan, execution, debug_progress_event)
            .map(|outcome| outcome.aborted);
    }
    match output_mode {
        OutputArg::Md => {
            ui::runner::run_plain(plan, execution, |_| {}).map(|outcome| outcome.aborted)
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
    targets: Vec<AnalyzeTarget>,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> impl FnOnce(ui::runner::ExecContext) -> Result<(), String> {
    move |exec_ctx: ui::runner::ExecContext| {
        let artifact = run_collect_with_ui(&exec_ctx, &targets)?;
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

fn emit_analyze_outputs(
    output_mode: OutputArg,
    policy: &AyniPolicy,
    artifact: &RunArtifact,
) -> Result<(), String> {
    match output_mode {
        OutputArg::Stdout => {
            ui::report::print_from_rows(&artifact.rows, policy.report.offenders_limit);
        }
        OutputArg::Md => {
            let summary = ui::md_report::build_markdown(artifact, policy.report.offenders_limit);
            println!("{summary}");
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
) -> Result<Vec<AnalyzeTarget>, String> {
    let file = file.map(|value| canonicalize_relative_posix(&value));
    let enabled_languages = policy.enabled_languages()?;
    let enabled_set: BTreeSet<Language> = enabled_languages.into_iter().collect();
    let registry = build_registry();
    let mut targets = Vec::new();
    for language in [
        Language::Rust,
        Language::Go,
        Language::Node,
        Language::Python,
    ] {
        if let Some(filter) = language_filter
            && filter != language
        {
            continue;
        }
        if !enabled_set.contains(&language) {
            continue;
        }
        for root in policy.roots_for(language) {
            let workdir = repo_root.join(root);
            let has_adapter_for_root = registry
                .detect(&workdir)
                .into_iter()
                .any(|adapter| adapter.language() == language);
            if !has_adapter_for_root {
                continue;
            }
            let Some(adapter) = registry
                .adapters()
                .iter()
                .find(|candidate| candidate.language() == language)
            else {
                continue;
            };
            let Some(execution) = adapter.resolve_execution(repo_root, &workdir) else {
                continue;
            };
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
                diff: None,
                execution,
                debug,
            };
            targets.push(AnalyzeTarget {
                language,
                root: root.clone(),
                run_context,
            });
        }
    }
    Ok(targets)
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

fn load_previous_artifact(repo_root: &Path) -> Option<RunArtifact> {
    let candidates = [
        repo_root.join(PREVIOUS_SIGNALS_SNAPSHOT),
        repo_root.join(SIGNALS_ARTIFACT),
    ];
    for candidate in candidates {
        let content = match fs::read_to_string(&candidate) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Ok(artifact) = serde_json::from_str::<RunArtifact>(&content) {
            return Some(artifact);
        }
    }
    None
}

#[cfg(test)]
mod tests;
