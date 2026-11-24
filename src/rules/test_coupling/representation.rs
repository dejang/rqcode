use super::{TestCouplingIndex, TestCouplingRuleId};
use crate::ast::codebase::TypeId;
use crate::ast::item::{InterfaceImplRole, Visibility};
use crate::config::TestCouplingConfig;
use crate::diagnostic::Diagnostic;
use std::collections::HashSet;

pub(super) fn check_repr_assertions(
    index: &TestCouplingIndex<'_>,
    cfg: &TestCouplingConfig,
) -> Vec<Diagnostic> {
    let codebase = index.codebase();
    let custom_debug_types: HashSet<TypeId> = codebase
        .interface_impls()
        .iter()
        .filter(|imp| {
            imp.role == InterfaceImplRole::CustomDebugRepr
                && !imp.in_test_module
                && !codebase.file(imp.file_id).is_test_file()
        })
        .filter_map(|imp| imp.target_id)
        .collect();
    let mut types_with_private_fields: HashSet<TypeId> = HashSet::new();
    for ty in codebase.types() {
        for field in &ty.decl.fields {
            if field.visibility == Visibility::Private {
                types_with_private_fields.insert(ty.id);
                break;
            }
        }
    }

    let mut diagnostics = Vec::new();
    for func in codebase.functions() {
        if !index.is_test_func(func) {
            continue;
        }
        for (idx, repr_assertion) in func.decl.coupling.repr_assertions.iter().enumerate() {
            let Some(tid) = func.resolved_repr_targets.get(idx).copied().flatten() else {
                continue;
            };
            let ty = codebase.ty(tid);
            if index.is_test_type(ty) {
                continue;
            }
            if custom_debug_types.contains(&tid) || !types_with_private_fields.contains(&tid) {
                continue;
            }
            diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestAssertsInternalRepresentation.as_str(),
                    cfg.severity,
                    repr_assertion.span.clone(),
                    format!(
                        "test `{}` asserts exact equality on debug representation of `{}` which exposes private fields",
                        func.display_name(),
                        ty.decl.name
                    ),
                )
                .with_related(
                    ty.decl.span.clone(),
                    format!("type `{}` with private fields declared here", ty.decl.name),
                )
                .with_suggestion(
                    "Assert on domain behavior or explicit public formatting instead of derived debug output.",
                ),
            );
        }
    }

    diagnostics
}
