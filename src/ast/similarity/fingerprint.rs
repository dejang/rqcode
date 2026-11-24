use crate::ast::similarity::tree::{
    AstKind, LitKind, NameSpace, NodeLabel, NormalizedTree, hash_str, mix_u64,
};
use std::collections::HashMap;

/// Semantic node edit weights used by Stage 3 weighted Tree Edit Distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelWeights {
    /// Weight for preserved exact tokens (callees, operators, constructor types).
    pub exact: f64,
    /// Weight for structural containers, symbols, and literals.
    pub structural: f64,
    /// Relabel cost between two symbols in the same namespace with different Baker back-distances.
    /// Kept small so mid-body Type-3 insertions that shift live variables stay localized while
    /// Stage 2A subtree-local Merkle hashes enforce exact variable co-reference.
    pub symbol_relabel: f64,
}

impl Default for LabelWeights {
    fn default() -> Self {
        Self {
            exact: 2.0,
            structural: 1.0,
            symbol_relabel: 0.25,
        }
    }
}

impl LabelWeights {
    /// Extract Stage 2A Merkle subtree fingerprints (with subtree-local Baker `prev` consistent
    /// renaming at each repeat's lowest common ancestor) AND the Stage 3 post-order
    /// [`CanonicalTree`].
    #[must_use]
    pub(crate) fn extract_unit(self, tree: &NormalizedTree) -> FingerprintedUnit {
        let nodes = tree.nodes();
        if nodes.is_empty() {
            return FingerprintedUnit {
                hashes: Vec::new(),
                canonical: CanonicalTree::empty(),
            };
        }

        let n = nodes.len();
        let mut pre_order = Vec::with_capacity(n);
        let mut stack = vec![0usize];
        while let Some(idx) = stack.pop() {
            pre_order.push(idx);
            for &child in nodes[idx].children.iter().rev() {
                stack.push(child);
            }
        }

        let mut name_occ: Vec<Option<(u32, u32)>> = vec![None; n];
        let mut back: Vec<u32> = Vec::new();
        let mut last: HashMap<(NameSpace, &str), u32> = HashMap::new();

        // Assign sequential pre-order symbol indices and Baker `prev` back-distances per namespace.
        for &idx in &pre_order {
            if let NodeLabel::Symbol { space, name } = &nodes[idx].label
                && *space != NameSpace::Own
            {
                let occ = u32::try_from(back.len()).unwrap_or(u32::MAX);
                let prev_dist = last
                    .insert((*space, name.as_str()), occ)
                    .map_or(0, |prev| occ.saturating_sub(prev));
                back.push(prev_dist);
                name_occ[idx] = Some((occ, prev_dist));
            }
        }

        let post_order = tree.post_order_indices();
        let mut hasher = SubtreeHasher::new(n, &back);

        // Stage 3 post-order arrays (1-indexed).
        let mut labels = Vec::with_capacity(n + 1);
        labels.push(CanonicalLabel::Struct(AstKind::Block));
        let mut leftmost = vec![0usize; n + 1];
        let mut arena_to_post = vec![0usize; n];
        let mut total_weight = 0.0f64;

        for (pos0, &idx) in post_order.iter().enumerate() {
            let post_id = pos0 + 1;
            arena_to_post[idx] = post_id;
            let node = &nodes[idx];

            let (val, canon_label) = match &node.label {
                NodeLabel::Exact(text) => {
                    let h = hash_str(HashDomain::Exact as u64, text);
                    hasher.all_hashes.push(h);
                    (NodeVal::Atom(h), CanonicalLabel::Exact(h))
                }
                NodeLabel::Literal(kind) => {
                    let h = mix_u64(HashDomain::Literal as u64, kind.tag());
                    // Bare literals are hashed into their enclosing parent expressions,
                    // not added as standalone fingerprints that cause free overlap.
                    (NodeVal::Atom(h), CanonicalLabel::Literal(*kind))
                }
                NodeLabel::Symbol { space, .. } => {
                    if let Some((occ, prev_dist)) = name_occ[idx] {
                        (
                            NodeVal::Name { occ, space: *space },
                            CanonicalLabel::Symbol(*space, prev_dist),
                        )
                    } else {
                        (
                            NodeVal::Atom(mix_u64(HashDomain::SymbolAtom as u64, space.tag())),
                            CanonicalLabel::Symbol(*space, 0),
                        )
                    }
                }
                NodeLabel::Struct(kind) => {
                    let list_val = hasher.hash_struct(*kind, &node.children);
                    (list_val, CanonicalLabel::Struct(*kind))
                }
            };

            hasher.vals[idx] = val;
            total_weight += canon_label.weight(self);
            labels.push(canon_label);
            leftmost[post_id] = node
                .children
                .first()
                .map_or(post_id, |&first_child| leftmost[arena_to_post[first_child]]);
        }

        let mut fingerprints = hasher.all_hashes;
        fingerprints.sort_unstable();
        fingerprints.dedup();

        let mut highest_for_leaf = vec![0usize; n + 1];
        for (post_id, &leaf) in leftmost.iter().enumerate().skip(1) {
            highest_for_leaf[leaf] = post_id;
        }
        let mut keyroots: Vec<usize> = highest_for_leaf.into_iter().filter(|&id| id > 0).collect();
        keyroots.sort_unstable();

        FingerprintedUnit {
            hashes: fingerprints,
            canonical: CanonicalTree {
                labels,
                leftmost,
                keyroots,
                total_weight,
            },
        }
    }
}

