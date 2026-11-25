use super::{CallReceiver, ImportOrigin};
use crate::ast::ModulePath;
use crate::diagnostic::Span;

/// Test-environment conditional hook inside production code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestHookSite {
    pub label: String,
    pub span: Span,
}

/// Runtime reflection or encapsulation-escape mechanism used by a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncapsulationEscape {
    pub mechanism: String,
    pub target_receiver: Option<CallReceiver>,
    pub span: Span,
}

/// Module-level mock replacing a dependency in a test file or test body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMock {
    pub resolved_module: Option<ModulePath>,
    pub raw_specifier: String,
    pub origin: ImportOrigin,
    pub in_test_context: bool,
    pub span: Span,
}

/// Spy installed on a receiver method inside a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpySite {
    pub target_receiver: Option<CallReceiver>,
    pub method_name: String,
    pub span: Span,
}

/// Breakdown of behavioral vs mock-interaction assertions inside a test function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssertionSummary {
    pub interaction_count: u32,
    pub behavioral_count: u32,
}

impl AssertionSummary {
    pub const fn record_interaction(&mut self) {
        self.interaction_count += 1;
    }

    pub const fn record_behavioral(&mut self) {
        self.behavioral_count += 1;
    }
}

/// Exact-equality assertion on a value's debug/internal representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReprAssertion {
    pub target_receiver: Option<CallReceiver>,
    pub span: Span,
}

/// Cohesive test-coupling evidence extracted from a function body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FunctionCouplingFacts {
    pub test_hooks: Vec<TestHookSite>,
    pub encapsulation_escapes: Vec<EncapsulationEscape>,
    pub module_mocks: Vec<ModuleMock>,
    pub spy_sites: Vec<SpySite>,
    pub assertions: AssertionSummary,
    pub repr_assertions: Vec<ReprAssertion>,
    pub has_ambient_io: bool,
}

impl FunctionCouplingFacts {
    pub const fn record_interaction_assertion(&mut self) {
        self.assertions.record_interaction();
    }

    pub const fn record_behavioral_assertion(&mut self) {
        self.assertions.record_behavioral();
    }

    pub const fn mark_ambient_io(&mut self) {
        self.has_ambient_io = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coupling_facts_track_interaction_and_behavioral_assertions() {
        let mut facts = FunctionCouplingFacts::default();
        facts.record_interaction_assertion();
        facts.record_behavioral_assertion();
        facts.record_interaction_assertion();
        assert_eq!(facts.assertions.interaction_count, 2);
        assert_eq!(facts.assertions.behavioral_count, 1);
    }
}
