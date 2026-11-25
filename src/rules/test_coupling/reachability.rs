use super::{CallerMap, FieldReaderMap, TestCouplingIndex, TestCouplingRuleId};
use crate::ast::ModulePath;
use crate::ast::codebase::{EffectiveVisibility, FuncId, IndexedFunc, TypeId};
use crate::ast::item::{CallKind, ConditionalGuard, TypeKind};
use crate::config::{PublicApiScope, TestCouplingConfig};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet, VecDeque};

type PropagatedCaller<'a> = (&'a IndexedFunc, Span, bool);
type EffectiveCallerMap<'a> = HashMap<FuncId, Vec<PropagatedCaller<'a>>>;

struct ProdCallerScopes<'a> {
    external_called: HashSet<FuncId>,
    same_mod_called: HashSet<FuncId>,
    calls_by_module: HashSet<(&'a ModulePath, FuncId)>,
}

pub(super) struct ReachabilityOutcome<'a> {
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) flagged_funcs: HashSet<FuncId>,
    pub(super) flagged_fields: HashSet<(TypeId, &'a str)>,
    pub(super) external_prod_called_funcs: HashSet<FuncId>,
    pub(super) same_mod_prod_called_funcs: HashSet<FuncId>,
    pub(super) prod_calls_by_module: HashSet<(&'a ModulePath, FuncId)>,
}

pub(super) struct ReachabilityChecker<'a> {
    index: &'a TestCouplingIndex<'a>,
    cfg: &'a TestCouplingConfig,
    check_public: bool,
    reachable: HashSet<FuncId>,
    queue: VecDeque<FuncId>,
    diagnostics: Vec<Diagnostic>,
    flagged_funcs: HashSet<FuncId>,
    flagged_fields: HashSet<(TypeId, &'a str)>,
}

impl<'a> ReachabilityChecker<'a> {
    pub(super) fn new(index: &'a TestCouplingIndex<'a>, cfg: &'a TestCouplingConfig) -> Self {
        let check_public = cfg.public_api_scope == PublicApiScope::All
            || index.codebase().is_binary_only_package();
        Self {
            index,
            cfg,
            check_public,
            reachable: HashSet::new(),
            queue: VecDeque::new(),
            diagnostics: Vec::new(),
            flagged_funcs: HashSet::new(),
            flagged_fields: HashSet::new(),
        }
    }

    pub(super) fn check(
        mut self,
        direct_test_callers: &CallerMap<'a>,
        test_field_readers: &FieldReaderMap<'a>,
    ) -> ReachabilityOutcome<'a> {
        self.compute_prod_reachable();
        let effective_callers = self.propagate_test_callers(direct_test_callers);
        let scopes = self.compute_prod_caller_scopes(&effective_callers);

        if self.cfg.core.flag_test_only_production_api || self.cfg.hooks.flag_production_test_hook {
            self.emit_test_only_api_diags(&effective_callers);
        }
        if self.cfg.core.flag_test_only_production_field || self.cfg.hooks.flag_production_test_hook
        {
            self.emit_test_only_field_diags(test_field_readers, &effective_callers);
        }
        ReachabilityOutcome {
            diagnostics: self.diagnostics,
            flagged_funcs: self.flagged_funcs,
            flagged_fields: self.flagged_fields,
            external_prod_called_funcs: scopes.external_called,
            same_mod_prod_called_funcs: scopes.same_mod_called,
            prod_calls_by_module: scopes.calls_by_module,
        }
    }

    fn compute_prod_reachable(&mut self) {
        let codebase = self.index.codebase();
        for func in codebase.functions() {
            if !self.index.is_test_func(func) && self.is_prod_root(func) {
                self.enqueue(func.id);
            }
        }
        for file in codebase.files() {
            if !file.is_test_file() {
                for ident in &file.top_level_idents {
                    self.seed_ident_roots(&file.module_path, ident.as_str());
                }
            }
        }
        while let Some(fid) = self.queue.pop_front() {
            let func = codebase.func(fid);
            for target_id in self.may_call_edges(func) {
                if target_id != fid {
                    self.enqueue(target_id);
                }
            }
        }
    }

    fn compute_prod_caller_scopes(
        &self,
        effective_callers: &EffectiveCallerMap<'a>,
    ) -> ProdCallerScopes<'a> {
        let codebase = self.index.codebase();
        let mut external_called = HashSet::new();
        let mut same_mod_called = HashSet::new();
        let mut calls_by_module = HashSet::new();

        for func in codebase.functions() {
            if self.index.is_test_func(func)
                || (!self.reachable.contains(&func.id) && effective_callers.contains_key(&func.id))
            {
                continue;
            }
            for target_id in self.resolved_call_edges(func) {
                if target_id != func.id
                    && func
                        .module_path
                        .is_same_or_child_of(&codebase.func(target_id).module_path)
                {
                    same_mod_called.insert(target_id);
                }
            }
            for target_id in self.may_call_edges(func) {
                if target_id == func.id {
                    continue;
                }
                calls_by_module.insert((&func.module_path, target_id));
                if !func
                    .module_path
                    .is_same_or_child_of(&codebase.func(target_id).module_path)
                {
                    external_called.insert(target_id);
                }
            }
        }

        for file in codebase.files() {
            if file.is_test_file() {
                continue;
            }
            for ident in &file.top_level_idents {
                for &fid in self
                    .index
                    .prod_free_in_mod(&file.module_path, ident.as_str())
                {
                    same_mod_called.insert(fid);
                    calls_by_module.insert((&file.module_path, fid));
                }
                for &fid in self
                    .index
                    .prod_free_named(ident.as_str())
                    .iter()
                    .chain(self.index.prod_methods_named(ident.as_str()))
                {
                    let target = codebase.func(fid);
                    if !self.index.is_callable_from(target, &file.module_path) {
                        continue;
                    }
                    calls_by_module.insert((&file.module_path, fid));
                    if !file.module_path.is_same_or_child_of(&target.module_path) {
                        external_called.insert(fid);
                    }
                }
            }
        }

        ProdCallerScopes {
            external_called,
            same_mod_called,
            calls_by_module,
        }
    }

    fn enqueue(&mut self, fid: FuncId) {
        if self.reachable.insert(fid) {
            self.queue.push_back(fid);
        }
    }

    fn is_prod_root(&self, func: &IndexedFunc) -> bool {
        if func.decl.role.is_entrypoint() {
            return true;
        }
        if func.decl.is_interface_impl && func.resolved_interface.is_none() {
            return true;
        }
        if func.decl.conditional_guard == ConditionalGuard::DebugOnly {
            return false;
        }
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        if func.decl.is_interface_impl {
            return func.resolved_interface.is_some_and(|iface_id| {
                !self.check_public
                    && surface.effective_type_visibility(codebase.ty(iface_id))
                        == EffectiveVisibility::Public
            });
        }
        !self.check_public
            && surface.effective_func_visibility(func, codebase.types())
                == EffectiveVisibility::Public
    }

    fn seed_ident_roots(&mut self, module_path: &'a ModulePath, ident: &'a str) {
        let codebase = self.index.codebase();
        for &fid in self.index.prod_free_in_mod(module_path, ident) {
            self.enqueue(fid);
        }
        for &fid in self
            .index
            .prod_free_named(ident)
            .iter()
            .chain(self.index.prod_methods_named(ident))
        {
            if self.index.is_callable_from(codebase.func(fid), module_path) {
                self.enqueue(fid);
            }
        }
    }

    /// Conservative over-approximation of outgoing production edges used for `ProdReachable`.
    fn may_call_edges(&self, func: &'a IndexedFunc) -> Vec<FuncId> {
        let mut edges = Vec::new();
        self.collect_may_call_edges(func, &mut edges);
        self.collect_member_ref_edges(func, &mut edges);
        self.collect_ident_edges(func, &mut edges);
        edges
    }

    /// Precise outgoing production edges (resolved calls + resolved owner interface impls)
    /// used when attributing transitive test callers to dead-in-prod islands.
    fn resolved_call_edges(&self, func: &'a IndexedFunc) -> Vec<FuncId> {
        let mut edges = Vec::new();
        for (call_idx, call) in func.decl.calls.iter().enumerate() {
            edges.extend(self.index.resolve_prod_call(func, call_idx));
            let receiver_ty = func
                .resolved_call_receivers
                .get(call_idx)
                .copied()
                .flatten();
            if let Some(owner_id) = receiver_ty
                && matches!(call.kind, CallKind::Method | CallKind::Scoped)
            {
                edges.extend_from_slice(
                    self.index
                        .prod_interface_methods(owner_id, call.callee.as_str()),
                );
                if self.index.codebase().ty(owner_id).decl.kind == TypeKind::Interface {
                    edges.extend_from_slice(
                        self.index
                            .prod_interface_impls_for_iface(owner_id, call.callee.as_str()),
                    );
                }
            }
        }
        edges
    }

    fn collect_may_call_edges(&self, func: &'a IndexedFunc, edges: &mut Vec<FuncId>) {
        let codebase = self.index.codebase();
        for (call_idx, call) in func.decl.calls.iter().enumerate() {
            let before = edges.len();
            edges.extend(self.index.resolve_prod_call(func, call_idx));

            let receiver_ty = func
                .resolved_call_receivers
                .get(call_idx)
                .copied()
                .flatten();
            let mut open_dispatch = false;
            if let Some(owner_id) = receiver_ty
                && matches!(call.kind, CallKind::Method | CallKind::Scoped)
            {
                edges.extend_from_slice(
                    self.index
                        .prod_interface_methods(owner_id, call.callee.as_str()),
                );
                if codebase.ty(owner_id).decl.kind == TypeKind::Interface {
                    open_dispatch = true;
                    edges.extend_from_slice(
                        self.index
                            .prod_interface_impls_for_iface(owner_id, call.callee.as_str()),
                    );
                }
            }

            let unresolved = edges.len() == before;
            if (unresolved || open_dispatch)
                && matches!(call.kind, CallKind::Method | CallKind::Scoped)
            {
                for &cand_id in self.index.prod_methods_named(call.callee.as_str()) {
                    if self
                        .index
                        .is_callable_from(codebase.func(cand_id), &func.module_path)
                    {
                        edges.push(cand_id);
                    }
                }
            }
        }
    }

    fn collect_member_ref_edges(&self, func: &'a IndexedFunc, edges: &mut Vec<FuncId>) {
        let codebase = self.index.codebase();
        for (idx, access) in func.decl.field_accesses.iter().enumerate() {
            let name = access.field_name.as_str();
            let mut fallback = true;
            if let Some(owner_id) = func.resolved_field_accesses.get(idx).copied().flatten() {
                let owner_methods = self.index.prod_methods_on_owner(owner_id, name);
                let iface_methods = self.index.prod_interface_methods(owner_id, name);
                edges.extend_from_slice(owner_methods);
                edges.extend_from_slice(iface_methods);
                fallback = owner_methods.is_empty()
                    && iface_methods.is_empty()
                    && !self.index.has_declared_field(owner_id, name);
            }
            if fallback {
                for &cand_id in self.index.prod_methods_named(name) {
                    if self
                        .index
                        .is_callable_from(codebase.func(cand_id), &func.module_path)
                    {
                        edges.push(cand_id);
                    }
                }
            }
        }
    }

    fn collect_ident_edges(&self, func: &'a IndexedFunc, edges: &mut Vec<FuncId>) {
        let codebase = self.index.codebase();
        for ident in &func.decl.referenced_idents {
            edges.extend_from_slice(
                self.index
                    .prod_free_in_mod(&func.module_path, ident.as_str()),
            );
            for &fid in self.index.prod_free_named(ident.as_str()) {
                if self
                    .index
                    .is_callable_from(codebase.func(fid), &func.module_path)
                {
                    edges.push(fid);
                }
            }
        }
        for ident in &func.decl.scoped_ident_refs {
            for &fid in self.index.prod_methods_named(ident.as_str()) {
                if self
                    .index
                    .is_callable_from(codebase.func(fid), &func.module_path)
                {
                    edges.push(fid);
                }
            }
        }
    }

    fn propagate_test_callers(
        &self,
        direct_test_callers: &CallerMap<'a>,
    ) -> EffectiveCallerMap<'a> {
        let codebase = self.index.codebase();
        let mut effective: EffectiveCallerMap<'a> = HashMap::new();
        let mut seen: HashSet<(FuncId, FuncId)> = HashSet::new();
        let mut queue: VecDeque<(FuncId, &'a IndexedFunc, Span)> = VecDeque::new();

        let mut seed_ids: Vec<FuncId> = direct_test_callers.keys().copied().collect();
        seed_ids.sort_unstable();

        for target_id in seed_ids {
            if self.reachable.contains(&target_id) {
                continue;
            }
            for (test_func, span, _single_target) in &direct_test_callers[&target_id] {
                if seen.insert((target_id, test_func.id)) {
                    effective
                        .entry(target_id)
                        .or_default()
                        .push((*test_func, span.clone(), true));
                    queue.push_back((target_id, *test_func, span.clone()));
                }
            }
        }

        let mut edge_cache: HashMap<FuncId, Vec<FuncId>> = HashMap::new();
        while let Some((prod_id, test_func, test_span)) = queue.pop_front() {
            let next_ids = edge_cache
                .entry(prod_id)
                .or_insert_with(|| self.resolved_call_edges(codebase.func(prod_id)));
            for &next_id in next_ids.iter() {
                if next_id == prod_id || self.reachable.contains(&next_id) {
                    continue;
                }
                if seen.insert((next_id, test_func.id)) {
                    effective.entry(next_id).or_default().push((
                        test_func,
                        test_span.clone(),
                        false,
                    ));
                    queue.push_back((next_id, test_func, test_span.clone()));
                }
            }
        }

        for callers in effective.values_mut() {
            callers.sort_by(|a, b| {
                a.0.id
                    .cmp(&b.0.id)
                    .then_with(|| a.1.start_line().cmp(&b.1.start_line()))
                    .then_with(|| a.1.start_col().cmp(&b.1.start_col()))
            });
        }
        effective
    }

    fn emit_test_only_api_diags(&mut self, effective_callers: &EffectiveCallerMap<'a>) {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        for func in codebase.functions() {
            let is_debug = func.decl.conditional_guard == ConditionalGuard::DebugOnly;
            let enabled = if is_debug {
                self.cfg.hooks.flag_production_test_hook
            } else {
                self.cfg.core.flag_test_only_production_api
            };
            if !enabled
                || self.index.is_test_func(func)
                || func.decl.is_interface_impl
                || func.decl.role.is_entrypoint()
                || (!is_debug
                    && !self.check_public
                    && surface.effective_func_visibility(func, codebase.types())
                        == EffectiveVisibility::Public)
                || self.reachable.contains(&func.id)
            {
                continue;
            }
            let Some(callers) = effective_callers.get(&func.id).filter(|c| !c.is_empty()) else {
                continue;
            };

            self.flagged_funcs.insert(func.id);
            let kind = if func.decl.is_inherent_method() {
                "method"
            } else {
                "function"
            };
            let (rule_id, prefix) = if is_debug {
                (TestCouplingRuleId::ProductionTestHook, "debug-guarded ")
            } else {
                (TestCouplingRuleId::TestOnlyProductionApi, "")
            };
            let mut diag = Diagnostic::new(
                rule_id.as_str(),
                self.cfg.severity,
                func.decl.span.clone(),
                format!(
                    "{prefix}production {kind} `{}` has no production callers and is only called by {} test(s)",
                    func.display_name(),
                    callers.len()
                ),
            )
            .with_suggestion(
                "Remove test-only hooks from production code and test behavior through the real production entry points.",
            );
            for (caller, call_span, is_direct) in callers {
                let rel_msg = if *is_direct {
                    format!("only called from test `{}`", caller.display_name())
                } else {
                    format!("transitively reached from test `{}`", caller.display_name())
                };
                diag = diag.with_related(call_span.clone(), rel_msg);
            }
            self.diagnostics.push(diag);
        }
    }

    fn emit_test_only_field_diags(
        &mut self,
        test_field_readers: &FieldReaderMap<'a>,
        effective_callers: &EffectiveCallerMap<'a>,
    ) {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        let mut prod_read_fields: HashSet<(TypeId, &str)> = HashSet::new();
        let mut prod_unresolved_field_names: HashSet<&str> = HashSet::new();

        for func in codebase.functions() {
            if self.index.is_test_func(func)
                || (!self.reachable.contains(&func.id) && effective_callers.contains_key(&func.id))
            {
                continue;
            }
            for (idx, access) in func.decl.field_accesses.iter().enumerate() {
                if !access.is_read {
                    continue;
                }
                let name = access.field_name.as_str();
                if let Some(type_id) = func.resolved_field_accesses.get(idx).copied().flatten()
                    && self.index.has_declared_field(type_id, name)
                {
                    prod_read_fields.insert((type_id, name));
                } else {
                    prod_unresolved_field_names.insert(name);
                }
            }
        }
        for file in codebase.files() {
            if !file.is_test_file() {
                for ident in &file.top_level_idents {
                    prod_unresolved_field_names.insert(ident.as_str());
                }
            }
        }

        for ty in codebase.types() {
            if self.index.is_test_type(ty) {
                continue;
            }
            for field in &ty.decl.fields {
                let Some(meta) = self.index.visible_field(ty.id, field.name.as_str()) else {
                    continue;
                };
                let is_debug = field.conditional_guard == ConditionalGuard::DebugOnly;
                let enabled = if is_debug {
                    self.cfg.hooks.flag_production_test_hook
                } else {
                    self.cfg.core.flag_test_only_production_field
                };
                if !enabled
                    || (!is_debug
                        && !self.check_public
                        && surface.effective_field_visibility(ty, field)
                            == EffectiveVisibility::Public)
                {
                    continue;
                }
                let key = (ty.id, field.name.as_str());
                let Some(readers) = test_field_readers.get(&key).filter(|r| !r.is_empty()) else {
                    continue;
                };
                if prod_read_fields.contains(&key)
                    || prod_unresolved_field_names.contains(field.name.as_str())
                {
                    continue;
                }

                self.flagged_fields.insert(key);
                let (rule_id, prefix) = if is_debug {
                    (TestCouplingRuleId::ProductionTestHook, "debug-guarded ")
                } else {
                    (TestCouplingRuleId::TestOnlyProductionField, "")
                };
                let mut diag = Diagnostic::new(
                    rule_id.as_str(),
                    self.cfg.severity,
                    meta.span.clone(),
                    format!(
                        "{prefix}production field `{}` on `{}` is never read in production and is only read by {} test(s)",
                        field.name,
                        ty.decl.name,
                        readers.len()
                    ),
                )
                .with_suggestion(
                    "Remove test-only state from production types and assert on observable outputs instead.",
                );
                for (reader, read_span) in readers {
                    diag = diag.with_related(
                        read_span.clone(),
                        format!("only read from test `{}`", reader.display_name()),
                    );
                }
                self.diagnostics.push(diag);
            }
        }
    }
}