/// Canonical node label for Stage 3 weighted Tree Edit Distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CanonicalLabel {
    Struct(AstKind),
    Exact(u64),
    Symbol(NameSpace, u32),
    Literal(LitKind),
}

impl CanonicalLabel {
    #[must_use]
    pub(crate) const fn weight(self, weights: LabelWeights) -> f64 {
        match self {
            Self::Exact(_) => weights.exact,
            Self::Struct(_) | Self::Symbol(_, _) | Self::Literal(_) => weights.structural,
        }
    }

    /// Symmetric relabel cost satisfying `relabel(a, b) <= weight(a) + weight(b)`.
    #[must_use]
    pub(crate) fn relabel_cost(self, other: Self, weights: LabelWeights) -> f64 {
        if self == other {
            return 0.0;
        }
        if let (Self::Symbol(left_ns, _), Self::Symbol(right_ns, _)) = (self, other)
            && left_ns == right_ns
        {
            return weights.symbol_relabel.min(weights.structural);
        }
        self.weight(weights).max(other.weight(weights))
    }
}

/// Post-order flattened tree prepared for Zhang-Shasha weighted Tree Edit Distance.
#[derive(Debug, Clone)]
pub(crate) struct CanonicalTree {
    /// 1-indexed post-order labels (index `0` is unused sentinel).
    labels: Vec<CanonicalLabel>,
    /// 1-indexed post-order leftmost-leaf descendants (`leftmost[i] <= i`).
    leftmost: Vec<usize>,
    /// Keyroots for Zhang-Shasha DP (sorted ascending post-order indices).
    keyroots: Vec<usize>,
    /// Sum of weights of all nodes in the tree.
    total_weight: f64,
}

