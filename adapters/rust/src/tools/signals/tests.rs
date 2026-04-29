use std::fs;
use std::path::Path;

use ayni_core::AYNI_POLICY_FILE;
use serde_json::json;
use tempfile::TempDir;

use super::*;

#[test]
fn size_check_sorts_offenders_and_applies_fail_threshold() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    create_workspace(root);
    fs::write(
        root.join(AYNI_POLICY_FILE),
        r#"
[size.thresholds]
"*.rs" = { warn = 2, fail = 4 }

[complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[deps.rust.forbidden]
"core" = ["cli"]
"#,
    )
    .expect("budgets write");

    fs::create_dir_all(root.join("core/src")).expect("core src dir");
    fs::write(root.join("core/src/a.rs"), "1\n2\n3\n").expect("write a");
    fs::write(root.join("core/src/b.rs"), "1\n2\n3\n4\n5\n6\n").expect("write b");

    let checks = run_signal_checks(root, &[SignalCheck::Size], &SignalScopeInput::default())
        .expect("checks run");
    let result = checks.first().expect("size result exists");

    assert!(!result.pass);
    assert_eq!(result.offenders.len(), 2);
    assert_eq!(result.offenders[0]["file"], json!("core/src/b.rs"));
    assert_eq!(result.offenders[0]["value"], json!(6));
    assert_eq!(result.result["warn_count"], json!(1));
    assert_eq!(result.result["fail_count"], json!(1));
    assert_eq!(result.delta_vs_baseline, json!({}));
    assert_eq!(result.delta_vs_previous, json!({}));
}

fn create_workspace(root: &Path) {
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"core\", \"cli\"]\n",
    )
    .expect("workspace cargo");
    fs::create_dir_all(root.join("core")).expect("core dir");
    fs::write(
        root.join("core/Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("core cargo");
    fs::create_dir_all(root.join("cli")).expect("cli dir");
    fs::write(
        root.join("cli/Cargo.toml"),
        "[package]\nname = \"cli\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cli cargo");
}

#[test]
fn parses_function_metric_from_rust_code_analysis_json() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    let value = json!({
        "path": "core/src/lib.rs",
        "name": "example_fn",
        "start_line": 42,
        "metrics": {
            "cyclomatic": { "max": 12.0, "sum": 12.0 },
            "cognitive": { "max": 7.0, "sum": 7.0 }
        }
    });
    let map = value.as_object().expect("object");
    let metric = parse_function_metric(map, root, None).expect("metric");

    assert_eq!(metric.file, "core/src/lib.rs");
    assert_eq!(metric.function, "example_fn");
    assert_eq!(metric.line, 42);
    assert_eq!(metric.cyclomatic, 12.0);
    assert_eq!(metric.cognitive, 7.0);
}

#[test]
fn walk_metric_tree_collects_functions_under_unit_file() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    let lib_path = format!("{}/pkg/src/lib.rs", root.display());
    let unit = json!({
        "name": lib_path,
        "kind": "unit",
        "start_line": 1,
        "end_line": 100,
        "spaces": [
            {
                "name": "alpha",
                "kind": "function",
                "start_line": 10,
                "metrics": {
                    "cyclomatic": { "max": 5.0, "sum": 5.0 },
                    "cognitive_complexity": { "max": 3.0, "sum": 3.0 }
                }
            },
            {
                "name": "beta",
                "kind": "function",
                "start_line": 22,
                "metrics": {
                    "cyclomatic": { "max": 11.0 },
                    "cognitive": { "max": 9.0 }
                }
            }
        ]
    });

    let mut metrics = Vec::new();
    walk_metric_tree(&unit, root, None, &mut metrics);
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].function, "alpha");
    assert_eq!(metrics[1].function, "beta");
    assert_eq!(metrics[1].cognitive, 9.0);
    assert!(metrics[1].file.ends_with("pkg/src/lib.rs"));
}
