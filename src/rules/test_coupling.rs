use crate::ast::ModulePath;
use crate::ast::codebase::{Codebase, FileId, FuncId, IndexedFunc, TypeId};
use crate::ast::item::{CallKind, CallReceiver, Visibility};
use crate::config::{Config, PublicApiScope, TestCouplingConfig};
use crate::diagnostic::{Diagnostic, Span};
use crate::rules::Rule;
use std::collections::{HashMap, HashSet};

/// Diagnostic IDs emitted by [`TestCouplingRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestCouplingRuleId {
    TestOnlyProductionApi,
    TestCallsPrivateApi,
    TestAccessesPrivateField,
}

impl TestCouplingRuleId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnlyProductionApi => "test-only-production-api",
            Self::TestCallsPrivateApi => "test-calls-private-api",
            Self::TestAccessesPrivateField => "test-accesses-private-field",
        }
    }
}

/// Detects production code reshaped for tests and tests coupled to private implementation details:
/// - production functions/methods whose only callers are tests (`test-only-production-api`)
/// - tests calling private production functions or methods (`test-calls-private-api`)
/// - tests directly reading, mutating, or initializing private fields (`test-accesses-private-field`)
pub struct TestCouplingRule;

impl Rule for TestCouplingRule {
    fn id(&self) -> &'static str {
        "test-coupling"
    }

    fn description(&self) -> &'static str {
        "Detects test-only production APIs and tests coupled to private functions or fields"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.test_coupling;
        if !cfg.enabled {
            return Vec::new();
        }
        TestCouplingIndex::build(codebase).diagnose(cfg)
    }
}

#[derive(Default)]
struct ScopedCounter<'a> {
    by_mod: HashMap<(&'a ModulePath, &'a str), usize>,
    global: HashMap<&'a str, usize>,
}

impl<'a> ScopedCounter<'a> {
    fn record(&mut self, module: &'a ModulePath, ident: &'a str) {
        *self.by_mod.entry((module, ident)).or_insert(0) += 1;
        *self.global.entry(ident).or_insert(0) += 1;
    }

    fn count(&self, module: &ModulePath, ident: &str, private: bool) -> usize {
        if private {
            self.by_mod.get(&(module, ident)).copied().unwrap_or(0)
        } else {
            self.global.get(ident).copied().unwrap_or(0)
        }
    }
}

struct DiagAccumulator<'a> {
    seen_fields: HashSet<(TypeId, &'a str)>,
    diagnostics: Vec<Diagnostic>,
}

