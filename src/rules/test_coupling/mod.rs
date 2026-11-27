mod escapes;
mod hooks;
mod imports;
mod mocking;
mod private_access;
mod reachability;
mod representation;
mod surface;

use self::escapes::check_encapsulation_escapes;
use self::hooks::check_production_test_hooks;
use self::mocking::MockingChecker;
use self::private_access::TestUsageCollector;
use self::reachability::ReachabilityChecker;
use self::representation::check_repr_assertions;
use self::surface::SurfaceChecker;
use crate::ast::ModulePath;
use crate::ast::codebase::{Codebase, FileId, FuncId, IndexedFunc, IndexedType, TypeId};
use crate::ast::item::{CallKind, CallReceiver, FieldDecl, ImportDecl, ImportKind, Visibility};
use crate::config::{Config, TestCouplingConfig};
use crate::diagnostic::{Diagnostic, Span};
use crate::rules::Rule;
use std::collections::{HashMap, HashSet};

/// Diagnostic IDs emitted by [`TestCouplingRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestCouplingRuleId {
    TestOnlyProductionApi,
    TestOnlyProductionField,
    TestCallsPrivateApi,
    TestAccessesPrivateField,
    TestBypassesPublicSurface,
    OverexposedForTests,
    ProductionTestHook,
    TestEncapsulationEscape,
    TestMocksInternalCollaborator,
    TestSpiesOnSut,
    InteractionOnlyTest,
    MockOnlyAbstraction,
    TestAssertsInternalRepresentation,
}

impl TestCouplingRuleId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnlyProductionApi => "test-only-production-api",
            Self::TestOnlyProductionField => "test-only-production-field",
            Self::TestCallsPrivateApi => "test-calls-private-api",
            Self::TestAccessesPrivateField => "test-accesses-private-field",
            Self::TestBypassesPublicSurface => "test-bypasses-public-surface",
            Self::OverexposedForTests => "overexposed-for-tests",
            Self::ProductionTestHook => "production-test-hook",
            Self::TestEncapsulationEscape => "test-encapsulation-escape",
            Self::TestMocksInternalCollaborator => "test-mocks-internal-collaborator",
            Self::TestSpiesOnSut => "test-spies-on-sut",
            Self::InteractionOnlyTest => "interaction-only-test",
            Self::MockOnlyAbstraction => "mock-only-abstraction",
            Self::TestAssertsInternalRepresentation => "test-asserts-internal-representation",
        }
    }
}

/// Detects production code reshaped for tests and tests coupled to private/internal details:
/// - production functions/methods whose only callers are tests (`test-only-production-api`)
/// - production fields written in production but read only by tests (`test-only-production-field`)
/// - tests calling private production functions or methods (`test-calls-private-api`)
/// - tests directly reading, mutating, or initializing private fields (`test-accesses-private-field`)
/// - tests bypassing a package's public surface to access internal code (`test-bypasses-public-surface`)
/// - internal APIs widened explicitly only for external tests (`overexposed-for-tests`)
/// - test-only conditional branches or debug-guarded hooks in production code (`production-test-hook`)
/// - runtime reflection or closed-class subclassing escapes in tests (`test-encapsulation-escape`)
/// - tests mocking internal modules, spying on the SUT, or asserting only interactions (`test-mocks-internal-collaborator`, `test-spies-on-sut`, `interaction-only-test`)
/// - single-implementation interfaces created only for test doubles (`mock-only-abstraction`)
/// - exact-equality assertions on debug representations exposing private fields (`test-asserts-internal-representation`)
pub struct TestCouplingRule;

impl Rule for TestCouplingRule {
    fn id(&self) -> &'static str {
        "test-coupling"
    }

    fn description(&self) -> &'static str {
        "Detects test-only production APIs/fields, private test coupling, surface bypasses, test hooks, and mocking anti-patterns"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.test_coupling;
        if !cfg.enabled {
            return Vec::new();
        }
        TestCouplingIndex::build(codebase, cfg.max_reexport_depth).diagnose(cfg)
    }
}

