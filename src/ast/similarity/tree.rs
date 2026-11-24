use std::collections::HashSet;

/// Namespace of an abstracted identifier for consistent parameterized matching.
/// Identical spellings in different namespaces represent distinct symbols (e.g., `self.x = x`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameSpace {
    /// Local variable, parameter, or binding target.
    Plain,
    /// Object or struct field / member access (`self.x`, `order.total`).
    Member,
    /// Named argument label or struct/object literal key (`f(x: y)`, `{ x: y }`).
    Label,
    /// Type or module name when abstracted.
    Type,
    /// Enclosing function's own name (isolated from locals and members).
    Own,
}

impl NameSpace {
    #[must_use]
    pub(crate) const fn tag(self) -> u64 {
        match self {
            Self::Plain => 1,
            Self::Member => 2,
            Self::Label => 3,
            Self::Type => 4,
            Self::Own => 5,
        }
    }
}

/// Type-level category for abstracted literal values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    Number,
    String,
    Bool,
    Null,
    Char,
    Other,
}

impl LitKind {
    #[must_use]
    pub(crate) const fn tag(self) -> u64 {
        match self {
            Self::Number => 1,
            Self::String => 2,
            Self::Bool => 3,
            Self::Null => 4,
            Self::Char => 5,
            Self::Other => 6,
        }
    }
}

/// Canonical language-agnostic structural AST node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AstKind {
    Block,
    If,
    Branch,
    BranchArm,
    Loop,
    Return,
    Try,
    Closure,
    Bind,
    Assign,
    Call,
    Construct,
    Field,
    Index,
    Binary,
    Unary,
    Collection,
    Arg,
    Other,
}

impl AstKind {
    #[must_use]
    pub(crate) const fn tag(self) -> u64 {
        match self {
            Self::Block => 1,
            Self::If => 2,
            Self::Branch => 3,
            Self::BranchArm => 4,
            Self::Loop => 5,
            Self::Return => 6,
            Self::Try => 7,
            Self::Closure => 8,
            Self::Bind => 9,
            Self::Assign => 10,
            Self::Call => 11,
            Self::Construct => 12,
            Self::Field => 13,
            Self::Index => 14,
            Self::Binary => 15,
            Self::Unary => 16,
            Self::Collection => 17,
            Self::Arg => 18,
            Self::Other => 19,
        }
    }

    /// Whether consecutive structurally identical children inside this container should be folded
    /// during Stage 0 boilerplate size measurement (Higo & Kusumoto 2012).
    const fn supports_sibling_folding(self) -> bool {
        matches!(
            self,
            Self::Block | Self::Branch | Self::Construct | Self::Collection | Self::Call
        )
    }

    /// Whether this structural kind counts toward semantic/structural diversity (`TKS`).
    const fn counts_for_diversity(self) -> bool {
        !matches!(self, Self::Block | Self::Arg | Self::Other)
    }
}

/// Payload of a single node in a [`NormalizedTree`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeLabel {
    /// Internal structural container node.
    Struct(AstKind),
    /// Preserved exact symbol: callee/method name, operator (`+`, `==`, `&&`), or constructor type.
    Exact(String),
    /// Abstracted identifier that participates in consistent parameterized matching within its namespace.
    Symbol { space: NameSpace, name: String },
    /// Type-abstracted literal value.
    Literal(LitKind),
}

/// Arena-stored node inside a [`NormalizedTree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub(crate) label: NodeLabel,
    pub(crate) children: Vec<usize>,
}

impl TreeNode {
    #[must_use]
    pub fn label(&self) -> &NodeLabel {
        &self.label
    }

    #[must_use]
    pub fn children(&self) -> &[usize] {
        &self.children
    }
}

/// Stage 0 structural eligibility metrics computed over a [`NormalizedTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EligibilityStats {
    /// Total normalized AST nodes before sibling folding.
    pub raw_nodes: usize,
    /// Effective AST nodes after folding consecutive structurally identical siblings.
    pub folded_nodes: usize,
    /// Number of distinct non-trivial structural kinds and exact operators/callees (`TKS`).
    pub distinct_kinds: usize,
}

impl EligibilityStats {
    #[must_use]
    pub const fn is_eligible(self, min_nodes: usize, min_distinct_kinds: usize) -> bool {
        self.folded_nodes >= min_nodes && self.distinct_kinds >= min_distinct_kinds
    }
}

