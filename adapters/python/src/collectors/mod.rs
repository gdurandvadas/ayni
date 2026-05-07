use ayni_core::{AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow};

pub mod complexity;
pub mod coverage;
pub mod deps;
pub mod mutation;
pub mod size;
pub mod test;
pub mod util;

#[derive(Debug, Default)]
pub struct PythonCollector;

impl SignalCollector for PythonCollector {
    fn collect(&self, kind: SignalKind, context: &RunContext) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => test::collect(context),
            SignalKind::Coverage => coverage::collect(context),
            SignalKind::Size => size::collect(context),
            SignalKind::Complexity => complexity::collect(context),
            SignalKind::Deps => deps::collect(context),
            SignalKind::Mutation => mutation::collect(context),
        }
        .map_err(|message| AdapterError::new(Language::Python, message))
    }
}
