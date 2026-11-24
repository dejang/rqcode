use super::model::{
    FileId, FuncId, IndexedFunc, IndexedType, ResolvedFieldInit, ResolvedInterfaceImpl,
    ResolvedParam, TypeId,
};
use super::resolve::TypeResolver;
use crate::ast::item::{CallKind, CallReceiver, FunctionDecl, InterfaceImplRole, TypeKind};
use crate::ast::{FileAst, ModulePath};
use std::collections::{HashMap, HashSet};

/// Binds function declarations, local variables, and call receivers to resolved [`TypeId`]s.
pub(super) struct FunctionBinder<'a> {
    resolver: TypeResolver<'a>,
}

struct ReturnTypeIndex {
    prod_in_file: HashMap<(FileId, String), TypeId>,
    test_in_file: HashMap<(FileId, String), TypeId>,
    in_module: HashMap<(ModulePath, String), TypeId>,
    global: HashMap<String, Option<TypeId>>,
    method: HashMap<(TypeId, String), TypeId>,
}

impl ReturnTypeIndex {
    fn build(functions: &[IndexedFunc]) -> Self {
        let mut prod_in_file: HashMap<(FileId, String), TypeId> = HashMap::new();
        let mut test_in_file: HashMap<(FileId, String), TypeId> = HashMap::new();
        let mut in_module: HashMap<(ModulePath, String), TypeId> = HashMap::new();
        let mut global: HashMap<String, Option<TypeId>> = HashMap::new();
        let mut method: HashMap<(TypeId, String), TypeId> = HashMap::new();

        for func in functions {
            let Some(ret_ty) = func.resolved_return_type else {
                continue;
            };
            if func.decl.is_free_function() {
                if func.decl.is_test_code() {
                    test_in_file.insert((func.file_id, func.decl.name.clone()), ret_ty);
                } else {
                    prod_in_file.insert((func.file_id, func.decl.name.clone()), ret_ty);
                    in_module.insert((func.module_path.clone(), func.decl.name.clone()), ret_ty);
                    global
                        .entry(func.decl.name.clone())
                        .and_modify(|slot| {
                            if *slot != Some(ret_ty) {
                                *slot = None;
                            }
                        })
                        .or_insert(Some(ret_ty));
                }
            } else if let Some(owner_id) = func.resolved_owner
                && !func.decl.is_test_code()
            {
                method.insert((owner_id, func.decl.name.clone()), ret_ty);
            }
        }

        Self {
            prod_in_file,
            test_in_file,
            in_module,
            global,
            method,
        }
    }

    fn lookup_free(
        &self,
        file_id: FileId,
        module: &ModulePath,
        name: &str,
        is_test_caller: bool,
    ) -> Option<TypeId> {
        let key = (file_id, name.to_string());
        if is_test_caller
            && let Some(&tid) = self.test_in_file.get(&key)
        {
            return Some(tid);
        }
        self.prod_in_file
            .get(&key)
            .copied()
            .or_else(|| {
                self.in_module
                    .get(&(module.clone(), name.to_string()))
                    .copied()
            })
            .or_else(|| self.global.get(name).copied().flatten())
    }

    fn lookup_method(&self, owner: TypeId, name: &str) -> Option<TypeId> {
        self.method.get(&(owner, name.to_string())).copied()
    }
}

struct FreeFuncTable {
    prod_by_file: HashMap<(FileId, String), FuncId>,
    test_by_file: HashMap<(FileId, String), FuncId>,
}

impl FreeFuncTable {
    fn build(functions: &[IndexedFunc]) -> Self {
        let mut prod_by_file = HashMap::new();
        let mut test_by_file = HashMap::new();
        for func in functions {
            if !func.decl.is_free_function() {
                continue;
            }
            if func.decl.is_test_code() {
                test_by_file.insert((func.file_id, func.decl.name.clone()), func.id);
            } else {
                prod_by_file.insert((func.file_id, func.decl.name.clone()), func.id);
            }
        }
        Self {
            prod_by_file,
            test_by_file,
        }
    }

    fn lookup(&self, caller: &IndexedFunc, name: &str) -> Option<FuncId> {
        let key = (caller.file_id, name.to_string());
        if caller.decl.is_test_code()
            && let Some(&fid) = self.test_by_file.get(&key)
        {
            return Some(fid);
        }
        self.prod_by_file.get(&key).copied()
    }
}

impl<'a> FunctionBinder<'a> {
    pub(super) fn new(files: &'a [FileAst], types: &'a [IndexedType]) -> Self {
        Self {
            resolver: TypeResolver::new(files, types),
        }
    }

