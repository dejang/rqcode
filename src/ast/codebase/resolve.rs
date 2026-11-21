use super::model::{FileId, IndexedType, TypeId};
use crate::ast::item::{ExportDecl, ImportDecl, ImportKind, TypeRef};
use crate::ast::{FileAst, ModulePath, QualifiedPath};
use std::collections::{BTreeMap, HashMap};

/// Pre-indexed lookup table for resolving type references across files, imports, and re-exports.
pub(super) struct TypeResolver<'a> {
    files: &'a [FileAst],
    types: &'a [IndexedType],
    types_by_name: HashMap<&'a str, Vec<TypeId>>,
    types_by_mod_name: HashMap<(&'a ModulePath, &'a str), TypeId>,
    imports_by_file: Vec<HashMap<&'a str, &'a ImportDecl>>,
    imports_by_module: HashMap<&'a ModulePath, HashMap<&'a str, &'a ImportDecl>>,
    wildcard_imports_by_file: Vec<Vec<&'a ImportDecl>>,
    exports_by_module: BTreeMap<&'a ModulePath, Vec<&'a ExportDecl>>,
    default_type_by_module: BTreeMap<&'a ModulePath, TypeId>,
}

impl<'a> TypeResolver<'a> {
    pub(super) fn new(files: &'a [FileAst], types: &'a [IndexedType]) -> Self {
        let mut types_by_name: HashMap<&'a str, Vec<TypeId>> = HashMap::new();
        let mut types_by_mod_name: HashMap<(&'a ModulePath, &'a str), TypeId> = HashMap::new();
        let mut default_type_by_module: BTreeMap<&'a ModulePath, TypeId> = BTreeMap::new();
        let mut single_non_test_by_mod: HashMap<&'a ModulePath, Option<TypeId>> = HashMap::new();

        for ty in types {
            let is_test = ty.decl.in_test_module || files[ty.file_id.0].is_test_file();
            types_by_name
                .entry(ty.decl.name.as_str())
                .or_default()
                .push(ty.id);
            if is_test {
                types_by_mod_name
                    .entry((&ty.module_path, ty.decl.name.as_str()))
                    .or_insert(ty.id);
            } else {
                types_by_mod_name.insert((&ty.module_path, ty.decl.name.as_str()), ty.id);
                if ty.decl.is_default_export {
                    default_type_by_module.insert(&ty.module_path, ty.id);
                }
                single_non_test_by_mod
                    .entry(&ty.module_path)
                    .and_modify(|slot| *slot = None)
                    .or_insert(Some(ty.id));
            }
        }
        for candidates in types_by_name.values_mut() {
            candidates.sort_by_key(|tid| {
                let ty = &types[tid.0];
                ty.decl.in_test_module || files[ty.file_id.0].is_test_file()
            });
        }
        for (mod_path, maybe_tid) in single_non_test_by_mod {
            if let Some(tid) = maybe_tid {
                default_type_by_module.entry(mod_path).or_insert(tid);
            }
        }

        let (imports_by_file, wildcard_imports_by_file): (Vec<_>, Vec<_>) = files
            .iter()
            .map(|f| {
                let named = f
                    .imports
                    .iter()
                    .filter(|i| i.kind != ImportKind::Wildcard)
                    .map(|i| (i.local_name(), i))
                    .collect();
                let wildcards = f
                    .imports
                    .iter()
                    .filter(|i| i.kind == ImportKind::Wildcard)
                    .collect();
                (named, wildcards)
            })
            .unzip();

        let mut exports_by_module: BTreeMap<&'a ModulePath, Vec<&'a ExportDecl>> = BTreeMap::new();
        let mut imports_by_module: HashMap<&'a ModulePath, HashMap<&'a str, &'a ImportDecl>> =
            HashMap::new();
        for file in files {
            if file.is_test_file() {
                continue;
            }
            if !file.exports.is_empty() {
                exports_by_module
                    .entry(&file.module_path)
                    .or_default()
                    .extend(file.exports.iter());
            }
            if !file.imports.is_empty() {
                let mod_imports = imports_by_module.entry(&file.module_path).or_default();
                for imp in &file.imports {
                    if !imp.in_test_module && imp.kind != ImportKind::Wildcard {
                        mod_imports.insert(imp.local_name(), imp);
                    }
                }
            }
        }

        Self {
            files,
            types,
            types_by_name,
            types_by_mod_name,
            imports_by_file,
            imports_by_module,
            wildcard_imports_by_file,
            exports_by_module,
            default_type_by_module,
        }
    }

    pub(super) fn resolve_type_ref(
        &self,
        file_id: FileId,
        module_path: &ModulePath,
        owner: Option<TypeId>,
        ty_ref: &TypeRef,
    ) -> Option<TypeId> {
        match ty_ref {
            TypeRef::SelfType => owner,
            TypeRef::Named(name) => self.resolve(file_id, module_path, name),
        }
    }

