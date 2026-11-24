mod fingerprint;
mod index;
mod ted;
mod tree;

pub use self::fingerprint::LabelWeights;
pub use self::tree::{
    AstKind, EligibilityStats, LitKind, NameSpace, NodeLabel, NormalizedTree, TreeBuilder, TreeNode,
};

use self::fingerprint::FingerprintedUnit;
use self::index::find_jaccard_candidates;
use self::ted::TedScratch;
use rayon::prelude::*;

/// Eligibility parameters for Stage 0 pre-filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EligibilityConfig {
    pub min_nodes: usize,
    pub min_distinct_kinds: usize,
}

impl Default for EligibilityConfig {
    fn default() -> Self {
        Self {
            min_nodes: 25,
            min_distinct_kinds: 4,
        }
    }
}

/// Complete tuning parameters for the 4-stage structural AST similarity pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimilarityParams {
    /// Stage 0 minimum folded node count and structural diversity (`TKS`).
    pub eligibility: EligibilityConfig,
    /// Maximum consecutive structurally identical siblings kept during Stage 0 boilerplate folding.
    pub max_identical_siblings: usize,
    /// Blend weight on Stage 2A Merkle subtree Jaccard (`jaccard_weight + tsed_weight == 1.0`).
    pub jaccard_weight: f64,
    /// Blend weight on Stage 3 weighted Zhang-Shasha Tree Edit Distance.
    pub tsed_weight: f64,
    /// Recall-for-speed floor on the Stage 2B Jaccard candidate gate when low thresholds would
    /// otherwise relax `(threshold - tsed_weight) / jaccard_weight` below this floor.
    pub min_candidate_jaccard: f64,
    /// Semantic weights for Stage 3 weighted TSED.
    pub label_weights: LabelWeights,
    /// Maximum `left.len() * right.len()` DP cells allowed for Stage 3 weighted TSED before
    /// falling back to Stage 2 Merkle subtree Jaccard.
    pub max_ted_work: usize,
    /// Floating-point boundary tolerance for Jaccard and threshold comparisons.
    pub eps: f64,
}

impl Default for SimilarityParams {
    fn default() -> Self {
        Self {
            eligibility: EligibilityConfig::default(),
            max_identical_siblings: 2,
            jaccard_weight: 0.30,
            tsed_weight: 0.70,
            min_candidate_jaccard: 0.35,
            label_weights: LabelWeights::default(),
            max_ted_work: 160_000,
            eps: 1e-12,
        }
    }
}

/// A pair of analyzed units whose verified similarity met or exceeded the configured threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimilarityMatch {
    left_id: usize,
    right_id: usize,
    score: f64,
    jaccard: f64,
    tsed: Option<f64>,
}

impl SimilarityMatch {
    #[must_use]
    pub const fn left_id(self) -> usize {
        self.left_id
    }

    #[must_use]
    pub const fn right_id(self) -> usize {
        self.right_id
    }

    #[must_use]
    pub const fn score(self) -> f64 {
        self.score
    }

    #[must_use]
    pub const fn jaccard(self) -> f64 {
        self.jaccard
    }

    #[must_use]
    pub const fn tsed(self) -> Option<f64> {
        self.tsed
    }
}

#[derive(Debug)]
struct CorpusUnit {
    id: usize,
    unit: FingerprintedUnit,
}

/// 4-stage language-agnostic structural similarity engine:
/// - Stage 0: Sibling-folded effective node count and structural diversity (`TKS`) pre-filter
/// - Stage 1: Normalized AST preserving callees/operators with namespace-scoped symbols
/// - Stage 2: Merkle subtree fingerprints (with LCA Baker `prev` renaming) + prefix-filtered Jaccard
/// - Stage 3: Parallel weighted Zhang-Shasha Tree Edit Distance verification on surviving candidates
#[derive(Debug, Default)]
pub struct SimilarityCorpus {
    params: SimilarityParams,
    units: Vec<CorpusUnit>,
}

