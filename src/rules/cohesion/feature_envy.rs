use super::CohesionRuleId;
use super::lexical::{LexicalCorpus, cosine_similarity};
use super::type_cohesion::is_trivial_accessor;
use crate::ast::codebase::{Codebase, FuncId, IndexedFunc, IndexedType, TypeId};
use crate::ast::item::{CallKind, CallReceiver, ParamAccess, TypeKind, Visibility};
use crate::ast::similarity::{AstKind, NodeLabel};
use crate::config::CohesionConfig;
use crate::diagnostic::Diagnostic;
use std::collections::{HashMap, HashSet};

struct TypeSurface<'a> {
    fields: HashSet<&'a str>,
    methods: Vec<&'a IndexedFunc>,
    methods_by_name: HashMap<&'a str, Vec<&'a IndexedFunc>>,
}

impl<'a> TypeSurface<'a> {
    fn from_type(codebase: &'a Codebase, ty: &'a IndexedType) -> Self {
        let fields: HashSet<&'a str> = ty.decl.fields.iter().map(|f| f.name.as_str()).collect();
        let mut methods = Vec::new();
        let mut methods_by_name: HashMap<&'a str, Vec<&'a IndexedFunc>> = HashMap::new();
        for m in codebase.inherent_methods_for(ty.id) {
            if !m.decl.has_self_receiver()
                || m.decl.role.is_test_code()
                || codebase.file(m.file_id).is_test_file()
            {
                continue;
            }
            methods.push(m);
            methods_by_name
                .entry(m.decl.name.as_str())
                .or_default()
                .push(m);
        }
        Self {
            fields,
            methods,
            methods_by_name,
        }
    }

    fn has_field_or_method(&self, name: &str) -> bool {
        self.fields.contains(name) || self.methods_by_name.contains_key(name)
    }

