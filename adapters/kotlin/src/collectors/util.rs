use ayni_adapters_common::exec::{ExecutionError, run_command_for_context_structured};
use ayni_core::{Language, RunContext, SignalKind};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MANAGED_OUTPUT_ROOT: &str = "AYNI_GRADLE_OUTPUT_ROOT";
const OUTPUT_INIT_SCRIPT: &str = r#"gradle.beforeProject { project ->
    def outputRoot = new File(System.getenv("AYNI_GRADLE_OUTPUT_ROOT"))
    def projectKey = project.path == ":" ? "root" : project.path.substring(1).replace(':', '/')
    project.layout.buildDirectory.set(new File(outputRoot, System.getProperty("ayni.signal") + "/" + projectKey + "/build"))
}

gradle.projectsEvaluated {
    allprojects { project ->
        project.tasks.matching { it.name.startsWith("detekt") }.configureEach { task ->
            if (task.hasProperty("reports")) {
                def report = new File(project.layout.buildDirectory.get().asFile, "reports/detekt/${task.name}.xml")
                task.reports.xml.required.set(true)
                task.reports.xml.outputLocation.set(report)
            }
        }
    }
}
"#;

pub fn combined_gradle_command(context: &RunContext, coverage_task: &str) -> (String, Vec<String>) {
    let default_args = || {
        managed_gradle_args(
            context,
            SignalKind::Coverage,
            vec![
                String::from("test"),
                coverage_task.to_string(),
                String::from("--console=plain"),
            ],
        )
    };
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(Language::Kotlin, SignalKind::Coverage)
    {
        // Explicit arguments attest that the wrapper itself runs both tasks. Keep
        // its task arguments intact while still adding Ayni's managed Gradle
        // isolation flags when managed execution is active.
        let args = if override_cmd.args.is_empty() {
            default_args()
        } else {
            managed_gradle_args(context, SignalKind::Coverage, override_cmd.args.clone())
        };
        return (override_cmd.command.clone(), args);
    }
    (context.execution.runner.clone(), default_args())
}

