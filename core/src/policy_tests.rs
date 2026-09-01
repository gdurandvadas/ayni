use super::*;
use crate::adapter::{ComplexityThresholdKind, PolicyEffectivenessFacts};
use crate::language::Language;

#[test]
fn empty_rust_table_parses() {
    let document = r#"
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
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    assert!(policy.rust.complexity.is_none());
    assert!(policy.rust.size.is_empty());
}

#[test]
fn rust_size_map_parses() {
    let document = r#"
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
"*.rs" = { warn = 400, fail = 700 }
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    let size = policy.size_rules_for(Language::Rust);
    let rule = size.get("*.rs").expect("*.rs rule");
    assert_eq!(rule.warn, 400);
    assert_eq!(rule.fail, 700);
}

#[test]
fn rust_complexity_parses() {
    let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = true
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.complexity]
fn_cyclomatic = { warn = 10.0, fail = 20.0 }
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    let c = policy
        .rust
        .complexity
        .as_ref()
        .expect("complexity")
        .fn_cyclomatic
        .expect("cyclomatic");
    assert_eq!(c.warn, 10.0);
    assert_eq!(c.fail, 20.0);
}

#[test]
fn language_tooling_overrides_parse() {
    let document = r#"
[checks]
test = true
coverage = true
size = false
complexity = false
deps = false
mutation = true

[languages]
enabled = ["rust", "go", "node"]

[rust.tooling.test]
command = "cargo"
args = ["nextest", "run"]

[go.tooling.coverage]
command = "gotestsum"
args = ["--", "./..."]

[node.tooling.mutation]
command = "pnpm"
args = ["exec", "stryker", "run"]
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    let rust_test = policy
        .tool_override_for(Language::Rust, SignalKind::Test)
        .expect("rust test override");
    assert_eq!(rust_test.command, "cargo");
    assert_eq!(rust_test.args, vec!["nextest", "run"]);

    let go_coverage = policy
        .tool_override_for(Language::Go, SignalKind::Coverage)
        .expect("go coverage override");
    assert_eq!(go_coverage.command, "gotestsum");

    let node_mutation = policy
        .tool_override_for(Language::Node, SignalKind::Mutation)
        .expect("node mutation override");
    assert_eq!(node_mutation.command, "pnpm");
}

#[test]
fn repository_environment_tools_packages_and_docker_capabilities_parse() {
    let document = r#"
[languages]
enabled = ["rust"]

[environment.tools]
protoc = "35.1"
"cargo:cargo-nextest" = "0.9.100"

[environment.debian]
packages = ["libssl-dev", "protobuf-compiler=3.21.12+ABC-3"]

[environment.docker]
access = "socket"
network = "bridge"

[environment.resources]
cpus = 6
memory_mib = 12288
memory_swap_mib = 16384
pids = 4096
nofile = 16384
"#;
    let policy = AyniPolicy::parse(document).expect("policy");
    assert_eq!(
        policy.environment_tools().get("protoc"),
        Some(&String::from("35.1"))
    );
    assert_eq!(
        policy.environment_debian_packages(),
        ["libssl-dev", "protobuf-compiler=3.21.12+ABC-3"]
    );
    assert_eq!(
        policy.environment_capabilities().docker,
        DockerAccess::Socket
    );
    assert_eq!(
        policy.environment_capabilities().network,
        NetworkAccess::Bridge
    );
    assert_eq!(policy.environment_resource_limits().cpus, 6);
    assert_eq!(policy.environment_resource_limits().memory_mib, 12_288);
    assert_eq!(policy.environment_resource_limits().memory_swap_mib, 16_384);
    assert_eq!(policy.environment_resource_limits().pids, 4_096);
    assert_eq!(policy.environment_resource_limits().nofile, 16_384);
}

#[test]
fn environment_resource_defaults_are_bounded_and_invalid_values_fail() {
    let policy = AyniPolicy::parse("[languages]\nenabled = [\"rust\"]\n").expect("policy");
    assert_eq!(policy.environment_resource_limits().cpus, 4);
    assert_eq!(policy.environment_resource_limits().memory_mib, 8_192);
    assert_eq!(policy.environment_resource_limits().memory_swap_mib, 8_192);
    assert_eq!(policy.environment_resource_limits().pids, 2_048);
    assert_eq!(policy.environment_resource_limits().nofile, 8_192);

    let memory_only = AyniPolicy::parse(
        "[languages]\nenabled = [\"rust\"]\n[environment.resources]\nmemory_mib = 4096\n",
    )
    .expect("memory-only override");
    assert_eq!(memory_only.environment_resource_limits().memory_mib, 4_096);
    assert_eq!(
        memory_only.environment_resource_limits().memory_swap_mib,
        4_096,
        "omitted swap should preserve the no-additional-swap default"
    );

    let larger_memory = AyniPolicy::parse(
        "[languages]\nenabled = [\"rust\"]\n[environment.resources]\nmemory_mib = 16384\n",
    )
    .expect("memory can exceed the default without also spelling swap");
    assert_eq!(
        larger_memory.environment_resource_limits().memory_swap_mib,
        16_384
    );

    let error = AyniPolicy::parse(
        "[languages]\nenabled = [\"rust\"]\n[environment.resources]\nmemory_mib = 8192\nmemory_swap_mib = 4096\n",
    )
    .expect_err("swap below memory must fail");
    assert!(error.contains("memory_swap_mib"));
}

