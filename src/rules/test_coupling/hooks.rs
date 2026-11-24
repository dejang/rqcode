use super::{TestCouplingIndex, TestCouplingRuleId};
use crate::config::TestCouplingConfig;
use crate::diagnostic::Diagnostic;
use std::collections::HashSet;
use std::path::Path;

pub(super) fn check_production_test_hooks(
    index: &TestCouplingIndex<'_>,
    cfg: &TestCouplingConfig,
) -> Vec<Diagnostic> {
    let codebase = index.codebase();
    let mut diagnostics = Vec::new();
    let mut seen: HashSet<(&Path, usize, usize)> = HashSet::new();

    for file in codebase.files() {
        if file.is_test_file() {
            continue;
        }
        for hook in &file.test_hooks {
            if !seen.insert((
                hook.span.file().as_path(),
                hook.span.start_line(),
                hook.span.start_col(),
            )) {
                continue;
            }
            diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::ProductionTestHook.as_str(),
                    cfg.severity,
                    hook.span.clone(),
                    format!(
                        "production code contains test-only conditional hook `{}`",
                        hook.label
                    ),
                )
                .with_suggestion(
                    "Remove test-environment conditionals from production code and inject dependencies or configuration explicitly.",
                ),
            );
        }
    }

    for func in codebase.functions() {
        if index.is_test_func(func) {
            continue;
        }
        for hook in &func.decl.coupling.test_hooks {
            if !seen.insert((
                hook.span.file().as_path(),
                hook.span.start_line(),
                hook.span.start_col(),
            )) {
                continue;
            }
            diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::ProductionTestHook.as_str(),
                    cfg.severity,
                    hook.span.clone(),
                    format!(
                        "production function `{}` contains test-only conditional hook `{}`",
                        func.display_name(),
                        hook.label
                    ),
                )
                .with_suggestion(
                    "Remove test-environment conditionals from production code and inject dependencies or configuration explicitly.",
                ),
            );
        }
    }

    diagnostics
}
