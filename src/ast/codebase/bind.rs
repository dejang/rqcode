use super::model::{
    FileId, FuncId, IndexedFunc, IndexedType, ResolvedFieldInit, ResolvedParam, TypeId,
};
use super::resolve::TypeResolver;
use crate::ast::item::{CallKind, CallReceiver, FunctionDecl};
use crate::ast::{FileAst, ModulePath};
use std::collections::{HashMap, HashSet};

/// Binds function declarations, local variables, and call receivers to resolved [`TypeId`]s.
pub(super) struct FunctionBinder<'a> {
    resolver: TypeResolver<'a>,
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
            resolved_return_type,
            resolved_params,
            resolved_constructions,
            resolved_local_types,
            resolved_call_receivers: Vec::new(),
            resolved_field_inits,
        }
    }

    pub(super) fn finalize_functions(&self, functions: &mut [IndexedFunc]) {
        let mut return_by_file_name: HashMap<(FileId, String), TypeId> = HashMap::new();
        let mut return_by_mod_name: HashMap<(ModulePath, String), TypeId> = HashMap::new();
        let mut return_by_global_name: HashMap<String, Option<TypeId>> = HashMap::new();

        for func in functions.iter() {
            let Some(ret_ty) = func
                .decl
                .is_free_function()
                .then_some(func.resolved_return_type)
                .flatten()
            else {
                continue;
            };
            return_by_file_name.insert((func.file_id, func.decl.name.clone()), ret_ty);
            if !func.decl.is_test_code() {
                return_by_mod_name
                    .insert((func.module_path.clone(), func.decl.name.clone()), ret_ty);
                return_by_global_name
                    .entry(func.decl.name.clone())
                    .and_modify(|slot| {
                        if *slot != Some(ret_ty) {
                            *slot = None;
                        }
                    })
                    .or_insert(Some(ret_ty));
            }
        }

        for func in functions.iter_mut() {
            for binding in &func.decl.local_bindings {
                if func.resolved_local_types.contains_key(&binding.name) {
                    continue;
                }
                let Some(callee) = binding.init_callee.as_ref() else {
                    continue;
                };
                let inferred = return_by_file_name
                    .get(&(func.file_id, callee.clone()))
                    .copied()
                    .or_else(|| {
                        return_by_mod_name
                            .get(&(func.module_path.clone(), callee.clone()))
                            .copied()
                    })
                    .or_else(|| return_by_global_name.get(callee).copied().flatten());
                if let Some(tid) = inferred {
                    func.resolved_local_types.insert(binding.name.clone(), tid);
                }
            }

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
        }
    }
}
