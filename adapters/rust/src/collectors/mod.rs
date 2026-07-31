mod complexity;
mod coverage;
mod deps;
mod mutation;
mod size;
pub mod test;

use ayni_core::{
    AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow,
    VerificationSelection,
};

#[derive(Debug, Default)]
pub struct RustCollector;

impl SignalCollector for RustCollector {
    fn collect_verification(
        &self,
        kind: SignalKind,
        context: &RunContext,
        selection: &VerificationSelection,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => test::collect_selected_with_lines(context, selection, on_line)
                .map_err(|message| AdapterError::new(Language::Rust, message)),
            _ => self.collect_streaming(kind, context, on_line),
        }
    }
    fn collect_streaming(
        &self,
        kind: SignalKind,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => test::collect_with_lines(context, on_line)
                .map_err(|message| AdapterError::new(Language::Rust, message)),
            _ => self.collect(kind, context),
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
        .map_err(|message| AdapterError::new(Language::Rust, message))
    }
}