impl SimilarityCorpus {
    #[must_use]
    pub fn new(eligibility: EligibilityConfig) -> Self {
        Self::with_params(SimilarityParams {
            eligibility,
            ..SimilarityParams::default()
        })
    }

    #[must_use]
    pub fn with_params(params: SimilarityParams) -> Self {
        Self {
            params,
            units: Vec::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Evaluate Stage 0 eligibility and, if eligible, extract Stage 2A fingerprints and Stage 3
    /// canonical tree into the corpus. Returns `true` when the unit was admitted.
    pub fn add_unit(&mut self, id: usize, tree: &NormalizedTree) -> bool {
        let stats = tree.eligibility_stats(self.params.max_identical_siblings);
        if !stats.is_eligible(
            self.params.eligibility.min_nodes,
            self.params.eligibility.min_distinct_kinds,
        ) {
            return false;
        }
        let unit = self.params.label_weights.extract_unit(tree);
        if unit.hashes.is_empty() {
            return false;
        }
        self.units.push(CorpusUnit { id, unit });
        true
    }

    /// Run Stage 2B prefix-filtered candidate retrieval followed by Stage 3 weighted TSED
    /// verification, returning matches with `score >= threshold` sorted deterministically by
    /// descending score, then `(left_id, right_id)`.
    ///
    /// Returns an empty list if `threshold` is outside `(0.0, 1.0]`, if blend weights are
    /// non-finite, negative, or sum to zero, or if any label weight is non-finite or negative.
    ///
    /// Because the blended score is `w_j * J + w_t * T` with `T <= 1.0`, any match must satisfy
    /// `J >= (threshold - w_t) / w_j`. Stage 2B uses that exact bound (floored at
    /// `params.min_candidate_jaccard` as a deliberate recall-for-speed guard at low thresholds).
    #[must_use]
    pub fn similar_pairs(&self, threshold: f64) -> Vec<SimilarityMatch> {
        let eps = self.effective_eps();
        if self.units.len() < 2
            || !threshold.is_finite()
            || threshold <= 0.0
            || threshold > 1.0
            || !self.has_valid_weights(eps)
        {
            return Vec::new();
        }

        let candidate_threshold = self.candidate_jaccard_bound(threshold, eps);

        let hash_slices: Vec<&[u64]> = self
            .units
            .iter()
            .map(|u| u.unit.hashes.as_slice())
            .collect();

        let candidates = find_jaccard_candidates(&hash_slices, candidate_threshold, eps);

        let mut matches: Vec<SimilarityMatch> = candidates
            .par_iter()
            .map_init(TedScratch::default, |scratch, cand| {
                let left = &self.units[cand.left];
                let right = &self.units[cand.right];
                let left_tree = &left.unit.canonical;
                let right_tree = &right.unit.canonical;

                let within_ted_budget =
                    left_tree.len().saturating_mul(right_tree.len()) <= self.params.max_ted_work;
                if within_ted_budget {
                    let max_weight = left_tree.total_weight().max(right_tree.total_weight());
                    if max_weight > eps {
                        let max_tsed = (1.0
                            - (left_tree.total_weight() - right_tree.total_weight()).abs()
                                / max_weight)
                            .clamp(0.0, 1.0);
                        let upper_bound = self.params.jaccard_weight * cand.jaccard
                            + self.params.tsed_weight * max_tsed;
                        if !upper_bound.is_finite() || upper_bound + eps < threshold {
                            return None;
                        }
                    }
                }

                let tsed = scratch.weighted_tsed(
                    left_tree,
                    right_tree,
                    self.params.label_weights,
                    self.params.max_ted_work,
                    eps,
                );
                let score = match tsed {
                    Some(t) => {
                        self.params.jaccard_weight * cand.jaccard + self.params.tsed_weight * t
                    }
                    None => cand.jaccard,
                };
                if !score.is_finite() || score + eps < threshold {
                    return None;
                }
                let (left_id, right_id) = if left.id <= right.id {
                    (left.id, right.id)
                } else {
                    (right.id, left.id)
                };
                Some(SimilarityMatch {
                    left_id,
                    right_id,
                    score,
                    jaccard: cand.jaccard,
                    tsed,
                })
            })
            .flatten()
            .collect();

        matches.sort_unstable_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.left_id.cmp(&b.left_id))
                .then_with(|| a.right_id.cmp(&b.right_id))
        });

