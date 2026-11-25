use super::CohesionRuleId;
use crate::ast::codebase::{Codebase, FileId};
use crate::ast::item::{ExportOrigin, ImportKind};
use crate::ast::{FileRole, LanguageId, ModulePath};
use crate::config::CohesionConfig;
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

pub(super) struct ModuleNode<'a> {
    pub(super) language: LanguageId,
    pub(super) module_path: &'a ModulePath,
    pub(super) files: Vec<FileId>,
    pub(super) crossing_exempt: bool,
}

struct ModuleGroup<'a> {
    module_path: &'a ModulePath,
    files: Vec<FileId>,
    has_entry_role: bool,
    all_barrel: bool,
    all_empty: bool,
}

pub(super) struct ModuleGraph<'a> {
    codebase: &'a Codebase,
    nodes: Vec<ModuleNode<'a>>,
    edges: Vec<Vec<(usize, &'a Span)>>,
}

struct EdgeResolver<'a> {
    node_by_key: HashMap<LanguageId, HashMap<&'a [String], usize>>,
    buf: Vec<String>,
}

impl<'a> EdgeResolver<'a> {
    fn new(nodes: &[ModuleNode<'a>]) -> Self {
        let mut node_by_key: HashMap<LanguageId, HashMap<&'a [String], usize>> = HashMap::new();
        for (idx, node) in nodes.iter().enumerate() {
            node_by_key
                .entry(node.language)
                .or_default()
                .insert(node.module_path.segments(), idx);
        }
        Self {
            node_by_key,
            buf: Vec::new(),
        }
    }

    fn resolve(
        &mut self,
        language: LanguageId,
        target: &ModulePath,
        symbol: Option<&str>,
    ) -> Option<usize> {
        let by_mod = self.node_by_key.get(&language)?;
        self.buf.clear();
        self.buf.extend(target.segments().iter().cloned());
        if let Some(sym) = symbol {
            self.buf.push(sym.to_owned());
            if let Some(&idx) = by_mod.get(self.buf.as_slice()) {
                return Some(idx);
            }
            self.buf.pop();
        }
        by_mod.get(self.buf.as_slice()).copied()
    }
}

struct CrossingThresholds {
    tau_in: usize,
    tau_out: usize,
}

impl<'a> ModuleGraph<'a> {
    pub(super) fn build(codebase: &'a Codebase) -> Self {
        type ModuleGroups<'m> = HashMap<(LanguageId, &'m [String]), ModuleGroup<'m>>;
        let mut grouped: ModuleGroups<'a> = HashMap::new();
        for (idx, file) in codebase.files().iter().enumerate() {
            if file.is_test_file() {
                continue;
            }
            let group = grouped
                .entry((file.language, file.module_path.segments()))
                .or_insert_with(|| ModuleGroup {
                    module_path: &file.module_path,
                    files: Vec::new(),
                    has_entry_role: false,
                    all_barrel: true,
                    all_empty: true,
                });
            group.files.push(FileId(idx));
            if matches!(
                file.role,
                FileRole::LibraryRoot | FileRole::BinaryEntrypoint
            ) {
                group.has_entry_role = true;
            }
            if !file.is_barrel() {
                group.all_barrel = false;
            }
            if !file.types.is_empty() || !file.functions.is_empty() {
                group.all_empty = false;
            }
        }

        let mut nodes: Vec<ModuleNode<'a>> = grouped
            .into_iter()
            .map(|((language, _), mut group)| {
                group.files.sort_by(|&a, &b| {
                    codebase
                        .file(a)
                        .relative_path
                        .cmp(&codebase.file(b).relative_path)
                });
                ModuleNode {
                    language,
                    module_path: group.module_path,
                    files: group.files,
                    crossing_exempt: group.has_entry_role || group.all_barrel || group.all_empty,
                }
            })
            .collect();
        nodes.sort_by(|a, b| {
            codebase
                .file(a.files[0])
                .relative_path
                .cmp(&codebase.file(b.files[0]).relative_path)
                .then_with(|| a.language.as_str().cmp(b.language.as_str()))
        });

        let mut resolver = EdgeResolver::new(&nodes);
        let node_count = nodes.len();
        let mut graph = Self {
            codebase,
            nodes,
            edges: vec![Vec::new(); node_count],
        };
        graph.populate_edges(&mut resolver);
        graph
    }