/// Domain-separation tags for Stage 0 skeleton hashing.
#[derive(Clone, Copy)]
#[repr(u64)]
enum SkeletonDomain {
    Kind = 0x517c_c1b7_2722_0a95,
    Exact = 0x9e37_79b9_7f4a_7c15,
    Symbol = 0x243f_6a88_85a3_08d3,
    Literal = 0x1319_8a2e_0370_7344,
}

/// Compact, arena-allocated language-agnostic normalized AST for a function or test body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedTree {
    nodes: Vec<TreeNode>,
}

impl NormalizedTree {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn nodes(&self) -> &[TreeNode] {
        &self.nodes
    }

    /// Compute Stage 0 eligibility metrics: raw size, sibling-folded effective size (keeping at
    /// most `max_identical_siblings` consecutive structurally identical siblings), and distinct
    /// structural/exact diversity (`TKS`).
    #[must_use]
    pub fn eligibility_stats(&self, max_identical_siblings: usize) -> EligibilityStats {
        if self.nodes.is_empty() {
            return EligibilityStats::default();
        }

        let cap = max_identical_siblings.max(1);
        let order = self.post_order_indices();
        let n = self.nodes.len();
        let mut skeleton = vec![0u64; n];
        let mut folded_size = vec![0usize; n];
        let mut folded_skip = vec![false; n];

        for &idx in &order {
            let node = &self.nodes[idx];
            skeleton[idx] = match &node.label {
                NodeLabel::Struct(kind) => {
                    let mut h = mix_u64(SkeletonDomain::Kind as u64, kind.tag());
                    for &child in &node.children {
                        h = mix_u64(h, skeleton[child]);
                    }
                    h
                }
                NodeLabel::Exact(text) => hash_str(SkeletonDomain::Exact as u64, text),
                NodeLabel::Symbol { space, .. } => {
                    mix_u64(SkeletonDomain::Symbol as u64, space.tag())
                }
                NodeLabel::Literal(_) => SkeletonDomain::Literal as u64,
            };

            let can_fold =
                matches!(node.label, NodeLabel::Struct(k) if k.supports_sibling_folding());
            let mut size = 1usize;
            let mut prev_skel: Option<u64> = None;
            let mut run_len = 0usize;

            for &child in &node.children {
                let child_skel = skeleton[child];
                if can_fold && prev_skel == Some(child_skel) {
                    run_len += 1;
                    if run_len > cap {
                        folded_skip[child] = true;
                        continue;
                    }
                } else {
                    prev_skel = Some(child_skel);
                    run_len = 1;
                }
                size += folded_size[child];
            }
            folded_size[idx] = size;
        }

        let distinct_kinds = self.count_distinct_kinds(&folded_skip);

        EligibilityStats {
            raw_nodes: n,
            folded_nodes: folded_size[0],
            distinct_kinds,
        }
    }

    /// Compute an iterative post-order traversal of node indices starting from root `0`.
    #[must_use]
    pub(crate) fn post_order_indices(&self) -> Vec<usize> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<(usize, bool)> = vec![(0, false)];
        while let Some((idx, expanded)) = stack.pop() {
            if expanded {
                order.push(idx);
            } else {
                stack.push((idx, true));
                for &child in self.nodes[idx].children.iter().rev() {
                    stack.push((child, false));
                }
            }
        }
        order
    }

    fn count_distinct_kinds(&self, folded_skip: &[bool]) -> usize {
        let mut kinds: HashSet<u64> = HashSet::new();
        let mut stack = vec![0usize];
        while let Some(idx) = stack.pop() {
            if folded_skip[idx] {
                continue;
            }
            let node = &self.nodes[idx];
            match &node.label {
                NodeLabel::Struct(kind) if kind.counts_for_diversity() => {
                    kinds.insert(mix_u64(SkeletonDomain::Kind as u64, kind.tag()));
                }
                NodeLabel::Exact(text) => {
                    kinds.insert(hash_str(SkeletonDomain::Exact as u64, text));
                }
                NodeLabel::Struct(_) | NodeLabel::Symbol { .. } | NodeLabel::Literal(_) => {}
            }
            for &child in &node.children {
                stack.push(child);
            }
        }
        kinds.len()
    }
}

