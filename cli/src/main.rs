use ayni_adapters_go::GoAdapter;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::{Arc, Mutex};
use std::thread;

mod ui;

use ayni_adapters_node::NodeAdapter;
use ayni_adapters_python::PythonAdapter;
use ayni_adapters_rust::RustAdapter;
use ayni_core::{
    AYNI_POLICY_FILE, AYNI_SIGNAL_SCHEMA_VERSION, AdapterRegistry, AyniPolicy, Budget,
    CatalogEntry, ConcurrencyPolicy, Delta, InstallContext, Installer, Language,
    NodePackageManager, PythonPackageManager, RunArtifact, RunContext, Scope, SignalKind,
    SignalResult, SignalRow, ToolStatus, VersionCheck, detect_node_package_manager,
    detect_python_package_manager,
};
use clap::{Parser, Subcommand, ValueEnum};

const AYNI_POLICY_TOML: &str = include_str!("../../.ayni.toml");
const ARTIFACTS_DIR: &str = ".ayni/last";
const HISTORY_DIR: &str = ".ayni/history";
const SIGNALS_ARTIFACT: &str = ".ayni/last/signals.json";
const PREVIOUS_SIGNALS_SNAPSHOT: &str = ".ayni/history/previous-signals.jsonl";
const AGENTS_MANAGED_BEGIN: &str = "<!-- AYNI:BEGIN -->";
const AGENTS_MANAGED_END: &str = "<!-- AYNI:END -->";
const ALL_LANGUAGES: [Language; 4] = [
    Language::Rust,
    Language::Go,
    Language::Node,
    Language::Python,
];