    fn populate_edges(&mut self, resolver: &mut EdgeResolver<'a>) {
        let node_count = self.nodes.len();
        let mut seen_targets: HashSet<usize> = HashSet::new();
        for from_idx in 0..node_count {
            seen_targets.clear();
            let language = self.nodes[from_idx].language;
            let from_mod = self.nodes[from_idx].module_path;
            let file_ids: Vec<FileId> = self.nodes[from_idx].files.clone();

            for file_id in file_ids {
                let file = self.codebase.file(file_id);
                for imp in &file.imports {
                    if imp.in_test_module {
                        continue;
                    }
                    let Some(target) = imp.resolved_module.as_ref() else {
                        continue;
                    };
                    let symbol = (imp.kind == ImportKind::Named && !imp.symbol.is_empty())
                        .then_some(imp.symbol.as_str());
                    if let Some(to_idx) = resolver.resolve(language, target, symbol) {
                        self.try_add_edge(from_idx, from_mod, to_idx, &imp.span, &mut seen_targets);
                    }
                }
                for exp in &file.exports {
                    if exp.origin != ExportOrigin::DirectInternal {
                        continue;
                    }
                    let Some(target) = exp.source_module.as_ref() else {
                        continue;
                    };
                    let symbol =
                        (!exp.is_wildcard && !exp.symbol.is_empty()).then_some(exp.symbol.as_str());
                    if let Some(to_idx) = resolver.resolve(language, target, symbol) {
                        self.try_add_edge(from_idx, from_mod, to_idx, &exp.span, &mut seen_targets);
                    }
                }
            }
        }
    }

    pub(super) fn check_cyclic_modules(&self, cfg: &CohesionConfig) -> Vec<Diagnostic> {
        let mut sccs = TarjanScc::new(self.edges.len()).find_cycles(&self.edges);
        for scc in &mut sccs {
            scc.sort_by(|&a, &b| self.primary_path(a).cmp(self.primary_path(b)));
        }
        sccs.sort_by(|a, b| self.primary_path(a[0]).cmp(self.primary_path(b[0])));

        let mut diagnostics = Vec::new();
        let mut in_scc: HashSet<usize> = HashSet::new();
        let mut cycle_desc = String::new();

        for scc in &sccs {
            in_scc.clear();
            for &member in scc {
                in_scc.insert(member);
            }

            let primary_idx = scc[0];
            let Some((primary_target, primary_span)) = self.first_edge_in_scc(primary_idx, &in_scc)
            else {
                continue;
            };

            let message = if scc.len() == 2 {
                format!(
                    "cyclic module dependency between `{}` and `{}`",
                    self.nodes[scc[0]].module_path, self.nodes[scc[1]].module_path
                )
            } else {
                cycle_desc.clear();
                for (pos, &member) in scc.iter().enumerate() {
                    if pos > 0 {
                        cycle_desc.push_str(", ");
                    }
                    let _ = write!(cycle_desc, "`{}`", self.nodes[member].module_path);
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
                self.nodes[primary_idx].module_path, self.nodes[primary_target].module_path
            ));

            for &other_idx in &scc[1..] {
                if let Some((target, span)) = self.first_edge_in_scc(other_idx, &in_scc) {
                    diag = diag.with_related(
                        span.clone(),
                        format!(
                            "`{}` imports `{}`",
                            self.nodes[other_idx].module_path, self.nodes[target].module_path
                        ),
                    );
                }
            }

            diagnostics.push(diag);
        }

        diagnostics
    }