#[test]
fn repository_environment_rejects_shell_like_package_and_tool_values() {
    let invalid_package = r#"
[languages]
enabled = ["rust"]
[environment.debian]
packages = ["curl; id"]
"#;
    assert!(AyniPolicy::parse(invalid_package).is_err());
    let apt_option = r#"
[languages]
enabled = ["rust"]
[environment.debian]
packages = ["--allow-unauthenticated"]
"#;
    assert!(AyniPolicy::parse(apt_option).is_err());

    let invalid_tool = r#"
[languages]
enabled = ["rust"]
[environment.tools]
"node;id" = "24.0.0"
"#;
    assert!(AyniPolicy::parse(invalid_tool).is_err());
}

#[test]
fn report_policy_defaults_when_omitted() {
    let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    assert_eq!(policy.report.offenders_limit, usize::MAX);
    assert_eq!(policy.concurrency, ConcurrencyPolicy::default());
}

#[test]
fn report_policy_parses_explicit_offenders_limit() {
    let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[report]
offenders_limit = 4
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    assert_eq!(policy.report.offenders_limit, 4);
}

#[test]
fn rust_size_exclude_parses() {
    let document = r#"
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
"*.rs" = { warn = 400, fail = 700, exclude = ["target/**", "node_modules/**"] }
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    let rule = policy
        .size_rules_for(Language::Rust)
        .get("*.rs")
        .expect("rule");
    assert_eq!(rule.exclude, vec!["target/**", "node_modules/**"]);
}

#[test]
fn multi_language_size_maps_are_independent() {
    let document = r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust", "node"]

[rust.size]
"*.rs" = { warn = 400, fail = 700 }

[node.size]
"**/*.ts" = { warn = 300, fail = 600 }
"**/*.tsx" = { warn = 200, fail = 400 }
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    assert_eq!(policy.size_rules_for(Language::Rust).len(), 1);
    assert_eq!(policy.size_rules_for(Language::Node).len(), 2);
    assert!(policy.size_rules_for(Language::Go).is_empty());
}

#[test]
fn default_roots_to_current_directory() {
    let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]
"#;
    let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
    policy.normalize_and_validate().expect("valid");
    assert_eq!(policy.roots_for(Language::Rust), ["."]);
    assert_eq!(policy.roots_for(Language::Go), ["."]);
    assert_eq!(policy.roots_for(Language::Node), ["."]);
    assert_eq!(policy.roots_for(Language::Python), ["."]);
}

#[test]
fn python_policy_sections_parse() {
    let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["python"]

[python]
roots = ["src"]

[python.size]
"**/*.py" = { warn = 400, fail = 800, exclude = [".venv/**"] }

[python.complexity]
fn_cognitive = { warn = 10, fail = 15 }

[python.coverage]
line_percent = { warn = 80, fail = 60 }

[python.deps.forbidden]
"src/domain/**" = ["src/presentation/**"]
"#;
    let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
    policy.normalize_and_validate().expect("valid");
    assert_eq!(
        policy.enabled_languages().expect("languages"),
        [Language::Python]
    );
    assert_eq!(policy.roots_for(Language::Python), ["src"]);
    assert_eq!(
        policy
            .size_rules_for(Language::Python)
            .get("**/*.py")
            .expect("size")
            .fail,
        800
    );
    assert_eq!(
        policy
            .python
            .complexity
            .as_ref()
            .expect("complexity")
            .fn_cognitive
            .expect("cognitive")
            .fail,
        15.0
    );
    assert_eq!(
        policy
            .python
            .coverage
            .as_ref()
            .expect("coverage")
            .line_percent
            .expect("coverage threshold")
            .warn,
        80.0
    );
    assert_eq!(
        policy
            .python
            .deps
            .as_ref()
            .expect("deps")
            .forbidden
            .get("src/domain/**")
            .expect("rule"),
        &vec![String::from("src/presentation/**")]
    );
}

#[test]
fn rejects_auto_language_selection() {
    let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["auto"]
"#;
    let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
    let error = policy.normalize_and_validate().expect_err("must fail");
    assert!(error.contains("not supported in v0"));
}

#[test]
fn normalizes_roots_entries() {
    let document = r#"
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
roots = ["./", "apps\\service//", "apps/service"]
"#;
    let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
    policy.normalize_and_validate().expect("valid");
    assert_eq!(policy.rust.roots, vec![".", "apps/service"]);
}

#[test]
fn rejects_parent_components() {
    for root in [
        "..",
        "./..",
        "../outside",
        "apps/../outside",
        "apps/./../outside",
    ] {
        let error = normalize_root_entry("rust", root).expect_err("must fail");
        assert!(
            error.contains("must stay within repository root"),
            "{root}: {error}"
        );
    }
}

