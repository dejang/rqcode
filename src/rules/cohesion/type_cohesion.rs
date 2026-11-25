use super::CohesionRuleId;
use super::lexical::{LexicalCorpus, SparseTfIdf, cosine_similarity};
use crate::ast::codebase::{Codebase, IndexedFunc, IndexedType};
use crate::ast::item::CallReceiver;
use crate::ast::similarity::{AstKind, NodeLabel};
use crate::config::CohesionConfig;
use crate::diagnostic::Diagnostic;
use std::collections::HashMap;

/// Structural predicate identifying trivial single-field getters and setters without name heuristics.
#[must_use]
pub fn is_trivial_accessor(
    func: &IndexedFunc,
    ty: &IndexedType,
    max_accessor_nodes: usize,
) -> bool {
    if func.resolved_owner != Some(ty.id)
        || !func.decl.is_inherent_method()
        || !func.decl.has_self_receiver()
        || func.decl.role.is_test_code()
        || !func.decl.calls.is_empty()
        || func.decl.body_tree.len() > max_accessor_nodes
        || func.decl.field_accesses.len() != 1
        || func.resolved_field_accesses.first().copied().flatten() != Some(ty.id)
    {
        return false;
    }

    let access = &func.decl.field_accesses[0];
    if !access.receiver_is_self {
        return false;
    }
    let mut is_declared_field = false;
    for field in &ty.decl.fields {
        if field.name == access.field_name {
            is_declared_field = true;
            break;
        }
    }
    if !is_declared_field {
        return false;
    }

    let mut assign_count = 0usize;
    for node in func.decl.body_tree.nodes() {
        if let NodeLabel::Struct(kind) = node.label() {
            match kind {
                AstKind::If
                | AstKind::Branch
                | AstKind::Loop
                | AstKind::Try
                | AstKind::Closure
                | AstKind::Binary
                | AstKind::Index
                | AstKind::Construct
                | AstKind::Bind
                | AstKind::Call => return false,
                AstKind::Assign => assign_count += 1,
                _ => {}
            }
        }
    }

    let is_read_accessor = func.decl.params.is_empty() && assign_count == 0 && access.is_read;
    let is_write_accessor =
        func.decl.params.len() == 1 && access.is_write && !access.is_read && assign_count <= 1;
    is_read_accessor || is_write_accessor
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            if ra < rb {
                self.parent[rb] = ra;
            } else {
                self.parent[ra] = rb;
            }
        }
    }

    fn into_components(mut self) -> Vec<Vec<usize>> {
        let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
        for idx in 0..self.parent.len() {
            let root = self.find(idx);
            by_root.entry(root).or_default().push(idx);
        }
        let mut comps: Vec<Vec<usize>> = by_root.into_values().collect();
        comps.sort_by_key(|c| c[0]);
        comps
    }
}

struct MethodGraph {
    m_elig: Vec<usize>,
    direct_calls: Vec<Vec<usize>>,
    closure: Vec<Vec<bool>>,
}

