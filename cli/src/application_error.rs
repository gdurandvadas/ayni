use ayni_core::RunOutcome;
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplicationErrorKind {
    InvalidInput,
    Environment,
    Execution,
}

#[derive(Debug)]
pub(crate) struct ApplicationError {
    pub(crate) kind: ApplicationErrorKind,
    pub(crate) message: String,
}

impl ApplicationError {
    pub(crate) fn input(message: impl Into<String>) -> Self {
        Self {
            kind: ApplicationErrorKind::InvalidInput,
            message: message.into(),
        }
    }

    pub(crate) fn environment(message: impl Into<String>) -> Self {
        Self {
            kind: ApplicationErrorKind::Environment,
            message: message.into(),
        }
    }

    pub(crate) fn execution(message: impl Into<String>) -> Self {
        Self {
            kind: ApplicationErrorKind::Execution,
            message: message.into(),
        }
    }

    pub(crate) const fn exit_code(&self) -> u8 {
        match self.kind {
            ApplicationErrorKind::InvalidInput => 2,
            ApplicationErrorKind::Environment => 3,
            ApplicationErrorKind::Execution => 4,
        }
    }
}

pub(crate) fn render_error(error: ApplicationError) -> ExitCode {
    eprintln!("{}", error.message);
    ExitCode::from(error.exit_code())
}

pub(crate) fn outcome_exit(outcome: RunOutcome) -> ExitCode {
    match outcome {
        RunOutcome::Passed => ExitCode::SUCCESS,
        RunOutcome::QualityFailed => ExitCode::from(1),
        RunOutcome::ExecutionIncomplete => ExitCode::from(4),
    }
}

impl From<ayni_environment::BackendError> for ApplicationError {
    fn from(error: ayni_environment::BackendError) -> Self {
        let kind = match error.kind {
            ayni_environment::BackendErrorKind::Input => ApplicationErrorKind::InvalidInput,
            ayni_environment::BackendErrorKind::Environment => ApplicationErrorKind::Environment,
            ayni_environment::BackendErrorKind::Execution => ApplicationErrorKind::Execution,
        };
        Self {
            kind,
            message: error.message,
        }
    }
}
