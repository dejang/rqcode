use super::model::{FileId, IndexedFunc, IndexedType, TypeId};
use super::surface::{BarrelBoundary, EffectiveVisibility, PublicSurface, SurfaceData};
use crate::ast::item::{ExportOrigin, ImportDecl, ImportKind, Visibility};
use crate::ast::{FileAst, FileRole, LanguageId, ModulePath};
use std::collections::{HashMap, HashSet, VecDeque};

struct ReexportClosure {
    wildcard_modules: HashSet<ModulePath>,
    exported_symbols: HashSet<(ModulePath, String)>,
}

struct ModuleTreeSurface {
    languages: HashSet<LanguageId>,
    public_modules: HashSet<ModulePath>,
    bin_only_modules: HashSet<ModulePath>,
    reexports: ReexportClosure,
}

#[derive(Default)]
struct TypeReachState {
    reachable: HashSet<TypeId>,
    queue: VecDeque<TypeId>,
}

impl TypeReachState {
    fn push(&mut self, tid: TypeId) {
        if self.reachable.insert(tid) {
            self.queue.push_back(tid);
        }
    }
}

#[derive(Default)]
struct ExpansionState {
    public_modules: HashSet<ModulePath>,
    mod_queue: VecDeque<ModulePath>,
    wildcard_modules: HashSet<ModulePath>,
    visited_wildcards: HashSet<ModulePath>,
    wildcard_queue: VecDeque<ModulePath>,
    exported_symbols: HashSet<(ModulePath, String)>,
    symbol_queue: VecDeque<(ModulePath, String)>,
}

impl ExpansionState {
    fn has_pending_work(&self) -> bool {
        !self.mod_queue.is_empty()
            || !self.wildcard_queue.is_empty()
            || !self.symbol_queue.is_empty()
    }

    fn push_public_module(&mut self, module: &ModulePath) {
        if self.public_modules.insert(module.clone()) {
            self.mod_queue.push_back(module.clone());
        }
    }

    fn push_wildcard(&mut self, src: &ModulePath) {
        self.wildcard_modules.insert(src.clone());
        if self.visited_wildcards.insert(src.clone()) {
            self.wildcard_queue.push_back(src.clone());
        }
    }

    fn push_symbol(&mut self, src: &ModulePath, sym: &str) {
        let pair = (src.clone(), sym.to_string());
        if self.exported_symbols.insert(pair.clone()) {
            self.symbol_queue.push_back(pair);
        }
    }

    fn expand_file_exports(&mut self, file: &FileAst, requested_sym: Option<&str>) {
        for exp in &file.exports {
            if exp.visibility != Visibility::Public || exp.origin == ExportOrigin::DirectExternal {
                continue;
            }
            if let Some(req) = requested_sym {
                let matches =
                    (exp.is_wildcard && exp.exported_name.is_empty()) || exp.exported_name == req;
                if !matches {
                    continue;
                }
                if exp.is_wildcard && exp.exported_name == req {
                    if let Some(src) = exp.source_module.as_ref() {
                        self.push_wildcard(src);
                    }
                    continue;
                }
                if exp.is_wildcard && exp.exported_name.is_empty() {
                    if let Some(src) = exp.source_module.as_ref() {
                        self.push_symbol(src, req);
                    }
                    continue;
                }
                if !exp.symbol.is_empty() {
                    self.resolve_named_export(file, exp.source_module.as_ref(), &exp.symbol);
                }
            } else if exp.is_wildcard {
                if let Some(src) = exp.source_module.as_ref() {
                    self.push_wildcard(src);
                }
            } else if !exp.symbol.is_empty() {
                self.resolve_named_export(file, exp.source_module.as_ref(), &exp.symbol);
            }
        }
    }

    fn resolve_named_export(
        &mut self,
        file: &FileAst,
        source_module: Option<&ModulePath>,
        symbol: &str,
    ) {
        if let Some(src) = source_module {
            self.push_symbol(src, symbol);
            return;
        }
        for imp in &file.imports {
            if imp.in_test_module || imp.local_name() != symbol {
                continue;
            }
            let Some(src) = imp.resolved_module.as_ref() else {
                continue;
            };
            match imp.kind {
                ImportKind::Named => self.push_symbol(src, &imp.symbol),
                ImportKind::Default => self.push_symbol(src, "default"),
                ImportKind::Namespace | ImportKind::Wildcard => self.push_wildcard(src),
            }
            return;
        }
        self.push_symbol(&file.module_path, symbol);
    }
}

