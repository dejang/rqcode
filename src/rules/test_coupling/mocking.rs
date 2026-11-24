use super::{TestCouplingIndex, TestCouplingRuleId};
use crate::ast::ModulePath;
use crate::ast::codebase::{
    EffectiveVisibility, FileId, FuncId, IndexedFunc, IndexedType, ResolvedInterfaceImpl, TypeId,
};
use crate::ast::item::{
    CallKind, CallReceiver, ImportOrigin, ModuleMock, SpySite, TypeKind, Visibility,
};
use crate::config::{PublicApiScope, TestCouplingConfig};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet, VecDeque};

struct SpyReachability<'a> {
    owner_self_calls: HashMap<(TypeId, &'a str), Vec<&'a str>>,
    bfs: BfsScratch<&'a str>,
}

struct BfsScratch<T> {
    visited: HashSet<T>,
    queue: VecDeque<T>,
}

impl<T> Default for BfsScratch<T> {
    fn default() -> Self {
        Self {
            visited: HashSet::new(),
            queue: VecDeque::new(),
        }
    }
}

#[derive(Default)]
struct ProdImplScratch<'b> {
    seen_targets: HashSet<TypeId>,
    seen_unresolved: HashSet<&'b str>,
    subclass_queue: VecDeque<TypeId>,
}

