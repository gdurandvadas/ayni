use ayni_core::{Delta, Language, RunArtifact, SignalKind, SignalResult, SignalRow};
use std::collections::{BTreeSet, HashMap};

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

pub fn annotate_deltas_vs_previous(
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