#[derive(Parser, Debug)]
#[command(name = "ayni")]
#[command(version, about = "Local-first code quality signals for AI agents")]
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
        } => analyze(
            &config,
            AnalyzeOptions {
                package,
                file,
                language_filter: language.map(|value| value.as_language()),
                output_mode: output,
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

fn install_impl(
    repo_root: &str,
    language_filter: Option<Language>,
    apply: bool,
) -> Result<(), Vec<String>> {
    let root = PathBuf::from(repo_root);
    let policy = prepare_install_policy(&root, language_filter).map_err(|error| vec![error])?;
    if apply {
        let failures = collect_install_failures(&root, &policy, language_filter);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    } else {
        print_install_requirements(&root, &policy, language_filter);
        Ok(())
    }
}

fn print_install_requirements(
    repo_root: &Path,
    policy: &AyniPolicy,
    language_filter: Option<Language>,
) {
    println!("Ayni tooling requirements (from adapter catalogs)");
    println!(
        "Scaffolding is already updated (`.ayni.toml`, `.gitignore`, `AGENTS.md` when needed)."
    );
    println!(
        "Run `ayni install --apply` to install missing or outdated tools (may use cargo, rustup, go, npm, …).\n"
    );
    let registry = build_registry();
    let mut any_tool_row = false;
    for adapter in registry.adapters() {
        let language = adapter.language();
        if should_skip_install_language(policy, language, language_filter) {
            continue;
        }
        for root_entry in policy.roots_for(language) {
            let root_path = repo_root.join(root_entry);
            if !adapter.detect(&root_path).detected {
                continue;
            }
            let label = root_label_for_install(root_entry);
            println!("## {} — {}", language.as_str(), label);
            let node_manager = if language == Language::Node {
                detect_node_package_manager(&root_path).or(Some(NodePackageManager::Npm))
            } else {
                None
            };
            let python_manager = if language == Language::Python {
                detect_python_package_manager(&root_path).or(Some(PythonPackageManager::Pip))
            } else {
                None
            };
            let install_context = InstallContext {
                cwd: Some(root_path.as_path()),
                node_package_manager: node_manager,
                python_package_manager: python_manager,
            };
            for entry in adapter.catalog() {
                if entry.opt_in && !check_enabled_for_entry(policy, entry) {
                    continue;
                }
                any_tool_row = true;
                let status = entry.status_in(install_context);
                let status_str = match status {
                    ToolStatus::Missing => "missing",
                    ToolStatus::Outdated => "outdated",
                    ToolStatus::Current => "ok",
                };
                let signals = entry
                    .for_signals
                    .iter()
                    .map(|k| signal_kind_slug(*k))
                    .collect::<Vec<_>>()
                    .join(", ");
                let note = catalog_entry_requirement_note(entry);
                println!(
                    "  {:<30}  {:<8}  signals: {:<24}  {}",
                    entry.name, status_str, signals, note
                );
            }
            println!();
        }
    }
    if !any_tool_row {
        println!("No catalog tools listed for the current policy and detected workspaces.");
        println!("Enable languages in `.ayni.toml`, adjust `[checks]`, or pass `--language`.");
    }
}

fn root_label_for_install(root_entry: &str) -> String {
    if root_entry == "." {
        String::from("workspace root")
    } else {
        root_entry.to_string()
    }
}

fn catalog_entry_requirement_note(entry: &CatalogEntry) -> String {
    let mut parts = Vec::new();
    if let Some(check) = &entry.check {
        parts.push(version_check_summary(check));
    }
    parts.push(installer_summary(&entry.installer));
    parts.join(" · ")
}

fn version_check_summary(check: &VersionCheck) -> String {
    let cmd = format!("{} {}", check.command, check.args.join(" "));
    match check.contains {
        Some(s) => format!("check `{cmd}` → stdout contains {s:?}"),
        None => format!("check `{cmd}` → succeeds"),
    }
}

fn installer_summary(inst: &Installer) -> String {
    match inst {
        Installer::Bundled => String::from("install: (bundled with toolchain)"),
        Installer::Cargo {
            crate_name,
            version,
        } => fmt_cargo_install(crate_name, *version),
        Installer::Rustup { component } => format!("install: rustup component add {component}"),
        Installer::GoInstall { module, version } => fmt_go_install(module, *version),
        Installer::NpmGlobal { package, version } => fmt_npm_global(package, *version),
        Installer::NodePackage {
            package,
            version,
            dev,
        } => fmt_node_package(package, *version, *dev),
        Installer::PythonPackage {
            package,
            version,
            dev,
            ..
        } => fmt_python_package(package, *version, *dev),
        Installer::UvTool { package, version } => fmt_uv_tool(package, *version),
        Installer::PythonRuntime => String::from("install: (python runtime on PATH)"),
        Installer::Custom { program, args } => format!("install: {} {}", program, args.join(" ")),
    }
}

fn fmt_cargo_install(crate_name: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("install: cargo install {crate_name} --version {v}"),
        None => format!("install: cargo install {crate_name}"),
    }
}

fn fmt_go_install(module: &str, version: Option<&str>) -> String {
    let ver = version.unwrap_or("latest");
    format!("install: go install {module}@{ver}")
}

fn fmt_npm_global(package: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("install: npm install -g {package}@{v}"),
        None => format!("install: npm install -g {package}"),
    }
}

fn fmt_node_package(package: &str, version: Option<&str>, dev: bool) -> String {
    let scope = if dev { "devDependency" } else { "dependency" };
    match version {
        Some(v) => format!("install: add {scope} {package}@{v} via package manager"),
        None => format!("install: add {scope} {package} via package manager"),
    }
}

fn fmt_python_package(package: &str, version: Option<&str>, dev: bool) -> String {
    let scope = if dev { "devDependency" } else { "dependency" };
    match version {
        Some(v) => format!("install: add Python {scope} {package}=={v} via package manager"),
        None => format!("install: add Python {scope} {package} via package manager"),
    }
}

fn fmt_uv_tool(package: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("install: uv tool install {package}=={v}"),
        None => format!("install: uv tool install {package}"),
    }
}

fn collect_install_failures(
    root: &Path,
    policy: &AyniPolicy,
    language_filter: Option<Language>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let registry = build_registry();
    for adapter in registry.adapters() {
        let language = adapter.language();
        if should_skip_install_language(policy, language, language_filter) {
            continue;
        }
        for root_entry in policy.roots_for(language) {
            failures.extend(install_for_root(
                adapter.as_ref(),
                policy,
                root,
                language,
                root_entry,
            ));
        }
    }
    failures
}