#[test]
fn rejects_absolute_rooted_and_windows_prefixed_roots() {
    for root in [
        "/outside",
        "\\outside",
        "C:/outside",
        "c:\\outside",
        "D:outside",
        "\\\\server\\share",
    ] {
        let error = normalize_root_entry("rust", root).expect_err("must fail");
        assert!(error.contains("repo-relative"), "{root}: {error}");
    }
}

#[test]
fn concurrency_policy_parses() {
    let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[concurrency]
per_language = true
amount = 3
"#;
    let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
    policy.normalize_and_validate().expect("valid");
    assert!(policy.concurrency.per_language);
    assert_eq!(policy.concurrency.amount, 3);
}

#[test]
fn rejects_zero_concurrency_amount() {
    let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[concurrency]
amount = 0
"#;
    let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
    let error = policy.normalize_and_validate().expect_err("must fail");
    assert!(error.contains("at least 1"));
}

#[test]
fn effectiveness_warnings_cover_empty_enabled_rules_and_required_thresholds() {
    let document = r#"
[checks]
test = false
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust", "python"]

[rust.complexity]
fn_cognitive = { warn = 10, fail = 20 }

[python.coverage]
branch_percent = { warn = 80, fail = 70 }
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    let warnings = policy.effectiveness_warnings(&[
        PolicyEffectivenessFacts::new(Language::Rust, vec![ComplexityThresholdKind::FnCyclomatic]),
        PolicyEffectivenessFacts::new(Language::Python, vec![ComplexityThresholdKind::FnCognitive]),
    ]);

    let actual = warnings
        .iter()
        .map(|warning| (warning.code.as_str(), warning.policy_path.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (
                "policy.effectiveness.coverage.no_threshold",
                "rust.coverage"
            ),
            ("policy.effectiveness.size.no_rules", "rust.size"),
            (
                "policy.effectiveness.complexity.missing_required_threshold",
                "rust.complexity.fn_cyclomatic"
            ),
            (
                "policy.effectiveness.deps.no_forbidden_edges",
                "rust.deps.forbidden"
            ),
            ("policy.effectiveness.size.no_rules", "python.size"),
            (
                "policy.effectiveness.complexity.missing_required_threshold",
                "python.complexity.fn_cognitive"
            ),
            (
                "policy.effectiveness.deps.no_forbidden_edges",
                "python.deps.forbidden"
            ),
        ]
    );
    assert!(warnings.iter().all(|warning| warning.language.is_some()));
    assert!(warnings.iter().all(|warning| warning.signal.is_some()));
}

#[test]
fn effectiveness_warnings_report_configuration_hidden_by_disabled_check() {
    let document = r#"
[checks]
test = false
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 700 }

[rust.coverage]
line_percent = { warn = 80, fail = 70 }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[rust.deps.forbidden]
"src" = ["legacy"]

[rust.tooling.test]
command = "cargo"

[rust.tooling.mutation]
command = "cargo"
"#;
    let policy: AyniPolicy = toml::from_str(document).expect("parse");
    let warnings = policy.effectiveness_warnings(&[]);
    assert_eq!(warnings.len(), 6);
    assert!(warnings.iter().all(|warning| {
        warning.code == "policy.effectiveness.disabled_check_hides_configuration"
    }));
    assert_eq!(warnings[0].policy_path, "checks.test");
    assert_eq!(warnings[5].policy_path, "checks.mutation");
}

#[test]
fn policy_validation_rejects_invalid_thresholds_empty_commands_and_duplicate_languages() {
    for (document, expected) in [
        (
            r#"
[languages]
enabled = ["rust"]
[rust.coverage]
line_percent = { warn = 101, fail = 70 }
"#,
            "rust.coverage.line_percent.warn must be finite and between 0 and 100",
        ),
        (
            r#"
[languages]
enabled = ["rust"]
[rust.coverage]
line_percent = { warn = nan, fail = 70 }
"#,
            "rust.coverage.line_percent.warn must be finite and between 0 and 100",
        ),
        (
            r#"
[languages]
enabled = ["rust"]
[rust.complexity]
fn_cyclomatic = { warn = -1, fail = 20 }
"#,
            "rust.complexity.fn_cyclomatic.warn must be finite and at least 0",
        ),
        (
            r#"
[languages]
enabled = ["rust"]
[rust.tooling.test]
"#,
            "rust.tooling.test.command must be a non-empty command",
        ),
        (
            r#"
[languages]
enabled = ["rust", "rust"]
"#,
            "languages.enabled contains duplicate language 'rust'",
        ),
        (
            r#"
[environment.tools]
node = "latest"
[languages]
enabled = ["rust"]
"#,
            "environment.tools.node must declare a non-floating exact version",
        ),
    ] {
        let error = AyniPolicy::parse(document).expect_err("invalid policy");
        assert_eq!(error, expected);
    }
}
