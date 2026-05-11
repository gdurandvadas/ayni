use crate::discovery::discover_language_roots;
use crate::signal_kind_slug;
use crate::{
    AGENTS_MANAGED_BEGIN, AGENTS_MANAGED_END, AYNI_POLICY_FILE, GO_POLICY_TEMPLATE,
    KOTLIN_POLICY_TEMPLATE, NODE_POLICY_TEMPLATE, PYTHON_POLICY_TEMPLATE, RUST_POLICY_TEMPLATE,
};
use ayni_core::{
    AyniPolicy, CatalogEntry, ExecutionResolution, InstallContext, Installer, Language,
    NodePackageManager, PythonPackageManager, RunArtifact, SignalKind, ToolStatus, VersionCheck,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn install_impl(
    repo_root: &str,
    language_filter: Option<Language>,
    apply: bool,
) -> Result<(), Vec<String>> {
    let root = PathBuf::from(repo_root);
    let policy = prepare_install_policy(&root, language_filter).map_err(|error| vec![error])?;
    if apply {
        let mut failures = collect_install_failures(&root, &policy, language_filter);
        if failures.is_empty() {
            failures.extend(validate_install_foundation(&root, &policy, language_filter));
        }
        if failures.is_empty() {
            println!("foundation validation passed");
            Ok(())
        } else {
            Err(failures)
        }
    } else {
        print_install_requirements(&root, &policy, language_filter);
        Ok(())
    }
}

pub(crate) fn print_install_requirements(
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
    let registry = crate::build_registry();
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
            let Some(execution) = adapter.resolve_execution(repo_root, &root_path) else {
                continue;
            };
            let install_context = install_context_for_execution(&execution);
            println!(
                "  runner: runner={} source={} kind={} resolved_from={} confidence={} ambiguous={}",
                execution.runner,
                execution.source,
                execution.kind,
                execution.resolved_from.display(),
                execution.confidence,
                execution.ambiguous
            );
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
        Installer::GradleTask { task } => format!("install: provided by Gradle task `{task}`"),
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
    let registry = crate::build_registry();
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
    let Some(execution) = adapter.resolve_execution(root, &root_path) else {
        failures.push(format!(
            "install {language}:{root_entry}: repo setup issue: unable to resolve execution"
        ));
        return failures;
    };
    prepare_node_manager(language, root_entry, &execution, &mut failures);
    prepare_kotlin_gradle_plugins(language, root_entry, &execution, &mut failures);
    println!(
        "install {language}:{root_entry} runner={} source={} kind={} resolved_from={} confidence={} ambiguous={}",
        execution.runner,
        execution.source,
        execution.kind,
        execution.resolved_from.display(),
        execution.confidence,
        execution.ambiguous
    );
    let install_context = install_context_for_execution(&execution);

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
    execution: &ExecutionResolution,
    failures: &mut Vec<String>,
) {
    if language != Language::Node {
        return;
    }

    let manager =
        NodePackageManager::from_executable(&execution.runner).unwrap_or(NodePackageManager::Npm);
    if let Err(error) = install_node_dependencies(&execution.install_cwd, manager) {
        failures.push(format!("node install ({language}:{root_entry}): {error}"));
    }
}

fn prepare_kotlin_gradle_plugins(
    language: Language,
    root_entry: &str,
    execution: &ExecutionResolution,
    failures: &mut Vec<String>,
) {
    if language != Language::Kotlin {
        return;
    }
    if let Err(error) = ayni_adapters_kotlin::install::ensure_gradle_plugins(&execution.install_cwd)
    {
        failures.push(format!("kotlin install ({language}:{root_entry}): {error}"));
    }
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

fn install_context_for_execution(execution: &ExecutionResolution) -> InstallContext<'_> {
    InstallContext {
        cwd: Some(execution.install_cwd.as_path()),
        node_package_manager: NodePackageManager::from_executable(&execution.runner),
        python_package_manager: PythonPackageManager::from_executable(&execution.runner),
        gradle_runner: Some(execution.runner.as_str()),
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
    let scaffold = scaffold_files(root, language_filter)?;
    let policy = AyniPolicy::load(root)?;
    if scaffold.policy_created {
        let enabled_languages = policy.enabled_languages()?;
        let registry = crate::build_registry();
        let discovered_roots =
            discover_language_roots(root, &enabled_languages, language_filter, &registry);
        update_policy_roots(root, &discovered_roots)?;
        update_foundation_settings(root, &discovered_roots)?;
    }
    AyniPolicy::load(root)
}

pub(crate) fn validate_install_foundation(
    repo_root: &Path,
    policy: &AyniPolicy,
    language_filter: Option<Language>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let registry = crate::build_registry();
    for adapter in registry.adapters() {
        let language = adapter.language();
        if should_skip_install_language(policy, language, language_filter)
            || policy
                .language_tooling(language)
                .foundation
                .as_ref()
                .and_then(|value| value.validate_install)
                == Some(false)
        {
            continue;
        }
        for root_entry in policy.roots_for(language) {
            let root_path = repo_root.join(root_entry);
            if !adapter.detect(&root_path).detected {
                continue;
            }
            let Some(execution) = adapter.resolve_execution(repo_root, &root_path) else {
                failures.push(format!(
                    "foundation validation failed ({language}:{root_entry}): repo_setup_issue: unable to resolve execution for {}",
                    root_path.display()
                ));
                continue;
            };
            let artifact_dir = repo_root
                .join(".ayni")
                .join("work")
                .join(language.as_str())
                .join(root_slug_for_path(root_entry));
            if let Err(error) = fs::create_dir_all(&artifact_dir) {
                failures.push(format!(
                    "foundation validation failed ({language}:{root_entry}): repo_setup_issue: unable to create artifact path {}: {error}; runner={} source={} kind={} resolved_from={}",
                    artifact_dir.display(),
                    execution.runner,
                    execution.source,
                    execution.kind,
                    execution.resolved_from.display()
                ));
            }
            let install_context = install_context_for_execution(&execution);
            for entry in adapter.catalog() {
                if entry.opt_in && !check_enabled_for_entry(policy, entry) {
                    continue;
                }
                if entry.status_in(install_context) == ToolStatus::Missing {
                    failures.push(format!(
                        "foundation validation failed ({language}:{root_entry}): repo_setup_issue: {} is not invocable; runner={} source={} kind={} resolved_from={} install_cwd={} exec_cwd={}",
                        entry.name,
                        execution.runner,
                        execution.source,
                        execution.kind,
                        execution.resolved_from.display(),
                        execution.install_cwd.display(),
                        execution.exec_cwd.display()
                    ));
                }
            }
        }
    }
    failures
}

fn root_slug_for_path(root_entry: &str) -> String {
    if root_entry == "." {
        String::from("workspace")
    } else {
        root_entry.replace(['/', '\\'], "__")
    }
}

#[derive(Debug, Clone, Copy)]
struct ScaffoldOutcome {
    policy_created: bool,
}

fn scaffold_files(
    repo_root: &Path,
    language_filter: Option<Language>,
) -> Result<ScaffoldOutcome, String> {
    let policy_path = repo_root.join(".ayni.toml");
    let policy_created = !policy_path.exists();
    if policy_created {
        fs::write(&policy_path, default_policy_toml(language_filter))
            .map_err(|error| format!("failed to create {}: {error}", policy_path.display()))?;
    }
    ensure_ayni_gitignore_entry(&repo_root.join(".gitignore"))?;
    ensure_agents_managed_section(repo_root)?;
    Ok(ScaffoldOutcome { policy_created })
}

pub(crate) fn default_policy_toml(language_filter: Option<Language>) -> String {
    let language = language_filter.unwrap_or(Language::Rust);
    match language {
        Language::Rust => RUST_POLICY_TEMPLATE,
        Language::Go => GO_POLICY_TEMPLATE,
        Language::Node => NODE_POLICY_TEMPLATE,
        Language::Python => PYTHON_POLICY_TEMPLATE,
        Language::Kotlin => KOTLIN_POLICY_TEMPLATE,
    }
    .to_string()
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

fn update_foundation_settings(
    repo_root: &Path,
    discovered_roots: &BTreeMap<Language, Vec<String>>,
) -> Result<(), String> {
    let registry = crate::build_registry();
    let mut languages_requiring_workspace_runner = BTreeSet::new();
    for adapter in registry.adapters() {
        let language = adapter.language();
        let Some(roots) = discovered_roots.get(&language) else {
            continue;
        };
        if roots
            .iter()
            .filter(|root| root.as_str() != ".")
            .any(|root| {
                adapter
                    .resolve_execution(repo_root, &repo_root.join(root))
                    .is_some_and(|value| value.kind == "workspace_ancestor")
            })
        {
            languages_requiring_workspace_runner.insert(language);
        }
    }
    if languages_requiring_workspace_runner.is_empty() {
        return Ok(());
    }
    let policy_path = repo_root.join(AYNI_POLICY_FILE);
    let content = fs::read_to_string(&policy_path)
        .map_err(|error| format!("failed to read {}: {error}", policy_path.display()))?;
    let mut document = content.parse::<toml::Value>().map_err(|error| {
        format!(
            "failed to parse {} as toml for foundation updates: {error}",
            policy_path.display()
        )
    })?;
    let Some(table) = document.as_table_mut() else {
        return Err(format!("{} is not a TOML table", policy_path.display()));
    };
    for language in languages_requiring_workspace_runner {
        let key = language.as_str();
        if !table.contains_key(key) {
            table.insert(key.to_string(), toml::Value::Table(toml::Table::new()));
        }
        let language_table = table
            .get_mut(key)
            .and_then(toml::Value::as_table_mut)
            .ok_or_else(|| format!("[{key}] must be a table in {}", policy_path.display()))?;
        let foundation = language_table
            .entry("foundation")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let foundation_table = foundation.as_table_mut().ok_or_else(|| {
            format!(
                "[{key}.foundation] must be a table in {}",
                policy_path.display()
            )
        })?;
        foundation_table.insert(
            String::from("runner"),
            toml::Value::String(String::from("workspace")),
        );
        foundation_table.insert(String::from("validate_install"), toml::Value::Boolean(true));
    }
    let serialized = toml::to_string_pretty(&document)
        .map_err(|error| format!("failed to serialize {}: {error}", policy_path.display()))?;
    fs::write(&policy_path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", policy_path.display()))
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

pub(crate) fn upsert_managed_block(existing: &str, managed: &str) -> String {
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

pub(crate) fn enabled_signal_kinds(policy: &AyniPolicy) -> Vec<SignalKind> {
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

pub(crate) fn persist_artifact(repo_root: &Path, artifact: &RunArtifact) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("failed to serialize artifact: {error}"))?;
    fs::write(
        repo_root.join(crate::SIGNALS_ARTIFACT),
        format!("{serialized}\n"),
    )
    .map_err(|error| format!("failed to write {}: {error}", crate::SIGNALS_ARTIFACT))?;
    fs::write(
        repo_root.join(crate::PREVIOUS_SIGNALS_SNAPSHOT),
        format!("{serialized}\n"),
    )
    .map_err(|error| {
        format!(
            "failed to write previous signals snapshot {}: {error}",
            crate::PREVIOUS_SIGNALS_SNAPSHOT
        )
    })?;
    Ok(())
}