struct TestCouplingIndex<'a> {
    codebase: &'a Codebase,
    prod_free_by_mod_name: HashMap<(&'a ModulePath, &'a str), FuncId>,
    prod_free_by_name: HashMap<&'a str, Vec<FuncId>>,
    prod_methods_by_owner_name: HashMap<(TypeId, &'a str), FuncId>,
    test_free_by_file_name: HashSet<(FileId, &'a str)>,
    private_fields: HashMap<(TypeId, &'a str), &'a Span>,
    prod_idents: ScopedCounter<'a>,
    prod_scoped_idents: ScopedCounter<'a>,
    prod_unresolved_methods: ScopedCounter<'a>,
    prod_method_calls_by_owner: HashSet<(TypeId, &'a str)>,
}

impl<'a> TestCouplingIndex<'a> {
    fn build(codebase: &'a Codebase) -> Self {
        let mut index = Self {
            codebase,
            prod_free_by_mod_name: HashMap::new(),
            prod_free_by_name: HashMap::new(),
            prod_methods_by_owner_name: HashMap::new(),
            test_free_by_file_name: HashSet::new(),
            private_fields: HashMap::new(),
            prod_idents: ScopedCounter::default(),
            prod_scoped_idents: ScopedCounter::default(),
            prod_unresolved_methods: ScopedCounter::default(),
            prod_method_calls_by_owner: HashSet::new(),
        };
        index.index_types_and_files();
        index.index_functions();
        index
    }

    fn index_types_and_files(&mut self) {
        for ty in self.codebase.types() {
            if ty.decl.in_test_module || self.codebase.file(ty.file_id).is_test_file() {
                continue;
            }
            for field in &ty.decl.fields {
                if field.visibility == Visibility::Private {
                    self.private_fields
                        .insert((ty.id, field.name.as_str()), &field.span);
                }
            }
        }
        for file in self.codebase.files() {
            if file.is_test_file() {
                continue;
            }
            for ident in &file.top_level_idents {
                self.prod_idents.record(&file.module_path, ident.as_str());
                self.prod_scoped_idents
                    .record(&file.module_path, ident.as_str());
            }
        }
    }

    fn index_functions(&mut self) {
        for func in self.codebase.functions() {
            let is_test =
                func.decl.is_test_code() || self.codebase.file(func.file_id).is_test_file();
            if is_test {
                if func.decl.is_free_function() {
                    self.test_free_by_file_name
                        .insert((func.file_id, func.decl.name.as_str()));
                }
                continue;
            }

            if func.decl.is_free_function() {
                self.prod_free_by_mod_name
                    .insert((&func.module_path, func.decl.name.as_str()), func.id);
                self.prod_free_by_name
                    .entry(func.decl.name.as_str())
                    .or_default()
                    .push(func.id);
            } else if func.decl.is_inherent_method()
                && let Some(owner_id) = func.resolved_owner
            {
                self.prod_methods_by_owner_name
                    .insert((owner_id, func.decl.name.as_str()), func.id);
            }
            self.record_prod_usage(func);
        }
    }

    fn record_prod_usage(&mut self, func: &'a IndexedFunc) {
        for ident in &func.decl.referenced_idents {
            self.prod_idents.record(&func.module_path, ident.as_str());
        }
        for ident in &func.decl.scoped_ident_refs {
            self.prod_scoped_idents
                .record(&func.module_path, ident.as_str());
        }
        for (call_idx, call) in func.decl.calls.iter().enumerate() {
            if !matches!(call.kind, CallKind::Method | CallKind::Scoped) {
                continue;
            }
            let receiver_ty = func
                .resolved_call_receivers
                .get(call_idx)
                .copied()
                .flatten();
            if let Some(owner_id) = receiver_ty {
                let is_self_rec =
                    func.resolved_owner == Some(owner_id) && func.decl.name == call.callee;
                if !is_self_rec {
                    self.prod_method_calls_by_owner
                        .insert((owner_id, call.callee.as_str()));
                }
            } else if call.kind == CallKind::Method {
                self.prod_unresolved_methods
                    .record(&func.module_path, call.callee.as_str());
            }
        }
    }

    fn diagnose(&self, cfg: &TestCouplingConfig) -> Vec<Diagnostic> {
        let mut acc = DiagAccumulator {
            seen_fields: HashSet::new(),
            diagnostics: Vec::new(),
        };
        let mut test_callers: HashMap<FuncId, Vec<(&'a IndexedFunc, Span)>> = HashMap::new();
        let mut seen_call_targets: HashSet<FuncId> = HashSet::new();

        for func in self.codebase.functions() {
            let is_test =
                func.decl.is_test_code() || self.codebase.file(func.file_id).is_test_file();
            if !is_test {
                continue;
            }

            seen_call_targets.clear();
            for (call_idx, call) in func.decl.calls.iter().enumerate() {
                let Some(target_id) = self.resolve_prod_call(func, call_idx) else {
                    continue;
                };
                if !seen_call_targets.insert(target_id) {
                    continue;
                }
                test_callers
                    .entry(target_id)
                    .or_default()
                    .push((func, call.span.clone()));

                let target = self.codebase.func(target_id);
                if cfg.flag_private_test_coupling && target.decl.visibility == Visibility::Private {
                    let kind = if target.decl.is_inherent_method() {
                        "private method"
                    } else {
                        "private function"
                    };
                    let target_name = target.display_name();
                    acc.diagnostics.push(
                        Diagnostic::new(
                            TestCouplingRuleId::TestCallsPrivateApi.as_str(),
                            cfg.severity,
                            call.span.clone(),
                            format!(
                                "test `{}` directly calls {kind} `{target_name}`",
                                func.display_name()
                            ),
                        )
                        .with_related(
                            target.decl.span.clone(),
                            format!("{kind} `{target_name}` declared here"),
                        )
                        .with_suggestion(format!(
                            "Exercise `{target_name}` indirectly through the module's public behavior so internal refactors do not break tests."
                        )),
                    );
                }
            }

            if cfg.flag_private_test_coupling {
                self.collect_private_field_diags(func, cfg, &mut acc);
            }
        }

        if cfg.flag_test_only_production_api {
            self.check_test_only_prod_apis(&test_callers, cfg, &mut acc.diagnostics);
        }
        acc.diagnostics
    }

    fn collect_private_field_diags(
        &self,
        test_func: &'a IndexedFunc,
        cfg: &TestCouplingConfig,
        acc: &mut DiagAccumulator<'a>,
    ) {
        acc.seen_fields.clear();
        let init_action = format!(
            "initializes via {}",
            self.codebase.field_init_noun(test_func.file_id)
        );
        let field_events = test_func
            .decl
            .field_accesses
            .iter()
            .filter_map(|access| {
                let &type_id = test_func.resolved_local_types.get(&access.receiver_param)?;
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

        for (type_id, field_name, span, action) in field_events {
            let Some(&field_span) = self.private_fields.get(&(type_id, field_name)) else {
                continue;
            };
            if !acc.seen_fields.insert((type_id, field_name)) {
                continue;
            }
            let ty = self.codebase.ty(type_id);
            acc.diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestAccessesPrivateField.as_str(),
                    cfg.severity,
                    span.clone(),
                    format!(
                        "test `{}` directly {action} private field `{field_name}` of `{}`",
                        test_func.display_name(),
                        ty.decl.name
                    ),
                )
                .with_related(
                    field_span.clone(),
                    format!("private field `{field_name}` declared here"),
                )
                .with_suggestion(format!(
                    "Use `{}`'s public API/constructor instead of accessing private field `{field_name}` directly.",
                    ty.decl.name
                )),
            );
        }
    }

    fn check_test_only_prod_apis(
        &self,
        test_callers: &HashMap<FuncId, Vec<(&'a IndexedFunc, Span)>>,
        cfg: &TestCouplingConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let check_public =
            cfg.public_api_scope == PublicApiScope::All || self.codebase.is_binary_only_package();

        for func in self.codebase.functions() {
            if func.decl.is_test_code()
                || self.codebase.file(func.file_id).is_test_file()
                || func.decl.is_interface_impl
                || func.decl.is_entrypoint()
                || (func.decl.visibility == Visibility::Public && !check_public)
            {
                continue;
            }
            let Some(callers) = test_callers.get(&func.id) else {
                continue;
            };
            if callers.is_empty() || self.is_used_in_production(func) {
                continue;
            }

            let kind = if func.decl.is_inherent_method() {
                "method"
            } else {
                "function"
            };
            let mut diag = Diagnostic::new(
                TestCouplingRuleId::TestOnlyProductionApi.as_str(),
                cfg.severity,
                func.decl.span.clone(),
                format!(
                    "production {kind} `{}` has no production callers and is only called by {} test(s)",
                    func.display_name(),
                    callers.len()
                ),
            )
            .with_suggestion(
                "Remove test-only hooks from production code and test behavior through the real production entry points.",
            );
            for (caller, call_span) in callers {
                diag = diag.with_related(
                    call_span.clone(),
                    format!("only called from test `{}`", caller.display_name()),
                );
            }
            diagnostics.push(diag);
        }
    }

    fn is_used_in_production(&self, func: &IndexedFunc) -> bool {
        let is_private = func.decl.visibility == Visibility::Private;
        let name = func.decl.name.as_str();

        if func.decl.is_free_function() {
            let self_ref = usize::from(
                func.decl
                    .referenced_idents
                    .binary_search(&func.decl.name)
                    .is_ok(),
            );
            return self.prod_idents.count(&func.module_path, name, is_private) > self_ref;
        }

        if let Some(owner_id) = func.resolved_owner
            && self.prod_method_calls_by_owner.contains(&(owner_id, name))
        {
            return true;
        }
        if self
            .prod_unresolved_methods
            .count(&func.module_path, name, is_private)
            > 0
        {
            return true;
        }
        let self_scoped = usize::from(
            func.decl
                .scoped_ident_refs
                .binary_search(&func.decl.name)
                .is_ok(),
        );
        self.prod_scoped_idents
            .count(&func.module_path, name, is_private)
            > self_scoped
    }

    fn resolve_prod_call(&self, caller: &IndexedFunc, call_idx: usize) -> Option<FuncId> {
        let call = &caller.decl.calls[call_idx];
        let receiver_ty = caller
            .resolved_call_receivers
            .get(call_idx)
            .copied()
            .flatten();

        match call.kind {
            CallKind::Free => {
                if self
                    .test_free_by_file_name
                    .contains(&(caller.file_id, call.callee.as_str()))
                {
                    return None;
                }
                if let Some(&fid) = self
                    .prod_free_by_mod_name
                    .get(&(&caller.module_path, call.callee.as_str()))
                {
                    return Some(fid);
                }
                if self
                    .codebase
                    .file_imports(caller.file_id)
                    .contains(&call.callee)
                    && let Some(candidates) = self.prod_free_by_name.get(call.callee.as_str())
                    && candidates.len() == 1
                {
                    return Some(candidates[0]);
                }
                None
            }
            CallKind::SameModule => self
                .prod_free_by_mod_name
                .get(&(&caller.module_path, call.callee.as_str()))
                .copied(),
            CallKind::Method => self
                .prod_methods_by_owner_name
                .get(&(receiver_ty?, call.callee.as_str()))
                .copied(),
            CallKind::Scoped => {
                if let Some(owner_id) = receiver_ty {
                    return self
                        .prod_methods_by_owner_name
                        .get(&(owner_id, call.callee.as_str()))
                        .copied();
                }
                let Some(CallReceiver::Named(path)) = call.receiver.as_ref() else {
                    return None;
                };
                for &fid in self.prod_free_by_name.get(call.callee.as_str())? {
                    if self
                        .codebase
                        .func(fid)
                        .module_path
                        .matches_qualified_path(path)
                    {
                        return Some(fid);
                    }
                }
                None
            }
        }
    }
}