pub(super) struct SurfaceBuilder<'a> {
    files: &'a [FileAst],
    types: &'a [IndexedType],
    functions: &'a [IndexedFunc],
    is_binary_only_package: bool,
    files_by_mod: HashMap<&'a ModulePath, Vec<&'a FileAst>>,
    binary_entry_files: HashSet<FileId>,
    lib_root_languages: HashSet<LanguageId>,
    bin_entry_languages: HashSet<LanguageId>,
}

impl<'a> SurfaceBuilder<'a> {
    pub(super) fn new(
        files: &'a [FileAst],
        types: &'a [IndexedType],
        functions: &'a [IndexedFunc],
        is_binary_only_package: bool,
    ) -> Self {
        let mut files_by_mod: HashMap<&'a ModulePath, Vec<&'a FileAst>> = HashMap::new();
        let mut binary_entry_files = HashSet::new();
        let mut lib_root_languages = HashSet::new();
        let mut bin_entry_languages = HashSet::new();

        for (idx, file) in files.iter().enumerate() {
            if file.is_test_file() {
                continue;
            }
            match file.role {
                FileRole::LibraryRoot => {
                    lib_root_languages.insert(file.language);
                }
                FileRole::BinaryEntrypoint => {
                    binary_entry_files.insert(FileId(idx));
                    bin_entry_languages.insert(file.language);
                    continue;
                }
                FileRole::Module | FileRole::UnitTest | FileRole::IntegrationTest => {}
            }
            files_by_mod
                .entry(&file.module_path)
                .or_default()
                .push(file);
        }

        Self {
            files,
            types,
            functions,
            is_binary_only_package,
            files_by_mod,
            binary_entry_files,
            lib_root_languages,
            bin_entry_languages,
        }
    }

    pub(super) fn build(self) -> PublicSurface {
        let file_languages: Vec<LanguageId> = self.files.iter().map(|f| f.language).collect();
        let mod_tree = self.compute_module_tree_surface();
        let barrels = self.compute_barrel_boundaries();
        let active_languages = self.compute_active_languages(&barrels);

        let mut surface = PublicSurface::from_parts(SurfaceData {
            active_languages,
            file_languages,
            binary_entry_files: self.binary_entry_files.clone(),
            module_tree_languages: mod_tree.languages,
            public_modules: mod_tree.public_modules,
            bin_only_modules: mod_tree.bin_only_modules,
            wildcard_reexported_modules: mod_tree.reexports.wildcard_modules,
            reexported_symbols: mod_tree.reexports.exported_symbols,
            barrels,
        });
        let reachable = self.compute_reachable_public_types(&surface);
        surface.set_reachable_public_types(reachable);
        surface
    }

    fn compute_active_languages(&self, barrels: &[BarrelBoundary]) -> HashSet<LanguageId> {
        if self.is_binary_only_package {
            return HashSet::new();
        }
        let mut candidates: HashSet<LanguageId> = self.lib_root_languages.clone();
        let barrel_mods: HashSet<&ModulePath> = barrels.iter().map(BarrelBoundary::barrel_module).collect();
        for file in self.files {
            if !file.is_test_file() && file.is_directory_barrel() && barrel_mods.contains(&file.module_path) {
                candidates.insert(file.language);
            }
        }
        for ty in self.types {
            if !ty.decl.in_test_module
                && !self.files[ty.file_id.0].is_test_file()
                && !self.binary_entry_files.contains(&ty.file_id)
                && ty.decl.visibility == Visibility::Public
            {
                candidates.insert(self.files[ty.file_id.0].language);
            }
        }
        for func in self.functions {
            if !func.decl.role.is_test_code()
                && !self.files[func.file_id.0].is_test_file()
                && !self.binary_entry_files.contains(&func.file_id)
                && func.decl.visibility == Visibility::Public
            {
                candidates.insert(self.files[func.file_id.0].language);
            }
        }
        candidates.retain(|lang| {
            self.lib_root_languages.contains(lang) || !self.bin_entry_languages.contains(lang)
        });
        candidates
    }