    pub(super) fn resolve(
        &self,
        file_id: FileId,
        module_path: &ModulePath,
        raw_name: &str,
    ) -> Option<TypeId> {
        let cleaned = raw_name.trim();
        if cleaned.is_empty() {
            return None;
        }
        if let Some(qualified) = QualifiedPath::parse(cleaned) {
            return self.resolve_qualified(file_id, qualified);
        }
        if let Some(tid) = self.resolve_in_module(module_path, cleaned, 0) {
            return Some(tid);
        }
        if let Some(&import) = self.imports_by_file[file_id.0].get(cleaned)
            && let Some(tid) = self.resolve_import_at_depth(import, 0)
        {
            return Some(tid);
        }
        for import in &self.wildcard_imports_by_file[file_id.0] {
            if let Some(resolved_mod) = import.resolved_module.as_ref()
                && let Some(tid) = self.resolve_in_module(resolved_mod, cleaned, 0)
            {
                return Some(tid);
            }
        }
        self.unique_type_by_name(cleaned)
    }

    fn unique_type_by_name(&self, name: &str) -> Option<TypeId> {
        let candidates = self.types_by_name.get(name)?;
        if candidates.len() == 1 {
            return Some(candidates[0]);
        }
        let mut prod_match = None;
        for &tid in candidates {
            let ty = &self.types[tid.0];
            if ty.decl.in_test_module || self.files[ty.file_id.0].is_test_file() {
                continue;
            }
            if prod_match.is_some() {
                return None;
            }
            prod_match = Some(tid);
        }
        prod_match
    }

    fn resolve_qualified(&self, file_id: FileId, qualified: QualifiedPath<'_>) -> Option<TypeId> {
        let mut prefix_segs = qualified.prefix().split("::");
        if let Some(first) = prefix_segs.next()
            && let Some(&import) = self.imports_by_file[file_id.0].get(first)
        {
            let start_mod = match import.kind {
                ImportKind::Namespace => import.resolved_module.as_ref(),
                ImportKind::Named => import
                    .resolved_module
                    .as_ref()
                    .and_then(|m| self.resolve_namespace_in_module(m, &import.symbol, 0)),
                ImportKind::Default | ImportKind::Wildcard => None,
            };
            if let Some(mut cur_mod) = start_mod {
                let mut matched = true;
                for seg in prefix_segs {
                    let Some(next) = self.resolve_namespace_in_module(cur_mod, seg, 0) else {
                        matched = false;
                        break;
                    };
                    cur_mod = next;
                }
                if matched && let Some(tid) = self.resolve_in_module(cur_mod, qualified.member(), 0)
                {
                    return Some(tid);
                }
            } else if import.kind == ImportKind::Namespace && !qualified.prefix().contains("::") {
                return self.resolve_by_module_segments(
                    &import.module_segments,
                    qualified.member(),
                    0,
                );
            }
        }

        if let Some(tid) = self.unique_type_by_name(qualified.member()) {
            return Some(tid);
        }
        if let Some(candidates) = self.types_by_name.get(qualified.member()) {
            for &tid in candidates {
                if qualified.matches_module(&self.types[tid.0].module_path) {
                    return Some(tid);
                }
            }
        }
        for &mod_path in self.exports_by_module.keys() {
            if qualified.matches_module(mod_path)
                && let Some(tid) = self.resolve_in_module(mod_path, qualified.member(), 0)
            {
                return Some(tid);
            }
        }
        None
    }

    fn resolve_import_at_depth(&self, import: &ImportDecl, depth: usize) -> Option<TypeId> {
        if depth >= 8 {
            return None;
        }
        match import.kind {
            ImportKind::Default => {
                if let Some(resolved_mod) = import.resolved_module.as_ref() {
                    return self.resolve_default_in_module(resolved_mod, depth + 1);
                }
                if import.module_segments.len() < 2 {
                    return None;
                }
                for (&mod_path, &tid) in &self.default_type_by_module {
                    if mod_path
                        .matches_suffix(import.module_segments.iter().rev().map(String::as_str))
                    {
                        return Some(tid);
                    }
                }
                for &mod_path in self.exports_by_module.keys() {
                    if mod_path
                        .matches_suffix(import.module_segments.iter().rev().map(String::as_str))
                        && let Some(tid) = self.resolve_default_in_module(mod_path, depth + 1)
                    {
                        return Some(tid);
                    }
                }
                None
            }
            ImportKind::Named => {
                if let Some(resolved_mod) = import.resolved_module.as_ref() {
                    return self.resolve_in_module(resolved_mod, &import.symbol, depth + 1);
                }
                if import.module_segments.is_empty() {
                    return None;
                }
                self.unique_type_by_name(&import.symbol).or_else(|| {
                    self.resolve_by_module_segments(
                        &import.module_segments,
                        &import.symbol,
                        depth + 1,
                    )
                })
            }
            ImportKind::Namespace | ImportKind::Wildcard => None,
        }
    }