    fn has_no_transitive_effects(&self, func: &IndexedFunc, visited: &mut HashSet<FuncId>) -> bool {
        if !visited.insert(func.id) {
            return true;
        }
        if inspect_pure_body(func).is_none() {
            return false;
        }
        for call in &func.decl.calls {
            if call.receiver == Some(CallReceiver::SelfInstance)
                && let Some(targets) = self.methods_by_name.get(call.callee.as_str())
            {
                for &callee in targets {
                    if !self.has_no_transitive_effects(callee, visited) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

struct EnvyScratch<'a> {
    self_members: HashSet<&'a str>,
    foreign_seen: HashSet<(TypeId, &'a str)>,
    foreign_counts: HashMap<TypeId, usize>,
    query_visited: HashSet<FuncId>,
    effect_visited: HashSet<FuncId>,
}

impl<'a> EnvyScratch<'a> {
    fn new() -> Self {
        Self {
            self_members: HashSet::new(),
            foreign_seen: HashSet::new(),
            foreign_counts: HashMap::new(),
            query_visited: HashSet::new(),
            effect_visited: HashSet::new(),
        }
    }

    fn clear_accesses(&mut self) {
        self.self_members.clear();
        self.foreign_seen.clear();
        self.foreign_counts.clear();
    }

    fn record_foreign(&mut self, target_id: TypeId, member: &'a str) {
        if self.foreign_seen.insert((target_id, member)) {
            *self.foreign_counts.entry(target_id).or_insert(0) += 1;
        }
    }

    fn dominant_target(&self) -> Option<(TypeId, usize)> {
        let mut best: Option<(TypeId, usize)> = None;
        for (&tid, &count) in &self.foreign_counts {
            let is_better = match best {
                None => true,
                Some((best_tid, best_count)) => {
                    count > best_count || (count == best_count && tid < best_tid)
                }
            };
            if is_better {
                best = Some((tid, count));
            }
        }
        best
    }
}

/// Evaluates inherent instance methods for structural + conceptual Feature Envy.
pub(super) struct FeatureEnvyAnalyzer<'a> {
    codebase: &'a Codebase,
    cfg: &'a CohesionConfig,
    corpus: &'a LexicalCorpus,
    dedup_pairs: &'a HashSet<(FuncId, TypeId)>,
    surfaces: HashMap<TypeId, TypeSurface<'a>>,
}

impl<'a> FeatureEnvyAnalyzer<'a> {
    pub(super) fn new(
        codebase: &'a Codebase,
        cfg: &'a CohesionConfig,
        corpus: &'a LexicalCorpus,
        dedup_pairs: &'a HashSet<(FuncId, TypeId)>,
    ) -> Self {
        let mut surfaces = HashMap::new();
        for ty in codebase.types() {
            if !ty.decl.is_stateful_type()
                || ty.decl.in_test_module
                || codebase.file(ty.file_id).is_test_file()
            {
                continue;
            }
            surfaces.insert(ty.id, TypeSurface::from_type(codebase, ty));
        }
        Self {
            codebase,
            cfg,
            corpus,
            dedup_pairs,
            surfaces,
        }
    }

    pub(super) fn check(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut scratch = EnvyScratch::new();

        for func in self.codebase.functions() {
            if let Some(diag) = self.analyze_method(func, &mut scratch) {
                diagnostics.push(diag);
            }
        }
        diagnostics
    }

    fn analyze_method(
        &self,
        func: &'a IndexedFunc,
        scratch: &mut EnvyScratch<'a>,
    ) -> Option<Diagnostic> {
        let owner_id = self.eligible_owner(func)?;
        let owner = self.codebase.ty(owner_id);
        let owner_surface = self.surfaces.get(&owner_id)?;

        self.collect_accesses(func, owner_id, owner_surface, scratch);
        let (target_id, best_count) = scratch.dominant_target()?;
        let fdp = scratch.foreign_counts.len();
        let fe = &self.cfg.feature_envy;
        if best_count < fe.min_foreign_accesses || fdp > fe.max_foreign_providers {
            return None;
        }

        let self_count = scratch.self_members.len();
        let foreign_total: usize = scratch.foreign_counts.values().sum();
        let laa = usize_to_f64(self_count) / usize_to_f64(self_count + foreign_total);
        if laa >= fe.max_laa || laa.is_nan() {
            return None;
        }

        let target = self.codebase.ty(target_id);
        if self.is_exempt_target(func, owner, target, scratch) {
            return None;
        }

        let v_m = self.corpus.method_vector(func);
        let v_own = self
            .corpus
            .type_vector_excluding(self.codebase, owner, Some(func.id));
        let v_target = self
            .corpus
            .type_vector_excluding(self.codebase, target, None);
        let cos_target = cosine_similarity(&v_m, &v_target);
        let cos_own = cosine_similarity(&v_m, &v_own);
        if cos_target <= cos_own || cos_target < fe.min_conceptual_affinity {
            return None;
        }

        Some(
            Diagnostic::new(
                CohesionRuleId::FeatureEnvy.as_str(),
                self.cfg.advisory_severity,
                func.decl.span.clone(),
                format!(
                    "Method `{}` exhibits feature envy toward `{}` ({best_count} foreign accesses, LAA = {laa:.2})",
                    func.decl.name, target.decl.name
                ),
            )
            .with_suggestion(format!(
                "Move `{}` onto `{}` so behavior lives with the state it operates on.",
                func.decl.name, target.decl.name
            ))
            .with_related(
                target.decl.span.clone(),
                format!("envied target type `{}`", target.decl.name),
            ),
        )
    }

    fn eligible_owner(&self, func: &IndexedFunc) -> Option<TypeId> {
        if !func.decl.is_inherent_method()
            || !func.decl.has_self_receiver()
            || func.decl.role.is_test_code()
            || self.codebase.file(func.file_id).is_test_file()
            || func.decl.is_lifecycle
            || func.decl.returns_self_type()
        {
            return None;
        }
        let owner_id = func.resolved_owner?;
        if func.resolved_return_type == Some(owner_id) || !self.surfaces.contains_key(&owner_id) {
            return None;
        }
        Some(owner_id)
    }

    fn collect_accesses(
        &self,
        func: &'a IndexedFunc,
        owner_id: TypeId,
        owner_surface: &TypeSurface<'a>,
        scratch: &mut EnvyScratch<'a>,
    ) {
        scratch.clear_accesses();
        for (fa, resolved) in func
            .decl
            .field_accesses
            .iter()
            .zip(&func.resolved_field_accesses)
        {
            let Some(tid) = *resolved else {
                continue;
            };
            let name = fa.field_name.as_str();
            if tid == owner_id {
                if fa.receiver_is_self && owner_surface.has_field_or_method(name) {
                    scratch.self_members.insert(name);
                }
            } else if let Some(target_surface) = self.surfaces.get(&tid)
                && target_surface.has_field_or_method(name)
            {
                scratch.record_foreign(tid, name);
            }
        }
        for (call, resolved) in func.decl.calls.iter().zip(&func.resolved_call_receivers) {
            let name = call.callee.as_str();
            if call.receiver == Some(CallReceiver::SelfInstance) {
                if owner_surface.methods_by_name.contains_key(name) {
                    scratch.self_members.insert(name);
                }
                continue;
            }
            if call.kind != CallKind::Method {
                continue;
            }
            if let Some(tid) = *resolved
                && tid != owner_id
                && let Some(target_surface) = self.surfaces.get(&tid)
                && target_surface.methods_by_name.contains_key(name)
            {
                scratch.record_foreign(tid, name);
            }
        }
    }

    fn is_exempt_target(
        &self,
        func: &IndexedFunc,
        owner: &IndexedType,
        target: &IndexedType,
        scratch: &mut EnvyScratch<'_>,
    ) -> bool {
        if target.module_path == owner.module_path && target.decl.visibility == Visibility::Private
        {
            return true;
        }
        if self.dedup_pairs.contains(&(func.id, target.id)) {
            return true;
        }
        self.is_non_behavioral_target(target, scratch)
    }

    fn is_non_behavioral_target(&self, ty: &IndexedType, scratch: &mut EnvyScratch<'_>) -> bool {
        let Some(surface) = self.surfaces.get(&ty.id) else {
            return true;
        };
        if surface.methods.is_empty() {
            return ty.decl.exposed_field_count() == ty.decl.fields.len();
        }
        let max_nodes = self.cfg.type_cohesion.max_accessor_nodes;
        for &method in &surface.methods {
            let exempt_method = method.decl.is_lifecycle
                || method.decl.returns_self_type()
                || method.resolved_return_type == Some(ty.id)
                || is_trivial_accessor(method, ty, max_nodes);
            if exempt_method {
                continue;
            }
            scratch.query_visited.clear();
            if !self.is_read_only_query(method, ty, surface, scratch) {
                return false;
            }
        }
        true
    }

    fn is_read_only_query(
        &self,
        func: &IndexedFunc,
        ty: &IndexedType,
        surface: &TypeSurface<'_>,
        scratch: &mut EnvyScratch<'_>,
    ) -> bool {
        if !scratch.query_visited.insert(func.id) {
            return true;
        }
        if !func.decl.params.is_empty() || func.decl.coupling.has_ambient_io {
            return false;
        }
        let Some(call_node_count) = inspect_pure_body(func) else {
            return false;
        };

        match ty.decl.kind {
            TypeKind::Struct => {
                if func.decl.self_receiver != Some(ParamAccess::SharedRef) {
                    return false;
                }
                for call in &func.decl.calls {
                    if call.receiver == Some(CallReceiver::SelfInstance)
                        && let Some(targets) = surface.methods_by_name.get(call.callee.as_str())
                    {
                        for &callee in targets {
                            scratch.effect_visited.clear();
                            if !surface
                                .has_no_transitive_effects(callee, &mut scratch.effect_visited)
                            {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            TypeKind::Class => {
                if call_node_count != func.decl.calls.len() {
                    return false;
                }
                let max_nodes = self.cfg.type_cohesion.max_accessor_nodes;
                for call in &func.decl.calls {
                    if call.receiver != Some(CallReceiver::SelfInstance) {
                        return false;
                    }
                    let Some(targets) = surface.methods_by_name.get(call.callee.as_str()) else {
                        return false;
                    };
                    for &callee in targets {
                        scratch.effect_visited.clear();
                        if !surface.has_no_transitive_effects(callee, &mut scratch.effect_visited) {
                            return false;
                        }
                        let ok = is_trivial_accessor(callee, ty, max_nodes)
                            || self.is_read_only_query(callee, ty, surface, scratch);
                        if !ok {
                            return false;
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}

pub(super) fn check_feature_envy(
    codebase: &Codebase,
    cfg: &CohesionConfig,
    corpus: &LexicalCorpus,
    dedup_pairs: &HashSet<(FuncId, TypeId)>,
) -> Vec<Diagnostic> {
    FeatureEnvyAnalyzer::new(codebase, cfg, corpus, dedup_pairs).check()
}

/// Returns `Some(call_node_count)` if `func` has zero direct field writes, `Assign`, or `Loop` nodes.
fn inspect_pure_body(func: &IndexedFunc) -> Option<usize> {
    for access in &func.decl.field_accesses {
        if access.is_write {
            return None;
        }
    }
    let mut call_nodes = 0usize;
    for node in func.decl.body_tree.nodes() {
        if let NodeLabel::Struct(kind) = node.label() {
            match kind {
                AstKind::Assign | AstKind::Loop => return None,
                AstKind::Call => call_nodes += 1,
                _ => {}
            }
        }
    }
    Some(call_nodes)
}

fn usize_to_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}
