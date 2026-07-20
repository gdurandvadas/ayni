use ayni_core::{
    AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow, TestSelection,
};

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
    fn collect_selected_test(
        &self,
        context: &RunContext,
        selection: &TestSelection,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        test::collect_selected(context, selection, on_line)
            .map_err(|message| AdapterError::new(Language::Python, message))
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
        .map_err(|message| AdapterError::new(Language::Python, message))
    }
}
