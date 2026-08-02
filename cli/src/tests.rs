use super::{
    AgentsCommands, Cli, Commands, InstallOutputArg, LanguageArg, OutputArg, SIGNALS_ARTIFACT,
    VERIFY_SIGNALS_ARTIFACT, VerifyCommands, persist_artifact_at, resolve_output_mode,
    selected_install_languages, serialize_artifact,
};

#[test]
fn verify_test_cli_parses_focused_node_selectors() {
    let cli = Cli::try_parse_from([
        "ayni",
        "verify",
        "test",
        "--language",
        "node",
        "--package",
        "@guita/web",
        "--file",
        "frontend/apps/web/src/lib/money.test.ts",
        "--name",
        "formats money",
        "--json",
    ])
    .expect("arguments parse");
    let Commands::Verify {
        command: VerifyCommands::Test { options, name },
    } = cli.command
    else {
        panic!("verify test command");
    };
    assert_eq!(options.package.as_deref(), Some("@guita/web"));
    assert_eq!(
        options.file.as_deref(),
        Some("frontend/apps/web/src/lib/money.test.ts")
    );
    assert_eq!(name.as_deref(), Some("formats money"));
    assert!(options.common.json);
}

#[test]
fn verify_exposes_every_canonical_signal_and_name_is_test_only() {
    for signal in ["test", "coverage", "size", "complexity", "deps", "mutation"] {
        Cli::try_parse_from(["ayni", "verify", signal, "--language", "rust"])
            .unwrap_or_else(|error| panic!("verify {signal} should parse: {error}"));
    }

    let error = Cli::try_parse_from(["ayni", "verify", "size", "--name", "unit"])
        .expect_err("--name must remain test-only");
    assert!(error.to_string().contains("unexpected argument"));
    for (signal, selector) in [
        ("coverage", "--file"),
        ("size", "--package"),
        ("mutation", "--file"),
    ] {
        let error = Cli::try_parse_from(["ayni", "verify", signal, selector, "target"])
            .expect_err("signal-invalid selector must be rejected by the CLI");
        assert!(error.to_string().contains("unexpected argument"));
    }
}

#[test]
fn verify_artifact_does_not_replace_analyze_artifact() {
    assert_ne!(VERIFY_SIGNALS_ARTIFACT, SIGNALS_ARTIFACT);
    assert_eq!(VERIFY_SIGNALS_ARTIFACT, ".ayni/verify/last/signals.json");
}

#[test]
fn completion_planner_represents_an_undetected_configured_root() {
    let dir = TempDir::new().expect("tempdir");
    let policy: AyniPolicy = toml::from_str(
        r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["missing"]
"#,
    )
    .expect("policy");

    let planning =
        super::build_analyze_targets(dir.path(), &policy, None, None, Some(Language::Rust), false)
            .expect("planning");

    assert_eq!(planning.expected_targets, 1);
    assert_eq!(planning.detected_targets, 0);
    assert!(planning.targets.is_empty());
    assert_eq!(planning.issues.len(), 1);
    assert_eq!(planning.issues[0].configured_root, "missing");
    assert_eq!(
        planning.issues[0].stage,
        ayni_core::CompletionStage::Detection
    );
}

#[test]
fn completion_artifact_writer_atomically_replaces_existing_evidence() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join(SIGNALS_ARTIFACT);
    fs::create_dir_all(path.parent().expect("parent")).expect("artifact directory");
    fs::write(&path, "stale\n").expect("stale artifact");

    persist_artifact_at(dir.path(), SIGNALS_ARTIFACT, "current\n").expect("persist");

    assert_eq!(fs::read_to_string(path).expect("artifact"), "current\n");
}
use crate::agents::{MANAGED_BEGIN, MANAGED_END, managed_block, sync_impl, upsert_managed_block};
use crate::install::{
    catalog_entry_enabled_for_policy, default_policy_toml, install_impl,
    validate_install_foundation,
};
use ayni_core::{
    AYNI_SIGNAL_SCHEMA_VERSION, AyniPolicy, Budget, CatalogEntry, ExecutionResolution, Installer,
    InvocationContext, Language, Offenders, OutputContext, RunArtifact, RunArtifactMetadata,
    RunContext, Scope, SignalKind, SignalResult, TestResult, VersionCheck,
};
use clap::Parser;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn agents_sync_creates_managed_file_when_absent() {
    let dir = TempDir::new().expect("tempdir");

    sync_impl(&dir.path().to_string_lossy()).expect("sync");

    assert_eq!(
        fs::read_to_string(dir.path().join("AGENTS.md")).expect("agents"),
        managed_block()
    );
}