    pub(super) fn bind_func(
        &self,
        id: FuncId,
        file_id: FileId,
        file: &FileAst,
        decl: &FunctionDecl,
    ) -> IndexedFunc {
        let m = &file.module_path;
        let owner = decl
            .owner_type
            .as_deref()
            .and_then(|o| self.resolver.resolve(file_id, m, o));
        let resolved_interface = decl
            .interface_name
            .as_ref()
            .and_then(|r| self.resolver.resolve_type_ref(file_id, m, owner, r));
        let resolved_return_type = decl
            .return_type
            .as_ref()
            .and_then(|r| self.resolver.resolve_type_ref(file_id, m, owner, r));
        let resolved_params: Vec<ResolvedParam> = decl
            .params
            .iter()
            .map(|p| ResolvedParam {
                decl: p.clone(),
                resolved_type: p
                    .type_name
                    .as_ref()
                    .and_then(|r| self.resolver.resolve_type_ref(file_id, m, owner, r)),
            })
            .collect();

        let mut resolved_local_types = HashMap::new();
        for p in &resolved_params {
            if let Some(tid) = p.resolved_type {
                resolved_local_types.insert(p.decl.name.clone(), tid);
            }
        }
        for b in &decl.local_bindings {
            if let Some(tid) = b
                .type_name
                .as_ref()
                .and_then(|r| self.resolver.resolve_type_ref(file_id, m, owner, r))
            {
                resolved_local_types.insert(b.name.clone(), tid);
            }
        }

        let mut seen = HashSet::new();
        let resolved_constructions = decl
            .constructed_types
            .iter()
            .filter_map(|c| self.resolver.resolve_type_ref(file_id, m, owner, c))
            .filter(|tid| seen.insert(*tid))
            .collect();
        let resolved_field_inits = decl
            .field_inits
            .iter()
            .filter_map(|init| {
                let type_id = self
                    .resolver
                    .resolve_type_ref(file_id, m, owner, &init.type_name)?;
                Some(ResolvedFieldInit {
                    type_id,
                    field_name: init.field_name.clone(),
                    span: init.span.clone(),
                })
            })
            .collect();

        IndexedFunc {
            id,
            file_id,
            module_path: m.clone(),
            member_separator: file.member_separator,
            decl: decl.clone(),
            resolved_owner: owner,
            resolved_interface,
            resolved_return_type,
            resolved_params,
            resolved_constructions,
            resolved_local_types,
            resolved_call_receivers: Vec::new(),
            resolved_field_accesses: Vec::new(),
            resolved_field_inits,
            resolved_escape_targets: Vec::new(),
            resolved_spy_targets: Vec::new(),
            resolved_repr_targets: Vec::new(),
        }
    }