/// Deterministic, fast 64-bit `SplitMix64` mixing primitive.
#[inline]
#[must_use]
pub(crate) const fn mix_u64(acc: u64, value: u64) -> u64 {
    let mut z = acc.wrapping_add(value.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Deterministic 64-bit hash over a byte slice with a domain-separation seed.
#[must_use]
pub(crate) fn hash_str(seed: u64, text: &str) -> u64 {
    let mut h = seed ^ 0xcbf2_9ce4_8422_2325;
    for &b in text.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    mix_u64(h, text.len() as u64)
}

/// Stack-based builder for constructing a [`NormalizedTree`] in a single CST walk.
///
/// Always maintains an implicit root [`AstKind::Block`] at index `0` so sequential top-level
/// statements remain siblings rather than nesting under the first statement. Frontends should
/// explicitly enter [`AstKind::Block`] for block-delimited function bodies; single-node
/// expression bodies (e.g., arrow expressions) unwrap cleanly to that structural root.
///
/// If [`TreeBuilder::enter`] and [`TreeBuilder::exit`] are ever mismatched, the builder marks
/// itself unbalanced and [`TreeBuilder::finish`] returns an empty [`NormalizedTree`] rather
/// than silently emitting a corrupted tree.
#[derive(Debug)]
pub struct TreeBuilder {
    nodes: Vec<TreeNode>,
    stack: Vec<usize>,
    unbalanced: bool,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: vec![TreeNode {
                label: NodeLabel::Struct(AstKind::Block),
                children: Vec::new(),
            }],
            stack: vec![0],
            unbalanced: false,
        }
    }

    fn current_parent(&self) -> usize {
        self.stack.last().copied().unwrap_or(0)
    }

    /// Push a new structural container node onto the stack and return its node index mark.
    pub fn enter(&mut self, kind: AstKind) -> usize {
        let parent = self.current_parent();
        let idx = self.nodes.len();
        self.nodes.push(TreeNode {
            label: NodeLabel::Struct(kind),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(idx);
        self.stack.push(idx);
        idx
    }

    /// Pop the matching structural container node from the stack.
    pub fn exit(&mut self, mark: usize) {
        if self.stack.len() <= 1 || self.stack.last().copied() != Some(mark) {
            self.unbalanced = true;
            return;
        }
        self.stack.pop();
    }

    /// Emit a preserved exact leaf (callee/method name, operator, or constructor type).
    pub fn exact(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.push_leaf(NodeLabel::Exact(text));
    }

    /// Emit a namespace-scoped abstracted identifier leaf for consistent parameterized matching.
    pub fn symbol(&mut self, space: NameSpace, name: impl Into<String>) {
        let name = name.into();
        if name.is_empty() {
            return;
        }
        self.push_leaf(NodeLabel::Symbol { space, name });
    }

    /// Emit a type-abstracted literal leaf.
    pub fn literal(&mut self, kind: LitKind) {
        self.push_leaf(NodeLabel::Literal(kind));
    }

    fn push_leaf(&mut self, label: NodeLabel) {
        let parent = self.current_parent();
        let idx = self.nodes.len();
        self.nodes.push(TreeNode {
            label,
            children: Vec::new(),
        });
        self.nodes[parent].children.push(idx);
    }

    /// Finalize the builder into an immutable [`NormalizedTree`], unwrapping the implicit root
    /// wrapper when the body already consists of a single structural node.
    #[must_use]
    pub fn finish(mut self) -> NormalizedTree {
        if self.unbalanced || self.stack.len() != 1 || self.nodes[0].children.is_empty() {
            return NormalizedTree::default();
        }
        if self.nodes[0].children.len() == 1
            && self.nodes[0].children[0] == 1
            && matches!(self.nodes[1].label, NodeLabel::Struct(_))
        {
            self.nodes.remove(0);
            for node in &mut self.nodes {
                for child in &mut node.children {
                    *child -= 1;
                }
            }
        }
        NormalizedTree { nodes: self.nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_unclosed_mismatched_and_single_leaf_tree_edge_cases() {
        let empty = NormalizedTree::default();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.eligibility_stats(2), EligibilityStats::default());
        assert!(!empty.eligibility_stats(2).is_eligible(1, 1));

        // Unclosed enter produces an empty tree rather than a corrupted partial tree.
        let mut unclosed = TreeBuilder::new();
        let _leaked = unclosed.enter(AstKind::If);
        unclosed.symbol(NameSpace::Plain, "cond");
        assert!(unclosed.finish().is_empty());

        // Mismatched exit mark also produces an empty tree cleanly without panicking.
        let mut mismatched = TreeBuilder::new();
        let outer = mismatched.enter(AstKind::Block);
        let inner = mismatched.enter(AstKind::If);
        mismatched.exit(outer);
        mismatched.exit(inner);
        assert!(mismatched.finish().is_empty());

        // Single bare leaf keeps the implicit Block root intact.
        let mut leaf_only = TreeBuilder::new();
        leaf_only.symbol(NameSpace::Plain, "value");
        let leaf_tree = leaf_only.finish();
        assert_eq!(leaf_tree.len(), 2);
        assert_eq!(
            leaf_tree.nodes()[0].label(),
            &NodeLabel::Struct(AstKind::Block)
        );
        assert_eq!(leaf_tree.nodes()[0].children(), &[1]);
    }

    #[test]
    fn sequential_top_level_enters_remain_siblings_under_block_root() {
        let mut b = TreeBuilder::new();
        let first = b.enter(AstKind::Assign);
        b.exact("=");
        b.symbol(NameSpace::Plain, "a");
        b.literal(LitKind::Number);
        b.exit(first);

        let second = b.enter(AstKind::Return);
        b.exact("return");
        b.symbol(NameSpace::Plain, "a");
        b.exit(second);

        let tree = b.finish();
        assert_eq!(tree.nodes()[0].label(), &NodeLabel::Struct(AstKind::Block));
        assert_eq!(tree.nodes()[0].children().len(), 2);
    }

    #[test]
    fn folds_repeated_identical_siblings_for_stage0_eligibility() {
        // Simulate a 12-field constructor: `self.f0 = p0; self.f1 = p1; ... self.f11 = p11;`
        let mut b = TreeBuilder::new();
        let root = b.enter(AstKind::Block);
        for i in 0..12 {
            let assign = b.enter(AstKind::Assign);
            b.exact("=");
            let field = b.enter(AstKind::Field);
            b.symbol(NameSpace::Plain, "self");
            b.symbol(NameSpace::Member, format!("field_{i}"));
            b.exit(field);
            b.symbol(NameSpace::Plain, format!("param_{i}"));
            b.exit(assign);
        }
        b.exit(root);

        let tree = b.finish();
        let stats = tree.eligibility_stats(2);
        // Raw: 1 block + 12 * 6 = 73 nodes
        assert_eq!(stats.raw_nodes, 73);
        // Folded: 1 block + 2 * 6 = 13 nodes
        assert_eq!(stats.folded_nodes, 13);
        // Distinct kinds: Assign, "=", Field -> 3
        assert_eq!(stats.distinct_kinds, 3);
        assert!(!stats.is_eligible(25, 4));
    }

    #[test]
    fn does_not_fold_distinct_statements() {
        let mut b = TreeBuilder::new();
        let root = b.enter(AstKind::Block);

        let bind = b.enter(AstKind::Bind);
        b.symbol(NameSpace::Plain, "sum");
        b.literal(LitKind::Number);
        b.exit(bind);

        let lp = b.enter(AstKind::Loop);
        b.symbol(NameSpace::Plain, "x");
        b.symbol(NameSpace::Plain, "xs");
        let cond = b.enter(AstKind::If);
        let cmp = b.enter(AstKind::Binary);
        b.exact(">");
        b.symbol(NameSpace::Plain, "x");
        b.literal(LitKind::Number);
        b.exit(cmp);
        let add = b.enter(AstKind::Assign);
        b.exact("+=");
        b.symbol(NameSpace::Plain, "sum");
        b.symbol(NameSpace::Plain, "x");
        b.exit(add);
        b.exit(cond);
        b.exit(lp);

        let ret = b.enter(AstKind::Return);
        b.exact("return");
        b.symbol(NameSpace::Plain, "sum");
        b.exit(ret);

        b.exit(root);

        let tree = b.finish();
        let stats = tree.eligibility_stats(2);
        assert_eq!(stats.raw_nodes, stats.folded_nodes);
        assert!(stats.distinct_kinds >= 8);
    }
}
