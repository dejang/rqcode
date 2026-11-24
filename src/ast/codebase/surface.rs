use super::model::{FileId, IndexedFunc, IndexedType, TypeId};
use crate::ast::item::{FieldDecl, Visibility};
use crate::ast::{LanguageId, ModulePath};
use std::collections::HashSet;

/// Effective visibility of a production declaration relative to the package's public API surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectiveVisibility {
    /// Strictly private to the declaring type, file, or module.
    Private,
    /// Accessible inside the package/module/barrel boundary (or via visibility-bypassing imports),
    /// but NOT part of the public API surface.
    Internal,
    /// Reachable from external consumers through the package's public surface.
    Public,
}

impl EffectiveVisibility {
    const fn from_syntactic(vis: Visibility) -> Self {
        match vis {
            Visibility::Private => Self::Private,
            Visibility::Internal => Self::Internal,
            Visibility::Public => Self::Public,
        }
    }
}

pub(super) struct BarrelBoundary {
    barrel_module: ModulePath,
    wildcard_modules: HashSet<ModulePath>,
    exported_symbols: HashSet<(ModulePath, String)>,
}

impl BarrelBoundary {
    pub(super) const fn new(
        barrel_module: ModulePath,
        wildcard_modules: HashSet<ModulePath>,
        exported_symbols: HashSet<(ModulePath, String)>,
    ) -> Self {
        Self {
            barrel_module,
            wildcard_modules,
            exported_symbols,
        }
    }
    pub(super) const fn barrel_module(&self) -> &ModulePath {
        &self.barrel_module
    }

    fn exports(&self, target_module: &ModulePath, symbol: &str, is_default_export: bool) -> bool {
        (!is_default_export && self.wildcard_modules.contains(target_module))
            || self
                .exported_symbols
                .contains(&(target_module.clone(), symbol.to_string()))
            || (is_default_export
                && self
                    .exported_symbols
                    .contains(&(target_module.clone(), String::from("default"))))
    }
}

pub(super) struct SurfaceData {
    pub(super) active_languages: HashSet<LanguageId>,
    pub(super) file_languages: Vec<LanguageId>,
    pub(super) binary_entry_files: HashSet<FileId>,
    pub(super) module_tree_languages: HashSet<LanguageId>,
    pub(super) public_modules: HashSet<ModulePath>,
    pub(super) bin_only_modules: HashSet<ModulePath>,
    pub(super) wildcard_reexported_modules: HashSet<ModulePath>,
    pub(super) reexported_symbols: HashSet<(ModulePath, String)>,
    pub(super) barrels: Vec<BarrelBoundary>,
}

/// Language-agnostic public API surface model for a [`super::Codebase`].
///
/// Combines per-language module trees (`FileRole::LibraryRoot` + `ModuleDecl` + `ExportDecl`),
/// directory barrel boundaries (`FileAst::is_barrel` + `ExportDecl`), public signature type
/// reachability, and package-level visibility declarations into a uniform [`EffectiveVisibility`] API.
pub struct PublicSurface {
    active_languages: HashSet<LanguageId>,
    file_languages: Vec<LanguageId>,
    binary_entry_files: HashSet<FileId>,
    module_tree_languages: HashSet<LanguageId>,
    public_modules: HashSet<ModulePath>,
    bin_only_modules: HashSet<ModulePath>,
    wildcard_reexported_modules: HashSet<ModulePath>,
    reexported_symbols: HashSet<(ModulePath, String)>,
    reachable_public_types: HashSet<TypeId>,
    barrels: Vec<BarrelBoundary>,
}

impl PublicSurface {
    pub(super) fn from_parts(data: SurfaceData) -> Self {
        Self {
            active_languages: data.active_languages,
            file_languages: data.file_languages,
            binary_entry_files: data.binary_entry_files,
            module_tree_languages: data.module_tree_languages,
            public_modules: data.public_modules,
            bin_only_modules: data.bin_only_modules,
            wildcard_reexported_modules: data.wildcard_reexported_modules,
            reexported_symbols: data.reexported_symbols,
            reachable_public_types: HashSet::new(),
            barrels: data.barrels,
        }
    }

    pub(super) fn set_reachable_public_types(&mut self, reachable: HashSet<TypeId>) {
        self.reachable_public_types = reachable;
    }