pub fn prepare_combined_gradle_execution(context: &RunContext) -> Result<(), String> {
    prepare_gradle_execution(context, SignalKind::Coverage)?;
    if managed_output_root(context).is_none() {
        for (segments, evidence) in [
            (&["build", "test-results", "test"][..], "test"),
            (&["build", "reports", "kover"][..], "Kover coverage"),
            (&["build", "reports", "jacoco"][..], "JaCoCo coverage"),
        ] {
            for report_dir in find_report_dirs(&context.workdir, segments) {
                fs::remove_dir_all(&report_dir).map_err(|error| {
                    format!(
                        "failed to clear stale Kotlin {evidence} reports {}: {error}",
                        report_dir.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

pub fn gradle_command(
    context: &RunContext,
    kind: SignalKind,
    default_task: &str,
) -> (String, Vec<String>) {
    if let Some(override_cmd) = context.policy.tool_override_for(Language::Kotlin, kind) {
        let args = if override_cmd.args.is_empty() {
            default_gradle_args(context, kind, default_task)
        } else {
            managed_gradle_args(context, kind, override_cmd.args.clone())
        };
        return (override_cmd.command.clone(), args);
    }
    (
        context.execution.runner.clone(),
        default_gradle_args(context, kind, default_task),
    )
}

fn default_gradle_args(context: &RunContext, kind: SignalKind, task: &str) -> Vec<String> {
    managed_gradle_args(
        context,
        kind,
        vec![task.to_string(), String::from("--console=plain")],
    )
}

fn managed_gradle_args(
    context: &RunContext,
    kind: SignalKind,
    mut args: Vec<String>,
) -> Vec<String> {
    if context
        .execution
        .environment
        .contains_key("AYNI_GRADLE_OFFLINE")
    {
        if !args.iter().any(|arg| arg == "--offline") {
            args.push(String::from("--offline"));
        }
        if !args.iter().any(|arg| arg == "--no-daemon") {
            args.push(String::from("--no-daemon"));
        }
    }
    if let Some(root) = managed_output_root(context) {
        let signal = signal_name(kind);
        args.extend([
            String::from("--project-cache-dir"),
            root.join(signal)
                .join("project-cache")
                .display()
                .to_string(),
            String::from("--init-script"),
            root.join(signal)
                .join("output.init.gradle")
                .display()
                .to_string(),
            format!("-Dayni.signal={signal}"),
            format!(
                "-Dkotlin.project.persistent.dir={}",
                root.join(signal).join("kotlin-persistent").display()
            ),
        ]);
    }
    args
}

pub fn prepare_gradle_execution(context: &RunContext, kind: SignalKind) -> Result<(), String> {
    let Some(root) = managed_output_root(context) else {
        if kind == SignalKind::Test {
            for report_dir in find_report_dirs(&context.workdir, &["build", "test-results", "test"])
            {
                fs::remove_dir_all(&report_dir).map_err(|error| {
                    format!(
                        "failed to clear stale Kotlin test reports {}: {error}",
                        report_dir.display()
                    )
                })?;
            }
        }
        return Ok(());
    };
    let signal_root = root.join(signal_name(kind));
    match fs::remove_dir_all(&signal_root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to clear managed Gradle output {}: {error}",
                signal_root.display()
            ));
        }
    }
    fs::create_dir_all(&signal_root).map_err(|error| {
        format!(
            "failed to create managed Gradle output {}: {error}",
            signal_root.display()
        )
    })?;
    let script = signal_root.join("output.init.gradle");
    fs::write(&script, OUTPUT_INIT_SCRIPT)
        .map_err(|error| format!("failed to write {}: {error}", script.display()))
}

pub fn report_root(context: &RunContext, kind: SignalKind) -> PathBuf {
    managed_output_root(context)
        .map(|root| root.join(signal_name(kind)))
        .unwrap_or_else(|| context.workdir.clone())
}

fn managed_output_root(context: &RunContext) -> Option<PathBuf> {
    context
        .execution
        .environment
        .get(MANAGED_OUTPUT_ROOT)
        .map(PathBuf::from)
}

fn signal_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

pub fn resolve_gradle_task(
    context: &RunContext,
    preferred_tasks: &[&str],
) -> Result<Option<String>, Box<ExecutionError>> {
    let args = managed_gradle_args(
        context,
        SignalKind::Coverage,
        vec![
            String::from("tasks"),
            String::from("--all"),
            String::from("--quiet"),
        ],
    );
    let program = context
        .policy
        .tool_override_for(Language::Kotlin, SignalKind::Coverage)
        .map_or(context.execution.runner.as_str(), |command| {
            command.command.as_str()
        });
    let output = run_command_for_context_structured(context, program, &args)?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(preferred_tasks
        .iter()
        .find(|task| gradle_task_list_contains(&stdout, task))
        .map(|task| (*task).to_string()))
}

fn gradle_task_list_contains(stdout: &str, task: &str) -> bool {
    let suffix = format!(":{task}");
    stdout.lines().any(|line| {
        let first = line.split_whitespace().next().unwrap_or("");
        first == task || first.ends_with(&suffix)
    })
}

pub fn find_reports(root: &Path, segments: &[&str], extension: &str) -> Vec<PathBuf> {
    let mut reports: Vec<PathBuf> = find_report_dirs(root, segments)
        .into_iter()
        .flat_map(|dir| {
            WalkDir::new(dir)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
                })
                .map(|entry| entry.path().to_path_buf())
                .collect::<Vec<_>>()
        })
        .collect();
    reports.sort();
    reports.dedup();
    reports
}

fn find_report_dirs(root: &Path, segments: &[&str]) -> Vec<PathBuf> {
    let suffix: PathBuf = segments.iter().collect();
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | ".gradle")
            )
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .filter(|entry| entry.path().ends_with(&suffix))
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

#[cfg(test)]
mod managed_tests {
    use super::*;
    use ayni_core::{AyniPolicy, ExecutionResolution, Scope};
    use tempfile::TempDir;

    fn context(managed: bool) -> RunContext {
        let root = PathBuf::from("/repo");
        let mut execution = ExecutionResolution::direct("./gradlew", root.clone(), "test", 100);
        if managed {
            execution
                .environment
                .insert("AYNI_GRADLE_OFFLINE".into(), "1".into());
        }
        RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root,
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution,
            cancellation: Default::default(),
            debug: false,
        }
    }

    #[test]
    fn default_coverage_commands_schedule_test_before_each_report_task() {
        let context = context(false);
        for task in ["koverXmlReport", "jacocoTestReport"] {
            let (program, args) = combined_gradle_command(&context, task);
            assert_eq!(program, "./gradlew");
            assert_eq!(args, ["test", task, "--console=plain"]);
        }
    }

    #[test]
    fn host_test_preparation_clears_only_gradle_test_reports() {
        let repo = TempDir::new().expect("repo");
        let report_dir = repo.path().join("module/build/test-results/test");
        fs::create_dir_all(&report_dir).expect("report dir");
        fs::write(report_dir.join("stale.xml"), "<testsuite/>").expect("stale report");
        fs::write(repo.path().join("fixture.xml"), "<testsuite/>").expect("fixture");
        let mut context = context(false);
        context.repo_root = repo.path().to_path_buf();
        context.target_root = repo.path().to_path_buf();
        context.workdir = repo.path().to_path_buf();
        context.execution.exec_cwd = repo.path().to_path_buf();

        prepare_gradle_execution(&context, SignalKind::Test).expect("prepare test");

        assert!(!report_dir.exists());
        assert!(repo.path().join("fixture.xml").is_file());
    }

    #[test]
    fn combined_host_preparation_clears_test_and_coverage_reports() {
        let repo = TempDir::new().expect("repo");
        let test_report = repo.path().join("module/build/test-results/test/stale.xml");
        let kover_report = repo.path().join("module/build/reports/kover/stale.xml");
        let jacoco_report = repo.path().join("module/build/reports/jacoco/stale.xml");
        for report in [&test_report, &kover_report, &jacoco_report] {
            fs::create_dir_all(report.parent().expect("report parent")).expect("report dir");
            fs::write(report, "stale").expect("stale report");
        }
        let mut context = context(false);
        context.repo_root = repo.path().to_path_buf();
        context.target_root = repo.path().to_path_buf();
        context.workdir = repo.path().to_path_buf();
        context.execution.exec_cwd = repo.path().to_path_buf();

        prepare_combined_gradle_execution(&context).expect("prepare combined output");

        assert!(!test_report.exists());
        assert!(!kover_report.exists());
        assert!(!jacoco_report.exists());
    }

    #[test]
    fn managed_output_is_redirected_below_the_writable_state_root() {
        let state = TempDir::new().expect("state");
        let mut context = context(true);
        context.execution.environment.insert(
            MANAGED_OUTPUT_ROOT.into(),
            state.path().display().to_string(),
        );

        prepare_gradle_execution(&context, SignalKind::Coverage).expect("prepare output");
        let args = default_gradle_args(&context, SignalKind::Coverage, "koverXmlReport");

        assert!(args.iter().any(|arg| arg == "-Dayni.signal=coverage"));
        assert!(args.iter().any(|arg| {
            arg == &format!(
                "-Dkotlin.project.persistent.dir={}",
                state.path().join("coverage/kotlin-persistent").display()
            )
        }));
        assert!(args.iter().any(|arg| {
            arg == &state
                .path()
                .join("coverage/output.init.gradle")
                .display()
                .to_string()
        }));
        let script = fs::read_to_string(state.path().join("coverage/output.init.gradle"))
            .expect("output script");
        assert!(script.contains("task.reports.xml.outputLocation.set(report)"));
        assert_eq!(
            report_root(&context, SignalKind::Coverage),
            state.path().join("coverage")
        );
    }

    #[test]
    fn managed_commands_are_explicitly_offline_and_daemonless() {
        assert_eq!(
            default_gradle_args(&context(false), SignalKind::Test, "test"),
            ["test", "--console=plain"]
        );
        assert_eq!(
            default_gradle_args(&context(true), SignalKind::Test, "test"),
            ["test", "--console=plain", "--offline", "--no-daemon"]
        );
    }
}
