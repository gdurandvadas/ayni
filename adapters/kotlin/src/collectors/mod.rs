mod complexity;
mod coverage;
mod deps;
mod mutation;
mod size;
mod test;
mod util;

use ayni_core::{
    AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow,
    VerificationSelection,
};

#[derive(Debug, Default)]
pub struct KotlinCollector;

impl SignalCollector for KotlinCollector {
    fn collect_verification(
        &self,
        kind: SignalKind,
        context: &RunContext,
        selection: &VerificationSelection,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => test::collect_selected(context, selection, on_line)
                .map_err(|message| AdapterError::new(Language::Kotlin, message)),
            _ => self.collect_streaming(kind, context, on_line),
        }
    }

    fn collect(&self, kind: SignalKind, context: &RunContext) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => test::collect(context),
            SignalKind::Coverage => coverage::collect(context),
            SignalKind::Size => size::collect(context),
            SignalKind::Complexity => complexity::collect(context),
            SignalKind::Deps => deps::collect(context),
            SignalKind::Mutation => mutation::collect(context),
        }
        .map_err(|message| AdapterError::new(Language::Kotlin, message))
    }
}