fn should_skip_install_language(
    policy: &AyniPolicy,
    language: Language,
    language_filter: Option<Language>,
) -> bool {
    matches!(language_filter, Some(filter) if filter != language)
        || !language_enabled(policy, language)
}

fn install_for_root(
    adapter: &dyn ayni_core::LanguageAdapter,
    policy: &AyniPolicy,
    root: &Path,
    language: Language,
    root_entry: &str,
) -> Vec<String> {
    let root_path = root.join(root_entry);
    if !adapter.detect(&root_path).detected {
        return Vec::new();
    }

    let mut failures = Vec::new();
    let node_manager = prepare_node_manager(language, root_entry, &root_path, &mut failures);
    let python_manager = if language == Language::Python {
        detect_python_package_manager(&root_path).or(Some(PythonPackageManager::Pip))
    } else {
        None
    };
    let install_context = InstallContext {
        cwd: Some(&root_path),
        node_package_manager: node_manager,
        python_package_manager: python_manager,
    };

    for entry in adapter.catalog() {
        if entry.opt_in && !check_enabled_for_entry(policy, entry) {
            continue;
        }
        if matches!(
            entry.status_in(install_context),
            ToolStatus::Missing | ToolStatus::Outdated
        ) && let Err(error) = entry.install_in(install_context)
        {
            failures.push(format!("{} ({language}:{root_entry}): {error}", entry.name));
        }
    }

    failures
}

fn prepare_node_manager(
    language: Language,
    root_entry: &str,
    root_path: &Path,
    failures: &mut Vec<String>,
) -> Option<NodePackageManager> {
    if language != Language::Node {
        return None;
    }

    let manager = detect_node_package_manager(root_path).unwrap_or(NodePackageManager::Npm);
    if let Err(error) = install_node_dependencies(root_path, manager) {
        failures.push(format!("node install ({language}:{root_entry}): {error}"));
    }
    Some(manager)
}

fn install_node_dependencies(root_path: &Path, manager: NodePackageManager) -> Result<(), String> {
    let status = Command::new(manager.executable())
        .arg("install")
        .current_dir(root_path)
        .status()
        .map_err(|error| {
            format!(
                "failed to run {} install in {}: {error}",
                manager.executable(),
                root_path.display()
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} install failed in {} (exit {})",
            manager.executable(),
            root_path.display(),
            status.code().unwrap_or(-1)
        ))
    }
}

fn language_enabled(policy: &AyniPolicy, language: Language) -> bool {
    policy.language_allowed(language)
}

fn check_enabled_for_entry(policy: &AyniPolicy, entry: &CatalogEntry) -> bool {
    entry.for_signals.iter().all(|kind| match kind {
        SignalKind::Test => policy.checks.test,
        SignalKind::Coverage => policy.checks.coverage,
        SignalKind::Size => policy.checks.size,
        SignalKind::Complexity => policy.checks.complexity,
        SignalKind::Deps => policy.checks.deps,
        SignalKind::Mutation => policy.checks.mutation,
    })
}

