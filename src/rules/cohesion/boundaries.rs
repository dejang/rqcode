use super::CohesionRuleId;
use crate::ast::codebase::{Codebase, FuncId, IndexedFunc, IndexedType, TypeId};
use crate::ast::item::{ImportDecl, ImportKind, Visibility};
use crate::config::CohesionConfig;
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

pub(super) fn check_cyclic_modules(codebase: &Codebase, cfg: &CohesionConfig) -> Vec<Diagnostic> {
    let files = codebase.files();
    let graph = ModuleGraph::build(codebase);
    let mut sccs = TarjanScc::new(graph.edges().len()).find_cycles(graph.edges());
    let mut diagnostics = Vec::new();
    let mut in_scc: HashSet<usize> = HashSet::new();
    let mut cycle_desc = String::new();

    for scc in &mut sccs {
        scc.sort_by(|&a, &b| files[a].relative_path.cmp(&files[b].relative_path));
        in_scc.clear();
        for &member in scc.iter() {
            in_scc.insert(member);
        }

        let primary_idx = scc[0];
        let Some((primary_target, primary_span)) = graph.first_edge_in_scc(primary_idx, &in_scc)
        else {
            continue;
        };

        let message = if scc.len() == 2 {
            format!(
                "cyclic module dependency between `{}` and `{}`",
                files[scc[0]].module_path, files[scc[1]].module_path
            )
        } else {
            cycle_desc.clear();
            for (pos, &member) in scc.iter().enumerate() {
                if pos > 0 {
                    cycle_desc.push_str(", ");
                }
                let _ = write!(cycle_desc, "`{}`", files[member].module_path);
            }
            format!(
                "cyclic module dependency across {} modules: {cycle_desc}",
                scc.len()
            )
        };

        let mut diag = Diagnostic::new(
            CohesionRuleId::CyclicModuleDependency.as_str(),
            cfg.severity,
            primary_span.clone(),
            message,
        )
        .with_suggestion(format!(
            "Extract shared types or stateless helpers out of `{}` / `{}` into a lower-level module so dependencies flow in one direction.",
            files[primary_idx].module_path, files[primary_target].module_path
        ));

        for &other_idx in &scc[1..] {
            if let Some((target, span)) = graph.first_edge_in_scc(other_idx, &in_scc) {
                diag = diag.with_related(
                    span.clone(),
                    format!(
                        "`{}` imports `{}`",
                        files[other_idx].module_path, files[target].module_path
                    ),
                );
            }
        }

        diagnostics.push(diag);
    }

    diagnostics
}

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
        if func.decl.is_test_code()
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

struct ModuleGraph<'a> {
    file_by_module: HashMap<&'a [String], usize>,
    edges: Vec<Vec<(usize, &'a Span)>>,
    resolved_buf: Vec<String>,
}

impl<'a> ModuleGraph<'a> {
    fn build(codebase: &'a Codebase) -> Self {
        let files = codebase.files();
        let mut file_by_module: HashMap<&'a [String], usize> = HashMap::with_capacity(files.len());
        for (idx, file) in files.iter().enumerate() {
            if !file.is_test_file() {
                file_by_module.insert(file.module_path.segments(), idx);
            }
        }

        let mut graph = Self {
            file_by_module,
            edges: vec![Vec::new(); files.len()],
            resolved_buf: Vec::new(),
        };
        let mut seen_targets: HashSet<usize> = HashSet::new();

        for (from_idx, file) in files.iter().enumerate() {
            if file.is_test_file() {
                continue;
            }
            seen_targets.clear();
            for imp in &file.imports {
                if imp.in_test_module {
                    continue;
                }
                let Some(to_idx) = graph.resolve_import_target(imp) else {
                    continue;
                };
                if to_idx == from_idx {
                    continue;
                }
                let to_mod = &files[to_idx].module_path;
                if file.module_path.is_same_or_child_of(to_mod)
                    || to_mod.is_same_or_child_of(&file.module_path)
                {
                    continue;
                }
                if seen_targets.insert(to_idx) {
                    graph.edges[from_idx].push((to_idx, &imp.span));
                }
            }
        }

        graph
    }

    fn edges(&self) -> &[Vec<(usize, &'a Span)>] {
        &self.edges
    }

    fn first_edge_in_scc(
        &self,
        from_idx: usize,
        in_scc: &HashSet<usize>,
    ) -> Option<(usize, &'a Span)> {
        for &(target, span) in &self.edges[from_idx] {
            if in_scc.contains(&target) {
                return Some((target, span));
            }
        }
        None
    }

    fn resolve_import_target(&mut self, imp: &ImportDecl) -> Option<usize> {
        let target = imp.resolved_module.as_ref()?;
        self.resolved_buf.clear();
        self.resolved_buf.extend(target.segments().iter().cloned());
        if imp.kind == ImportKind::Named && !imp.symbol.is_empty() {
            self.resolved_buf.push(imp.symbol.clone());
            if let Some(&idx) = self.file_by_module.get(self.resolved_buf.as_slice()) {
                return Some(idx);
            }
            self.resolved_buf.pop();
        }
        self.file_by_module
            .get(self.resolved_buf.as_slice())
            .copied()
    }
}

struct TarjanScc {
    next_index: usize,
    indices: Vec<Option<usize>>,
    lowlinks: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    sccs: Vec<Vec<usize>>,
}

impl TarjanScc {
    fn new(count: usize) -> Self {
        Self {
            next_index: 0,
            indices: vec![None; count],
            lowlinks: vec![0; count],
            on_stack: vec![false; count],
            stack: Vec::with_capacity(count),
            sccs: Vec::new(),
        }
    }

    fn find_cycles(mut self, edges: &[Vec<(usize, &Span)>]) -> Vec<Vec<usize>> {
        for node in 0..edges.len() {
            if self.indices[node].is_none() {
                self.strongconnect(node, edges);
            }
        }
        self.sccs
    }

    fn strongconnect(&mut self, v: usize, edges: &[Vec<(usize, &Span)>]) {
        let idx = self.next_index;
        self.next_index += 1;
        self.indices[v] = Some(idx);
        self.lowlinks[v] = idx;
        let stack_start = self.stack.len();
        self.stack.push(v);
        self.on_stack[v] = true;

        for &(w, _) in &edges[v] {
            if let Some(w_idx) = self.indices[w] {
                if self.on_stack[w] && w_idx < self.lowlinks[v] {
                    self.lowlinks[v] = w_idx;
                }
            } else {
                self.strongconnect(w, edges);
                if self.lowlinks[w] < self.lowlinks[v] {
                    self.lowlinks[v] = self.lowlinks[w];
                }
            }
        }

        if self.lowlinks[v] == idx {
            let scc = self.stack.split_off(stack_start);
            for &member in &scc {
                self.on_stack[member] = false;
            }
            if scc.len() >= 2 {
                self.sccs.push(scc);
            }
        }
    }
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
            if func.decl.is_test_code() || codebase.file(func.file_id).is_test_file() {
                continue;
            }

            for init in &func.resolved_field_inits {
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

            for access in &func.decl.field_accesses {
                if let Some(&type_id) = func.resolved_local_types.get(&access.receiver_param) {
                    let ty = codebase.ty(type_id);
                    if func.module_path != ty.module_path {
                        external_field_accesses.entry(type_id).or_default().push(
                            ExternalBoundaryRef {
                                func,
                                span: access.span.clone(),
                                field_name: access.field_name.as_str(),
                            },
                        );
                    }
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
            if field.visibility == Visibility::Internal {
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
