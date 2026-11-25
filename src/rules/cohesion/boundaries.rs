use super::CohesionRuleId;
use crate::ast::FileRole;
use crate::ast::codebase::{Codebase, FuncId, IndexedFunc, IndexedType, TypeId};
use crate::ast::item::{ExportDecl, Visibility};
use crate::config::CohesionConfig;
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet};

pub(super) fn check_leaky_boundaries(codebase: &Codebase, cfg: &CohesionConfig) -> Vec<Diagnostic> {
    let index = BoundaryIndex::build(codebase);
    let mut scratch = BoundaryScratch {
        restricted_fields: HashSet::new(),
        seen_funcs: HashSet::new(),
        related: Vec::new(),
    };
    let mut diagnostics = Vec::new();

    for ty in codebase.types() {
        if !ty.decl.is_stateful_type()
            || ty.decl.in_test_module
            || codebase.file(ty.file_id).is_test_file()
        {
            continue;
        }
        let inherent_count = codebase.inherent_methods_for(ty.id).len();
        if let Some(diag) = index.check_type(ty, inherent_count, cfg, &mut scratch) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

pub(super) fn check_stateless_impl_methods(
    codebase: &Codebase,
    cfg: &CohesionConfig,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for func in codebase.functions() {
        if func.decl.role.is_test_code()
            || codebase.file(func.file_id).is_test_file()
            || !func.decl.is_inherent_method()
            || func.decl.has_self_receiver()
        {
            continue;
        }
        let returns_self = func.decl.returns_self_type()
            || (func.resolved_owner.is_some() && func.resolved_return_type == func.resolved_owner);
        if returns_self {
            continue;
        }
        let Some(owner) = func.decl.owner_type.as_deref() else {
            continue;
        };

        let mut diag = Diagnostic::new(
            CohesionRuleId::StatelessImplMethod.as_str(),
            cfg.severity,
            func.decl.span.clone(),
            format!(
                "method `{}` takes no instance receiver and does not return `{owner}`; either behavior does not belong to this concept or the state is improperly defined",
                func.display_name(),
            ),
        )
        .with_suggestion(format!(
            "Make `{}` an instance method on `{owner}` (holding the required state on `{owner}`), return `{owner}` if this is a constructor, or move the behavior onto the type it actually operates on.",
            func.decl.name
        ));

        if let Some(owner_id) = func.resolved_owner {
            let ty = codebase.ty(owner_id);
            diag = diag.with_related(
                ty.decl.span.clone(),
                format!("`{}` defined here", ty.decl.name),
            );
        }

        diagnostics.push(diag);
    }
    diagnostics
}

pub(super) fn check_barrel_modules(codebase: &Codebase, cfg: &CohesionConfig) -> Vec<Diagnostic> {
    let barrels_cfg = &cfg.boundaries.barrels;
    let mut diagnostics = Vec::new();
    let mut seen_spans = HashSet::new();
    let span_key = |s: &Span| (s.start_line(), s.start_col(), s.end_line(), s.end_col());
    for file in codebase.files() {
        if file.is_test_file()
            || !file.is_barrel()
            || (barrels_cfg.allow_library_root_barrel && file.role == FileRole::LibraryRoot)
        {
            continue;
        }
        let mut reexports = file.exports.iter().filter(|exp| ExportDecl::is_reexport(exp));
        let Some(first) = reexports.next() else {
            continue;
        };
        let mut diag = Diagnostic::new(
            CohesionRuleId::NoBarrelModule.as_str(),
            cfg.severity,
            first.span.clone(),
            format!(
                "file `{}` acts as a barrel module re-exporting symbols from other modules",
                file.relative_path.display()
            ),
        )
        .with_suggestion(
            "Remove this barrel file and import symbols directly from their defining source modules.",
        );

        seen_spans.clear();
        seen_spans.insert(span_key(&first.span));
        let mut added = 0usize;
        for exp in reexports {
            if added >= barrels_cfg.max_barrel_related {
                break;
            }
            if seen_spans.insert(span_key(&exp.span)) {
                diag = diag.with_related(
                    exp.span.clone(),
                    String::from("additional re-export in barrel module"),
                );
                added += 1;
            }
        }
        diagnostics.push(diag);
    }
    diagnostics
}

struct ExternalBoundaryRef<'a> {
    func: &'a IndexedFunc,
    span: Span,
    field_name: &'a str,
}

struct BoundaryScratch<'a> {
    restricted_fields: HashSet<&'a str>,
    seen_funcs: HashSet<FuncId>,
    related: Vec<(Span, String)>,
}

impl<'a> BoundaryScratch<'a> {
    fn reset_related(&mut self) {
        self.seen_funcs.clear();
        self.related.clear();
    }

