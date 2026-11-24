use super::reachability::ReachabilityOutcome;
use super::{CallerMap, TestCouplingIndex, TestCouplingRuleId};
use crate::ast::codebase::{EffectiveVisibility, FileId, FuncId, IndexedFunc, TypeId};
use crate::ast::item::{ImportKind, Visibility};
use crate::config::TestCouplingConfig;
use crate::diagnostic::{Diagnostic, Span};
use std::collections::HashSet;

pub(super) struct SurfaceChecker<'a> {
    index: &'a TestCouplingIndex<'a>,
    cfg: &'a TestCouplingConfig,
    reachability: &'a ReachabilityOutcome<'a>,
    diagnostics: Vec<Diagnostic>,
    reported_barrel_import_funcs: HashSet<(FileId, FuncId)>,
    reported_barrel_import_types: HashSet<(FileId, TypeId)>,
}

impl<'a> SurfaceChecker<'a> {
    pub(super) fn new(
        index: &'a TestCouplingIndex<'a>,
        cfg: &'a TestCouplingConfig,
        reachability: &'a ReachabilityOutcome<'a>,
    ) -> Self {
        Self {
            index,
            cfg,
            reachability,
            diagnostics: Vec::new(),
            reported_barrel_import_funcs: HashSet::new(),
            reported_barrel_import_types: HashSet::new(),
        }
    }

    pub(super) fn check(mut self, direct_test_callers: &CallerMap<'a>) -> Vec<Diagnostic> {
        if !self.index.codebase().public_surface().has_public_surface() {
            return Vec::new();
        }
        if self.cfg.surface.flag_test_bypasses_public_surface {
            self.check_barrel_bypassing_imports();
            self.check_cross_surface_calls_and_fields();
        }
        if self.cfg.surface.flag_overexposed_for_tests {
            self.check_overexposed_functions(direct_test_callers);
        }
        self.diagnostics
    }

    fn check_barrel_bypassing_imports(&mut self) {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        for (file_idx, file) in codebase.files().iter().enumerate() {
            let file_id = FileId(file_idx);
            let is_test_file = file.is_test_file();
            for import in &file.imports {
                if !is_test_file && !import.in_test_module {
                    continue;
                }
                if !matches!(import.kind, ImportKind::Named | ImportKind::Default) {
                    continue;
                }
                let Some(target_mod) = import.resolved_module.as_ref() else {
                    continue;
                };

                let target_func = match import.kind {
                    ImportKind::Named => {
                        let funcs = self
                            .index
                            .prod_free_in_mod(target_mod, import.symbol.as_str());
                        (funcs.len() == 1).then(|| funcs[0])
                    }
                    ImportKind::Default => self.index.prod_default_func_in_mod(target_mod),
                    ImportKind::Wildcard | ImportKind::Namespace => None,
                };
                if let Some(func_id) = target_func {
                    let target = codebase.func(func_id);
                    if !self.reachability.flagged_funcs.contains(&target.id)
                        && surface.effective_func_visibility(target, codebase.types())
                            == EffectiveVisibility::Internal
                        && surface.bypasses_enclosing_barrel(
                            &file.module_path,
                            &target.module_path,
                            &target.decl.name,
                            target.decl.is_default_export,
                        )
                        && self
                            .reported_barrel_import_funcs
                            .insert((file_id, target.id))
                    {
                        self.emit_barrel_import_diag(
                            &import.span,
                            "function",
                            &target.display_name(),
                            &target.decl.span,
                        );
                    }
                    continue;
                }

                let target_ty = match import.kind {
                    ImportKind::Named => self
                        .index
                        .prod_type_in_mod(target_mod, import.symbol.as_str()),
                    ImportKind::Default => self.index.prod_default_type_in_mod(target_mod),
                    ImportKind::Wildcard | ImportKind::Namespace => None,
                };
                if let Some(ty_id) = target_ty {
                    let ty = codebase.ty(ty_id);
                    if surface.effective_type_visibility(ty) == EffectiveVisibility::Internal
                        && surface.bypasses_enclosing_barrel(
                            &file.module_path,
                            &ty.module_path,
                            &ty.decl.name,
                            ty.decl.is_default_export,
                        )
                        && self.reported_barrel_import_types.insert((file_id, ty.id))
                    {
                        self.emit_barrel_import_diag(
                            &import.span,
                            "type",
                            &ty.decl.name,
                            &ty.decl.span,
                        );
                    }
                }
            }
        }
    }