#[test]
fn agents_managed_guidance_describes_discovery_policy_and_quality_workflow() {
    let managed = managed_block();

    for guidance in [
        "`ayni help`",
        "`ayni help <command> [subcommand]`",
        "`ayni <command> --help`",
        "`.ayni.toml` as the authoritative repository quality policy",
        "`ayni contract display`",
        "ayni verify <signal> [selectors]",
        "full repository analysis as the completion gate",
        "`.ayni/last/signals.json`",
        "narrowest supported `ayni verify <signal>`",
        "exact verification command supplied by a finding",
        "incomplete artifacts as failure",
        "never loosen `.ayni.toml` merely",
        "detailed, typed signal results",
        "completion state and target accounting",
        "exact verification command",
    ] {
        assert!(managed.contains(guidance), "missing guidance: {guidance}");
    }
    assert!(!managed.contains("ayni <command> help"));
    assert!(!managed.contains("schema-v2"));
    assert!(!managed.contains("deltas"));
    assert!(!managed.contains("and repair the listed offenders and\nand repair"));
}

#[test]
fn agents_sync_replaces_only_managed_section_and_preserves_user_content() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("AGENTS.md");
    fs::write(
        &path,
        format!("head\n\n{MANAGED_BEGIN}\nold\n{MANAGED_END}\n\ntail\n"),
    )
    .expect("agents");

    sync_impl(&dir.path().to_string_lossy()).expect("sync");

    let updated = fs::read_to_string(path).expect("agents");
    assert!(updated.starts_with("head\n\n"));
    assert!(updated.ends_with("tail\n"));
    assert!(updated.contains("## Code quality guidance for AI agents"));
    assert!(!updated.contains("\nold\n"));
}

#[test]
fn agents_sync_is_idempotent() {
    let dir = TempDir::new().expect("tempdir");
    sync_impl(&dir.path().to_string_lossy()).expect("first sync");
    let once = fs::read_to_string(dir.path().join("AGENTS.md")).expect("agents");
    sync_impl(&dir.path().to_string_lossy()).expect("second sync");
    let twice = fs::read_to_string(dir.path().join("AGENTS.md")).expect("agents");

    assert_eq!(once, twice);
}

#[test]
fn upsert_managed_block_appends_when_missing() {
    let existing = "# Repository Rules\n\nKeep this text.\n";
    let updated = upsert_managed_block(existing, &managed_block());
    assert!(updated.contains("Keep this text."));
    assert!(updated.contains(MANAGED_BEGIN));
    assert!(updated.contains(MANAGED_END));
}

#[test]
fn language_arg_accepts_python() {
    assert_eq!(LanguageArg::Python.as_language(), Language::Python);
    assert_eq!(LanguageArg::Kotlin.as_language(), Language::Kotlin);
}

#[test]
fn install_parser_accepts_repeated_languages_and_deduplicates_them() {
    let cli = Cli::try_parse_from([
        "ayni",
        "install",
        "--language",
        "python",
        "--language",
        "rust",
        "--language",
        "python",
    ])
    .expect("arguments parse");
    let Commands::Install { language, .. } = cli.command else {
        panic!("install command");
    };

    assert_eq!(
        selected_install_languages(language),
        BTreeSet::from([Language::Rust, Language::Python])
    );
}