    fn push_restricted_refs(&mut self, refs: Option<&[ExternalBoundaryRef<'a>]>, verb: &str) {
        let Some(refs) = refs else {
            return;
        };
        for ext in refs {
            if self.restricted_fields.contains(ext.field_name)
                && self.seen_funcs.insert(ext.func.id)
            {
                self.related.push((
                    ext.span.clone(),
                    format!(
                        "`{}` in `{}` {verb} internal field `{}`",
                        ext.func.display_name(),
                        ext.func.module_path,
                        ext.field_name
                    ),
                ));
            }
        }
    }

    fn finish_diagnostic(
        &mut self,
        ty: &IndexedType,
        cfg: &CohesionConfig,
        message: String,
        suggestion: String,
    ) -> Diagnostic {
        let mut diag = Diagnostic::new(
            CohesionRuleId::LeakyModuleBoundary.as_str(),
            cfg.severity,
            ty.decl.span.clone(),
            message,
        )
        .with_suggestion(suggestion);
        for (span, label) in self.related.drain(..) {
            diag = diag.with_related(span, label);
        }
        diag
    }
}

struct BoundaryIndex<'a> {
    external_field_inits: HashMap<TypeId, Vec<ExternalBoundaryRef<'a>>>,
    external_field_accesses: HashMap<TypeId, Vec<ExternalBoundaryRef<'a>>>,
}

impl<'a> BoundaryIndex<'a> {
    fn build(codebase: &'a Codebase) -> Self {
        let mut external_field_inits: HashMap<TypeId, Vec<ExternalBoundaryRef<'a>>> =
            HashMap::new();
        let mut external_field_accesses: HashMap<TypeId, Vec<ExternalBoundaryRef<'a>>> =
            HashMap::new();

        for func in codebase.functions() {
            if func.decl.role.is_test_code() || codebase.file(func.file_id).is_test_file() {
                continue;
            }

            for init in &func.resolved_field_inits {
                if func.resolved_owner == Some(init.type_id) {
                    continue;
                }
                let ty = codebase.ty(init.type_id);
                if func.module_path != ty.module_path {
                    external_field_inits.entry(init.type_id).or_default().push(
                        ExternalBoundaryRef {
                            func,
                            span: init.span.clone(),
                            field_name: init.field_name.as_str(),
                        },
                    );
                }
            }

            for (access, resolved_ty) in func
                .decl
                .field_accesses
                .iter()
                .zip(&func.resolved_field_accesses)
            {
                let Some(type_id) = *resolved_ty else {
                    continue;
                };
                if access.receiver_is_self || func.resolved_owner == Some(type_id) {
                    continue;
                }
                let ty = codebase.ty(type_id);
                if func.module_path != ty.module_path {
                    external_field_accesses
                        .entry(type_id)
                        .or_default()
                        .push(ExternalBoundaryRef {
                            func,
                            span: access.span.clone(),
                            field_name: access.field_name.as_str(),
                        });
                }
            }
        }

        Self {
            external_field_inits,
            external_field_accesses,
        }
    }

    fn check_type(
        &self,
        ty: &'a IndexedType,
        inherent_method_count: usize,
        cfg: &CohesionConfig,
        scratch: &mut BoundaryScratch<'a>,
    ) -> Option<Diagnostic> {
        let mut exposed_mut_ref_count = 0usize;
        scratch.restricted_fields.clear();
        for field in &ty.decl.fields {
            if field.is_mut_ref && field.visibility.is_externally_visible() {
                exposed_mut_ref_count += 1;
            }
            if field.visibility == Visibility::Internal && field.explicit_visibility {
                scratch.restricted_fields.insert(field.name.as_str());
            }
        }

        if exposed_mut_ref_count >= 2 {
            scratch.reset_related();
            if let Some(inits) = self.external_field_inits.get(&ty.id) {
                for ext in inits {
                    if scratch.seen_funcs.insert(ext.func.id) {
                        scratch.related.push((
                            ext.span.clone(),
                            format!(
                                "constructed via field initializer in `{}` (`{}`)",
                                ext.func.module_path,
                                ext.func.display_name()
                            ),
                        ));
                    }
                }
            }
            if !scratch.related.is_empty() || ty.decl.visibility == Visibility::Internal {
                return Some(scratch.finish_diagnostic(
                    ty,
                    cfg,
                    format!(
                        "`{}` (in `{}`) exposes {} mutable reference field(s) across a module boundary",
                        ty.decl.name, ty.module_path, exposed_mut_ref_count
                    ),
                    format!(
                        "Own the accumulated output state inside `{}` (or a dedicated return type) instead of wiring mutable reference fields across modules.",
                        ty.decl.name
                    ),
                ));
            }
        }

        if inherent_method_count == 0 || scratch.restricted_fields.is_empty() {
            return None;
        }

        scratch.reset_related();
        scratch.push_restricted_refs(
            self.external_field_accesses.get(&ty.id).map(Vec::as_slice),
            "accesses",
        );
        scratch.push_restricted_refs(
            self.external_field_inits.get(&ty.id).map(Vec::as_slice),
            "initializes",
        );
        if scratch.related.is_empty() {
            return None;
        }

        Some(scratch.finish_diagnostic(
            ty,
            cfg,
            format!(
                "`{}` (in `{}`) has {} method(s) but leaks {} internal field(s) accessed directly outside its module",
                ty.decl.name,
                ty.module_path,
                inherent_method_count,
                scratch.restricted_fields.len()
            ),
            format!(
                "Make `{}`'s fields private and expose a cohesive method returning an owned result so callers do not reach into its internal state.",
                ty.decl.name
            ),
        ))
    }
}