pub(super) type CallerMap<'a> = HashMap<FuncId, Vec<(&'a IndexedFunc, Span, bool)>>;
pub(super) type FieldReaderMap<'a> = HashMap<(TypeId, &'a str), Vec<(&'a IndexedFunc, Span)>>;

#[derive(Clone, Copy)]
pub(super) struct VisibleFieldMeta<'a> {
    pub(super) span: &'a Span,
}

pub(super) struct TestCouplingIndex<'a> {
    codebase: &'a Codebase,
    max_reexport_depth: usize,
    prod_files_by_mod: HashMap<&'a ModulePath, Vec<FileId>>,
    explicit_imports_by_file_local: HashMap<FileId, HashMap<&'a str, Vec<&'a ImportDecl>>>,
    has_inline_test_imports_by_file: HashSet<FileId>,
    prod_types_by_mod_name: HashMap<(&'a ModulePath, &'a str), TypeId>,
    prod_default_type_by_mod: HashMap<&'a ModulePath, TypeId>,
    prod_default_func_by_mod: HashMap<&'a ModulePath, FuncId>,
    prod_free_by_mod_name: HashMap<&'a ModulePath, HashMap<&'a str, Vec<FuncId>>>,
    prod_free_by_name: HashMap<&'a str, Vec<FuncId>>,
    prod_methods_by_owner_name: HashMap<(TypeId, &'a str), Vec<FuncId>>,
    prod_interface_methods_by_owner_name: HashMap<(TypeId, &'a str), Vec<FuncId>>,
    prod_interface_methods_by_iface_name: HashMap<(TypeId, &'a str), Vec<FuncId>>,
    prod_methods_by_name: HashMap<&'a str, Vec<FuncId>>,
    test_free_by_file_name: HashSet<(FileId, &'a str)>,
    prod_fields: HashMap<(TypeId, &'a str), &'a FieldDecl>,
    private_fields: HashMap<(TypeId, &'a str), &'a Span>,
    visible_fields: HashMap<(TypeId, &'a str), VisibleFieldMeta<'a>>,
}

impl<'a> TestCouplingIndex<'a> {
    fn build(codebase: &'a Codebase, max_reexport_depth: usize) -> Self {
        let mut index = Self {
            codebase,
            max_reexport_depth,
            prod_files_by_mod: HashMap::new(),
            explicit_imports_by_file_local: HashMap::new(),
            has_inline_test_imports_by_file: HashSet::new(),
            prod_types_by_mod_name: HashMap::new(),
            prod_default_type_by_mod: HashMap::new(),
            prod_default_func_by_mod: HashMap::new(),
            prod_free_by_mod_name: HashMap::new(),
            prod_free_by_name: HashMap::new(),
            prod_methods_by_owner_name: HashMap::new(),
            prod_interface_methods_by_owner_name: HashMap::new(),
            prod_interface_methods_by_iface_name: HashMap::new(),
            prod_methods_by_name: HashMap::new(),
            test_free_by_file_name: HashSet::new(),
            prod_fields: HashMap::new(),
            private_fields: HashMap::new(),
            visible_fields: HashMap::new(),
        };
        for (idx, file) in codebase.files().iter().enumerate() {
            let file_id = FileId(idx);
            if !file.is_test_file() {
                index
                    .prod_files_by_mod
                    .entry(&file.module_path)
                    .or_default()
                    .push(file_id);
            }
            for imp in &file.imports {
                if !file.is_test_file() && imp.in_test_module {
                    index.has_inline_test_imports_by_file.insert(file_id);
                }
                if matches!(imp.kind, ImportKind::Default | ImportKind::Named) {
                    index
                        .explicit_imports_by_file_local
                        .entry(file_id)
                        .or_default()
                        .entry(imp.local_name())
                        .or_default()
                        .push(imp);
                }
            }
        }
        index.index_types();
        index.index_functions();
        index
    }

    pub(super) const fn codebase(&self) -> &'a Codebase {
        self.codebase
    }

    pub(super) const fn max_reexport_depth(&self) -> usize {
        self.max_reexport_depth
    }

    pub(super) fn explicit_imports_for(
        &self,
        file_id: FileId,
        local_name: &str,
    ) -> &[&'a ImportDecl] {
        self.explicit_imports_by_file_local
            .get(&file_id)
            .and_then(|by_name| by_name.get(local_name))
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn file_has_inline_test_imports(&self, file_id: FileId) -> bool {
        self.has_inline_test_imports_by_file.contains(&file_id)
    }

    pub(super) fn has_prod_module(&self, module: &ModulePath) -> bool {
        self.prod_files_by_mod.contains_key(module)
    }

    pub(super) fn prod_files_in_mod(&self, module: &ModulePath) -> &[FileId] {
        self.prod_files_by_mod
            .get(module)
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn prod_type_in_mod(&self, module: &'a ModulePath, name: &'a str) -> Option<TypeId> {
        self.prod_types_by_mod_name.get(&(module, name)).copied()
    }

    pub(super) fn prod_default_type_in_mod(&self, module: &'a ModulePath) -> Option<TypeId> {
        self.prod_default_type_by_mod.get(module).copied()
    }

    pub(super) fn prod_default_func_in_mod(&self, module: &ModulePath) -> Option<FuncId> {
        self.prod_default_func_by_mod.get(module).copied()
    }

    pub(super) fn has_declared_field(&self, type_id: TypeId, field_name: &str) -> bool {
        self.prod_fields.contains_key(&(type_id, field_name))
    }

    pub(super) fn prod_field(&self, type_id: TypeId, field_name: &str) -> Option<&'a FieldDecl> {
        self.prod_fields.get(&(type_id, field_name)).copied()
    }

    pub(super) fn private_field_span(&self, type_id: TypeId, field_name: &str) -> Option<&'a Span> {
        self.private_fields.get(&(type_id, field_name)).copied()
    }

    pub(super) fn visible_field(
        &self,
        type_id: TypeId,
        field_name: &str,
    ) -> Option<VisibleFieldMeta<'a>> {
        self.visible_fields.get(&(type_id, field_name)).copied()
    }

    pub(super) fn prod_free_in_mod(&self, module: &ModulePath, name: &str) -> &[FuncId] {
        self.prod_free_by_mod_name
            .get(module)
            .and_then(|by_name| by_name.get(name))
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn prod_free_named(&self, name: &str) -> &[FuncId] {
        self.prod_free_by_name.get(name).map_or(&[], Vec::as_slice)
    }

    pub(super) fn prod_methods_on_owner(&self, owner: TypeId, name: &'a str) -> &[FuncId] {
        self.prod_methods_by_owner_name
            .get(&(owner, name))
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn prod_interface_methods(&self, owner: TypeId, name: &'a str) -> &[FuncId] {
        self.prod_interface_methods_by_owner_name
            .get(&(owner, name))
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn prod_interface_impls_for_iface(&self, iface: TypeId, name: &'a str) -> &[FuncId] {
        self.prod_interface_methods_by_iface_name
            .get(&(iface, name))
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn prod_methods_named(&self, name: &str) -> &[FuncId] {
        self.prod_methods_by_name
            .get(name)
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn is_test_func(&self, func: &IndexedFunc) -> bool {
        func.decl.role.is_test_code() || self.codebase.file(func.file_id).is_test_file()
    }

    pub(super) fn is_test_type(&self, ty: &IndexedType) -> bool {
        ty.decl.in_test_module || self.codebase.file(ty.file_id).is_test_file()
    }

    pub(super) fn is_callable_from(
        &self,
        target: &IndexedFunc,
        caller_module: &ModulePath,
    ) -> bool {
        if caller_module.is_same_or_child_of(&target.module_path) {
            return true;
        }
        if target.decl.is_interface_impl {
            return target.resolved_interface.is_none_or(|iface_id| {
                self.codebase.ty(iface_id).decl.visibility != Visibility::Private
            });
        }
        target.decl.visibility != Visibility::Private
    }

    fn index_types(&mut self) {
        for ty in self.codebase.types() {
            if self.is_test_type(ty) {
                continue;
            }
            self.prod_types_by_mod_name
                .insert((&ty.module_path, ty.decl.name.as_str()), ty.id);
            if ty.decl.export_binding.is_default() {
                self.prod_default_type_by_mod.insert(&ty.module_path, ty.id);
            }
            for field in &ty.decl.fields {
                self.prod_fields.insert((ty.id, field.name.as_str()), field);
                if field.visibility == Visibility::Private {
                    self.private_fields
                        .insert((ty.id, field.name.as_str()), &field.span);
                } else if !ty.decl.has_implicit_field_readers {
                    self.visible_fields.insert(
                        (ty.id, field.name.as_str()),
                        VisibleFieldMeta { span: &field.span },
                    );
                }
            }
        }
    }

    fn index_functions(&mut self) {
        for func in self.codebase.functions() {
            if self.is_test_func(func) {
                if func.decl.is_free_function() {
                    self.test_free_by_file_name
                        .insert((func.file_id, func.decl.name.as_str()));
                }
                continue;
            }

            if func.decl.is_free_function() {
                self.prod_free_by_mod_name
                    .entry(&func.module_path)
                    .or_default()
                    .entry(func.decl.name.as_str())
                    .or_default()
                    .push(func.id);
                self.prod_free_by_name
                    .entry(func.decl.name.as_str())
                    .or_default()
                    .push(func.id);
                if func.decl.export_binding.is_default() {
                    self.prod_default_func_by_mod
                        .insert(&func.module_path, func.id);
                }
            } else if func.decl.is_inherent_method() {
                if let Some(owner_id) = func.resolved_owner {
                    self.prod_methods_by_owner_name
                        .entry((owner_id, func.decl.name.as_str()))
                        .or_default()
                        .push(func.id);
                }
                self.prod_methods_by_name
                    .entry(func.decl.name.as_str())
                    .or_default()
                    .push(func.id);
            } else if func.decl.is_interface_impl {
                if let Some(owner_id) = func.resolved_owner {
                    self.prod_interface_methods_by_owner_name
                        .entry((owner_id, func.decl.name.as_str()))
                        .or_default()
                        .push(func.id);
                }
                if let Some(iface_id) = func.resolved_interface {
                    self.prod_interface_methods_by_iface_name
                        .entry((iface_id, func.decl.name.as_str()))
                        .or_default()
                        .push(func.id);
                }
                self.prod_methods_by_name
                    .entry(func.decl.name.as_str())
                    .or_default()
                    .push(func.id);
            }
        }
    }

    fn diagnose(&self, cfg: &TestCouplingConfig) -> Vec<Diagnostic> {
        let mut summary = TestUsageCollector::new(self, cfg).collect();
        let need_reachability = cfg.core.flag_test_only_production_api
            || cfg.core.flag_test_only_production_field
            || cfg.surface.flag_test_bypasses_public_surface
            || cfg.surface.flag_overexposed_for_tests
            || cfg.hooks.flag_production_test_hook;
        if need_reachability {
            let outcome = ReachabilityChecker::new(self, cfg)
                .check(&summary.direct_test_callers, &summary.test_field_readers);
            summary
                .diagnostics
                .extend(outcome.diagnostics.iter().cloned());
            if cfg.surface.flag_test_bypasses_public_surface
                || cfg.surface.flag_overexposed_for_tests
            {
                let surface_diags =
                    SurfaceChecker::new(self, cfg, &outcome).check(&summary.direct_test_callers);
                summary.diagnostics.extend(surface_diags);
            }
        }
        if cfg.hooks.flag_production_test_hook {
            summary
                .diagnostics
                .extend(check_production_test_hooks(self, cfg));
        }
        if cfg.hooks.flag_test_encapsulation_escape {
            summary
                .diagnostics
                .extend(check_encapsulation_escapes(self, cfg));
        }
        if cfg.hooks.flag_test_asserts_internal_representation {
            summary.diagnostics.extend(check_repr_assertions(self, cfg));
        }
        let need_mocking = cfg.mocking.flag_test_mocks_internal_collaborator
            || cfg.mocking.flag_test_spies_on_sut
            || cfg.mocking.flag_interaction_only_test
            || cfg.abstractions.flag_mock_only_abstraction;
        if need_mocking {
            summary
                .diagnostics
                .extend(MockingChecker::new(self, cfg).check());
        }
        summary.diagnostics
    }

    pub(super) fn resolve_prod_call(
        &self,
        caller: &'a IndexedFunc,
        call_idx: usize,
    ) -> Vec<FuncId> {
        let call = &caller.decl.calls[call_idx];
        let receiver_ty = caller
            .resolved_call_receivers
            .get(call_idx)
            .copied()
            .flatten();

        match call.kind {
            CallKind::Free => {
                if self.is_test_func(caller)
                    && self
                        .test_free_by_file_name
                        .contains(&(caller.file_id, call.callee.as_str()))
                {
                    return Vec::new();
                }
                if let Some(imported) = imports::ImportResolver::new(self, self.max_reexport_depth)
                    .resolve_imported_free_funcs(
                        caller.file_id,
                        self.is_test_func(caller),
                        call.callee.as_str(),
                    )
                {
                    return imported.funcs;
                }
                let in_mod = self.prod_free_in_mod(&caller.module_path, call.callee.as_str());
                if !in_mod.is_empty() {
                    return in_mod.to_vec();
                }
                if self
                    .codebase
                    .file_imports(caller.file_id)
                    .contains(&call.callee)
                    && let Some(candidates) = self.prod_free_by_name.get(call.callee.as_str())
                    && candidates.len() == 1
                {
                    return vec![candidates[0]];
                }
                Vec::new()
            }
            CallKind::SameModule => self
                .prod_free_in_mod(&caller.module_path, call.callee.as_str())
                .to_vec(),
            CallKind::Method => receiver_ty.map_or_else(Vec::new, |owner_id| {
                self.prod_methods_by_owner_name
                    .get(&(owner_id, call.callee.as_str()))
                    .cloned()
                    .unwrap_or_default()
            }),
            CallKind::Scoped => {
                if let Some(owner_id) = receiver_ty {
                    return self
                        .prod_methods_by_owner_name
                        .get(&(owner_id, call.callee.as_str()))
                        .cloned()
                        .unwrap_or_default();
                }
                let Some(CallReceiver::Named(path)) = call.receiver.as_ref() else {
                    return Vec::new();
                };
                let target_mod = ModulePath::new(path.split("::").map(String::from).collect());
                if call.callee == "default" {
                    let resolved = imports::ImportResolver::new(self, self.max_reexport_depth)
                        .resolve_module_export(&target_mod, "default", true);
                    if !resolved.is_empty() {
                        return resolved;
                    }
                }
                let direct: Vec<FuncId> = self
                    .prod_free_named(call.callee.as_str())
                    .iter()
                    .copied()
                    .filter(|&fid| {
                        self.codebase
                            .func(fid)
                            .module_path
                            .matches_qualified_path(path)
                    })
                    .collect();
                if !direct.is_empty() {
                    return direct;
                }
                if self.has_prod_module(&target_mod) {
                    return imports::ImportResolver::new(self, self.max_reexport_depth)
                        .resolve_module_export(&target_mod, call.callee.as_str(), false);
                }
                Vec::new()
            }
        }
    }
}