#[test]
fn install_parser_preserves_single_language_selection() {
    let cli =
        Cli::try_parse_from(["ayni", "install", "--language", "kotlin"]).expect("arguments parse");
    let Commands::Install { language, .. } = cli.command else {
        panic!("install command");
    };

    assert_eq!(
        selected_install_languages(language),
        BTreeSet::from([Language::Kotlin])
    );
}

#[test]
fn install_parser_defaults_to_no_language_selection() {
    let cli = Cli::try_parse_from(["ayni", "install"]).expect("arguments parse");
    let Commands::Install { language, .. } = cli.command else {
        panic!("install command");
    };

    assert!(selected_install_languages(language).is_empty());
}

#[test]
fn install_check_parses_json_and_conflicts_with_apply() {
    let cli = Cli::try_parse_from(["ayni", "install", "--check", "--output", "json"])
        .expect("check arguments parse");
    let Commands::Install { check, output, .. } = cli.command else {
        panic!("install command");
    };
    assert!(check);
    assert_eq!(output, Some(InstallOutputArg::Json));

    let conflict = Cli::try_parse_from(["ayni", "install", "--check", "--apply"])
        .expect_err("check and apply conflict");
    assert!(conflict.to_string().contains("cannot be used with"));

    let without_check = Cli::try_parse_from(["ayni", "install", "--output", "json"])
        .expect_err("install JSON is check-only");
    assert!(without_check.to_string().contains("required arguments"));
    assert!(without_check.to_string().contains("--check"));
}

#[test]
fn agents_sync_parser_accepts_repo_root() {
    let cli = Cli::try_parse_from(["ayni", "agents", "sync", "--repo-root", "fixture"])
        .expect("arguments parse");
    let Commands::Agents {
        command: AgentsCommands::Sync { repo_root },
    } = cli.command
    else {
        panic!("agents sync command");
    };

    assert_eq!(repo_root, "fixture");
}

#[test]
fn analyze_json_selector_is_equivalent_to_output_json() {
    let short = Cli::try_parse_from(["ayni", "analyze", "--json"]).expect("arguments parse");
    let long =
        Cli::try_parse_from(["ayni", "analyze", "--output", "json"]).expect("arguments parse");
    let Commands::Analyze {
        output: short_output,
        json: short_json,
        ..
    } = short.command
    else {
        panic!("analyze command");
    };
    let Commands::Analyze {
        output: long_output,
        json: long_json,
        ..
    } = long.command
    else {
        panic!("analyze command");
    };

    assert_eq!(
        resolve_output_mode(short_output, short_json).expect("short selector"),
        resolve_output_mode(long_output, long_json).expect("long selector")
    );
}

#[test]
fn analyze_rejects_focused_scope_selectors() {
    for selector in ["--file", "--package", "--language"] {
        let error = Cli::try_parse_from(["ayni", "analyze", selector, "value"])
            .expect_err("analyze must be repository-only");
        assert!(
            error.to_string().contains("unexpected argument"),
            "unexpected error for {selector}: {error}"
        );
    }
}

#[test]
fn analyze_json_allows_same_output_mode_and_rejects_conflicts() {
    assert_eq!(
        resolve_output_mode(Some(OutputArg::Json), true).expect("same mode is allowed"),
        OutputArg::Json
    );
    assert_eq!(
        resolve_output_mode(None, false).expect("default output"),
        OutputArg::Stdout
    );
    assert_eq!(
        resolve_output_mode(Some(OutputArg::Md), true).expect_err("conflicting output"),
        "--json cannot be combined with --output md; use --output json or --json"
    );
}