    #[must_use]
    pub fn has_public_surface(&self) -> bool {
        !self.active_languages.is_empty()
    }

    /// Whether `test_module` crosses an enclosing barrel boundary that hides
    /// `(target_module, symbol)`. Uses the outermost barrel not containing `test_module`.
    #[must_use]
    pub fn bypasses_enclosing_barrel(
        &self,
        test_module: &ModulePath,
        target_module: &ModulePath,
        symbol: &str,
        is_default_export: bool,
    ) -> bool {
        let outermost = self
            .barrels
            .iter()
            .filter(|b| {
                target_module.is_same_or_child_of(&b.barrel_module)
                    && *target_module != b.barrel_module
                    && !test_module.is_same_or_child_of(&b.barrel_module)
            })
            .min_by_key(|b| b.barrel_module.segments().len());
        outermost.is_some_and(|b| !b.exports(target_module, symbol, is_default_export))
    }

    #[must_use]
    pub fn effective_symbol_visibility(
        &self,
        file_id: FileId,
        module: &ModulePath,
        symbol: &str,
        syntactic: Visibility,
        is_default_export: bool,
    ) -> EffectiveVisibility {
        if self.binary_entry_files.contains(&file_id) {
            return match syntactic {
                Visibility::Private => EffectiveVisibility::Private,
                Visibility::Internal | Visibility::Public => EffectiveVisibility::Internal,
            };
        }
        match syntactic {
            Visibility::Private => EffectiveVisibility::Private,
            Visibility::Internal => EffectiveVisibility::Internal,
            Visibility::Public => {
                if self.is_module_symbol_public(file_id, module, symbol, is_default_export) {
                    EffectiveVisibility::Public
                } else {
                    EffectiveVisibility::Internal
                }
            }
        }
    }

    #[must_use]
    pub fn effective_type_visibility(&self, ty: &IndexedType) -> EffectiveVisibility {
        if ty.decl.visibility == Visibility::Public && self.reachable_public_types.contains(&ty.id)
        {
            return EffectiveVisibility::Public;
        }
        self.effective_symbol_visibility(
            ty.file_id,
            &ty.module_path,
            &ty.decl.name,
            ty.decl.visibility,
            ty.decl.is_default_export,
        )
    }

    #[must_use]
    pub fn effective_func_visibility(
        &self,
        func: &IndexedFunc,
        types: &[IndexedType],
    ) -> EffectiveVisibility {
        if func.decl.is_interface_impl {
            let owner_eff = func
                .resolved_owner
                .map_or(EffectiveVisibility::Public, |id| {
                    self.effective_type_visibility(&types[id.0])
                });
            let iface_eff = func
                .resolved_interface
                .map_or(EffectiveVisibility::Public, |id| {
                    self.effective_type_visibility(&types[id.0])
                });
            return owner_eff.min(iface_eff);
        }
        if let Some(owner_id) = func.resolved_owner {
            let owner_eff = self.effective_type_visibility(&types[owner_id.0]);
            return EffectiveVisibility::from_syntactic(func.decl.visibility).min(owner_eff);
        }
        if !func.decl.is_free_function() {
            return EffectiveVisibility::Public;
        }
        self.effective_symbol_visibility(
            func.file_id,
            &func.module_path,
            &func.decl.name,
            func.decl.visibility,
            func.decl.is_default_export,
        )
    }

    #[must_use]
    pub fn effective_field_visibility(
        &self,
        ty: &IndexedType,
        field: &FieldDecl,
    ) -> EffectiveVisibility {
        EffectiveVisibility::from_syntactic(field.visibility)
            .min(self.effective_type_visibility(ty))
    }

    #[must_use]
    pub fn test_crosses_surface_for_type(
        &self,
        test_module: &ModulePath,
        bypasses_visibility: bool,
        ty: &IndexedType,
    ) -> bool {
        if self.effective_type_visibility(ty) != EffectiveVisibility::Internal {
            return false;
        }
        self.test_crosses_surface_for_symbol(
            (test_module, bypasses_visibility),
            (
                ty.file_id,
                &ty.module_path,
                &ty.decl.name,
                ty.decl.visibility,
                ty.decl.is_default_export,
            ),
        )
    }