impl CanonicalTree {
    #[must_use]
    pub(crate) fn empty() -> Self {
        Self {
            labels: vec![CanonicalLabel::Struct(AstKind::Block)],
            leftmost: vec![0],
            keyroots: Vec::new(),
            total_weight: 0.0,
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.labels.len().saturating_sub(1)
    }

    #[must_use]
    pub(crate) fn labels(&self) -> &[CanonicalLabel] {
        &self.labels
    }

    #[must_use]
    pub(crate) fn leftmost(&self) -> &[usize] {
        &self.leftmost
    }

    #[must_use]
    pub(crate) fn keyroots(&self) -> &[usize] {
        &self.keyroots
    }

    #[must_use]
    pub(crate) const fn total_weight(&self) -> f64 {
        self.total_weight
    }
}

/// Stage 2A + Stage 3 representation extracted in a single pass from a [`NormalizedTree`].
#[derive(Debug, Clone)]
pub(crate) struct FingerprintedUnit {
    /// Sorted, deduplicated 64-bit Merkle subtree fingerprints (excluding bare `:symbol` / `:lit` leaves).
    pub(crate) hashes: Vec<u64>,
    /// Post-order canonical tree with Baker `prev` labels for Stage 3 weighted TSED.
    pub(crate) canonical: CanonicalTree,
}

/// Domain-separation tags for Stage 2A Merkle subtree hashing.
#[derive(Clone, Copy)]
#[repr(u64)]
enum HashDomain {
    Kind = 0x6b65_7977_6f72_6421,
    Exact = 0x7374_7269_6e67_7321,
    Literal = 0x6c69_7465_7261_6c21,
    List = 0x6c69_7374_7321_2121,
    Repeat = 0x7265_7065_6174_7321,
    SymbolAtom = 0x7379_6d62_6f6c_2121,
}

/// Bottom-up fingerprint summary for one arena node during extraction.
#[derive(Clone, Copy)]
enum NodeVal {
    Atom(u64),
    Name {
        occ: u32,
        space: NameSpace,
    },
    List {
        hash: u64,
        names: [u32; 2],
        open: [usize; 2],
    },
}

impl NodeVal {
    const fn names(self) -> Option<[u32; 2]> {
        match self {
            Self::Name { occ, .. } => Some([occ, occ + 1]),
            Self::List { names, .. } if names[0] < names[1] => Some(names),
            Self::Atom(_) | Self::List { .. } => None,
        }
    }
}

struct SubtreeHasher<'a> {
    vals: Vec<NodeVal>,
    back: &'a [u32],
    open_pool: Vec<u32>,
    all_hashes: Vec<u64>,
}

impl<'a> SubtreeHasher<'a> {
    fn new(node_count: usize, back: &'a [u32]) -> Self {
        Self {
            vals: vec![NodeVal::Atom(0); node_count],
            back,
            open_pool: Vec::new(),
            all_hashes: Vec::with_capacity(node_count),
        }
    }

    fn hash_struct(&mut self, kind: AstKind, children: &[usize]) -> NodeVal {
        let mut h = mix_u64(
            HashDomain::List as u64,
            mix_u64(HashDomain::Kind as u64, kind.tag()),
        );
        let mut names = [u32::MAX, 0u32];

        for &child in children {
            let val = self.vals[child];
            let child_hash = match val {
                NodeVal::Atom(ah) => ah,
                NodeVal::Name { space, .. } => mix_u64(HashDomain::SymbolAtom as u64, space.tag()),
                NodeVal::List { hash, .. } => hash,
            };
            if let Some([first, end]) = val.names() {
                names = [names[0].min(first), names[1].max(end)];
            }
            h = mix_u64(h, child_hash);
        }

        let open_start = self.open_pool.len();
        let mut repeat_count = 0u64;

        for &child in children {
            match self.vals[child] {
                NodeVal::Name { occ, .. } => {
                    if let Some(pair) = self.close_or_propagate(occ, names[0]) {
                        h = mix_u64(h, pair);
                        repeat_count += 1;
                    }
                }
                NodeVal::List { open, .. } => {
                    for pos in open[0]..open[1] {
                        let occ = self.open_pool[pos];
                        if let Some(pair) = self.close_or_propagate(occ, names[0]) {
                            h = mix_u64(h, pair);
                            repeat_count += 1;
                        }
                    }
                }
                NodeVal::Atom(_) => {}
            }
        }

        if repeat_count > 0 {
            h = mix_u64(h, mix_u64(HashDomain::Repeat as u64, repeat_count));
        }

        self.all_hashes.push(h);

        let final_names = if names[0] < names[1] { names } else { [0, 0] };
        NodeVal::List {
            hash: h,
            names: final_names,
            open: [open_start, self.open_pool.len()],
        }
    }

