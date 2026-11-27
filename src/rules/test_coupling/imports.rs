use super::TestCouplingIndex;
use crate::ast::ModulePath;
use crate::ast::codebase::{FileId, FuncId};
use crate::ast::item::{ExportOrigin, ImportDecl, ImportKind, ImportOrigin, Visibility};
use std::collections::HashSet;

#[derive(Default)]
struct ExportWalk {
    visited: HashSet<(ModulePath, String, bool)>,
    out: Vec<FuncId>,
    depth_exhausted: bool,
}

/// Result of resolving explicit (`Default` or `Named`) imports for a local identifier.
pub(super) struct ResolvedImports {
    pub(super) funcs: Vec<FuncId>,
    /// True when every matching explicit import resolved definitively (a `StandardValue`
    /// package, a concrete target function, or a known production module whose bounded export
    /// walk completed without exhausting the re-export hop limit), so global name-only fallback
    /// should not run in [`ImportResolver::ident_targets`].
    pub(super) definitive: bool,
}

/// Resolves free-function calls, default exports, and barrel re-exports against a [`TestCouplingIndex`].
pub(super) struct ImportResolver<'r, 'a> {
    index: &'r TestCouplingIndex<'a>,
    max_reexport_depth: usize,
}

impl<'r, 'a> ImportResolver<'r, 'a> {
    pub(super) fn new(index: &'r TestCouplingIndex<'a>, max_reexport_depth: usize) -> Self {
        Self {
            index,
            max_reexport_depth,
        }
    }

    /// Resolve free-function calls bound by explicit (`Default` or `Named`) imports in `file_id`.
    ///
    /// Returns `None` when no matching explicit import binds `local_name`, allowing the caller
    /// to fall back to same-module lookup. Returns `Some(ResolvedImports)` when an explicit import
    /// binds `local_name` (including empty `funcs` for `StandardValue` external imports or
    /// shadowed private symbols).
    pub(super) fn resolve_imported_free_funcs(
        &self,
        file_id: FileId,
        caller_is_test: bool,
        local_name: &str,
    ) -> Option<ResolvedImports> {
        let candidates = self.index.explicit_imports_for(file_id, local_name);
        if candidates.is_empty() {
            return None;
        }
        let file = self.index.codebase().file(file_id);
        let has_inline_test_imports = self.index.file_has_inline_test_imports(file_id);
        let matches_scope = |imp: &ImportDecl| -> bool {
            if file.is_test_file() {
                true
            } else if !caller_is_test {
                !imp.in_test_module
            } else if has_inline_test_imports {
                imp.in_test_module
            } else {
                !imp.in_test_module
            }
        };

        let mut matched_any = false;
        let mut all_definitive = true;
        let mut targets = Vec::new();
        for &imp in candidates {
            if !matches_scope(imp) {
                continue;
            }
            matched_any = true;
            if let Some(ref target_mod) = imp.resolved_module {
                let is_default = imp.kind == ImportKind::Default;
                let (found, depth_exhausted) = self.resolve_module_export_with_status(
                    target_mod,
                    imp.symbol.as_str(),
                    is_default,
                );
                if found.is_empty() {
                    let known_mod = self.index.has_prod_module(target_mod);
                    let mut fallback_hit = false;
                    if !(is_default && known_mod)
                        && let Some(fid) =
                            self.unique_import_fallback(imp.symbol.as_str(), &file.module_path)
                    {
                        targets.push(fid);
                        fallback_hit = true;
                    }
                    if !fallback_hit && (!known_mod || depth_exhausted) {
                        all_definitive = false;
                    }
                } else {
                    targets.extend(found);
                }
            } else if imp.origin != ImportOrigin::StandardValue {
                if let Some(fid) =
                    self.unique_import_fallback(imp.symbol.as_str(), &file.module_path)
                {
                    targets.push(fid);
                } else {
                    all_definitive = false;
                }
            }
        }

        if !matched_any {
            return None;
        }
        targets.sort_unstable();
        targets.dedup();
        Some(ResolvedImports {
            funcs: targets,
            definitive: all_definitive,
        })
    }

    /// Resolve a symbol (or default export) exported by `module`, following up to
    /// `self.max_reexport_depth` barrel hops across direct hits, renamed local exports,
    /// named/wildcard re-exports, and two-step `ImportedBinding` exports.
    pub(super) fn resolve_module_export(
        &self,
        module: &ModulePath,
        symbol: &str,
        is_default: bool,
    ) -> Vec<FuncId> {
        self.resolve_module_export_with_status(module, symbol, is_default)
            .0
    }

    fn resolve_module_export_with_status(
        &self,
        module: &ModulePath,
        symbol: &str,
        is_default: bool,
    ) -> (Vec<FuncId>, bool) {
        let mut walk = ExportWalk::default();
        self.resolve_module_export_recursive(module, symbol, is_default, 0, &mut walk);
        walk.out.sort_unstable();
        walk.out.dedup();
        (walk.out, walk.depth_exhausted)
    }