    pub(super) fn finalize_functions(&self, functions: &mut [IndexedFunc]) {
        let returns = ReturnTypeIndex::build(functions);
        let free_funcs = FreeFuncTable::build(functions);
        let mut inferred_params: HashSet<(FuncId, usize)> = HashSet::new();

        for _ in 0..4 {
            let mut changed = self.infer_local_bindings(functions, &returns);
            let candidates = self.collect_helper_param_candidates(functions, &free_funcs);
            for ((target_id, param_idx), candidate) in candidates {
                let target = &mut functions[target_id.0];
                let Some(param) = target.resolved_params.get(param_idx) else {
                    continue;
                };
                if param.resolved_type.is_some() {
                    continue;
                }
                let param_name = param.decl.name.clone();
                match candidate {
                    Some(inferred_ty) => {
                        let already_inferred = inferred_params.contains(&(target_id, param_idx));
                        if !already_inferred && target.resolved_local_types.contains_key(&param_name)
                        {
                            continue;
                        }
                        if target.resolved_local_types.get(&param_name).copied()
                            != Some(inferred_ty)
                        {
                            target.resolved_local_types.insert(param_name, inferred_ty);
                            inferred_params.insert((target_id, param_idx));
                            changed = true;
                        }
                    }
                    None => {
                        if inferred_params.remove(&(target_id, param_idx)) {
                            target.resolved_local_types.remove(&param_name);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        self.resolve_calls_and_fields(functions);
    }

    fn infer_local_bindings(
        &self,
        functions: &mut [IndexedFunc],
        returns: &ReturnTypeIndex,
    ) -> bool {
        let mut changed = false;
        for func in functions.iter_mut() {
            let is_test = func.decl.is_test_code();
            for binding in &func.decl.local_bindings {
                if func.resolved_local_types.contains_key(&binding.name) {
                    continue;
                }
                let Some(callee) = binding.init_callee.as_ref() else {
                    continue;
                };
                let inferred = if let Some(recv) = binding.init_receiver.as_ref() {
                    self.resolve_receiver_for_func(func, recv)
                        .and_then(|owner_id| returns.lookup_method(owner_id, callee))
                } else {
                    returns.lookup_free(func.file_id, &func.module_path, callee, is_test)
                };
                if let Some(tid) = inferred {
                    func.resolved_local_types.insert(binding.name.clone(), tid);
                    changed = true;
                }
            }
        }
        changed
    }

    fn collect_helper_param_candidates(
        &self,
        functions: &[IndexedFunc],
        free_funcs: &FreeFuncTable,
    ) -> HashMap<(FuncId, usize), Option<TypeId>> {
        // Sound all-agree join: `Some(ty)` = every observed call site passes `ty`;
        // `None` = conflicting types or at least one untyped/unknown call site.
        let mut param_candidates: HashMap<(FuncId, usize), Option<TypeId>> = HashMap::new();

        for caller in functions {
            for call in &caller.decl.calls {
                if !matches!(call.kind, CallKind::Free | CallKind::SameModule) {
                    continue;
                }
                let Some(target_id) = free_funcs.lookup(caller, call.callee.as_str()) else {
                    continue;
                };
                let param_count = functions[target_id.0].resolved_params.len();
                for arg_idx in 0..param_count {
                    let arg_ty = call
                        .arg_receivers
                        .get(arg_idx)
                        .and_then(|r| r.as_ref())
                        .and_then(|recv| self.resolve_receiver_for_func(caller, recv));
                    param_candidates
                        .entry((target_id, arg_idx))
                        .and_modify(|slot| {
                            if *slot != arg_ty {
                                *slot = None;
                            }
                        })
                        .or_insert(arg_ty);
                }
            }
        }
        param_candidates
    }

    fn resolve_calls_and_fields(&self, functions: &mut [IndexedFunc]) {
        for func in functions.iter_mut() {
            func.resolved_call_receivers.clear();
            func.resolved_call_receivers.reserve(func.decl.calls.len());
            for call in &func.decl.calls {
                let resolved = if call.targets_self_owner() {
                    func.resolved_owner
                } else {
                    match (call.kind, call.receiver.as_ref()) {
                        (CallKind::Method, Some(CallReceiver::Named(recv))) => {
                            func.resolved_local_types.get(recv).copied().or_else(|| {
                                self.resolver.resolve(func.file_id, &func.module_path, recv)
                            })
                        }
                        (CallKind::Scoped, Some(CallReceiver::Named(recv))) => {
                            self.resolver.resolve(func.file_id, &func.module_path, recv)
                        }
                        _ => None,
                    }
                };
                func.resolved_call_receivers.push(resolved);
            }
            debug_assert_eq!(func.resolved_call_receivers.len(), func.decl.calls.len());

            func.resolved_field_accesses.clear();
            func.resolved_field_accesses
                .reserve(func.decl.field_accesses.len());
            for access in &func.decl.field_accesses {
                let resolved = if let Some(ty_ref) = access.receiver_type.as_ref() {
                    self.resolver.resolve_type_ref(
                        func.file_id,
                        &func.module_path,
                        func.resolved_owner,
                        ty_ref,
                    )
                } else if access.receiver_is_self {
                    func.resolved_owner
                } else if access.receiver_param.is_empty() {
                    None
                } else {
                    func.resolved_local_types
                        .get(&access.receiver_param)
                        .copied()
                };
                func.resolved_field_accesses.push(resolved);
            }
            debug_assert_eq!(
                func.resolved_field_accesses.len(),
                func.decl.field_accesses.len()
            );

            func.resolved_escape_targets.clear();
            func.resolved_escape_targets
                .reserve(func.decl.coupling.encapsulation_escapes.len());
            for esc in &func.decl.coupling.encapsulation_escapes {
                let resolved = esc
                    .target_receiver
                    .as_ref()
                    .and_then(|r| self.resolve_receiver_for_func(func, r));
                func.resolved_escape_targets.push(resolved);
            }

            func.resolved_spy_targets.clear();
            func.resolved_spy_targets
                .reserve(func.decl.coupling.spy_sites.len());
            for spy in &func.decl.coupling.spy_sites {
                let resolved = spy
                    .target_receiver
                    .as_ref()
                    .and_then(|r| self.resolve_receiver_for_func(func, r));
                func.resolved_spy_targets.push(resolved);
            }

            func.resolved_repr_targets.clear();
            func.resolved_repr_targets
                .reserve(func.decl.coupling.repr_assertions.len());
            for ra in &func.decl.coupling.repr_assertions {
                let resolved = ra
                    .target_receiver
                    .as_ref()
                    .and_then(|r| self.resolve_receiver_for_func(func, r));
                func.resolved_repr_targets.push(resolved);
            }
        }
    }


    pub(super) fn resolve_interface_impls(
        &self,
        files: &[FileAst],
        types: &[IndexedType],
    ) -> Vec<ResolvedInterfaceImpl> {
        let mut out = Vec::new();
        let mut seen_resolved: HashSet<(Option<TypeId>, TypeId, bool)> = HashSet::new();
        let mut seen_unresolved: HashSet<(Option<TypeId>, String, bool)> = HashSet::new();
        for (file_idx, file) in files.iter().enumerate() {
            let file_id = FileId(file_idx);
            let is_test_file = file.is_test_file();
            for imp in &file.interface_impls {
                let scope_mod = imp.module_path.as_ref().unwrap_or(&file.module_path);
                let target_id = self
                    .resolver
                    .resolve_type_ref(file_id, scope_mod, None, &imp.target_type);
                let interface_id = self
                    .resolver
                    .resolve_type_ref(file_id, scope_mod, target_id, &imp.interface_name)
                    .filter(|&iid| {
                        matches!(
                            types[iid.0].decl.kind,
                            TypeKind::Interface | TypeKind::Class
                        )
                    });
                let target_type_name = match &imp.target_type {
                    crate::ast::item::TypeRef::SelfType => String::from("Self"),
                    crate::ast::item::TypeRef::Named(name) => name.clone(),
                };
                let in_test = is_test_file || imp.in_test_module;
                let keep = imp.role != InterfaceImplRole::Normal
                    || match target_id {
                        Some(tid) => seen_resolved.insert((interface_id, tid, in_test)),
                        None => seen_unresolved.insert((
                            interface_id,
                            target_type_name.clone(),
                            in_test,
                        )),
                    };
                if keep {
                    out.push(ResolvedInterfaceImpl {
                        interface_id,
                        target_id,
                        target_type_name,
                        file_id,
                        in_test_module: in_test,
                        role: imp.role,
                        span: imp.span.clone(),
                    });
                }
            }
        }
        for ty in types {
            if ty.decl.kind == TypeKind::Interface {
                continue;
            }
            let in_test = ty.decl.in_test_module || files[ty.file_id.0].is_test_file();
            for &base_id in &ty.resolved_base_types {
                if types[base_id.0].decl.kind == TypeKind::Interface
                    && seen_resolved.insert((Some(base_id), ty.id, in_test))
                {
                    out.push(ResolvedInterfaceImpl {
                        interface_id: Some(base_id),
                        target_id: Some(ty.id),
                        target_type_name: ty.decl.name.clone(),
                        file_id: ty.file_id,
                        in_test_module: in_test,
                        role: InterfaceImplRole::Normal,
                        span: ty.decl.span.clone(),
                    });
                }
            }
        }
        out
    }

    fn resolve_receiver_for_func(&self, func: &IndexedFunc, recv: &CallReceiver) -> Option<TypeId> {
        match recv {
            CallReceiver::SelfInstance | CallReceiver::SelfType => func.resolved_owner,
            CallReceiver::Named(name) => func
                .resolved_local_types
                .get(name)
                .copied()
                .or_else(|| self.resolver.resolve(func.file_id, &func.module_path, name)),
        }
    }
}

pub(super) fn attach_type_bases(files: &[FileAst], types: &mut [IndexedType]) {
    let resolved: Vec<Vec<TypeId>> = {
        let resolver = TypeResolver::new(files, types);
        types
            .iter()
            .map(|ty| {
                let mut seen = HashSet::new();
                ty.decl
                    .base_types
                    .iter()
                    .filter_map(|base| {
                        resolver.resolve_type_ref(ty.file_id, &ty.module_path, Some(ty.id), base)
                    })
                    .filter(|tid| *tid != ty.id && seen.insert(*tid))
                    .collect()
            })
            .collect()
    };
    for (ty, bases) in types.iter_mut().zip(resolved) {
        ty.resolved_base_types = bases;
    }
}