    fn resolve_by_module_segments(
        &self,
        segments: &[String],
        symbol: &str,
        depth: usize,
    ) -> Option<TypeId> {
        if depth >= 8 || segments.is_empty() {
            return None;
        }
        if let Some(candidates) = self.types_by_name.get(symbol) {
            for &tid in candidates {
                if self.types[tid.0]
                    .module_path
                    .matches_suffix(segments.iter().rev().map(String::as_str))
                {
                    return Some(tid);
                }
            }
        }
        for &mod_path in self.exports_by_module.keys() {
            if mod_path.matches_suffix(segments.iter().rev().map(String::as_str))
                && let Some(tid) = self.resolve_in_module(mod_path, symbol, depth + 1)
            {
                return Some(tid);
            }
        }
        None
    }

    fn resolve_namespace_in_module(
        &self,
        module: &ModulePath,
        name: &str,
        depth: usize,
    ) -> Option<&'a ModulePath> {
        if depth >= 8 {
            return None;
        }
        for export in self.exports_by_module.get(module)? {
            let found = if export.is_wildcard && !export.exported_name.is_empty() {
                (export.exported_name == name)
                    .then_some(export.source_module.as_ref())
                    .flatten()
            } else if export.is_wildcard {
                export
                    .source_module
                    .as_ref()
                    .and_then(|src| self.resolve_namespace_in_module(src, name, depth + 1))
            } else if !export.is_default && export.exported_name == name {
                let target = if export.symbol.is_empty() {
                    name
                } else {
                    export.symbol.as_str()
                };
                if let Some(src) = export.source_module.as_ref() {
                    self.resolve_namespace_in_module(src, target, depth + 1)
                } else {
                    self.imports_by_module
                        .get(module)
                        .and_then(|m| m.get(target))
                        .and_then(|imp| match imp.kind {
                            ImportKind::Namespace => imp.resolved_module.as_ref(),
                            ImportKind::Named => imp.resolved_module.as_ref().and_then(|s| {
                                self.resolve_namespace_in_module(s, &imp.symbol, depth + 1)
                            }),
                            ImportKind::Default | ImportKind::Wildcard => None,
                        })
                }
            } else {
                None
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn resolve_in_module(&self, module: &ModulePath, symbol: &str, depth: usize) -> Option<TypeId> {
        if depth >= 8 {
            return None;
        }
        if let Some(&tid) = self.types_by_mod_name.get(&(module, symbol)) {
            return Some(tid);
        }
        for export in self.exports_by_module.get(module)? {
            if let Some(tid) = self.resolve_named_export(module, export, symbol, depth) {
                return Some(tid);
            }
        }
        None
    }

    fn resolve_named_export(
        &self,
        module: &ModulePath,
        export: &ExportDecl,
        symbol: &str,
        depth: usize,
    ) -> Option<TypeId> {
        if export.is_wildcard {
            return export
                .exported_name
                .is_empty()
                .then_some(export.source_module.as_ref())
                .flatten()
                .and_then(|src| self.resolve_in_module(src, symbol, depth + 1));
        }
        if export.is_default || export.exported_name != symbol {
            return None;
        }
        let target = if export.symbol.is_empty() {
            symbol
        } else {
            export.symbol.as_str()
        };
        if let Some(src) = export.source_module.as_ref() {
            return if target == "default" {
                self.resolve_default_in_module(src, depth + 1)
            } else {
                self.resolve_in_module(src, target, depth + 1)
            };
        }
        if target != symbol
            && let Some(&tid) = self.types_by_mod_name.get(&(module, target))
        {
            return Some(tid);
        }
        self.imports_by_module
            .get(module)
            .and_then(|m| m.get(target))
            .and_then(|imp| self.resolve_import_at_depth(imp, depth + 1))
    }

    fn resolve_default_in_module(&self, module: &ModulePath, depth: usize) -> Option<TypeId> {
        if depth >= 8 {
            return None;
        }
        if let Some(&tid) = self.default_type_by_module.get(module) {
            return Some(tid);
        }
        for export in self.exports_by_module.get(module)? {
            if !export.is_default && export.exported_name != "default" {
                continue;
            }
            let resolved = if let Some(src) = export.source_module.as_ref() {
                if export.symbol.is_empty() || export.symbol == "default" {
                    self.resolve_default_in_module(src, depth + 1)
                } else {
                    self.resolve_in_module(src, &export.symbol, depth + 1)
                }
            } else if !export.symbol.is_empty() {
                self.types_by_mod_name
                    .get(&(module, export.symbol.as_str()))
                    .copied()
                    .or_else(|| {
                        self.imports_by_module
                            .get(module)
                            .and_then(|m| m.get(export.symbol.as_str()))
                            .and_then(|imp| self.resolve_import_at_depth(imp, depth + 1))
                    })
            } else {
                None
            };
            if resolved.is_some() {
                return resolved;
            }
        }
        None
    }
}