    fn emit_barrel_import_diag(
        &mut self,
        import_span: &Span,
        kind: &str,
        symbol_name: &str,
        decl_span: &Span,
    ) {
        self.diagnostics.push(
            Diagnostic::new(
                TestCouplingRuleId::TestBypassesPublicSurface.as_str(),
                self.cfg.severity,
                import_span.clone(),
                format!(
                    "test imports internal {kind} `{symbol_name}` directly, bypassing the module's public surface"
                ),
            )
            .with_related(
                decl_span.clone(),
                format!("internal {kind} `{symbol_name}` declared here"),
            )
            .with_suggestion(
                "Import and exercise the module through its public entrypoint instead of reaching past the barrel boundary.",
            ),
        );
    }

    fn check_cross_surface_calls_and_fields(&mut self) {
        for func in self.index.codebase().functions() {
            if !self.index.is_test_func(func) {
                continue;
            }
            self.check_test_calls(func);
            self.check_test_fields(func);
        }
    }

    fn check_test_calls(&mut self, test_func: &'a IndexedFunc) {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        let mut seen_targets: HashSet<FuncId> = HashSet::new();

        for (call_idx, call) in test_func.decl.calls.iter().enumerate() {
            let targets = self.index.resolve_prod_call(test_func, call_idx);
            if targets.len() != 1 {
                continue;
            }
            let target = codebase.func(targets[0]);
            let bypasses_vis =
                codebase.file_bypasses_visibility_for(test_func.file_id, &target.module_path);
            if self.reachability.flagged_funcs.contains(&target.id)
                || (!bypasses_vis
                    && self
                        .reachability
                        .prod_calls_by_module
                        .contains(&(&test_func.module_path, target.id)))
                || self
                    .reported_barrel_import_funcs
                    .contains(&(test_func.file_id, target.id))
                || target.resolved_owner.is_some_and(|tid| {
                    self.reported_barrel_import_types
                        .contains(&(test_func.file_id, tid))
                })
                || !surface.test_crosses_surface_for_func(
                    &test_func.module_path,
                    bypasses_vis,
                    target,
                    codebase.types(),
                )
                || !seen_targets.insert(target.id)
            {
                continue;
            }

            let kind = if target.decl.is_inherent_method() {
                "method"
            } else {
                "function"
            };
            let target_name = target.display_name();
            self.diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestBypassesPublicSurface.as_str(),
                    self.cfg.severity,
                    call.span.clone(),
                    format!(
                        "test `{}` bypasses the public surface to call internal {kind} `{target_name}`",
                        test_func.display_name()
                    ),
                )
                .with_related(
                    target.decl.span.clone(),
                    format!("internal {kind} `{target_name}` declared here"),
                )
                .with_suggestion(format!(
                    "Exercise `{target_name}` indirectly through the package's public API surface."
                )),
            );
        }
    }

    fn check_test_fields(&mut self, test_func: &'a IndexedFunc) {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        let mut seen_fields: HashSet<(TypeId, &'a str)> = HashSet::new();
        let init_action = format!(
            "initializes via {}",
            codebase.field_init_noun(test_func.file_id)
        );

        let events = test_func
            .decl
            .field_accesses
            .iter()
            .enumerate()
            .filter_map(|(idx, access)| {
                let type_id = test_func
                    .resolved_field_accesses
                    .get(idx)
                    .copied()
                    .flatten()?;
                let action = if access.is_write { "mutates" } else { "reads" };
                Some((type_id, access.field_name.as_str(), &access.span, action))
            })
            .chain(test_func.resolved_field_inits.iter().map(|init| {
                (
                    init.type_id,
                    init.field_name.as_str(),
                    &init.span,
                    init_action.as_str(),
                )
            }));

        for (type_id, field_name, span, action) in events {
            let key = (type_id, field_name);
            if self.reachability.flagged_fields.contains(&key)
                || self
                    .reported_barrel_import_types
                    .contains(&(test_func.file_id, type_id))
            {
                continue;
            }
            let Some(field) = self.index.prod_field(type_id, field_name) else {
                continue;
            };
            let ty = codebase.ty(type_id);
            let bypasses_vis =
                codebase.file_bypasses_visibility_for(test_func.file_id, &ty.module_path);
            if !surface.test_crosses_surface_for_field(
                &test_func.module_path,
                bypasses_vis,
                ty,
                field,
            ) || !seen_fields.insert(key)
            {
                continue;
            }

            self.diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestBypassesPublicSurface.as_str(),
                    self.cfg.severity,
                    span.clone(),
                    format!(
                        "test `{}` bypasses the public surface to {action} internal field `{field_name}` of `{}`",
                        test_func.display_name(),
                        ty.decl.name
                    ),
                )
                .with_related(
                    field.span.clone(),
                    format!("internal field `{field_name}` declared here"),
                )
                .with_suggestion(format!(
                    "Use `{}`'s public API instead of accessing internal field `{field_name}` directly.",
                    ty.decl.name
                )),
            );
        }
    }

    fn check_overexposed_functions(&mut self, direct_test_callers: &CallerMap<'a>) {
        let codebase = self.index.codebase();
        let mut external_tests: Vec<(&'a IndexedFunc, &Span)> = Vec::new();
        for func in codebase.functions() {
            if self.index.is_test_func(func)
                || func.decl.is_interface_impl
                || func.decl.is_entrypoint()
                || self.reachability.flagged_funcs.contains(&func.id)
                || func.decl.visibility != Visibility::Internal
                || !func.decl.explicit_visibility
            {
                continue;
            }
            if self
                .reachability
                .external_prod_called_funcs
                .contains(&func.id)
                || !self
                    .reachability
                    .same_mod_prod_called_funcs
                    .contains(&func.id)
            {
                continue;
            }
            let Some(callers) = direct_test_callers.get(&func.id) else {
                continue;
            };
            external_tests.clear();
            external_tests.extend(
                callers
                    .iter()
                    .filter(|(test_func, _, single_target)| {
                        *single_target
                            && (codebase
                                .file_bypasses_visibility_for(test_func.file_id, &func.module_path)
                                || !test_func.module_path.is_same_or_child_of(&func.module_path))
                    })
                    .map(|(test_func, span, _)| (*test_func, span)),
            );
            if external_tests.is_empty() {
                continue;
            }
            external_tests.sort_by(|a, b| {
                a.0.id
                    .cmp(&b.0.id)
                    .then_with(|| a.1.start_line().cmp(&b.1.start_line()))
                    .then_with(|| a.1.start_col().cmp(&b.1.start_col()))
            });

            let kind = if func.decl.is_inherent_method() {
                "method"
            } else {
                "function"
            };
            let target_name = func.display_name();
            let mut diag = Diagnostic::new(
                TestCouplingRuleId::OverexposedForTests.as_str(),
                self.cfg.severity,
                func.decl.span.clone(),
                format!(
                    "internal {kind} `{target_name}` is only used by production code inside its own module; its explicit internal visibility appears widened only for {} external test(s)",
                    external_tests.len()
                ),
            )
            .with_suggestion(format!(
                "Make `{target_name}` private to its defining module and exercise it through the public API."
            ));
            for &(test_func, call_span) in &external_tests {
                diag = diag.with_related(
                    call_span.clone(),
                    format!("called from external test `{}`", test_func.display_name()),
                );
            }
            self.diagnostics.push(diag);
        }
    }
}
