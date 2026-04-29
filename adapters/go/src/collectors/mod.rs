mod complexity;
mod coverage;
mod deps;
mod mutation;
mod size;
mod test;
mod util;

use ayni_core::{AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow};

#[derive(Debug, Default)]
pub struct GoCollector;

impl SignalCollector for GoCollector {
    fn collect(&self, kind: SignalKind, context: &RunContext) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => test::collect(context),
            SignalKind::Coverage => coverage::collect(context),
            SignalKind::Size => size::collect(context),
            SignalKind::Complexity => complexity::collect(context),
            SignalKind::Deps => deps::collect(context),
            SignalKind::Mutation => mutation::collect(context),
        }
        .map_err(|message| AdapterError::new(Language::Go, message))
    }
}