#[test]
fn serialized_json_is_schema_v3_and_matches_persisted_artifact() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join(".ayni/last")).expect("artifact directory");
    let artifact = RunArtifact::new(
        RunArtifactMetadata {
            generated_at: String::from("2026-07-12T00:00:00Z"),
            ayni_version: String::from("0.4.2"),
            invocation: InvocationContext {
                command: String::from("analyze"),
                languages: vec![Language::Rust],
                scope: None,
            },
            output: OutputContext {
                format: String::from("json"),
                destination: String::from("stdout"),
            },
            config_path: String::from("./.ayni.toml"),
            repository_root: String::from("."),
        },
        ayni_core::RunCompletion::complete(ayni_core::CompletionScope::Repository, 1),
        vec![test_row(true, 1, 0)],
    );
    let serialized = serialize_artifact(&artifact).expect("serialize artifact");
    persist_artifact_at(dir.path(), SIGNALS_ARTIFACT, &serialized).expect("persist artifact");

    let value: serde_json::Value = serde_json::from_str(&serialized).expect("valid json");
    assert_eq!(value["schema_version"], AYNI_SIGNAL_SCHEMA_VERSION);
    assert_eq!(value["generated_at"], "2026-07-12T00:00:00Z");
    assert_eq!(value["output"]["format"], "json");
    assert!(value.get("aggregate").is_some());
    assert!(value.get("applied_thresholds").is_some());
    assert_eq!(
        fs::read_to_string(dir.path().join(".ayni/last/signals.json")).expect("artifact"),
        serialized
    );
}

#[test]
fn default_policy_templates_are_valid_for_each_language() {
    for language in [
        Language::Rust,
        Language::Go,
        Language::Node,
        Language::Python,
        Language::Kotlin,
    ] {
        let policy: ayni_core::AyniPolicy =
            toml::from_str(&default_policy_toml(&BTreeSet::from([language]))).expect("policy");

        assert_eq!(policy.enabled_languages().expect("languages"), [language]);
        assert_eq!(policy.roots_for(language), ["."]);
        assert!(!policy.size_rules_for(language).is_empty());
    }
}

#[test]
fn default_policy_template_falls_back_to_rust() {
    let policy: ayni_core::AyniPolicy =
        toml::from_str(&default_policy_toml(&BTreeSet::new())).expect("policy");

    assert_eq!(
        policy.enabled_languages().expect("languages"),
        [Language::Rust]
    );
    assert_eq!(policy.roots_for(Language::Rust), ["."]);
}

#[test]
fn default_policy_template_includes_every_selected_language() {
    let selected = BTreeSet::from([
        Language::Rust,
        Language::Go,
        Language::Node,
        Language::Python,
        Language::Kotlin,
    ]);
    let policy: AyniPolicy = toml::from_str(&default_policy_toml(&selected)).expect("policy");

    assert_eq!(
        policy.enabled_languages().expect("languages"),
        selected.into_iter().collect::<Vec<_>>()
    );
    for language in [
        Language::Rust,
        Language::Go,
        Language::Node,
        Language::Python,
        Language::Kotlin,
    ] {
        assert_eq!(policy.roots_for(language), ["."]);
        assert!(!policy.size_rules_for(language).is_empty());
    }
}

#[test]
fn install_bootstraps_policy_for_every_selected_language() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cargo manifest");
    fs::write(dir.path().join("package.json"), "{\"name\": \"fixture\"}\n")
        .expect("package manifest");
    let selected = BTreeSet::from([Language::Rust, Language::Node]);

    install_impl(&dir.path().to_string_lossy(), &selected, false).expect("install");
    let policy = AyniPolicy::load(dir.path()).expect("policy");

    assert_eq!(
        policy.enabled_languages().expect("languages"),
        selected.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(policy.roots_for(Language::Rust), ["."]);
    assert_eq!(policy.roots_for(Language::Node), ["."]);
}

#[test]
fn install_does_not_create_or_modify_agents_file() {
    let absent = TempDir::new().expect("tempdir");
    install_impl(&absent.path().to_string_lossy(), &BTreeSet::new(), false).expect("install");
    assert!(!absent.path().join("AGENTS.md").exists());

    let existing = TempDir::new().expect("tempdir");
    let path = existing.path().join("AGENTS.md");
    let original = String::from("# User instructions\n\nDo not change this.\n");
    fs::write(&path, &original).expect("agents");
    install_impl(&existing.path().to_string_lossy(), &BTreeSet::new(), false).expect("install");
    assert_eq!(fs::read_to_string(path).expect("agents"), original);
}

