use std::path::Path;
use std::process::ExitCode;

mod agents;
mod analysis;
mod application;
mod application_error;
mod args;
mod artifact_compare;
mod contract;
mod discovery;
mod environment;
mod environment_backend;
mod environment_lock;
mod impact;
mod init;
mod policy;
mod registry;
mod ui;
mod verification_command;
mod verification_list;
mod verify;

use agents::sync_impl;
use analysis::{AnalyzeOptions, OutputArg, analyze};
use clap::Parser;
use registry::build_registry;

fn main() -> ExitCode {
    dispatch(args::Cli::parse().into_operation())
}

fn dispatch(operation: application::Operation) -> ExitCode {
    use application::Operation;

    match operation {
        Operation::Init(operation) => init::run(operation, &build_registry()),
        operation @ (Operation::Check(_) | Operation::Verify(_) | Operation::ImpactRun(_)) => {
            dispatch_analysis(operation)
        }
        Operation::ImpactShow(operation) => impact::show(operation),
        operation @ (Operation::EnvShow(_)
        | Operation::EnvLock(_)
        | Operation::EnvDoctor(_)
        | Operation::EnvBuild(_)
        | Operation::EnvShell(_)
        | Operation::EnvRun(_)) => dispatch_environment(operation),
        Operation::ContractShow(operation) => dispatch_contract(operation),
        Operation::AgentsSync(operation) => agents_sync(&operation.repo_root),
        Operation::VerifyList(operation) => verification_list::run(&operation.artifact),
        Operation::ResultsCompare(operation) => artifact_compare::run(
            &operation.baseline,
            &operation.candidate,
            operation.output == application::OutputFormat::Json,
        ),
        Operation::GenerateDocs => {
            print!("{}", clap_markdown::help_markdown::<args::Cli>());
            ExitCode::SUCCESS
        }
    }
}

fn dispatch_analysis(operation: application::Operation) -> ExitCode {
    use application::{ExecutionMode, Operation};

    match operation {
        Operation::Check(operation) if operation.execution_mode == ExecutionMode::Host => {
            match analyze(
                operation.config.to_string_lossy().as_ref(),
                AnalyzeOptions {
                    output_mode: output_arg(operation.output),
                    debug: operation.debug,
                },
            ) {
                Ok(outcome) => crate::application_error::outcome_exit(outcome),
                Err(error) => crate::application_error::render_error(error),
            }
        }
        Operation::Verify(operation) if operation.execution_mode == ExecutionMode::Host => {
            run_verify_operation(operation)
        }
        Operation::ImpactRun(operation) if operation.execution_mode == ExecutionMode::Host => {
            impact::run(operation)
        }
        Operation::Check(operation) => environment_backend::check(operation, &build_registry()),
        Operation::Verify(operation) => environment_backend::verify(operation, &build_registry()),
        Operation::ImpactRun(operation) => {
            environment_backend::impact_run(operation, &build_registry())
        }
        _ => unreachable!("dispatch_analysis received a non-analysis operation"),
    }
}

fn dispatch_environment(operation: application::Operation) -> ExitCode {
    use application::Operation;

    let registry = build_registry();
    match operation {
        Operation::EnvShow(operation) => environment::show(operation, &registry),
        Operation::EnvLock(operation) => environment_lock::run(operation, &registry),
        Operation::EnvDoctor(operation) => environment_backend::doctor(operation, &registry),
        Operation::EnvBuild(operation) => environment_backend::build(operation, &registry),
        Operation::EnvShell(operation) => environment_backend::shell(operation, &registry),
        Operation::EnvRun(operation) => environment_backend::run(operation, &registry),
        _ => unreachable!("dispatch_environment received a non-environment operation"),
    }
}

fn dispatch_contract(operation: application::ContractOperation) -> ExitCode {
    use application::OutputFormat;

    contract_display(
        operation.config.to_string_lossy().as_ref(),
        operation.output == OutputFormat::Json,
    )
}

fn output_arg(output: application::OutputFormat) -> OutputArg {
    match output {
        application::OutputFormat::Human => OutputArg::Stdout,
        application::OutputFormat::Json => OutputArg::Json,
        application::OutputFormat::Markdown => OutputArg::Md,
    }
}

fn run_verify_operation(operation: application::VerifyOperation) -> ExitCode {
    let request = verify::Request {
        kind: operation.signal,
        config_path: operation.config,
        file: operation.file,
        package: operation.package,
        name: operation.name,
        language: operation.language,
        root: operation.root,
        output_mode: output_arg(operation.output),
        debug: operation.debug,
    };
    match verify::run(request) {
        Ok(outcome) => crate::application_error::outcome_exit(outcome),
        Err(error) => crate::application_error::render_error(error),
    }
}

fn contract_display(config_path: &str, json: bool) -> ExitCode {
    let adapter_facts = build_registry()
        .adapters()
        .iter()
        .map(|adapter| adapter.policy_effectiveness_facts())
        .collect::<Vec<_>>();
    match contract::display(Path::new(config_path), &adapter_facts, json) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn agents_sync(repo_root: &Path) -> ExitCode {
    match sync_impl(repo_root.to_string_lossy().as_ref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(4)
        }
    }
}
#[cfg(test)]
mod tests;
