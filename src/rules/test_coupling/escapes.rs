use super::{TestCouplingIndex, TestCouplingRuleId};
use crate::ast::codebase::TypeId;
use crate::ast::item::{SubclassPolicy, TypeKind};
use crate::config::TestCouplingConfig;
use crate::diagnostic::Diagnostic;
use std::collections::HashSet;

pub(super) fn check_encapsulation_escapes(
    index: &TestCouplingIndex<'_>,
    cfg: &TestCouplingConfig,
) -> Vec<Diagnostic> {
    let codebase = index.codebase();
    let mut diagnostics = Vec::new();

    // Check A: Runtime reflection or encapsulation-escape mechanisms targeting production types.
    for func in codebase.functions() {
        if !index.is_test_func(func) {
            continue;
        }
        for (idx, escape) in func.decl.coupling.encapsulation_escapes.iter().enumerate() {
            let Some(tid) = func.resolved_escape_targets.get(idx).copied().flatten() else {
                continue;
            };
            let ty = codebase.ty(tid);
            if index.is_test_type(ty) {
                continue;
            }
            diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestEncapsulationEscape.as_str(),
                    cfg.severity,
                    escape.span.clone(),
                    format!(
                        "test `{}` uses encapsulation escape `{}` to inspect `{}` internals",
                        func.display_name(),
                        escape.mechanism,
                        ty.decl.name
                    ),
                )
                .with_related(
                    ty.decl.span.clone(),
                    format!("production type `{}` inspected here", ty.decl.name),
                )
                .with_suggestion(
                    "Assert on observable outputs through the public API instead of inspecting private state via reflection.",
                ),
            );
        }
    }

    // Check B: Test-specific subclasses of closed production classes that have no production subclasses.
    let mut prod_subclassed: HashSet<TypeId> = HashSet::new();
    for ty in codebase.types() {
        if !index.is_test_type(ty) && ty.decl.kind == TypeKind::Class {
            prod_subclassed.extend(ty.resolved_base_types.iter().copied());
        }
    }

    for ty in codebase.types() {
        if !index.is_test_type(ty) || ty.decl.kind != TypeKind::Class {
            continue;
        }
        for &base_id in &ty.resolved_base_types {
            let base_ty = codebase.ty(base_id);
            if index.is_test_type(base_ty)
                || base_ty.decl.kind != TypeKind::Class
                || base_ty.decl.subclass_policy != SubclassPolicy::Closed
                || prod_subclassed.contains(&base_id)
            {
                continue;
            }
            diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestEncapsulationEscape.as_str(),
                    cfg.severity,
                    ty.decl.span.clone(),
                    format!(
                        "test class `{}` subclasses closed production class `{}` to reach internal or protected hooks",
                        ty.decl.name, base_ty.decl.name
                    ),
                )
                .with_related(
                    base_ty.decl.span.clone(),
                    format!("closed production class `{}` declared here", base_ty.decl.name),
                )
                .with_suggestion(
                    "Collaborate through a public interface or dependency injection instead of subclassing a concrete production class in tests.",
                ),
            );
        }
    }

    diagnostics
}
