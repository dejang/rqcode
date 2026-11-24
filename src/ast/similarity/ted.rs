use crate::ast::similarity::fingerprint::{CanonicalTree, LabelWeights};

/// Reusable per-thread DP scratch buffers for Stage 3 weighted Zhang-Shasha Tree Edit Distance.
#[derive(Debug, Default)]
pub(crate) struct TedScratch {
    tree_dist: Vec<f64>,
    forest_dist: Vec<f64>,
}

impl TedScratch {
    /// Stage 3: Weighted exact Tree Edit Distance (Zhang-Shasha 1989; TSED normalization, ACL 2024).
    ///
    /// Returns `Some(score)` in `[0.0, 1.0]` where `1.0` means identical canonical trees, or `None`
    /// if either tree is empty, `left.len() * right.len() > max_ted_work`, or total weight `<= eps`.
    #[must_use]
    pub(crate) fn weighted_tsed(
        &mut self,
        left: &CanonicalTree,
        right: &CanonicalTree,
        weights: LabelWeights,
        max_ted_work: usize,
        eps: f64,
    ) -> Option<f64> {
        let n1 = left.len();
        let n2 = right.len();
        if n1 == 0 || n2 == 0 || n1.saturating_mul(n2) > max_ted_work {
            return None;
        }
        let max_weight = left.total_weight().max(right.total_weight());
        if !max_weight.is_finite() || max_weight <= eps {
            return None;
        }

        let td_cols = n2 + 1;
        let cells = (n1 + 1) * td_cols;
        self.tree_dist.resize(cells, 0.0);
        self.tree_dist[..cells].fill(0.0);
        self.forest_dist.resize(cells, 0.0);

        let (left_labels, right_labels) = (left.labels(), right.labels());
        let (left_lm, right_lm) = (left.leftmost(), right.leftmost());

        for &k1 in left.keyroots() {
            for &k2 in right.keyroots() {
                let (l1, l2) = (left_lm[k1], right_lm[k2]);
                let (rows, cols) = (k1 - l1 + 1, k2 - l2 + 1);
                let stride = cols + 1;

                self.forest_dist[0] = 0.0;
                for i in 1..=rows {
                    let u = l1 + i - 1;
                    self.forest_dist[i * stride] =
                        self.forest_dist[(i - 1) * stride] + left_labels[u].weight(weights);
                }
                for j in 1..=cols {
                    let v = l2 + j - 1;
                    self.forest_dist[j] = self.forest_dist[j - 1] + right_labels[v].weight(weights);
                }

                for i in 1..=rows {
                    let u = l1 + i - 1;
                    let u_label = left_labels[u];
                    let u_weight = u_label.weight(weights);
                    let u_left = left_lm[u];

                    for j in 1..=cols {
                        let v = l2 + j - 1;
                        let v_label = right_labels[v];
                        let v_weight = v_label.weight(weights);
                        let v_left = right_lm[v];

                        let del = self.forest_dist[(i - 1) * stride + j] + u_weight;
                        let ins = self.forest_dist[i * stride + (j - 1)] + v_weight;

                        if u_left == l1 && v_left == l2 {
                            let relabel = u_label.relabel_cost(v_label, weights);
                            let sub = self.forest_dist[(i - 1) * stride + (j - 1)] + relabel;
                            let best = del.min(ins).min(sub);
                            self.forest_dist[i * stride + j] = best;
                            self.tree_dist[u * td_cols + v] = best;
                        } else {
                            let sub = self.forest_dist[(u_left - l1) * stride + (v_left - l2)]
                                + self.tree_dist[u * td_cols + v];
                            self.forest_dist[i * stride + j] = del.min(ins).min(sub);
                        }
                    }
                }
            }
        }

        let dist = self.tree_dist[n1 * td_cols + n2];
        Some((1.0 - dist / max_weight).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::similarity::{
        AstKind, EligibilityConfig, LitKind, NameSpace, NormalizedTree, SimilarityCorpus,
        SimilarityParams, TreeBuilder,
    };

    fn pair_tsed(left: &NormalizedTree, right: &NormalizedTree, max_ted_work: usize) -> Option<f64> {
        let params = SimilarityParams {
            eligibility: EligibilityConfig {
                min_nodes: 1,
                min_distinct_kinds: 1,
            },
            min_candidate_jaccard: 0.01,
            max_ted_work,
            ..SimilarityParams::default()
        };
        let mut corpus = SimilarityCorpus::with_params(params);
        assert!(corpus.add_unit(0, left));
        assert!(corpus.add_unit(1, right));
        let matches = corpus.similar_pairs(0.01);
        assert_eq!(matches.len(), 1);
        matches[0].tsed()
    }

    fn multi_var_body(vars: [&str; 4], callee: &str, mid_insert: bool) -> NormalizedTree {
        let [a, b, c, d] = vars;
        let mut tb = TreeBuilder::new();
        let root = tb.enter(AstKind::Block);

        // Initial declarations binding all 4 variables so they are live across the midpoint.
        for (idx, v) in [a, b, c, d].iter().enumerate() {
            let bind = tb.enter(AstKind::Bind);
            tb.symbol(NameSpace::Plain, *v);
            if idx == 0 {
                let call = tb.enter(AstKind::Call);
                tb.exact(callee);
                tb.literal(LitKind::Number);
                tb.exit(call);
            } else {
                tb.literal(LitKind::Number);
            }
            tb.exit(bind);
        }

        // Mid-body Type-3 insertion while `a`, `b`, `c`, `d` are all live.
        if mid_insert {
            let extra = tb.enter(AstKind::Bind);
            tb.symbol(NameSpace::Plain, "audit");
            let call = tb.enter(AstKind::Call);
            tb.exact("trace");
            tb.symbol(NameSpace::Plain, a);
            tb.exit(call);
            tb.exit(extra);
        }

        // Subsequent uses of all 4 live variables after the midpoint.
        let ret = tb.enter(AstKind::Return);
        tb.exact("return");
        let sum = tb.enter(AstKind::Binary);
        tb.exact("+");
        let lhs = tb.enter(AstKind::Binary);
        tb.exact("+");
        tb.symbol(NameSpace::Plain, a);
        tb.symbol(NameSpace::Plain, b);
        tb.exit(lhs);
        let rhs = tb.enter(AstKind::Binary);
        tb.exact("-");
        tb.symbol(NameSpace::Plain, c);
        tb.symbol(NameSpace::Plain, d);
        tb.exit(rhs);
        tb.exit(sum);
        tb.exit(ret);

        tb.exit(root);
        tb.finish()
    }

    #[test]
    fn weighted_tsed_verifies_reference_distances_renaming_callees_and_work_cap() {
        // Hand-verified exact distance: `f(h(a))` vs `g(h(a))` -> 1 Exact relabel (2.0) out of 7.0 -> TSED 5/7.
        let build_tiny = |callee: &str| {
            let mut tb = TreeBuilder::new();
            let outer = tb.enter(AstKind::Call);
            tb.exact(callee);
            let inner = tb.enter(AstKind::Call);
            tb.exact("h");
            tb.symbol(NameSpace::Plain, "a");
            tb.exit(inner);
            tb.exit(outer);
            tb.finish()
        };
        let tiny_f = build_tiny("f");
        let tiny_g = build_tiny("g");
        let tiny_score = pair_tsed(&tiny_f, &tiny_g, 100).expect("tiny trees fit work cap");
        let expected = 5.0 / 7.0;
        assert!(
            (tiny_score - expected).abs() < 1e-12,
            "expected exact 5/7 reference TSED, got {tiny_score}"
        );
        // Tight work cap (< 5*5=25) triggers graceful None fallback.
        assert!(pair_tsed(&tiny_f, &tiny_g, 20).is_none());

        // Full bodies: consistent renaming -> 1.0; callee swap pays elevated Exact weight.
        let a = multi_var_body(["w", "x", "y", "z"], "compute", false);
        let b = multi_var_body(["p", "q", "r", "s"], "compute", false);
        let c = multi_var_body(["w", "x", "y", "z"], "render", false);
        let renamed = pair_tsed(&a, &b, 160_000).expect("valid TSED");
        let swapped = pair_tsed(&a, &c, 160_000).expect("valid TSED");
        assert!((renamed - 1.0).abs() < 1e-12);
        assert!(swapped < renamed && swapped > 0.85);
    }

    #[test]
    fn mid_body_type3_insertion_stays_localized_and_is_symmetric() {
        let orig = multi_var_body(["a", "b", "c", "d"], "compute", false);
        let inserted = multi_var_body(["p", "q", "r", "s"], "compute", true);

        let ab = pair_tsed(&orig, &inserted, 160_000).expect("valid TSED");
        let ba = pair_tsed(&inserted, &orig, 160_000).expect("valid TSED");
        assert!((ab - ba).abs() < 1e-12, "TSED must be symmetric");
        // Even with 4 live variables crossing the mid-body insertion, same-namespace symbol
        // relabel cost keeps the edit localized so TSED stays comfortably high.
        assert!(
            ab > 0.78,
            "expected localized mid-body TSED > 0.78, got {ab}"
        );
    }
}