    fn collect_all_lib_modules(&self) -> (HashSet<LanguageId>, HashSet<ModulePath>) {
        let mut languages = HashSet::new();
        let mut all_lib_modules: HashSet<ModulePath> = HashSet::new();
        let mut queue: VecDeque<ModulePath> = VecDeque::new();
        for file in self.files {
            if file.role == FileRole::LibraryRoot {
                if !file.module_decls.is_empty() {
                    languages.insert(file.language);
                }
                if all_lib_modules.insert(file.module_path.clone()) {
                    queue.push_back(file.module_path.clone());
                }
            }
        }
        while let Some(mod_path) = queue.pop_front() {
            let Some(mod_files) = self.files_by_mod.get(&mod_path) else {
                continue;
            };
            for file in mod_files {
                if !file.module_decls.is_empty() {
                    languages.insert(file.language);
                }
                for decl in &file.module_decls {
                    let child = mod_path.push(&decl.name);
                    if all_lib_modules.insert(child.clone()) {
                        queue.push_back(child);
                    }
                }
            }
        }
        (languages, all_lib_modules)
    }

    fn enqueue_public_child_modules(&self, mod_path: &ModulePath, state: &mut ExpansionState) {
        let Some(mod_files) = self.files_by_mod.get(mod_path) else {
            return;
        };
        for file in mod_files {
            for decl in &file.module_decls {
                if decl.visibility == Visibility::Public {
                    state.push_public_module(&mod_path.push(&decl.name));
                }
            }
        }
    }

    fn compute_module_tree_surface(&self) -> ModuleTreeSurface {
        let (languages, all_lib_modules) = self.collect_all_lib_modules();
        let mut exp_state = ExpansionState::default();
        for file in self.files {
            if file.role == FileRole::LibraryRoot {
                exp_state.push_public_module(&file.module_path);
            }
        }

        while exp_state.has_pending_work() {
            while let Some(mod_path) = exp_state.mod_queue.pop_front() {
                if exp_state.visited_wildcards.insert(mod_path.clone()) {
                    exp_state.wildcard_queue.push_back(mod_path.clone());
                }
                self.enqueue_public_child_modules(&mod_path, &mut exp_state);
            }
            while let Some(mod_path) = exp_state.wildcard_queue.pop_front() {
                if exp_state.wildcard_modules.contains(&mod_path) {
                    self.enqueue_public_child_modules(&mod_path, &mut exp_state);
                }
                if let Some(mod_files) = self.files_by_mod.get(&mod_path) {
                    for file in mod_files {
                        exp_state.expand_file_exports(file, None);
                    }
                }
            }
            while let Some((mod_path, sym)) = exp_state.symbol_queue.pop_front() {
                let child_mod = mod_path.push(&sym);
                if self.files_by_mod.contains_key(&child_mod) {
                    exp_state.push_public_module(&child_mod);
                }
                if let Some(mod_files) = self.files_by_mod.get(&mod_path) {
                    for file in mod_files {
                        exp_state.expand_file_exports(file, Some(sym.as_str()));
                    }
                }
            }
        }

        let bin_only_modules = self.collect_bin_only_modules(&all_lib_modules);
        let public_modules = exp_state.public_modules;
        ModuleTreeSurface {
            languages,
            public_modules,
            bin_only_modules,
            reexports: ReexportClosure {
                wildcard_modules: exp_state.wildcard_modules,
                exported_symbols: exp_state.exported_symbols,
            },
        }
    }

    fn collect_bin_only_modules(
        &self,
        all_lib_modules: &HashSet<ModulePath>,
    ) -> HashSet<ModulePath> {
        let mut bin_only = HashSet::new();
        let mut queue: VecDeque<ModulePath> = VecDeque::new();
        for &bin_fid in &self.binary_entry_files {
            let file = &self.files[bin_fid.0];
            for decl in &file.module_decls {
                let child = file.module_path.push(&decl.name);
                if !all_lib_modules.contains(&child) && bin_only.insert(child.clone()) {
                    queue.push_back(child);
                }
            }
        }
        while let Some(mod_path) = queue.pop_front() {
            let Some(mod_files) = self.files_by_mod.get(&mod_path) else {
                continue;
            };
            for file in mod_files {
                for decl in &file.module_decls {
                    let child = mod_path.push(&decl.name);
                    if !all_lib_modules.contains(&child) && bin_only.insert(child.clone()) {
                        queue.push_back(child);
                    }
                }
            }
        }
        bin_only
    }

