use super::*;
use std::sync::Condvar;
use std::time::Duration;

type TargetCollectResult = Result<Vec<SignalRow>, String>;
type TargetResultSlots = Arc<Mutex<Vec<Option<TargetCollectResult>>>>;

const SCHEDULER_ABORT_POLL: Duration = Duration::from_millis(25);

struct ScheduledJob<T> {
    index: usize,
    language: Language,
    payload: T,
}

struct SchedulerQueue<T> {
    pending: VecDeque<ScheduledJob<T>>,
    active_by_language: BTreeMap<Language, usize>,
}

struct ActiveJobPermit<T> {
    queue: Arc<(Mutex<SchedulerQueue<T>>, Condvar)>,
    language: Language,
}

impl<T> Drop for ActiveJobPermit<T> {
    fn drop(&mut self) {
        let (queue_lock, queue_changed) = &*self.queue;
        if let Ok(mut guard) = queue_lock.lock() {
            guard.release(self.language);
            queue_changed.notify_all();
        }
    }
}

impl<T> SchedulerQueue<T> {
    fn new(jobs: Vec<ScheduledJob<T>>) -> Self {
        Self {
            pending: VecDeque::from(jobs),
            active_by_language: BTreeMap::new(),
        }
    }

    fn take_next(&mut self, language_caps: &BTreeMap<Language, usize>) -> Option<ScheduledJob<T>> {
        let position = self.pending.iter().position(|job| {
            let active = self
                .active_by_language
                .get(&job.language)
                .copied()
                .unwrap_or(0);
            let cap = language_caps
                .get(&job.language)
                .copied()
                .unwrap_or(usize::MAX)
                .max(1);
            active < cap
        })?;
        let job = self.pending.remove(position)?;
        *self.active_by_language.entry(job.language).or_default() += 1;
        Some(job)
    }

    fn release(&mut self, language: Language) {
        let Some(active) = self.active_by_language.get_mut(&language) else {
            return;
        };
        *active = active.saturating_sub(1);
        if *active == 0 {
            self.active_by_language.remove(&language);
        }
    }
}

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
    registry: Arc<AdapterRegistry>,
) -> Result<RunArtifact, String> {
    let concurrency = planning
        .targets
        .first()
        .map(|target| target.run_context.policy.concurrency.clone())
        .unwrap_or_default();
    let rows = collect_targets_with_ui(ctx, &planning.targets, &concurrency, registry)?;
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
    registry: Arc<AdapterRegistry>,
) -> Result<Vec<SignalRow>, String> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    if targets.len() == 1 || concurrency.amount <= 1 {
        return collect_targets_serial(ctx, targets, &registry);
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
            let adapter_cap = registry
                .adapters()
                .iter()
                .find(|adapter| adapter.language() == language)
                .and_then(|adapter| adapter.max_target_concurrency());
            let worker_limit = adapter_cap.map_or(concurrency.amount, |cap| {
                cap.clamp(1, concurrency.amount.max(1))
            });
            let registry = Arc::clone(&registry);
            group_handles.push(thread::spawn(move || {
                run_target_jobs(
                    &ctx,
                    jobs,
                    worker_limit,
                    BTreeMap::new(),
                    result_slots,
                    registry,
                )
            }));
        }
        for handle in group_handles {
            handle
                .join()
                .map_err(|_| String::from("analyze scheduler panicked"))??;
        }
    } else {
        let language_caps = adapter_target_caps(&registry);
        run_target_jobs(
            ctx,
            indexed_targets,
            concurrency.amount,
            language_caps,
            Arc::clone(&result_slots),
            registry,
        )?;
    }

    flatten_target_results(result_slots, ctx.is_aborted())
}

fn collect_targets_serial(
    ctx: &ui::runner::ExecContext,
    targets: &[AnalyzeTarget],
    registry: &AdapterRegistry,
) -> Result<Vec<SignalRow>, String> {
    let mut rows = Vec::new();
    for target in targets {
        rows.extend(collect_target_with_ui(ctx, target, registry)?);
    }
    Ok(rows)
}