#[test]
fn kotlin_analyze_targets_are_built_when_enabled() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("build.gradle.kts"), "plugins {}\n").expect("gradle build");
    let policy: ayni_core::AyniPolicy = toml::from_str(
        r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["kotlin"]

[kotlin]
roots = ["."]

[kotlin.size]
"**/*.kt" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");

    let planning = super::build_analyze_targets(
        dir.path(),
        &policy,
        None,
        None,
        Some(Language::Kotlin),
        false,
    )
    .expect("targets");

    assert_eq!(planning.targets.len(), 1);
    assert_eq!(planning.targets[0].language, Language::Kotlin);
    assert_eq!(planning.targets[0].run_context.execution.runner, "gradle");
}

#[test]
fn python_analyze_targets_are_built_when_enabled() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("pyproject.toml"), "").expect("pyproject");
    let policy: ayni_core::AyniPolicy = toml::from_str(
        r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["python"]

[python]
roots = ["."]

[python.size]
"**/*.py" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");

    let planning = super::build_analyze_targets(
        dir.path(),
        &policy,
        None,
        None,
        Some(Language::Python),
        false,
    )
    .expect("targets");
    assert_eq!(planning.targets.len(), 1);
    assert_eq!(planning.targets[0].language, Language::Python);
    assert_eq!(planning.targets[0].run_context.execution.runner, "python");
    assert_eq!(
        planning.targets[0].run_context.execution.kind,
        "direct_root"
    );
}

#[test]
fn foundation_validation_creates_generic_artifact_work_dir() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("cargo manifest");
    let policy: ayni_core::AyniPolicy = toml::from_str(
        r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["."]

[rust.size]
"*.rs" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");

    let failures =
        validate_install_foundation(dir.path(), &policy, &BTreeSet::from([Language::Rust]));

    assert!(failures.is_empty());
    assert!(dir.path().join(".ayni/work/rust/workspace").is_dir());
}

#[test]
fn disabled_catalog_entries_are_not_required_for_install_validation() {
    let policy: ayni_core::AyniPolicy = toml::from_str(
        r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");
    let entry = CatalogEntry {
        name: "rust-code-analysis-cli",
        check: Some(VersionCheck {
            command: "rust-code-analysis-cli",
            args: &["--version"],
            contains: None,
        }),
        installer: Installer::Cargo {
            crate_name: "rust-code-analysis-cli",
            version: None,
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    };

    assert!(!catalog_entry_enabled_for_policy(&policy, &entry));
}

#[test]
fn catalog_entry_is_eligible_for_any_enabled_signal() {
    let cases = [
        ("test-only", &[SignalKind::Test][..], true, false, true),
        (
            "coverage-only",
            &[SignalKind::Coverage][..],
            false,
            true,
            true,
        ),
        (
            "test-and-coverage-test-enabled",
            &[SignalKind::Test, SignalKind::Coverage][..],
            true,
            false,
            true,
        ),
        (
            "test-and-coverage-both-disabled",
            &[SignalKind::Test, SignalKind::Coverage][..],
            false,
            false,
            false,
        ),
        ("unassociated", &[][..], false, false, true),
    ];

    for (name, signals, test, coverage, expected) in cases {
        let policy: ayni_core::AyniPolicy = toml::from_str(&format!(
            "[checks]\ntest = {test}\ncoverage = {coverage}\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n"
        ))
        .expect("policy");
        let entry = CatalogEntry {
            name,
            check: None,
            installer: Installer::Bundled,
            for_signals: signals,
            opt_in: false,
        };
        assert_eq!(
            catalog_entry_enabled_for_policy(&policy, &entry),
            expected,
            "{name}"
        );
    }
}

#[test]
fn install_new_python_policy_uses_member_roots_for_uv_workspace() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("pyproject.toml"),
        r#"[tool.uv.workspace]
members = ["packages/*", "services/*"]
exclude = ["services/agent-runtime"]
"#,
    )
    .expect("root pyproject");
    fs::create_dir_all(dir.path().join("packages/config")).expect("config dir");
    fs::write(
        dir.path().join("packages/config/pyproject.toml"),
        "[project]\nname='config'\n",
    )
    .expect("config pyproject");
    fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
    fs::write(
        dir.path().join("services/api/pyproject.toml"),
        "[project]\nname='api'\n",
    )
    .expect("api pyproject");
    fs::create_dir_all(dir.path().join("services/agent-runtime")).expect("runtime dir");
    fs::write(
        dir.path().join("services/agent-runtime/pyproject.toml"),
        "[project]\nname='runtime'\n",
    )
    .expect("runtime pyproject");

    install_impl(
        &dir.path().to_string_lossy(),
        &BTreeSet::from([Language::Python]),
        false,
    )
    .expect("install");
    let policy = ayni_core::AyniPolicy::load(dir.path()).expect("policy");

    assert_eq!(
        policy.roots_for(Language::Python),
        ["packages/config", "services/api"]
    );
}