    /// Resolve identifier targets for production reachability and caller scopes.
    ///
    /// Includes explicit import targets and same-module free functions. Skips the global
    /// name-based `prod_free_named` fallback only when a same-module definition exists (so a
    /// private same-module helper does not keep an unimported cross-module function of the same
    /// name alive) or when every matching explicit import resolved definitively (a `StandardValue`
    /// package, a concrete target, or a known production module whose bounded export walk
    /// completed). Unresolved `Project` / `ExternalBoundary` imports (such as ambiguous `@/`
    /// path aliases) retain the conservative global fallback so production edges are never lost.
    /// When `include_methods` is true, also includes callable named methods.
    pub(super) fn ident_targets(
        &self,
        file_id: FileId,
        caller_is_test: bool,
        module_path: &'a ModulePath,
        ident: &'a str,
        include_methods: bool,
    ) -> Vec<FuncId> {
        let mut targets = Vec::new();
        let imported = self.resolve_imported_free_funcs(file_id, caller_is_test, ident);
        let in_mod = self.index.prod_free_in_mod(module_path, ident);
        let definitive_import = imported.as_ref().is_some_and(|res| res.definitive);
        if let Some(ref res) = imported {
            targets.extend_from_slice(&res.funcs);
        }
        targets.extend_from_slice(in_mod);
        if !definitive_import && in_mod.is_empty() {
            for &fid in self.index.prod_free_named(ident) {
                if self
                    .index
                    .is_callable_from(self.index.codebase().func(fid), module_path)
                {
                    targets.push(fid);
                }
            }
        }
        if include_methods {
            for &fid in self.index.prod_methods_named(ident) {
                if self
                    .index
                    .is_callable_from(self.index.codebase().func(fid), module_path)
                {
                    targets.push(fid);
                }
            }
        }
        targets
    }

    fn resolve_module_export_recursive(
        &self,
        module: &ModulePath,
        symbol: &str,
        is_default: bool,
        depth: usize,
        walk: &mut ExportWalk,
    ) {
        if depth > self.max_reexport_depth {
            walk.depth_exhausted = true;
            return;
        }
        if !walk
            .visited
            .insert((module.clone(), symbol.to_string(), is_default))
        {
            return;
        }
        if is_default {
            if let Some(fid) = self.index.prod_default_func_in_mod(module) {
                walk.out.push(fid);
                return;
            }
        } else {
            let direct = self.index.prod_free_in_mod(module, symbol);
            if !direct.is_empty() {
                walk.out.extend_from_slice(direct);
                return;
            }
        }

        for &file_id in self.index.prod_files_in_mod(module) {
            let file = self.index.codebase().file(file_id);
            for exp in &file.exports {
                let is_bare_wildcard = exp.is_wildcard && exp.exported_name.is_empty();
                let matches = if is_default {
                    exp.is_default
                } else if exp.is_wildcard {
                    is_bare_wildcard
                } else {
                    exp.exported_name == symbol
                };
                if !matches {
                    continue;
                }
                if let Some(ref src_mod) = exp.source_module {
                    if is_bare_wildcard {
                        self.resolve_module_export_recursive(
                            src_mod,
                            symbol,
                            false,
                            depth + 1,
                            walk,
                        );
                    } else if exp.symbol == "default" {
                        self.resolve_module_export_recursive(
                            src_mod,
                            "default",
                            true,
                            depth + 1,
                            walk,
                        );
                    } else {
                        self.resolve_module_export_recursive(
                            src_mod,
                            exp.symbol.as_str(),
                            false,
                            depth + 1,
                            walk,
                        );
                    }
                } else if exp.origin == ExportOrigin::ImportedBinding {
                    self.follow_imported_binding(file_id, exp.symbol.as_str(), depth + 1, walk);
                } else if exp.origin == ExportOrigin::Local {
                    walk.out.extend_from_slice(
                        self.index.prod_free_in_mod(module, exp.symbol.as_str()),
                    );
                }
            }
        }
    }

    fn follow_imported_binding(
        &self,
        file_id: FileId,
        symbol: &str,
        next_depth: usize,
        walk: &mut ExportWalk,
    ) {
        for &imp in self.index.explicit_imports_for(file_id, symbol) {
            if imp.in_test_module {
                continue;
            }
            if let Some(ref src_mod) = imp.resolved_module {
                match imp.kind {
                    ImportKind::Default => {
                        self.resolve_module_export_recursive(
                            src_mod, "default", true, next_depth, walk,
                        );
                    }
                    ImportKind::Named => {
                        self.resolve_module_export_recursive(
                            src_mod,
                            imp.symbol.as_str(),
                            false,
                            next_depth,
                            walk,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn unique_import_fallback(&self, symbol: &str, caller_mod: &ModulePath) -> Option<FuncId> {
        let mut filtered = self
            .index
            .prod_free_named(symbol)
            .iter()
            .copied()
            .filter(|&fid| {
                let target = self.index.codebase().func(fid);
                self.index.is_callable_from(target, caller_mod)
                    && !(target.module_path == *caller_mod
                        && target.decl.visibility == Visibility::Private)
            });
        let first = filtered.next()?;
        if filtered.next().is_some() {
            return None;
        }
        Some(first)
    }
}
