use super::*;

type TargetCollectResult = Result<Vec<SignalRow>, String>;
type TargetResultSlots = Arc<Mutex<Vec<Option<TargetCollectResult>>>>;

#[derive(Clone, Debug)]
pub(crate) struct AnalyzeOptions {
    pub(crate) output_mode: OutputArg,
    pub(crate) debug: bool,
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

pub(super) fn build_analyze_plan(targets: &[AnalyzeTarget]) -> ui::runner::Plan {
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

pub(super) fn run_collect_with_ui(
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
    let (completion, rows) = reconcile(planning, scope, None, rows);
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
                // Adapter, configuration, and parsing failures are not tool
                // outcomes. Leave their planned row absent so reconciliation
                // records incomplete collection evidence for this target.
                tool.line(error.to_string());
                tool.finished(ui::runner::ToolState::Failed);
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