        matches
    }

    fn effective_eps(&self) -> f64 {
        if self.params.eps.is_finite() && self.params.eps > 0.0 {
            self.params.eps
        } else {
            f64::EPSILON
        }
    }

    fn has_valid_weights(&self, eps: f64) -> bool {
        let jw = self.params.jaccard_weight;
        let tw = self.params.tsed_weight;
        let lw = self.params.label_weights;
        jw.is_finite()
            && jw >= 0.0
            && tw.is_finite()
            && tw >= 0.0
            && (jw + tw) > eps
            && lw.exact.is_finite()
            && lw.exact >= 0.0
            && lw.structural.is_finite()
            && lw.structural >= 0.0
            && lw.symbol_relabel.is_finite()
            && lw.symbol_relabel >= 0.0
    }

    fn candidate_jaccard_bound(&self, threshold: f64, eps: f64) -> f64 {
        let raw_floor = if self.params.min_candidate_jaccard.is_finite() {
            self.params.min_candidate_jaccard
        } else {
            0.0
        };
        let floor = raw_floor.clamp(0.0, threshold);
        if self.params.jaccard_weight <= eps {
            return floor.max(eps);
        }
        let exact_bound = (threshold - self.params.tsed_weight) / self.params.jaccard_weight;
        if !exact_bound.is_finite() {
            return floor.max(eps);
        }
        exact_bound.clamp(floor, threshold).max(eps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sum_loop(vars: [&str; 4], cmp_op: &str, extra_stmt: bool) -> NormalizedTree {
        let [acc, elem, items, limit] = vars;
        let mut tb = TreeBuilder::new();
        let root = tb.enter(AstKind::Block);

        let bind = tb.enter(AstKind::Bind);
        tb.symbol(NameSpace::Plain, acc);
        tb.literal(LitKind::Number);
        tb.exit(bind);

        let lp = tb.enter(AstKind::Loop);
        tb.symbol(NameSpace::Plain, elem);
        tb.symbol(NameSpace::Plain, items);
        let blk = tb.enter(AstKind::Block);
        let cond = tb.enter(AstKind::If);
        let cmp = tb.enter(AstKind::Binary);
        tb.exact(cmp_op);
        tb.symbol(NameSpace::Plain, elem);
        tb.literal(LitKind::Number);
        tb.exit(cmp);
        let add = tb.enter(AstKind::Assign);
        tb.exact("+=");
        tb.symbol(NameSpace::Plain, acc);
        tb.symbol(NameSpace::Plain, elem);
        tb.exit(add);
        tb.exit(cond);
        tb.exit(blk);
        tb.exit(lp);

        if extra_stmt {
            let extra = tb.enter(AstKind::Bind);
            tb.symbol(NameSpace::Plain, "bonus");
            tb.literal(LitKind::Number);
            tb.exit(extra);
        }

        let check = tb.enter(AstKind::If);
        let cmp2 = tb.enter(AstKind::Binary);
        tb.exact(">");
        tb.symbol(NameSpace::Plain, acc);
        tb.symbol(NameSpace::Plain, limit);
        tb.exit(cmp2);
        let ret = tb.enter(AstKind::Return);
        tb.exact("return");
        let sub = tb.enter(AstKind::Binary);
        tb.exact("-");
        tb.symbol(NameSpace::Plain, acc);
        tb.symbol(NameSpace::Plain, limit);
        tb.exit(sub);
        tb.exit(ret);
        tb.exit(check);

        tb.exit(root);
        tb.finish()
    }

    fn build_pipeline(callees: [&str; 3]) -> NormalizedTree {
        let mut tb = TreeBuilder::new();
        let root = tb.enter(AstKind::Block);
        for (idx, callee) in callees.iter().enumerate() {
            let bind = tb.enter(AstKind::Bind);
            tb.symbol(NameSpace::Plain, format!("v{idx}"));
            let call = tb.enter(AstKind::Call);
            tb.exact(*callee);
            tb.symbol(
                NameSpace::Plain,
                if idx == 0 {
                    "input".to_string()
                } else {
                    format!("v{}", idx - 1)
                },
            );
            tb.exit(call);
            tb.exit(bind);
        }
        let ret = tb.enter(AstKind::Return);
        tb.exact("return");
        tb.symbol(NameSpace::Plain, "v2");
        tb.exit(ret);
        tb.exit(root);
        tb.finish()
    }

    fn build_deep_branch_chain(depth: usize, leaf_op: &str) -> NormalizedTree {
        let mut tb = TreeBuilder::new();
        let root = tb.enter(AstKind::Block);
        let mut marks = Vec::with_capacity(depth);
        for level in 0..depth {
            let cond = tb.enter(AstKind::If);
            for step in 0..2 {
                let call = tb.enter(AstKind::Call);
                tb.exact(format!("step_{level}_{step}"));
                tb.symbol(NameSpace::Plain, "x");
                tb.exit(call);
            }
            marks.push(cond);
        }
        let leaf = tb.enter(AstKind::Binary);
        tb.exact(leaf_op);
        tb.symbol(NameSpace::Plain, "x");
        tb.literal(LitKind::Number);
        tb.exit(leaf);
        while let Some(mark) = marks.pop() {
            tb.exit(mark);
        }
        tb.exit(root);
        tb.finish()
    }

    #[test]
    fn detects_renamed_clone_and_rejects_different_callee_pipeline() {
        let mut corpus = SimilarityCorpus::new(EligibilityConfig {
            min_nodes: 12,
            min_distinct_kinds: 4,
        });

        let a = build_sum_loop(["acc", "price", "prices", "coupon"], ">", false);
        let b = build_sum_loop(["sum", "item", "items", "discount"], ">", false);
        let p1 = build_pipeline(["fetchUser", "validateEmail", "formatGreeting"]);
        let p2 = build_pipeline(["loadInvoice", "computeTax", "renderReceipt"]);

        assert!(corpus.add_unit(0, &a));
        assert!(corpus.add_unit(1, &b));
        assert!(corpus.add_unit(2, &p1));
        assert!(corpus.add_unit(3, &p2));

        let matches = corpus.similar_pairs(0.85);
        assert_eq!(
            matches.len(),
            1,
            "expected only (0, 1) to match, got {matches:?}"
        );
        assert_eq!((matches[0].left_id(), matches[0].right_id()), (0, 1));
        assert!((matches[0].score() - 1.0).abs() < 1e-12);
        assert!((matches[0].jaccard() - 1.0).abs() < 1e-12);
        assert!(matches[0].tsed().is_some_and(|t| (t - 1.0).abs() < 1e-12));
    }

    #[test]
    fn detects_deep_single_mutation_and_ted_fallback_and_rejects_flat_tables() {
        // 1. Mid-body Type-3 insertion and flat lookup-table rejection.
        let mut corpus = SimilarityCorpus::new(EligibilityConfig {
            min_nodes: 20,
            min_distinct_kinds: 4,
        });
        let orig = build_sum_loop(["acc", "price", "prices", "coupon"], ">", false);
        let type3 = build_sum_loop(["sum", "item", "items", "discount"], ">", true);
        assert!(corpus.add_unit(10, &orig));
        assert!(corpus.add_unit(11, &type3));

        let mut tb = TreeBuilder::new();
        let sw = tb.enter(AstKind::Branch);
        tb.symbol(NameSpace::Plain, "code");
        for _ in 0..10 {
            let arm = tb.enter(AstKind::BranchArm);
            tb.literal(LitKind::Number);
            tb.literal(LitKind::String);
            tb.exit(arm);
        }
        tb.exit(sw);
        assert!(!corpus.add_unit(12, &tb.finish()));

        let matches = corpus.similar_pairs(0.80);
        assert_eq!(
            matches.len(),
            1,
            "expected mid-body Type-3 match, got {matches:?}"
        );
        assert_eq!((matches[0].left_id(), matches[0].right_id()), (10, 11));
        assert!(matches[0].score() >= 0.80);

        // 2. Deep single-operator mutation: Jaccard falls below 0.60 (~0.58), yet blended score
        //    exceeds 0.85 because TSED is ~0.97. Exact derived gate (0.50 at t=0.85) retains it.
        let mut deep_corpus = SimilarityCorpus::new(EligibilityConfig {
            min_nodes: 20,
            min_distinct_kinds: 3,
        });
        let deep_a = build_deep_branch_chain(7, ">");
        let deep_b = build_deep_branch_chain(7, ">=");
        assert!(deep_corpus.add_unit(1, &deep_a));
        assert!(deep_corpus.add_unit(2, &deep_b));
        let deep_matches = deep_corpus.similar_pairs(0.85);
        assert_eq!(
            deep_matches.len(),
            1,
            "deep single mutation must survive derived Jaccard candidate gate"
        );
        assert!(deep_matches[0].jaccard() < 0.60);
        assert!(deep_matches[0].score() >= 0.85);

        // 3. Injected tiny `max_ted_work` exercises the Merkle-Jaccard-only fallback cleanly.
        let mut fallback_corpus = SimilarityCorpus::with_params(SimilarityParams {
            eligibility: EligibilityConfig {
                min_nodes: 12,
                min_distinct_kinds: 4,
            },
            max_ted_work: 16,
            ..SimilarityParams::default()
        });
        assert!(fallback_corpus.add_unit(1, &orig));
        assert!(fallback_corpus.add_unit(2, &orig));
        let fallback_matches = fallback_corpus.similar_pairs(0.85);
        assert_eq!(fallback_matches.len(), 1);
        assert_eq!(fallback_matches[0].tsed(), None);
        assert!((fallback_matches[0].score() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rejects_invalid_thresholds_and_degenerate_params() {
        let orig = build_sum_loop(["acc", "price", "prices", "coupon"], ">", false);
        let type3 = build_sum_loop(["sum", "item", "items", "discount"], ">", true);
        let non_positive = f64::from(0);
        let above_one = f64::from(2);
        let threshold = f64::from(4) / f64::from(5);
        let elig = EligibilityConfig {
            min_nodes: 12,
            min_distinct_kinds: 4,
        };

        let mut base = SimilarityCorpus::new(elig);
        assert!(base.add_unit(1, &orig));
        assert!(base.add_unit(2, &orig));
        assert_eq!(base.similar_pairs(f64::NAN), Vec::new());
        assert_eq!(base.similar_pairs(non_positive), Vec::new());
        assert_eq!(base.similar_pairs(above_one), Vec::new());

        let mut guarded_floor = SimilarityCorpus::with_params(SimilarityParams {
            eligibility: elig,
            min_candidate_jaccard: f64::NAN,
            eps: non_positive,
            ..SimilarityParams::default()
        });
        assert!(guarded_floor.add_unit(1, &orig));
        assert!(guarded_floor.add_unit(2, &type3));
        assert_eq!(guarded_floor.similar_pairs(threshold).len(), 1);

        // Invalid weights must be rejected up front, including on the Jaccard-only fallback path.
        let bad_configs = [
            SimilarityParams {
                eligibility: elig,
                jaccard_weight: f64::NAN,
                max_ted_work: 4,
                ..SimilarityParams::default()
            },
            SimilarityParams {
                eligibility: elig,
                tsed_weight: -f64::from(1),
                max_ted_work: 4,
                ..SimilarityParams::default()
            },
            SimilarityParams {
                eligibility: elig,
                label_weights: LabelWeights {
                    exact: f64::NAN,
                    ..LabelWeights::default()
                },
                max_ted_work: 4,
                ..SimilarityParams::default()
            },
        ];
        for params in bad_configs {
            let mut corpus = SimilarityCorpus::with_params(params);
            assert!(corpus.add_unit(1, &orig));
            assert!(corpus.add_unit(2, &orig));
            assert_eq!(corpus.similar_pairs(threshold), Vec::new());
        }
    }
}