impl MethodGraph {
    fn propagate_closure(&mut self, num_fields: usize) {
        let mut changed = true;
        while changed {
            changed = false;
            for (u, callees) in self.direct_calls.iter().enumerate() {
                for &v in callees {
                    if u == v {
                        continue;
                    }
                    for f in 0..num_fields {
                        if self.closure[v][f] && !self.closure[u][f] {
                            self.closure[u][f] = true;
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    fn compute_tcc(&self, m_field: &[usize]) -> f64 {
        let total_pairs = m_field.len() * (m_field.len() - 1) / 2;
        let mut connected = 0usize;
        for i in 0..m_field.len() {
            for j in (i + 1)..m_field.len() {
                if shares_any_true(&self.closure[m_field[i]], &self.closure[m_field[j]]) {
                    connected += 1;
                }
            }
        }
        f64::from(u32::try_from(connected).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(total_pairs).unwrap_or(u32::MAX))
    }
}

struct ClusterSummary<'a> {
    methods: Vec<&'a IndexedFunc>,
    field_mask: Vec<bool>,
}

impl<'a> ClusterSummary<'a> {
    fn from_component(
        comp: &[usize],
        methods: &[&'a IndexedFunc],
        graph: &MethodGraph,
        num_fields: usize,
    ) -> Option<Self> {
        let mut field_mask = vec![false; num_fields];
        let mut comp_methods = Vec::with_capacity(comp.len());
        for &elig_pos in comp {
            let inst_idx = graph.m_elig[elig_pos];
            comp_methods.push(methods[inst_idx]);
            for f in 0..num_fields {
                field_mask[f] |= graph.closure[inst_idx][f];
            }
        }
        has_any_true(&field_mask).then_some(Self {
            methods: comp_methods,
            field_mask,
        })
    }

    fn merged(group: &[usize], clusters: &[Self], num_fields: usize) -> Self {
        let mut field_mask = vec![false; num_fields];
        let mut methods = Vec::new();
        for &idx in group {
            methods.extend_from_slice(&clusters[idx].methods);
            for f in 0..num_fields {
                field_mask[f] |= clusters[idx].field_mask[f];
            }
        }
        Self {
            methods,
            field_mask,
        }
    }
}

/// Evaluates stateful types for structurally and conceptually disjoint responsibilities.
pub(super) struct TypeCohesionAnalyzer<'a> {
    codebase: &'a Codebase,
    cfg: &'a CohesionConfig,
    corpus: &'a LexicalCorpus,
}

impl<'a> TypeCohesionAnalyzer<'a> {
    pub(super) fn new(
        codebase: &'a Codebase,
        cfg: &'a CohesionConfig,
        corpus: &'a LexicalCorpus,
    ) -> Self {
        Self {
            codebase,
            cfg,
            corpus,
        }
    }

    pub(super) fn check(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for ty in self.codebase.types() {
            if !ty.decl.is_stateful_type()
                || ty.decl.in_test_module
                || self.codebase.file(ty.file_id).is_test_file()
                || ty.decl.fields.len() < self.cfg.type_cohesion.min_fields
            {
                continue;
            }
            if let Some(diag) = self.analyze_type(ty) {
                diagnostics.push(diag);
            }
        }
        diagnostics
    }

    fn analyze_type(&self, ty: &'a IndexedType) -> Option<Diagnostic> {
        let tc = &self.cfg.type_cohesion;
        let methods = self.collect_instance_methods(ty);
        let graph = self.build_method_graph(ty, &methods);
        if graph.m_elig.len() < tc.min_methods {
            return None;
        }

        let m_field: Vec<usize> = graph
            .m_elig
            .iter()
            .copied()
            .filter(|&inst_idx| has_any_true(&graph.closure[inst_idx]))
            .collect();
        if m_field.len() < tc.min_field_using_methods {
            return None;
        }

        let tcc = graph.compute_tcc(&m_field);
        if tcc >= tc.max_tcc || tcc.is_nan() {
            return None;
        }

        let structural = self.extract_substantial_clusters(ty, &methods, &graph);
        if structural.len() < 2 {
            return None;
        }

        let final_clusters = self.bridge_conceptually(ty, &structural);
        if final_clusters.len() < 2 {
            return None;
        }
        Some(self.build_diagnostic(ty, tcc, &final_clusters))
    }

    fn collect_instance_methods(&self, ty: &IndexedType) -> Vec<&'a IndexedFunc> {
        self.codebase
            .inherent_methods_for(ty.id)
            .into_iter()
            .filter(|m| {
                m.decl.has_self_receiver()
                    && !m.decl.role.is_test_code()
                    && !self.codebase.file(m.file_id).is_test_file()
            })
            .collect()
    }

    fn build_method_graph(&self, ty: &IndexedType, methods: &[&IndexedFunc]) -> MethodGraph {
        let num_fields = ty.decl.fields.len();
        let field_to_idx: HashMap<&str, usize> = ty
            .decl
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.name.as_str(), i))
            .collect();
        let mut name_to_inst: HashMap<&str, Vec<usize>> = HashMap::new();
        for (idx, m) in methods.iter().enumerate() {
            name_to_inst
                .entry(m.decl.name.as_str())
                .or_default()
                .push(idx);
        }

        let max_nodes = self.cfg.type_cohesion.max_accessor_nodes;
        let mut m_elig = Vec::new();
        let mut direct_fields = vec![vec![false; num_fields]; methods.len()];
        let mut direct_calls = vec![Vec::new(); methods.len()];

        for (idx, m) in methods.iter().enumerate() {
            let is_excluded = m.decl.is_lifecycle
                || m.decl.returns_self_type()
                || m.resolved_return_type == Some(ty.id)
                || is_trivial_accessor(m, ty, max_nodes);
            if !is_excluded {
                m_elig.push(idx);
            }
            if let Some(&fidx) = field_to_idx.get(m.decl.name.as_str()) {
                direct_fields[idx][fidx] = true;
            }
            for (access, resolved) in m.decl.field_accesses.iter().zip(&m.resolved_field_accesses) {
                if *resolved != Some(ty.id) || !access.receiver_is_self {
                    continue;
                }
                let fname = access.field_name.as_str();
                if let Some(&fidx) = field_to_idx.get(fname) {
                    direct_fields[idx][fidx] = true;
                    if access.is_write
                        && let Some(targets) = name_to_inst.get(fname)
                    {
                        direct_calls[idx].extend_from_slice(targets);
                    }
                } else if let Some(targets) = name_to_inst.get(fname) {
                    direct_calls[idx].extend_from_slice(targets);
                }
            }
            for call in &m.decl.calls {
                if call.receiver == Some(CallReceiver::SelfInstance)
                    && let Some(targets) = name_to_inst.get(call.callee.as_str())
                {
                    direct_calls[idx].extend_from_slice(targets);
                }
            }
        }

        let mut graph = MethodGraph {
            m_elig,
            direct_calls,
            closure: direct_fields,
        };
        graph.propagate_closure(num_fields);
        graph
    }

    fn extract_substantial_clusters(
        &self,
        ty: &IndexedType,
        methods: &[&'a IndexedFunc],
        graph: &MethodGraph,
    ) -> Vec<ClusterSummary<'a>> {
        let mut inst_to_elig: Vec<Option<usize>> = vec![None; graph.closure.len()];
        for (elig_pos, &inst_idx) in graph.m_elig.iter().enumerate() {
            inst_to_elig[inst_idx] = Some(elig_pos);
        }

        let mut dsu = DisjointSet::new(graph.m_elig.len());
        for a in 0..graph.m_elig.len() {
            let inst_a = graph.m_elig[a];
            for b in (a + 1)..graph.m_elig.len() {
                let inst_b = graph.m_elig[b];
                if shares_any_true(&graph.closure[inst_a], &graph.closure[inst_b]) {
                    dsu.union(a, b);
                }
            }
            for &callee_inst in &graph.direct_calls[inst_a] {
                if let Some(b) = inst_to_elig[callee_inst] {
                    dsu.union(a, b);
                }
            }
        }

        let min_comp = self.cfg.type_cohesion.min_component_methods;
        let num_fields = ty.decl.fields.len();
        let mut substantial = Vec::new();
        for comp in dsu.into_components() {
            if comp.len() < min_comp {
                continue;
            }
            if let Some(summary) = ClusterSummary::from_component(&comp, methods, graph, num_fields)
            {
                substantial.push(summary);
            }
        }
        substantial
    }

    fn bridge_conceptually(
        &self,
        ty: &IndexedType,
        structural: &[ClusterSummary<'a>],
    ) -> Vec<ClusterSummary<'a>> {
        let threshold = self.cfg.type_cohesion.max_conceptual_similarity;
        let mut field_buf: Vec<&str> = Vec::new();
        let mut vectors: Vec<SparseTfIdf> = Vec::with_capacity(structural.len());

        for cluster in structural {
            field_buf.clear();
            collect_masked_fields(ty, &cluster.field_mask, &mut field_buf);
            vectors.push(self.corpus.cluster_vector(&cluster.methods, &field_buf));
        }

        let mut dsu = DisjointSet::new(structural.len());
        for a in 0..vectors.len() {
            for b in (a + 1)..vectors.len() {
                if cosine_similarity(&vectors[a], &vectors[b]) >= threshold {
                    dsu.union(a, b);
                }
            }
        }

        let num_fields = ty.decl.fields.len();
        let mut merged = Vec::new();
        for group in dsu.into_components() {
            merged.push(ClusterSummary::merged(&group, structural, num_fields));
        }
        merged
    }

    fn build_diagnostic(
        &self,
        ty: &IndexedType,
        tcc: f64,
        clusters: &[ClusterSummary<'a>],
    ) -> Diagnostic {
        let mut diag = Diagnostic::new(
            CohesionRuleId::LowCohesionType.as_str(),
            self.cfg.advisory_severity,
            ty.decl.span.clone(),
            format!(
                "`{}` has low cohesion (TCC = {tcc:.2}, {} disjoint responsibility clusters)",
                ty.decl.name,
                clusters.len()
            ),
        )
        .with_suggestion(format!(
            "Split `{}` into {} cohesive types — one per responsibility cluster — to separate independent state and behavior.",
            ty.decl.name,
            clusters.len()
        ));

        let mut method_names: Vec<&str> = Vec::new();
        let mut field_names: Vec<&str> = Vec::new();
        for (idx, cluster) in clusters.iter().enumerate() {
            method_names.clear();
            field_names.clear();
            for m in cluster.methods.iter().take(4) {
                method_names.push(m.decl.name.as_str());
            }
            collect_masked_fields(ty, &cluster.field_mask, &mut field_names);
            let suffix = if cluster.methods.len() > 4 {
                format!(" (+{} more)", cluster.methods.len() - 4)
            } else {
                String::new()
            };
            diag = diag.with_related(
                cluster.methods[0].decl.span.clone(),
                format!(
                    "cluster {}: methods [{}{suffix}] use fields [{}]",
                    idx + 1,
                    method_names.join(", "),
                    field_names.join(", ")
                ),
            );
        }
        diag
    }
}

pub(super) fn check_low_cohesion_types(
    codebase: &Codebase,
    cfg: &CohesionConfig,
    corpus: &LexicalCorpus,
) -> Vec<Diagnostic> {
    TypeCohesionAnalyzer::new(codebase, cfg, corpus).check()
}

fn has_any_true(bits: &[bool]) -> bool {
    for &b in bits {
        if b {
            return true;
        }
    }
    false
}

fn shares_any_true(lhs: &[bool], rhs: &[bool]) -> bool {
    for (&a, &b) in lhs.iter().zip(rhs) {
        if a && b {
            return true;
        }
    }
    false
}

fn collect_masked_fields<'a>(ty: &'a IndexedType, mask: &[bool], out: &mut Vec<&'a str>) {
    for (idx, field) in ty.decl.fields.iter().enumerate() {
        if mask.get(idx).copied().unwrap_or(false) {
            out.push(field.name.as_str());
        }
    }
}