#[test]
fn install_existing_policy_does_not_rewrite_roots() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join(".ayni.toml"),
        r#"[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["python"]

[python]
roots = ["."]

[python.size]
"**/*.py" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");
    fs::write(
        dir.path().join("pyproject.toml"),
        r#"[tool.uv.workspace]
members = ["packages/*"]
"#,
    )
    .expect("root pyproject");
    fs::create_dir_all(dir.path().join("packages/config")).expect("config dir");
    fs::write(
        dir.path().join("packages/config/pyproject.toml"),
        "[project]\nname='config'\n",
    )
    .expect("config pyproject");

    install_impl(
        &dir.path().to_string_lossy(),
        &BTreeSet::from([Language::Python]),
        false,
    )
    .expect("install");
    let policy = ayni_core::AyniPolicy::load(dir.path()).expect("policy");

    assert_eq!(policy.roots_for(Language::Python), ["."]);
}

#[test]
fn collector_errors_are_preserved_as_failed_rows() {
    let policy: AyniPolicy = toml::from_str(
        r#"
[checks]
test = true
coverage = true
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["python"]
"#,
    )
    .expect("policy");
    let context = RunContext {
        repo_root: PathBuf::from("/repo"),
        target_root: PathBuf::from("/repo/packages/api"),
        workdir: PathBuf::from("/repo/packages/api"),
        policy,
        scope: Scope {
            workspace_root: String::from("/repo"),
            path: Some(String::from("packages/api")),
            package: None,
            file: None,
        },
        execution: ExecutionResolution::direct(
            "python",
            PathBuf::from("/repo/packages/api"),
            "test",
            100,
        ),
        debug: false,
    };

    let row = super::failed_signal_row(
        Language::Python,
        SignalKind::Coverage,
        &context,
        String::from("pytest-cov missing"),
    );

    assert!(!row.pass);
    assert_eq!(row.kind, SignalKind::Coverage);
    assert_eq!(row.scope.path.as_deref(), Some("packages/api"));
    match row.result {
        SignalResult::Coverage(result) => {
            assert_eq!(result.status, "error");
            let failure = result.failure.expect("failure");
            assert_eq!(failure.classification, "adapter_error");
            assert_eq!(failure.message, "pytest-cov missing");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

fn test_row(pass: bool, passed: u64, failed: u64) -> ayni_core::SignalRow {
    ayni_core::SignalRow {
        kind: SignalKind::Test,
        language: Language::Rust,
        scope: Scope::default(),
        pass,
        result: SignalResult::Test(TestResult {
            total_tests: passed + failed,
            passed,
            failed,
            duration_ms: Some(400),
            runner: String::from("cargo-test"),
            failure: None,
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(Vec::new()),
    }
}