pub(super) struct MockingChecker<'a> {
    index: &'a TestCouplingIndex<'a>,
    cfg: &'a TestCouplingConfig,
    external_boundary_files: HashSet<FileId>,
    external_prod_modules: HashSet<&'a ModulePath>,
    prod_files_by_module: HashMap<&'a ModulePath, Vec<FileId>>,
    files_with_internal_mocks: HashSet<FileId>,
    files_with_external_mocks: HashSet<FileId>,
    funcs_with_sut_spy: HashSet<FuncId>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> MockingChecker<'a> {
    pub(super) fn new(index: &'a TestCouplingIndex<'a>, cfg: &'a TestCouplingConfig) -> Self {
        let codebase = index.codebase();
        let mut external_boundary_files = HashSet::new();
        let mut external_prod_modules = HashSet::new();
        let mut prod_files_by_module: HashMap<&'a ModulePath, Vec<FileId>> = HashMap::new();

        for (file_idx, file) in codebase.files().iter().enumerate() {
            if file.is_test_file() {
                continue;
            }
            let file_id = FileId(file_idx);
            prod_files_by_module
                .entry(&file.module_path)
                .or_default()
                .push(file_id);
            for imp in &file.imports {
                if !imp.in_test_module && imp.origin == ImportOrigin::ExternalBoundary {
                    external_boundary_files.insert(file_id);
                    external_prod_modules.insert(&file.module_path);
                    break;
                }
            }
        }
        for func in codebase.functions() {
            if !index.is_test_func(func) && func.decl.coupling.has_ambient_io {
                external_boundary_files.insert(func.file_id);
                external_prod_modules.insert(&codebase.file(func.file_id).module_path);
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            for (file_idx, file) in codebase.files().iter().enumerate() {
                let file_id = FileId(file_idx);
                if file.is_test_file()
                    || external_boundary_files.contains(&file_id)
                    || !(file.is_barrel || (file.types.is_empty() && file.functions.is_empty()))
                {
                    continue;
                }
                let mut reexports_external = false;
                for exp in &file.exports {
                    if let Some(src) = exp.source_module.as_ref()
                        && external_prod_modules.contains(src)
                    {
                        reexports_external = true;
                        break;
                    }
                }
                if reexports_external {
                    external_boundary_files.insert(file_id);
                    external_prod_modules.insert(&file.module_path);
                    changed = true;
                }
            }
        }

        Self {
            index, cfg, external_boundary_files, external_prod_modules, prod_files_by_module,
            files_with_internal_mocks: HashSet::new(), files_with_external_mocks: HashSet::new(),
            funcs_with_sut_spy: HashSet::new(), diagnostics: Vec::new(),
        }
    }

    pub(super) fn check(mut self) -> Vec<Diagnostic> {
        if self.cfg.mocking.flag_test_mocks_internal_collaborator
            || self.cfg.mocking.flag_interaction_only_test
        {
            self.check_module_mocks();
        }
        if self.cfg.mocking.flag_test_spies_on_sut || self.cfg.mocking.flag_interaction_only_test {
            self.check_spy_sites();
        }
        if self.cfg.mocking.flag_interaction_only_test {
            self.check_interaction_only_tests();
        }
        if self.cfg.abstractions.flag_mock_only_abstraction {
            self.check_mock_only_abstractions();
        }
        self.diagnostics
    }

    fn check_module_mocks(&mut self) {
        let codebase = self.index.codebase();
        for (file_idx, file) in codebase.files().iter().enumerate() {
            let file_id = FileId(file_idx);
            let is_test_file = file.is_test_file();
            for mock in &file.module_mocks {
                if is_test_file || mock.in_test_context {
                    self.record_module_mock(file_id, mock);
                }
            }
        }
        for func in codebase.functions() {
            if !self.index.is_test_func(func) {
                continue;
            }
            for mock in &func.decl.coupling.module_mocks {
                self.record_module_mock(func.file_id, mock);
            }
        }
    }

    fn record_module_mock(&mut self, file_id: FileId, mock: &ModuleMock) {
        let Some(target_mod) = mock.resolved_module.as_ref() else {
            if mock.origin == ImportOrigin::ExternalBoundary {
                self.files_with_external_mocks.insert(file_id);
            }
            return;
        };
        let Some(prod_files) = self.prod_files_by_module.get(target_mod) else {
            return;
        };
        if self.external_prod_modules.contains(target_mod) {
            self.files_with_external_mocks.insert(file_id);
            return;
        }
        self.files_with_internal_mocks.insert(file_id);
        if !self.cfg.mocking.flag_test_mocks_internal_collaborator {
            return;
        }

        let codebase = self.index.codebase();
        let mut diag = Diagnostic::new(
            TestCouplingRuleId::TestMocksInternalCollaborator.as_str(),
            self.cfg.severity,
            mock.span.clone(),
            format!(
                "test mocks internal production module `{}` instead of an external boundary",
                mock.raw_specifier
            ),
        )
        .with_suggestion(
            "Mock only external I/O boundaries and exercise internal collaborators directly.",
        );
        for &prod_fid in prod_files {
            let target_file = codebase.file(prod_fid);
            let rel_span = target_file
                .types
                .first()
                .map(|t| t.span.clone())
                .or_else(|| target_file.functions.first().map(|f| f.span.clone()))
                .unwrap_or_else(|| {
                    Span::new(target_file.relative_path.display().to_string(), 1, 1, 1, 1)
                });
            diag = diag.with_related(
                rel_span,
                format!("internal module `{target_mod}` defined here"),
            );
        }
        self.diagnostics.push(diag);
    }

    fn check_spy_sites(&mut self) {
        let codebase = self.index.codebase();
        let mut reach = SpyReachability {
            owner_self_calls: self.build_owner_self_calls(),
            bfs: BfsScratch::default(),
        };

        for func in codebase.functions() {
            if !self.index.is_test_func(func) || func.decl.coupling.spy_sites.is_empty() {
                continue;
            }
            for (idx, spy) in func.decl.coupling.spy_sites.iter().enumerate() {
                let Some(tid) = func.resolved_spy_targets.get(idx).copied().flatten() else {
                    continue;
                };
                if !self.is_sut_spy_site(func, spy, tid, &mut reach) {
                    continue;
                }
                self.funcs_with_sut_spy.insert(func.id);
                if self.cfg.mocking.flag_test_spies_on_sut {
                    self.emit_sut_spy_diag(func, spy, codebase.ty(tid));
                }
            }
        }
    }

    fn build_owner_self_calls(&self) -> HashMap<(TypeId, &'a str), Vec<&'a str>> {
        let mut owner_self_calls: HashMap<(TypeId, &'a str), Vec<&'a str>> = HashMap::new();
        for func in self.index.codebase().functions() {
            if self.index.is_test_func(func) || !func.decl.is_inherent_method() {
                continue;
            }
            let Some(owner_id) = func.resolved_owner else {
                continue;
            };
            for call in &func.decl.calls {
                if call.kind == CallKind::Method && call.targets_self_owner() {
                    owner_self_calls
                        .entry((owner_id, func.decl.name.as_str()))
                        .or_default()
                        .push(call.callee.as_str());
                }
            }
        }
        owner_self_calls
    }

    fn is_sut_spy_site(
        &self,
        func: &'a IndexedFunc,
        spy: &'a SpySite,
        tid: TypeId,
        reach: &mut SpyReachability<'a>,
    ) -> bool {
        let codebase = self.index.codebase();
        let ty = codebase.ty(tid);
        if self.index.is_test_type(ty) || self.external_boundary_files.contains(&ty.file_id) {
            return false;
        }
        let mut spied_non_public = false;
        for &mid in self
            .index
            .prod_methods_on_owner(tid, spy.method_name.as_str())
        {
            if codebase.func(mid).decl.visibility != Visibility::Public {
                spied_non_public = true;
                break;
            }
        }
        let match_exact_recv = matches!(
            spy.target_receiver.as_ref(),
            Some(CallReceiver::Named(name)) if name != &ty.decl.name
        );
        for (call_idx, call) in func.decl.calls.iter().enumerate() {
            if func
                .resolved_call_receivers
                .get(call_idx)
                .copied()
                .flatten()
                != Some(tid)
                || call.callee == spy.method_name
                || (match_exact_recv && call.receiver != spy.target_receiver)
            {
                continue;
            }
            if spied_non_public {
                return true;
            }
            reach.bfs.visited.clear();
            reach.bfs.queue.clear();
            reach.bfs.visited.insert(call.callee.as_str());
            reach.bfs.queue.push_back(call.callee.as_str());
            while let Some(cur) = reach.bfs.queue.pop_front() {
                if let Some(callees) = reach.owner_self_calls.get(&(tid, cur)) {
                    for &next in callees {
                        if next == spy.method_name {
                            return true;
                        }
                        if reach.bfs.visited.insert(next) {
                            reach.bfs.queue.push_back(next);
                        }
                    }
                }
            }
        }
        false
    }

    fn emit_sut_spy_diag(&mut self, func: &IndexedFunc, spy: &SpySite, ty: &IndexedType) {
        let codebase = self.index.codebase();
        let related_span = self
            .index
            .prod_methods_on_owner(ty.id, spy.method_name.as_str())
            .first()
            .map_or_else(
                || ty.decl.span.clone(),
                |&mid| codebase.func(mid).decl.span.clone(),
            );
        self.diagnostics.push(
            Diagnostic::new(
                TestCouplingRuleId::TestSpiesOnSut.as_str(),
                self.cfg.severity,
                spy.span.clone(),
                format!(
                    "test `{}` spies on method `{}` of system-under-test `{}`",
                    func.display_name(),
                    spy.method_name,
                    ty.decl.name
                ),
            )
            .with_related(
                related_span,
                format!("method `{}` on `{}` declared here", spy.method_name, ty.decl.name),
            )
            .with_suggestion(
                "Assert on observable return values or state transitions instead of spying on the system under test's own methods.",
            ),
        );
    }

    fn check_interaction_only_tests(&mut self) {
        let codebase = self.index.codebase();
        let mut helpers_by_file_name: HashMap<(FileId, &'a str), Vec<FuncId>> = HashMap::new();
        for func in codebase.functions() {
            if self.index.is_test_func(func) && !func.decl.is_test() && func.decl.is_free_function()
            {
                helpers_by_file_name
                    .entry((func.file_id, func.decl.name.as_str()))
                    .or_default()
                    .push(func.id);
            }
        }

        let mut scratch: BfsScratch<FuncId> = BfsScratch::default();
        for func in codebase.functions() {
            if !func.decl.is_test() {
                continue;
            }
            let assertions = func.decl.coupling.assertions;
            if assertions.interaction_count == 0
                || assertions.behavioral_count > 0
                || self.helper_has_behavioral_assertion(func, &helpers_by_file_name, &mut scratch)
            {
                continue;
            }
            let coupled_signal = self.funcs_with_sut_spy.contains(&func.id)
                || self.files_with_internal_mocks.contains(&func.file_id);
            if !coupled_signal {
                continue;
            }
            self.diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::InteractionOnlyTest.as_str(),
                    self.cfg.severity,
                    func.decl.span.clone(),
                    format!(
                        "test `{}` only asserts mock/spy interactions ({} interaction assertion(s), 0 behavioral assertions)",
                        func.display_name(),
                        assertions.interaction_count
                    ),
                )
                .with_suggestion(
                    "Assert on observable outputs, return values, or state changes rather than only verifying call interactions.",
                ),
            );
        }
    }

    fn helper_has_behavioral_assertion(
        &self,
        root: &IndexedFunc,
        helpers_by_file_name: &HashMap<(FileId, &'a str), Vec<FuncId>>,
        scratch: &mut BfsScratch<FuncId>,
    ) -> bool {
        let codebase = self.index.codebase();
        scratch.visited.clear();
        scratch.queue.clear();
        scratch.visited.insert(root.id);
        scratch.queue.push_back(root.id);
        while let Some(fid) = scratch.queue.pop_front() {
            let cur = codebase.func(fid);
            for call in &cur.decl.calls {
                if !matches!(call.kind, CallKind::Free | CallKind::SameModule) {
                    continue;
                }
                if let Some(cands) = helpers_by_file_name.get(&(cur.file_id, call.callee.as_str()))
                {
                    for &hid in cands {
                        if codebase.func(hid).decl.coupling.assertions.behavioral_count > 0 {
                            return true;
                        }
                        if scratch.visited.insert(hid) {
                            scratch.queue.push_back(hid);
                        }
                    }
                }
            }
        }
        false
    }

    fn check_mock_only_abstractions(&mut self) {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        let check_public =
            self.cfg.public_api_scope == PublicApiScope::All || codebase.is_binary_only_package();
        let subclasses_by_base = self.build_prod_subclasses();

        let mut prod_impls_by_iface: HashMap<TypeId, Vec<&ResolvedInterfaceImpl>> = HashMap::new();
        let mut test_impls_by_iface: HashMap<TypeId, Vec<&ResolvedInterfaceImpl>> = HashMap::new();
        for imp in codebase.interface_impls() {
            let Some(iface_id) = imp.interface_id else {
                continue;
            };
            let is_test = imp.in_test_module
                || codebase.file(imp.file_id).is_test_file()
                || imp
                    .target_id
                    .is_some_and(|tid| self.index.is_test_type(codebase.ty(tid)));
            if is_test {
                test_impls_by_iface.entry(iface_id).or_default().push(imp);
            } else {
                prod_impls_by_iface.entry(iface_id).or_default().push(imp);
            }
        }

        let mut scratch = ProdImplScratch::default();
        for ty in codebase.types() {
            if ty.decl.kind != TypeKind::Interface || self.index.is_test_type(ty) {
                continue;
            }
            let ty_eff = surface.effective_type_visibility(ty);
            if !check_public && ty_eff == EffectiveVisibility::Public {
                continue;
            }
            let (Some(prod_impls), Some(test_impls)) = (
                prod_impls_by_iface.get(&ty.id),
                test_impls_by_iface.get(&ty.id).filter(|v| !v.is_empty()),
            ) else {
                continue;
            };
            if self.is_sealed_interface(ty, ty_eff) {
                continue;
            }
            let Some(sole_prod) =
                self.find_sole_internal_prod_impl(prod_impls, &subclasses_by_base, &mut scratch)
            else {
                continue;
            };
            self.emit_mock_only_diag(ty, sole_prod, test_impls);
        }
    }

    fn build_prod_subclasses(&self) -> HashMap<TypeId, Vec<TypeId>> {
        let codebase = self.index.codebase();
        let mut subclasses_by_base: HashMap<TypeId, Vec<TypeId>> = HashMap::new();
        for ty in codebase.types() {
            if self.index.is_test_type(ty) || ty.decl.kind != TypeKind::Class {
                continue;
            }
            for &base_id in &ty.resolved_base_types {
                let base_ty = codebase.ty(base_id);
                if !self.index.is_test_type(base_ty) && base_ty.decl.kind == TypeKind::Class {
                    subclasses_by_base.entry(base_id).or_default().push(ty.id);
                }
            }
        }
        subclasses_by_base
    }

    fn is_sealed_interface(&self, ty: &IndexedType, ty_eff: EffectiveVisibility) -> bool {
        let codebase = self.index.codebase();
        let surface = codebase.public_surface();
        for &base_id in &ty.resolved_base_types {
            if surface.effective_type_visibility(codebase.ty(base_id)) < ty_eff {
                return true;
            }
        }
        false
    }

    fn find_sole_internal_prod_impl<'b>(
        &self,
        prod_impls: &[&'b ResolvedInterfaceImpl],
        subclasses_by_base: &HashMap<TypeId, Vec<TypeId>>,
        scratch: &mut ProdImplScratch<'b>,
    ) -> Option<&'b ResolvedInterfaceImpl> {
        scratch.seen_targets.clear();
        scratch.seen_unresolved.clear();
        scratch.subclass_queue.clear();
        let mut prod_count = 0usize;
        let mut sole_prod: Option<&'b ResolvedInterfaceImpl> = None;

        for &imp in prod_impls {
            if let Some(tid) = imp.target_id {
                if scratch.seen_targets.insert(tid) {
                    prod_count += 1;
                    if sole_prod.is_none() {
                        sole_prod = Some(imp);
                    }
                    scratch.subclass_queue.push_back(tid);
                    while let Some(cur) = scratch.subclass_queue.pop_front() {
                        if let Some(subs) = subclasses_by_base.get(&cur) {
                            for &sub_id in subs {
                                if scratch.seen_targets.insert(sub_id) {
                                    prod_count += 1;
                                    scratch.subclass_queue.push_back(sub_id);
                                }
                            }
                        }
                    }
                }
            } else if scratch
                .seen_unresolved
                .insert(imp.target_type_name.as_str())
            {
                prod_count += 1;
                if sole_prod.is_none() {
                    sole_prod = Some(imp);
                }
            }
        }
        let sole = (prod_count == 1).then_some(sole_prod).flatten()?;
        let sole_tid = sole.target_id?;
        let codebase = self.index.codebase();
        if self.external_boundary_files.contains(&sole.file_id)
            || self
                .external_boundary_files
                .contains(&codebase.ty(sole_tid).file_id)
        {
            return None;
        }
        Some(sole)
    }

    fn emit_mock_only_diag(
        &mut self,
        ty: &IndexedType,
        sole_prod: &ResolvedInterfaceImpl,
        test_impls: &[&ResolvedInterfaceImpl],
    ) {
        let mut diag = Diagnostic::new(
            TestCouplingRuleId::MockOnlyAbstraction.as_str(),
            self.cfg.severity,
            ty.decl.span.clone(),
            format!(
                "interface `{}` has only 1 production implementation and {} test implementation(s), suggesting a mock-only abstraction",
                ty.decl.name,
                test_impls.len()
            ),
        )
        .with_related(
            sole_prod.span.clone(),
            format!("sole production implementation of `{}`", ty.decl.name),
        )
        .with_suggestion(
            "Remove single-implementation abstractions created solely for mocking, or reserve abstractions for external I/O boundaries.",
        );
        for test_imp in test_impls {
            diag = diag.with_related(
                test_imp.span.clone(),
                format!("test implementation of `{}`", ty.decl.name),
            );
        }
        self.diagnostics.push(diag);
    }
}