    /// Close a variable repeat at its lowest common ancestor subtree, or keep it open for
    /// enclosing ancestors when the previous occurrence preceded `subtree_first`.
    fn close_or_propagate(&mut self, occ: u32, subtree_first: u32) -> Option<u64> {
        let dist = self.back[occ as usize];
        if dist == 0 {
            return None;
        }
        if occ - dist >= subtree_first {
            let local_pos = occ - subtree_first;
            let pair = (u64::from(local_pos) << 32) | u64::from(dist);
            Some(mix_u64(HashDomain::Repeat as u64, pair))
        } else {
            self.open_pool.push(occ);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::similarity::tree::TreeBuilder;

    fn build_expr_assign(vars: [&str; 4], ops: [&str; 2]) -> NormalizedTree {
        let mut tb = TreeBuilder::new();
        let assign = tb.enter(AstKind::Assign);
        tb.exact("=");
        tb.symbol(NameSpace::Plain, vars[0]);
        let add = tb.enter(AstKind::Binary);
        tb.exact(ops[0]);
        tb.symbol(NameSpace::Plain, vars[1]);
        let mul = tb.enter(AstKind::Binary);
        tb.exact(ops[1]);
        tb.symbol(NameSpace::Plain, vars[2]);
        tb.symbol(NameSpace::Plain, vars[3]);
        tb.exit(mul);
        tb.exit(add);
        tb.exit(assign);
        tb.finish()
    }

    #[test]
    fn canonical_tree_empty_and_extracted_metadata() {
        let weights = LabelWeights::default();
        let empty_roots: &[usize] = &[];
        let empty = CanonicalTree::empty();
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.keyroots(), empty_roots);
        assert!((empty.total_weight() - 0.0).abs() < f64::EPSILON);

        let unit = weights.extract_unit(&build_expr_assign(["d", "a", "b", "c"], ["+", "*"]));
        assert_eq!(unit.canonical.len(), 10);
        assert_ne!(unit.canonical.keyroots(), empty_roots);
        // 3 Exact operators (weight 2.0 each = 6.0) + 3 Struct + 4 Symbol (weight 1.0 each = 7.0)
        assert!((unit.canonical.total_weight() - 13.0).abs() < 1e-12);
    }

    #[test]
    fn consistent_renaming_matches_isomorphic_vars_and_separates_non_isomorphic() {
        let weights = LabelWeights::default();
        let d_abc = weights.extract_unit(&build_expr_assign(["d", "a", "b", "c"], ["+", "*"]));
        let s_pqr = weights.extract_unit(&build_expr_assign(["s", "p", "q", "r"], ["+", "*"]));
        let x_xxx = weights.extract_unit(&build_expr_assign(["x", "x", "x", "x"], ["+", "*"]));
        let diff_op = weights.extract_unit(&build_expr_assign(["d", "a", "b", "c"], ["-", "*"]));

        assert_eq!(
            d_abc.hashes, s_pqr.hashes,
            "renamed isomorphic trees must match"
        );
        assert_ne!(
            d_abc.hashes, x_xxx.hashes,
            "x = x + x * x must not match d = a + b * c"
        );
        assert_ne!(
            d_abc.hashes, diff_op.hashes,
            "different operators must produce distinct fingerprints"
        );
    }

    #[test]
    fn separate_namespaces_keep_self_field_distinct_from_local_name() {
        let weights = LabelWeights::default();
        let build = |field: &str, param: &str| {
            let mut tb = TreeBuilder::new();
            let assign = tb.enter(AstKind::Assign);
            tb.exact("=");
            let fld = tb.enter(AstKind::Field);
            tb.symbol(NameSpace::Plain, "self");
            tb.symbol(NameSpace::Member, field);
            tb.exit(fld);
            tb.symbol(NameSpace::Plain, param);
            tb.exit(assign);
            weights.extract_unit(&tb.finish())
        };

        let same_name = build("x", "x");
        let diff_name = build("x", "y");
        assert_eq!(
            same_name.hashes, diff_name.hashes,
            "self.x = x and self.x = y should match because Member and Plain are distinct namespaces"
        );
    }

    #[test]
    fn different_callees_have_zero_shared_fingerprints() {
        let weights = LabelWeights::default();
        let build_call = |callee: &str| {
            let mut tb = TreeBuilder::new();
            let call = tb.enter(AstKind::Call);
            tb.exact(callee);
            tb.symbol(NameSpace::Plain, "x");
            tb.literal(LitKind::Number);
            tb.exit(call);
            weights.extract_unit(&tb.finish())
        };

        let a = build_call("validateEmail");
        let b = build_call("renderHeader");
        let overlap = a
            .hashes
            .iter()
            .filter(|h| b.hashes.binary_search(h).is_ok())
            .count();
        assert_eq!(
            overlap, 0,
            "unrelated calls must not share bare :symbol or :lit fingerprints"
        );
    }
}