fn prepare_install_policy(
    root: &Path,
    language_filter: Option<Language>,
) -> Result<AyniPolicy, String> {
    let scaffold = scaffold_files(root)?;
    let policy = AyniPolicy::load(root)?;
    if scaffold.policy_created {
        let enabled_languages = policy.enabled_languages()?;
        let discovered_roots = discover_language_roots(root, &enabled_languages, language_filter);
        update_policy_roots(root, &discovered_roots)?;
    }
    AyniPolicy::load(root)
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
                tool.finished(ui::runner::ToolState::Failed);
                return Err(error);
            }
        }
    }
    Ok(rows)
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
    } = options;

    let targets = build_analyze_targets(&workspace_root, &policy, package, file, language_filter)?;
    let plan = build_analyze_plan(&targets);
    let artifact_slot = Arc::new(Mutex::new(None));
    let aborted = execute_analyze_plan(output_mode, plan, targets, Arc::clone(&artifact_slot))?;
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
    plan: ui::runner::Plan,
    targets: Vec<AnalyzeTarget>,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<bool, String> {
    let execution = build_analyze_execution(targets, artifact_slot);
    match output_mode {
        OutputArg::Md => {
            ui::runner::run_plain(plan, execution, |_| {}).map(|outcome| outcome.aborted)
        }
        OutputArg::Stdout => run_stdout_plan(plan, execution),
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
) -> Result<Vec<AnalyzeTarget>, String> {
    let file = file.map(|value| canonicalize_relative_posix(&value));
    let enabled_languages = policy.enabled_languages()?;
    let enabled_set: BTreeSet<Language> = enabled_languages.into_iter().collect();
    let registry = build_registry();
    let mut targets = Vec::new();
    for language in ALL_LANGUAGES {
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
                workdir: workdir.clone(),
                policy: policy.clone(),
                scope,
                diff: None,
            };
            let has_adapter_for_root = registry
                .detect(&workdir)
                .into_iter()
                .any(|adapter| adapter.language() == language);
            if !has_adapter_for_root {
                continue;
            }
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SignalRowKey {
    kind: SignalKind,
    language: Language,
    workspace_root: String,
    path: Option<String>,
    package: Option<String>,
    file: Option<String>,
}

fn signal_row_key(row: &SignalRow) -> SignalRowKey {
    SignalRowKey {
        kind: row.kind,
        language: row.language,
        workspace_root: row.scope.workspace_root.clone(),
        path: row.scope.path.clone(),
        package: row.scope.package.clone(),
        file: row.scope.file.clone(),
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

fn annotate_deltas_vs_previous(
    artifact: &mut RunArtifact,
    previous_artifact: Option<&RunArtifact>,
) {
    let Some(previous_artifact) = previous_artifact else {
        for row in &mut artifact.rows {
            row.delta_vs_previous = Some(Delta {
                changes: serde_json::json!({ "status": "no_previous_run" }),
            });
        }
        return;
    };

    let mut previous_rows = HashMap::<SignalRowKey, &SignalRow>::new();
    for row in &previous_artifact.rows {
        previous_rows.insert(signal_row_key(row), row);
    }

    for row in &mut artifact.rows {
        let previous_row = previous_rows.get(&signal_row_key(row)).copied();
        row.delta_vs_previous = Some(compute_delta_vs_previous(row, previous_row));
    }
}

fn compute_delta_vs_previous(current: &SignalRow, previous: Option<&SignalRow>) -> Delta {
    let Some(previous) = previous else {
        return Delta {
            changes: serde_json::json!({ "status": "no_previous_target" }),
        };
    };

    let current_metrics = signal_result_metrics(&current.result);
    let previous_metrics = signal_result_metrics(&previous.result);
    let metric_names = current_metrics
        .keys()
        .chain(previous_metrics.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut metric_changes = serde_json::Map::new();
    for metric in metric_names {
        let current_value = current_metrics.get(metric).copied();
        let previous_value = previous_metrics.get(metric).copied();
        if current_value == previous_value {
            continue;
        }
        let mut change = serde_json::Map::new();
        if let Some(value) = previous_value {
            change.insert(String::from("from"), serde_json::Value::from(value));
        }
        if let Some(value) = current_value {
            change.insert(String::from("to"), serde_json::Value::from(value));
        }
        if let (Some(from), Some(to)) = (previous_value, current_value) {
            change.insert(String::from("delta"), serde_json::Value::from(to - from));
        }
        metric_changes.insert(metric.to_string(), serde_json::Value::Object(change));
    }

    let changed = current.pass != previous.pass || !metric_changes.is_empty();
    let mut changes = serde_json::Map::new();
    changes.insert(
        String::from("status"),
        serde_json::Value::from(if changed { "changed" } else { "unchanged" }),
    );
    if current.pass != previous.pass {
        changes.insert(
            String::from("pass"),
            serde_json::json!({ "from": previous.pass, "to": current.pass }),
        );
    }
    if !metric_changes.is_empty() {
        changes.insert(
            String::from("metrics"),
            serde_json::Value::Object(metric_changes),
        );
    }
    Delta {
        changes: serde_json::Value::Object(changes),
    }
}

fn signal_result_metrics(result: &SignalResult) -> HashMap<&'static str, f64> {
    let mut metrics = HashMap::new();
    match result {
        SignalResult::Test(value) => {
            metrics.insert("total_tests", value.total_tests as f64);
            metrics.insert("passed", value.passed as f64);
            metrics.insert("failed", value.failed as f64);
            if let Some(duration) = value.duration_ms {
                metrics.insert("duration_ms", duration as f64);
            }
        }
        SignalResult::Coverage(value) => {
            if let Some(percent) = value.percent {
                metrics.insert("percent", percent);
            }
            if let Some(percent) = value.line_percent {
                metrics.insert("line_percent", percent);
            }
            if let Some(percent) = value.branch_percent {
                metrics.insert("branch_percent", percent);
            }
        }
        SignalResult::Size(value) => {
            metrics.insert("max_lines", value.max_lines as f64);
            metrics.insert("total_files", value.total_files as f64);
            metrics.insert("warn_count", value.warn_count as f64);
            metrics.insert("fail_count", value.fail_count as f64);
        }
        SignalResult::Complexity(value) => {
            metrics.insert("measured_functions", value.measured_functions as f64);
            metrics.insert("max_fn_cyclomatic", value.max_fn_cyclomatic);
            if let Some(cognitive) = value.max_fn_cognitive {
                metrics.insert("max_fn_cognitive", cognitive);
            }
            metrics.insert("warn_count", value.warn_count as f64);
            metrics.insert("fail_count", value.fail_count as f64);
        }
        SignalResult::Deps(value) => {
            metrics.insert("crate_count", value.crate_count as f64);
            metrics.insert("edge_count", value.edge_count as f64);
            metrics.insert("violation_count", value.violation_count as f64);
        }
        SignalResult::Mutation(value) => {
            metrics.insert("killed", value.killed as f64);
            metrics.insert("survived", value.survived as f64);
            metrics.insert("timeout", value.timeout as f64);
            if let Some(score) = value.score {
                metrics.insert("score", score);
            }
        }
    }
    metrics
}

fn discover_language_roots(
    repo_root: &Path,
    enabled_languages: &[Language],
    language_filter: Option<Language>,
) -> BTreeMap<Language, Vec<String>> {
    let enabled_set: BTreeSet<Language> = enabled_languages.iter().copied().collect();
    let mut discovered = BTreeMap::new();
    for language in ALL_LANGUAGES {
        if let Some(filter) = language_filter
            && filter != language
        {
            continue;
        }
        if !enabled_set.contains(&language) {
            continue;
        }
        let roots = match language {
            Language::Rust => discover_rust_roots(repo_root),
            Language::Go => discover_go_roots(repo_root),
            Language::Node => discover_node_roots(repo_root),
            Language::Python => discover_python_roots(repo_root),
        };
        let selected = if roots.is_empty() {
            vec![String::from(".")]
        } else {
            roots
        };
        discovered.insert(language, selected);
    }
    discovered
}

fn discover_rust_roots(repo_root: &Path) -> Vec<String> {
    discover_file_parent_roots(repo_root, "Cargo.toml", |parts| {
        parts.contains(&"target") || parts.contains(&".git") || parts.contains(&"node_modules")
    })
}

fn discover_go_roots(repo_root: &Path) -> Vec<String> {
    discover_file_parent_roots(repo_root, "go.mod", |_| false)
}

fn discover_node_roots(repo_root: &Path) -> Vec<String> {
    let mut roots = discover_file_parent_roots(repo_root, "package.json", |parts| {
        parts.contains(&"node_modules")
    });

    let root_package_json = repo_root.join("package.json");
    if root_package_json.is_file()
        && let Ok(content) = fs::read_to_string(&root_package_json)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let mut patterns = Vec::new();
        if let Some(array) = value.get("workspaces").and_then(|v| v.as_array()) {
            patterns.extend(array.iter().filter_map(|item| item.as_str()));
        }
        if let Some(array) = value
            .get("workspaces")
            .and_then(|v| v.get("packages"))
            .and_then(|v| v.as_array())
        {
            patterns.extend(array.iter().filter_map(|item| item.as_str()));
        }
        for pattern in patterns {
            if !pattern.ends_with("/*") {
                continue;
            }
            let base = pattern.trim_end_matches("/*").trim_matches('/');
            if base.is_empty() {
                continue;
            }
            let base_path = repo_root.join(base);
            if let Ok(entries) = fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let candidate_dir = entry.path();
                    if !candidate_dir.is_dir() {
                        continue;
                    }
                    if candidate_dir.join("package.json").is_file()
                        && let Ok(relative) = candidate_dir.strip_prefix(repo_root)
                    {
                        roots.push(canonicalize_relative_posix(&relative.to_string_lossy()));
                    }
                }
            }
        }
    }

    dedupe_and_sort_roots(roots)
}

fn discover_python_roots(repo_root: &Path) -> Vec<String> {
    let mut roots = Vec::new();
    for marker in ["pyproject.toml", "requirements.txt", "Pipfile"] {
        roots.extend(discover_file_parent_roots(repo_root, marker, |parts| {
            parts.iter().any(|part| is_python_excluded_root(part))
        }));
    }
    dedupe_and_sort_roots(roots)
}

fn is_python_excluded_root(part: &str) -> bool {
    matches!(
        part,
        ".venv" | "venv" | "env" | "__pycache__" | ".tox" | ".nox" | ".git" | ".ayni"
    )
}

fn discover_file_parent_roots<F>(repo_root: &Path, file_name: &str, exclude: F) -> Vec<String>
where
    F: Fn(&[&str]) -> bool,
{
    let mut found = Vec::new();
    let mut stack = vec![repo_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(relative) = path.strip_prefix(repo_root) {
                    let text = canonicalize_relative_posix(&relative.to_string_lossy());
                    let parts: Vec<&str> = text.split('/').collect();
                    if exclude(&parts) {
                        continue;
                    }
                }
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|v| v.to_str()) != Some(file_name) {
                continue;
            }
            if let Some(parent) = path.parent()
                && let Ok(relative) = parent.strip_prefix(repo_root)
            {
                found.push(canonicalize_relative_posix(&relative.to_string_lossy()));
            }
        }
    }
    dedupe_and_sort_roots(found)
}