    pub(super) fn check_architectural_crossings(&self, cfg: &CohesionConfig) -> Vec<Diagnostic> {
        let node_count = self.nodes.len();
        let mut incoming: Vec<Vec<(usize, &'a Span)>> = vec![Vec::new(); node_count];
        for (src_idx, outgoing) in self.edges.iter().enumerate() {
            for &(dst_idx, span) in outgoing {
                incoming[dst_idx].push((src_idx, span));
            }
        }

        let mut by_language: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
        for (idx, node) in self.nodes.iter().enumerate() {
            if !node.crossing_exempt {
                by_language
                    .entry(node.language.as_str())
                    .or_default()
                    .push(idx);
            }
        }

        let mut sorted_in: Vec<usize> = Vec::new();
        let mut sorted_out: Vec<usize> = Vec::new();
        let mut flagged: Vec<(usize, CrossingThresholds)> = Vec::new();
        let quantile = if cfg.crossing.quantile.is_nan() {
            0.90
        } else {
            cfg.crossing.quantile.clamp(0.0, 1.0)
        };

        for candidates in by_language.values() {
            let n = candidates.len();
            if n == 0 || n < cfg.crossing.min_graph_modules {
                continue;
            }
            sorted_in.clear();
            sorted_out.clear();
            for &u in candidates {
                sorted_in.push(incoming[u].len());
                sorted_out.push(self.edges[u].len());
            }
            sorted_in.sort_unstable();
            sorted_out.sort_unstable();

            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let percentile_idx = ((quantile * (n - 1) as f64).floor() as usize).min(n - 1);
            let tau_in = cfg.crossing.min_fan_in.max(sorted_in[percentile_idx]);
            let tau_out = cfg.crossing.min_fan_out.max(sorted_out[percentile_idx]);

            for &u in candidates {
                if incoming[u].len() >= tau_in && self.edges[u].len() >= tau_out {
                    flagged.push((u, CrossingThresholds { tau_in, tau_out }));
                }
            }
        }

        flagged.sort_by_key(|&(u, _)| u);
        let mut outgoing_buf: Vec<(usize, &'a Span)> = Vec::new();
        let mut diagnostics = Vec::with_capacity(flagged.len());
        for (u, thresholds) in flagged {
            diagnostics.push(self.build_crossing_diagnostic(
                u,
                &incoming[u],
                &thresholds,
                cfg,
                &mut outgoing_buf,
            ));
        }
        diagnostics
    }

    fn build_crossing_diagnostic(
        &self,
        u: usize,
        incoming_edges: &[(usize, &'a Span)],
        thresholds: &CrossingThresholds,
        cfg: &CohesionConfig,
        outgoing_buf: &mut Vec<(usize, &'a Span)>,
    ) -> Diagnostic {
        let module_path = self.nodes[u].module_path;
        let fan_in = incoming_edges.len();
        let fan_out = self.edges[u].len();
        let anchor = Span::new(self.primary_path(u).to_path_buf(), 1, 1, 1, 1);

        let mut diag = Diagnostic::new(
            CohesionRuleId::ArchitecturalCrossing.as_str(),
            cfg.advisory_severity,
            anchor,
            format!(
                "module `{module_path}` is an architectural crossing with fan-in {fan_in} (threshold {}) and fan-out {fan_out} (threshold {})",
                thresholds.tau_in, thresholds.tau_out
            ),
        )
        .with_suggestion(format!(
            "Split `{module_path}` into smaller focused modules or extract shared contracts so callers do not transitively couple to all of its dependencies."
        ));

        let max_edges = cfg.crossing.max_related_edges;
        for &(src_idx, span) in incoming_edges.iter().take(max_edges) {
            diag = diag.with_related(
                span.clone(),
                format!(
                    "incoming dependency from `{}`",
                    self.nodes[src_idx].module_path
                ),
            );
        }

        outgoing_buf.clear();
        outgoing_buf.extend_from_slice(&self.edges[u]);
        outgoing_buf.sort_by_key(|&(dst_idx, _)| dst_idx);
        for &(dst_idx, span) in outgoing_buf.iter().take(max_edges) {
            diag = diag.with_related(
                span.clone(),
                format!(
                    "outgoing dependency to `{}`",
                    self.nodes[dst_idx].module_path
                ),
            );
        }

        diag
    }

    fn try_add_edge(
        &mut self,
        from_idx: usize,
        from_mod: &ModulePath,
        to_idx: usize,
        span: &'a Span,
        seen_targets: &mut HashSet<usize>,
    ) {
        if to_idx == from_idx {
            return;
        }
        let to_mod = self.nodes[to_idx].module_path;
        if from_mod.is_same_or_child_of(to_mod) || to_mod.is_same_or_child_of(from_mod) {
            return;
        }
        if seen_targets.insert(to_idx) {
            self.edges[from_idx].push((to_idx, span));
        }
    }

    fn primary_path(&self, idx: usize) -> &'a Path {
        self.codebase
            .file(self.nodes[idx].files[0])
            .relative_path
            .as_path()
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