    fn compute_barrel_boundaries(&self) -> Vec<BarrelBoundary> {
        let mut barrels = Vec::new();
        let mut internal_imported: HashSet<&str> = HashSet::new();
        for file in self.files {
            if file.is_test_file() || !file.is_directory_barrel() {
                continue;
            }
            internal_imported.clear();
            for imp in &file.imports {
                if !imp.in_test_module && imp.resolved_module.is_some() {
                    internal_imported.insert(ImportDecl::local_name(imp));
                }
            }
            let mut has_internal_reexport = false;
            for exp in &file.exports {
                let is_direct = exp.origin == ExportOrigin::DirectInternal && exp.source_module.is_some();
                let is_bound = exp.origin == ExportOrigin::ImportedBinding
                    && internal_imported.contains(exp.symbol.as_str());
                if is_direct || is_bound {
                    has_internal_reexport = true;
                    break;
                }
            }
            if !has_internal_reexport {
                continue;
            }
            let closure = self.collect_barrel_reexports(&file.module_path);
            barrels.push(BarrelBoundary::new(
                file.module_path.clone(),
                closure.wildcard_modules,
                closure.exported_symbols,
            ));
        }
        barrels
    }

    fn collect_barrel_reexports(&self, seed: &ModulePath) -> ReexportClosure {
        let mut state = ExpansionState::default();
        state.visited_wildcards.insert(seed.clone());
        state.wildcard_queue.push_back(seed.clone());

        while !state.wildcard_queue.is_empty() || !state.symbol_queue.is_empty() {
            while let Some(mod_path) = state.wildcard_queue.pop_front() {
                let Some(mod_files) = self.files_by_mod.get(&mod_path) else {
                    continue;
                };
                for file in mod_files {
                    state.expand_file_exports(file, None);
                }
            }
            while let Some((mod_path, sym)) = state.symbol_queue.pop_front() {
                let Some(mod_files) = self.files_by_mod.get(&mod_path) else {
                    continue;
                };
                for file in mod_files {
                    state.expand_file_exports(file, Some(sym.as_str()));
                }
            }
        }

        ReexportClosure {
            wildcard_modules: state.wildcard_modules,
            exported_symbols: state.exported_symbols,
        }
    }

    fn compute_reachable_public_types(&self, surface: &PublicSurface) -> HashSet<TypeId> {
        let mut methods_by_owner: HashMap<TypeId, Vec<&IndexedFunc>> = HashMap::new();
        for func in self.functions {
            if func.decl.role.is_test_code() || self.files[func.file_id.0].is_test_file() {
                continue;
            }
            if let Some(owner_id) = func.resolved_owner
                && (func.decl.visibility == Visibility::Public || func.decl.is_interface_impl)
            {
                methods_by_owner.entry(owner_id).or_default().push(func);
            }
        }

        let mut state = TypeReachState::default();
        for ty in self.types {
            if ty.decl.in_test_module || self.files[ty.file_id.0].is_test_file() {
                continue;
            }
            if surface.effective_type_visibility(ty) == EffectiveVisibility::Public {
                state.push(ty.id);
            }
        }
        for func in self.functions {
            if func.decl.role.is_test_code() || self.files[func.file_id.0].is_test_file() {
                continue;
            }
            if surface.effective_func_visibility(func, self.types) == EffectiveVisibility::Public {
                self.enqueue_sig_types(func, &mut state);
            }
        }

        while let Some(tid) = state.queue.pop_front() {
            let Some(methods) = methods_by_owner.get(&tid) else {
                continue;
            };
            for &method in methods {
                self.enqueue_sig_types(method, &mut state);
            }
        }
        state.reachable
    }

    fn enqueue_sig_types(&self, func: &IndexedFunc, state: &mut TypeReachState) {
        for tid in func
            .resolved_return_type
            .into_iter()
            .chain(func.resolved_params.iter().filter_map(|p| p.resolved_type))
        {
            let ty = &self.types[tid.0];
            if !ty.decl.in_test_module
                && !self.files[ty.file_id.0].is_test_file()
                && ty.decl.visibility == Visibility::Public
            {
                state.push(tid);
            }
        }
    }
}