    #[must_use]
    pub fn test_crosses_surface_for_func(
        &self,
        test_module: &ModulePath,
        bypasses_visibility: bool,
        func: &IndexedFunc,
        types: &[IndexedType],
    ) -> bool {
        if func.decl.is_interface_impl
            || !self.is_surface_active_for(func.file_id, &func.module_path)
        {
            return false;
        }
        if let Some(owner_id) = func.resolved_owner {
            let owner = &types[owner_id.0];
            if self.effective_type_visibility(owner) == EffectiveVisibility::Private {
                return false;
            }
            return match func.decl.visibility {
                Visibility::Private => false,
                Visibility::Internal => {
                    bypasses_visibility || !test_module.is_same_or_child_of(&func.module_path)
                }
                Visibility::Public => {
                    self.test_crosses_surface_for_type(test_module, bypasses_visibility, owner)
                }
            };
        }
        if !func.decl.is_free_function() {
            return false;
        }
        self.test_crosses_surface_for_symbol(
            (test_module, bypasses_visibility),
            (
                func.file_id,
                &func.module_path,
                &func.decl.name,
                func.decl.visibility,
                func.decl.is_default_export,
            ),
        )
    }

    #[must_use]
    pub fn test_crosses_surface_for_field(
        &self,
        test_module: &ModulePath,
        bypasses_visibility: bool,
        ty: &IndexedType,
        field: &FieldDecl,
    ) -> bool {
        if !self.is_surface_active_for(ty.file_id, &ty.module_path)
            || self.effective_type_visibility(ty) == EffectiveVisibility::Private
        {
            return false;
        }
        match field.visibility {
            Visibility::Private => false,
            Visibility::Internal => {
                bypasses_visibility || !test_module.is_same_or_child_of(&ty.module_path)
            }
            Visibility::Public => {
                self.test_crosses_surface_for_type(test_module, bypasses_visibility, ty)
            }
        }
    }

    fn is_surface_active_for(&self, file_id: FileId, module: &ModulePath) -> bool {
        let Some(&lang) = self.file_languages.get(file_id.0) else {
            return false;
        };
        self.active_languages.contains(&lang)
            && !self.binary_entry_files.contains(&file_id)
            && !self.bin_only_modules.contains(module)
    }

    fn test_crosses_surface_for_symbol(
        &self,
        test_origin: (&ModulePath, bool),
        target: (FileId, &ModulePath, &str, Visibility, bool),
    ) -> bool {
        let (test_module, bypasses_visibility) = test_origin;
        let (file_id, target_module, symbol, syntactic, is_default_export) = target;
        if !self.is_surface_active_for(file_id, target_module)
            || (!bypasses_visibility && test_module.is_same_or_child_of(target_module))
        {
            return false;
        }
        if self.effective_symbol_visibility(
            file_id,
            target_module,
            symbol,
            syntactic,
            is_default_export,
        ) != EffectiveVisibility::Internal
        {
            return false;
        }
        if syntactic == Visibility::Internal {
            return true;
        }
        let lang = self.file_languages[file_id.0];
        if self.module_tree_languages.contains(&lang)
            && !self.is_public_in_module_tree(target_module, symbol, is_default_export)
        {
            return true;
        }
        self.bypasses_enclosing_barrel(test_module, target_module, symbol, is_default_export)
    }

    fn is_public_in_module_tree(
        &self,
        module: &ModulePath,
        symbol: &str,
        is_default_export: bool,
    ) -> bool {
        self.public_modules.contains(module)
            || (!is_default_export && self.wildcard_reexported_modules.contains(module))
            || self
                .reexported_symbols
                .contains(&(module.clone(), symbol.to_string()))
            || (is_default_export
                && self
                    .reexported_symbols
                    .contains(&(module.clone(), String::from("default"))))
    }

    fn is_module_symbol_public(
        &self,
        file_id: FileId,
        module: &ModulePath,
        symbol: &str,
        is_default_export: bool,
    ) -> bool {
        if let Some(&lang) = self.file_languages.get(file_id.0)
            && self.module_tree_languages.contains(&lang)
            && !self.is_public_in_module_tree(module, symbol, is_default_export)
        {
            return false;
        }
        let outermost_barrel = self
            .barrels
            .iter()
            .filter(|b| module.is_same_or_child_of(&b.barrel_module) && *module != b.barrel_module)
            .min_by_key(|b| b.barrel_module.segments().len());
        if let Some(barrel) = outermost_barrel {
            return barrel.exports(module, symbol, is_default_export);
        }
        true
    }
}