fn dedupe_and_sort_roots(mut roots: Vec<String>) -> Vec<String> {
    roots.sort();
    roots.dedup();
    roots
}

fn update_policy_roots(
    repo_root: &Path,
    discovered_roots: &BTreeMap<Language, Vec<String>>,
) -> Result<(), String> {
    if discovered_roots.is_empty() {
        return Ok(());
    }
    let policy_path = repo_root.join(AYNI_POLICY_FILE);
    let content = fs::read_to_string(&policy_path)
        .map_err(|error| format!("failed to read {}: {error}", policy_path.display()))?;
    let mut document = content.parse::<toml::Value>().map_err(|error| {
        format!(
            "failed to parse {} as toml for root updates: {error}",
            policy_path.display()
        )
    })?;
    let Some(table) = document.as_table_mut() else {
        return Err(format!("{} is not a TOML table", policy_path.display()));
    };
    for (language, roots) in discovered_roots {
        let key = language.as_str();
        if !table.contains_key(key) {
            table.insert(key.to_string(), toml::Value::Table(toml::Table::new()));
        }
        let lang_table = table
            .get_mut(key)
            .and_then(toml::Value::as_table_mut)
            .ok_or_else(|| format!("[{key}] must be a table in {}", policy_path.display()))?;
        lang_table.insert(
            String::from("roots"),
            toml::Value::Array(
                roots
                    .iter()
                    .map(|root| toml::Value::String(root.clone()))
                    .collect(),
            ),
        );
    }
    let serialized = toml::to_string_pretty(&document)
        .map_err(|error| format!("failed to serialize {}: {error}", policy_path.display()))?;
    fs::write(&policy_path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", policy_path.display()))
}

struct ScaffoldOutcome {
    policy_created: bool,
}

fn scaffold_files(repo_root: &Path) -> Result<ScaffoldOutcome, String> {
    let policy_path = repo_root.join(".ayni.toml");
    let policy_created = !policy_path.exists();
    if policy_created {
        fs::write(&policy_path, AYNI_POLICY_TOML)
            .map_err(|error| format!("failed to create {}: {error}", policy_path.display()))?;
    }
    ensure_ayni_gitignore_entry(&repo_root.join(".gitignore"))?;
    ensure_agents_managed_section(repo_root)?;
    Ok(ScaffoldOutcome { policy_created })
}

fn ensure_agents_managed_section(repo_root: &Path) -> Result<(), String> {
    let path = repo_root.join("AGENTS.md");
    let content = if path.exists() {
        fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
    } else {
        String::new()
    };
    let managed = managed_agents_block();
    let updated = upsert_managed_block(&content, &managed);
    if updated != content {
        fs::write(&path, updated)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn managed_agents_block() -> String {
    [
        AGENTS_MANAGED_BEGIN,
        "## Code quality guidance for AI agents",
        "",
        "When modifying this repository:",
        "",
        "- Preserve clear module boundaries.",
        "- Prefer small, testable units.",
        "- Keep CLI, core logic, command execution, and reporting separate.",
        "- Avoid adding network dependencies unless explicitly required.",
        "- Update tests when behavior changes.",
        "",
        "Run:",
        "",
        "```sh",
        "ayni analyze",
        "```",
        AGENTS_MANAGED_END,
        "",
    ]
    .join("\n")
}

fn upsert_managed_block(existing: &str, managed: &str) -> String {
    let normalized_existing = if existing.is_empty() {
        String::new()
    } else if existing.ends_with('\n') {
        existing.to_string()
    } else {
        format!("{existing}\n")
    };

    let begin = normalized_existing.find(AGENTS_MANAGED_BEGIN);
    let end = normalized_existing.find(AGENTS_MANAGED_END);
    if let (Some(begin_idx), Some(end_idx)) = (begin, end)
        && begin_idx <= end_idx
    {
        let end_exclusive = end_idx + AGENTS_MANAGED_END.len();
        let mut result = String::new();
        result.push_str(&normalized_existing[..begin_idx]);
        result.push_str(managed);
        if end_exclusive < normalized_existing.len() {
            let remainder = normalized_existing[end_exclusive..].trim_start_matches('\n');
            if !remainder.is_empty() {
                result.push_str(remainder);
                if !result.ends_with('\n') {
                    result.push('\n');
                }
            }
        }
        return result;
    }

    if normalized_existing.is_empty() {
        managed.to_string()
    } else {
        format!("{normalized_existing}\n{managed}")
    }
}

fn ensure_ayni_gitignore_entry(path: &Path) -> Result<(), String> {
    let mut content = if path.exists() {
        fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
    } else {
        String::new()
    };
    let present = content
        .lines()
        .map(str::trim)
        .any(|line| line == ".ayni/" || line == ".ayni");
    if !present {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(".ayni/\n");
        fs::write(path, content)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(())
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

fn persist_artifact(repo_root: &Path, artifact: &RunArtifact) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("failed to serialize artifact: {error}"))?;
    fs::write(repo_root.join(SIGNALS_ARTIFACT), format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {SIGNALS_ARTIFACT}: {error}"))?;
    fs::write(
        repo_root.join(PREVIOUS_SIGNALS_SNAPSHOT),
        format!("{serialized}\n"),
    )
    .map_err(|error| {
        format!("failed to write previous signals snapshot {PREVIOUS_SIGNALS_SNAPSHOT}: {error}")
    })?;
    Ok(())
}

#[cfg(test)]
mod tests;