fn collect_target_with_ui(
    ctx: &ui::runner::ExecContext,
    target: &AnalyzeTarget,
    registry: &AdapterRegistry,
) -> Result<Vec<SignalRow>, String> {
    let Some(adapter) = registry
        .adapters()
        .iter()
        .find(|candidate| candidate.language() == target.language)
    else {
        return Ok(Vec::new());
    };
    let mut run_context = target.run_context.clone();
    run_context.cancellation = ctx.cancellation_token();
    let mut rows = Vec::new();
    for kind in enabled_signal_kinds(&run_context.policy) {
        if ctx.is_aborted() {
            return Err(String::from("operation aborted"));
        }
        let tool = ctx.tool(&tool_id(target.language, &target.root, kind))?;
        tool.started();
        let row_result = adapter
            .collector()
            .collect_streaming(kind, &run_context, &mut |line| {
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
            let warn = budget.and_then(|value| value.line_percent_warn);
            let fail = budget.and_then(|value| value.line_percent_fail);
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
            let cyclo_warn = budget
                .and_then(|value| value.fn_cyclomatic.as_ref())
                .map(|value| value.warn);
            let cyclo_fail = budget
                .and_then(|value| value.fn_cyclomatic.as_ref())
                .map(|value| value.fail);
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
    language_caps: BTreeMap<Language, usize>,
    result_slots: TargetResultSlots,
    registry: Arc<AdapterRegistry>,
) -> Result<(), String> {
    let scheduled = jobs
        .into_iter()
        .map(|(index, target)| ScheduledJob {
            index,
            language: target.language,
            payload: target,
        })
        .collect();
    let worker_ctx = ctx.clone();
    let abort_ctx = ctx.clone();
    run_scheduled_jobs(
        scheduled,
        worker_limit,
        language_caps,
        result_slots,
        move |target| {
            let result = collect_target_with_ui(&worker_ctx, &target, &registry);
            if result.is_err() {
                worker_ctx.abort();
            }
            result
        },
        move || abort_ctx.is_aborted(),
    )
}

fn adapter_target_caps(registry: &AdapterRegistry) -> BTreeMap<Language, usize> {
    registry
        .adapters()
        .iter()
        .filter_map(|adapter| {
            adapter
                .max_target_concurrency()
                .map(|cap| (adapter.language(), cap.max(1)))
        })
        .collect()
}

fn run_scheduled_jobs<T, R, F, A>(
    jobs: Vec<ScheduledJob<T>>,
    worker_limit: usize,
    language_caps: BTreeMap<Language, usize>,
    result_slots: Arc<Mutex<Vec<Option<R>>>>,
    execute: F,
    is_aborted: A,
) -> Result<(), String>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> R + Send + Sync + 'static,
    A: Fn() -> bool + Send + Sync + 'static,
{
    if jobs.is_empty() {
        return Ok(());
    }
    let worker_count = effective_worker_count(&jobs, worker_limit, &language_caps);
    let queue = Arc::new((Mutex::new(SchedulerQueue::new(jobs)), Condvar::new()));
    let language_caps = Arc::new(language_caps);
    let execute = Arc::new(execute);
    let is_aborted = Arc::new(is_aborted);
    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let language_caps = Arc::clone(&language_caps);
        let result_slots = Arc::clone(&result_slots);
        let execute = Arc::clone(&execute);
        let is_aborted = Arc::clone(&is_aborted);
        handles.push(thread::spawn(move || -> Result<(), String> {
            loop {
                let job = {
                    let (queue_lock, queue_changed) = &*queue;
                    let mut guard = queue_lock
                        .lock()
                        .map_err(|_| String::from("analyze queue mutex poisoned"))?;
                    loop {
                        if is_aborted() || guard.pending.is_empty() {
                            return Ok(());
                        }
                        if let Some(job) = guard.take_next(&language_caps) {
                            break job;
                        }
                        let (next_guard, _) = queue_changed
                            .wait_timeout(guard, SCHEDULER_ABORT_POLL)
                            .map_err(|_| String::from("analyze queue mutex poisoned"))?;
                        guard = next_guard;
                    }
                };
                let index = job.index;
                let language = job.language;
                let permit = ActiveJobPermit {
                    queue: Arc::clone(&queue),
                    language,
                };
                let result = execute(job.payload);
                {
                    let mut guard = result_slots
                        .lock()
                        .map_err(|_| String::from("analyze result mutex poisoned"))?;
                    guard[index] = Some(result);
                }
                drop(permit);
            }
        }));
    }
    let mut first_error = None;
    for handle in handles {
        let result = handle
            .join()
            .map_err(|_| String::from("analyze worker panicked"))
            .and_then(|result| result);
        if first_error.is_none() {
            first_error = result.err();
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

fn effective_worker_count<T>(
    jobs: &[ScheduledJob<T>],
    worker_limit: usize,
    language_caps: &BTreeMap<Language, usize>,
) -> usize {
    let mut jobs_by_language = BTreeMap::<Language, usize>::new();
    for job in jobs {
        *jobs_by_language.entry(job.language).or_default() += 1;
    }
    let runnable_capacity =
        jobs_by_language
            .into_iter()
            .fold(0_usize, |total, (language, count)| {
                let capacity = language_caps
                    .get(&language)
                    .copied()
                    .map_or(count, |cap| count.min(cap.max(1)));
                total.saturating_add(capacity)
            });
    worker_limit
        .max(1)
        .min(jobs.len())
        .min(runnable_capacity.max(1))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Activity {
        active_total: usize,
        max_total: usize,
        active_by_language: BTreeMap<Language, usize>,
        max_by_language: BTreeMap<Language, usize>,
    }

    #[test]
    fn global_scheduler_enforces_total_and_adapter_limits() {
        let jobs = [
            Language::Rust,
            Language::Rust,
            Language::Rust,
            Language::Node,
            Language::Node,
            Language::Go,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, language)| ScheduledJob {
            index,
            language,
            payload: (language, index),
        })
        .collect::<Vec<_>>();
        let result_slots = Arc::new(Mutex::new(
            std::iter::repeat_with(|| None)
                .take(jobs.len())
                .collect::<Vec<Option<usize>>>(),
        ));
        let activity = Arc::new(Mutex::new(Activity::default()));
        let observed = Arc::clone(&activity);

        run_scheduled_jobs(
            jobs,
            3,
            BTreeMap::from([(Language::Rust, 1), (Language::Node, 2)]),
            Arc::clone(&result_slots),
            move |(language, value)| {
                {
                    let mut activity = observed.lock().expect("activity lock");
                    activity.active_total += 1;
                    activity.max_total = activity.max_total.max(activity.active_total);
                    let active = activity.active_by_language.entry(language).or_default();
                    *active += 1;
                    let active = *active;
                    let maximum = activity.max_by_language.entry(language).or_default();
                    *maximum = (*maximum).max(active);
                }
                thread::sleep(Duration::from_millis(40));
                {
                    let mut activity = observed.lock().expect("activity lock");
                    activity.active_total -= 1;
                    *activity
                        .active_by_language
                        .get_mut(&language)
                        .expect("language is active") -= 1;
                }
                value
            },
            || false,
        )
        .expect("scheduler completes");

        let activity = activity.lock().expect("activity lock");
        assert_eq!(activity.max_total, 3, "global worker bound is exercised");
        assert_eq!(activity.max_by_language[&Language::Rust], 1);
        assert_eq!(activity.max_by_language[&Language::Node], 2);
        drop(activity);

        let results = result_slots.lock().expect("result lock");
        assert_eq!(
            *results,
            vec![Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)],
            "completion order must not affect deterministic result slots"
        );
    }

    #[test]
    fn registered_adapter_caps_include_rust_serialization() {
        let registry = crate::build_registry();
        let caps = adapter_target_caps(&registry);
        assert_eq!(caps.get(&Language::Rust), Some(&1));
    }

    #[test]
    fn worker_pool_is_bounded_by_effective_language_capacity() {
        let jobs = (0..100)
            .map(|index| ScheduledJob {
                index,
                language: Language::Rust,
                payload: (),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            effective_worker_count(&jobs, 100, &BTreeMap::from([(Language::Rust, 1)])),
            1
        );
    }
}
