# Code Similarity Architecture & Design Rationale

This document explains **why** `rqcode`'s `duplicate-function` and `overlapping-tests` rules work the way they do: what the clone-detection literature establishes, which Rust crates and algorithms we evaluated and rejected, how the 4-stage pipeline works in detail, and why Program Dependence Graphs (PDGs) are deliberately deferred.

---

## 1. Requirements & Why the Naive Approach Failed

`rqcode` is a zero-build, offline, single-binary polyglot quality gate (`Rust`, `TypeScript`, `Swift`, plus injectable custom `LanguageFrontend`s). Its similarity engine must satisfy five constraints:

1. **Zero compiler/build requirement:** Analyze source directly from `tree-sitter` CSTs (`ast-grep-core`) without `rustc`, `tsc`, or `swiftc`.
2. **Strict language isolation:** `src/ast/` and `src/rules/` must remain 100% language-agnostic (enforced by `tests/architecture_guardrails.rs`).
3. **Sub-second latency:** Whole-repository scans should complete in tens to hundreds of milliseconds on typical codebases.
4. **Generic false-positive suppression:** Never hardcode framework or API names (e.g., SwiftUI `View`, `body`, `init`) to hide noise.
5. **High precision on both production functions and tests:** Separate copy-pasted logic from repetitive boilerplate (field-assignment constructors, flat literal tables, repeated statement-level calls/views).

### Why v0.1's Flat-Token TF-IDF Cosine Failed

`rqcode` v0.1 flattened each function body into a token stream, blindly replaced every identifier with `IDENT`, built token n-grams, and scored all pairs with TF-IDF cosine similarity. During v0.2 evaluation on an 87-file Swift/SwiftUI application, that design emitted 518 `duplicate-function` diagnostics of which only 4 were genuine copy-pasted methods. Four structural flaws caused the noise:

- **Blind identifier collapsing destroys semantics:** Replacing callees and variables alike made `validateEmail(x)` and `renderHeader(y)` indistinguishable and equated `x = x + x * x` with `d = a + b * c`.
- **Flat n-grams erase AST hierarchy:** Unrelated nested expressions collided whenever they shared short local token windows.
- **Cosine similarity is scale-invariant:** $\cos(A, B) = \frac{A \cdot B}{\lVert A \rVert \lVert B \rVert}$ normalizes away length, so a repeated block (`S; S;`) scores $\approx 1.00$ against `S;` (exact `1.00` modulo seam n-grams), and a 3-field initializer can score above `0.95` against a 15-field one.
- **Raw token counts mistake boilerplate for logic:** Flat constructors, literal lookup tables, and repetitive view blocks easily cleared `min_tokens` despite low cyclomatic complexity and minimal structural diversity.

---

## 2. Literature Survey: Clone Taxonomy & Algorithm Families

Code clone research uses the standard four-tier taxonomy (Roy & Cordy, 2007; Roy, Cordy, & Koschke, 2009):

- **Type-1 (Exact):** Identical except whitespace, layout, and comments.
- **Type-2 (Parameterized):** Structurally identical with renamed variables, literals, or types.
- **Type-3 (Near-miss):** Type-2 plus added, deleted, or modified statements (subdivided in BigCloneBench, Svajlenko et al., 2014, into Very-Strongly, Strongly, and Moderately Type-3).
- **Type-4 (Semantic):** Different syntax implementing the same computation (e.g., imperative loop vs. `.filter().map()`).

Decades of clone detectors cluster into six families:

| Family | Landmark Systems | Core Mechanism | Strengths | Why Adopted / Rejected for `rqcode` |
| :--- | :--- | :--- | :--- | :--- |
| **1. Text / LCS** | NiCad (Roy & Cordy, 2008) | Pretty-printing + consistent renaming + line LCS | High T1–ST3 recall and precision with consistent renaming | Rejected: requires a pretty-printer grammar per language; line-oriented. |
| **2. Lexical / Token** | Baker's `prev` / Dup (1993/1995), CCFinder (Kamiya et al., 2002), Winnowing/MOSS (Schleimer et al., 2003), FRISC (Murakami et al., 2012), SourcererCC (Sajnani et al., 2016), NIL (Nakagawa et al., 2021) | Parameterized `prev` encoding; repeated-instruction folding; suffix trees/winnowing; rarity-sorted prefix-filtered inverted index | Scales near-linearly to huge corpora; SourcererCC's prefix filter is exact (zero false negatives) | **Indexing, renaming, and folding concepts adopted** (Baker `prev`, Bayardo/SourcererCC prefix filter, folded repetitions adapted to AST siblings); flat token streams rejected for losing AST hierarchy. |
| **3. AST / Subtree** | CloneDR (Baxter et al., 1998), Deckard (Jiang et al., 2007), `cpd-similarity`, GumTree (Falleri et al., 2014), Zhang-Shasha (1989) / APTED (Pawlik & Augsten, 2016) / TSED (Song et al., 2024) | Subtree Merkle hashes; LSH over AST node vectors; top-down/bottom-up tree diff; weighted Tree Edit Distance | High structural precision; directly exploits `tree-sitter` CSTs; TSED outperforms BLEU and lexical Jaccard at predicting execution match | **Core architecture adopted:** Merkle subtree Jaccard for fast retrieval + weighted Zhang-Shasha TSED for candidate verification. |
| **4. Metrics** | Mayrand et al. (1996); token-kind diversity (`TKS`, CCFinderX) | Control-flow/nesting counts and distinct structural-kind diversity | $O(N)$; rejects structurally trivial boilerplate generically | **Adopted as Stage 0 pre-filter**; too coarse as a standalone matcher. |
| **5. Graph / PDG** | PDG-DUP (Komondoor & Horwitz, 2001), GPLAG (Liu et al., 2006) | Control + data dependence graphs + subgraph isomorphism / slicing | Strong on reordered T3 and partial T4 | **Deferred** (Section 5): full PDG requires compiler type/alias info; syntactic PDG adds scoping complexity with limited gain over Stages 0–3. |
| **6. ML / Neural** | Oreo (Saini et al., 2018), CodeBERT (Feng et al., 2020) | Learned embeddings / Siamese classifiers | High in-distribution MT3/T4 recall | Rejected: heavy model footprint, slow, non-deterministic, degrades out-of-distribution. |

Production linters reinforce the same design lessons: preserving identifiers/callees (rather than blind renaming) and enforcing minimum structural complexity are the primary defenses against false positives (e.g., PMD CPD defaults to `ignoreIdentifiers = false` and `ignoreLiterals = false`; SonarQube preserves identifiers and skips declarative languages; JetBrains separates variable/function/field anonymization; `cpd-similarity` combines `tree-sitter` AST normalization with callee/operator preservation, namespace-scoped Baker `prev` renaming, and Merkle subtree Jaccard).

---

## 3. Why Native Instead of an Off-the-Shelf Crate

Before writing the engine, we audited candidate Rust crates on `crates.io` (versions inspected during v0.2 evaluation):

| Crate | Algorithm | Why We Did Not Adopt It Directly |
| :--- | :--- | :--- |
| **`cpd-similarity`** (`0.1.1`) | `tree-sitter` AST normalization (keeps callees/operators, consistent LCA `prev` renaming) + Merkle subtree exact Jaccard | **Best algorithmic blueprint** (directly inspired our Stages 1–2A), but had no Cargo feature flags (unconditionally pulling 14 `tree-sitter-*` C grammars + `oxc_*`), double-parsed source strings instead of reusing `ast-grep-core`'s CST, and skipped inline test modules by default. |
| **`dupes-core`** (`0.2.1`) | Parser-free 59-variant `NormalizedNode` tree + positional fingerprints/grouping | Lightweight and parser-free, but its near-duplicate scorer zipped children positionally (collapsing on single Type-3 statement insertions), and its `Call`/`MethodCall` nodes did not carry callee names by default. |
| **`cinereus`** (`2.0.3`) | GumTree (Merkle top-down + Dice bottom-up + Chawathe edit script) | Clean 2-dependency pairwise tree differ, but offers no candidate-retrieval index, and weighted Zhang-Shasha TSED gave us explicit per-label edit weights (callees/operators vs. shifted variables). |
| **`similarity-core`** (`0.5.2`) | Tree-sitter AST similarity | Unconditionally pulled `tree-sitter ^0.26` (conflicting with `rqcode`'s `0.27`), bundled 6 C grammars without Rust or Swift, and stored `Rc<TreeNode>` (`!Send`, `!Sync`), incompatible with `rayon`. |

Because `rqcode` already parses every file via `ast-grep-core`, implementing the pipeline natively in `src/ast/similarity/` gave us **zero new dependencies**, **single-pass CST parsing**, full **`rayon` parallelism**, uniform handling of `duplicate-function` and `overlapping-tests`, and strict adherence to `tests/architecture_guardrails.rs`.

---

## 4. The 4-Stage Pipeline

During file extraction, each language frontend runs **Stage 1** once per function/test body to produce a `NormalizedTree`. When `SimilarityRule` populates a `SimilarityCorpus`, `add_unit` applies **Stage 0** before indexing into **Stage 2A**, and `similar_pairs` runs **Stage 2B** followed by **Stage 3**:

```text
  tree-sitter CST (per function/test body)
         │
         ▼
  Stage 1: Callee/Operator-Preserving AST Normalization (src/lang/)
    CstNormalizer + CstNormalizeDialect -> NormalizedTree
         │
         ▼
  Stage 0: Sibling-Folding & Diversity Eligibility Gate (tree.rs)
    Fold consecutive identical siblings -> check folded_nodes & distinct_kinds
         │
         ▼
  Stage 2A: Subtree-Local Merkle Fingerprints + Canonical Tree (fingerprint.rs)
    Pre-order Baker `prev` distances -> post-order SplitMix64 LCA hashes + CanonicalTree
         │
         ▼
  Stage 2B: Exact Prefix-Filtered Jaccard Candidate Join (index.rs)
    Bayardo/SourcererCC inverted index sorted rarest-first by (df, hash)
         │  (surviving candidates, parallel via rayon)
         ▼
  Stage 3: Weighted Zhang-Shasha TSED Verifier (ted.rs, mod.rs)
    Weight-ratio upper-bound prune (within budget) -> weighted DP -> blended score
```

### Stage 1: Principled Normalization (`src/lang/common.rs`, `src/lang/*/normalize.rs`)

A shared bounded-recursion driver (`CstNormalizer`, default `max_depth = 512`) delegates syntax mapping to thin per-language `CstNormalizeDialect` implementations (`RustNormalizeDialect`, `TsNormalizeDialect`, `SwiftNormalizeDialect`). Each body lowers into an arena-allocated `NormalizedTree` (`src/ast/similarity/tree.rs`) with four `NodeLabel` variants:

- **`Struct(AstKind)`:** 19 canonical structural kinds (`Block`, `If`, `Branch`, `BranchArm`, `Loop`, `Return`, `Try`, `Closure`, `Bind`, `Assign`, `Call`, `Construct`, `Field`, `Index`, `Binary`, `Unary`, `Collection`, `Arg`, `Other`).
- **`Exact(String)`:** Preserved verbatim for **callees/method names, operators (`+`, `==`, `&&`), constructor/enum-variant names, keywords like `return`, and pinned primitives**. Preserving callees and operators strongly separates functions with identical control flow that call different domain operations (while a single callee change inside a large otherwise-identical function still appropriately scores high).
- **`Symbol { space, name }`:** Abstracted identifier participating in consistent parameterized matching within its **`NameSpace`** (`Plain` locals/params, `Member` fields, `Label` named arguments/keys, `Type`, and `Own`). Namespace separation ensures `self.x = x` matches `self.x = y` rather than conflating `Member("x")` with `Plain("x")`. (`Own` is reserved as an isolated non-parameterized atom and is not currently emitted by the three built-in dialects.)
- **`Literal(LitKind)`:** Abstracted to 6 type categories (`Number`, `String`, `Bool`, `Null`, `Char`, `Other`).

Dialects canonicalize superficial language quirks without leaking grammar types into core: TypeScript `this` and Swift implicit `self` normalize to `Plain("self")`; Rust macro token trees are normalized in `src/lang/rust/normalize.rs` (using token helpers in `src/lang/rust/macro_tokens.rs`); Swift normalizes shorthand `if let`, nested enum paths, subscripts (`AstKind::Index`), capture lists, and trailing-closure labels.

### Stage 0: Sibling Folding & Structural Diversity (`src/ast/similarity/tree.rs`)

Before entering the index, `NormalizedTree::eligibility_stats` computes `EligibilityStats` in three linear steps (post-order index build, bottom-up sibling folding, and a diversity DFS over non-folded nodes):

1. **Consecutive identical sibling folding** (adapting token-based repeated-instruction folding from Murakami et al., SCAM 2012, to AST containers): Inside `Block`, `Branch`, `Construct`, `Collection`, and `Call`, consecutive children with identical structural skeletons (ignoring `Symbol` spellings and `Literal` variants, while keeping `Struct` kinds and `Exact` text) are capped at `max_identical_siblings` (default `2`) for size counting. A 12-field constructor (`self.f0 = p0; ... self.f11 = p11;`, 73 raw nodes) folds to 13 effective nodes and is rejected automatically, with zero constructor-name hardcoding.
2. **Structural diversity (`TKS`, distinct non-trivial kinds/tokens):** Counts distinct `AstKind`s (excluding `Block`, `Arg`, `Other`) plus distinct `NodeLabel::Exact` strings across the remaining (non-folded) nodes. Functions must satisfy both `folded_nodes >= min_nodes` and `distinct_kinds >= min_distinct_kinds` (`EligibilityConfig`, defaults `25` and `4`).

**Known boundaries:** Because folding only compares consecutive siblings and `NodeLabel::Exact` text is part of both the skeleton hash and the diversity count, Stage 0 folds uniform sibling repetitions (such as field-assignment constructors, literal-keyed tables like `1 => "a", 2 => "b"`, and repeated statement-level calls like `b.push(x); b.push(y);` or consecutive `Text(...)` views), but does **not** fold:
- **Fluent method chains** (`Cfg::builder().field("a", 1).field("b", 2)....build()`), because chained receiver calls normalize as *nested* `Call` nodes rather than siblings; even a uniform fluent chain never folds and can clear the 25/4 thresholds if it has enough distinct kinds.
- **Dispatch tables keyed by distinct enum variants** (`Status::A => "..."`) or blocks invoking many distinct methods, because each distinct `Exact` string produces a distinct skeleton hash and increments `TKS`.

### Stage 2A: Subtree-Local Merkle Fingerprints (`src/ast/similarity/fingerprint.rs`)

`LabelWeights::extract_unit` runs a pre-order pass followed by a post-order pass to emit both the Stage 2 fingerprint set (`hashes`) and the Stage 3 post-order `CanonicalTree`:

- **Baker's `prev` encoding (STOC 1993 / JCSS 1996):** The pre-order pass assigns each non-`Own` symbol occurrence a sequential index `occ` and back-distance `prev_dist` to its previous occurrence in the same `NameSpace` (`0` on first use).
- **LCA repeat closure:** During bottom-up `SplitMix64` Merkle hashing, open symbol occurrences propagate upward in `open_pool` until reaching the **lowest common ancestor (LCA)** subtree containing both occurrences, where `(local_pos, prev_dist)` is mixed into that subtree's hash once. This enforces 1-to-1 consistent renaming (`x = x + x * x` $\neq$ `d = a + b * c`) and keeps each subtree hash invariant to variables introduced outside that subtree. Worst-case propagation cost is $O(n \cdot h)$ where $h$ is `NormalizedTree` height (`CstNormalizer` caps CST walk depth at `max_depth = 512`, though individual CST nodes can emit multiple structural levels and direct `TreeBuilder` callers are unbounded); in practice this stays near-linear on typical function depths.
- **Leaf emission policy:** Bare `Symbol` and `Literal` leaves are hashed into their enclosing parent expressions and are **not** emitted as standalone entries in `hashes` (so unrelated calls like `validateEmail(x, 1)` and `renderHeader(x, 1)` share zero bare-symbol/literal overlap). `NodeLabel::Exact` leaves (callees, operators, keywords like `return`/`=`) *are* emitted as standalone fingerprints so shared operations contribute directly to Jaccard.

### Stage 2B: Exact Prefix-Filtered Jaccard Join (`src/ast/similarity/index.rs`)

Instead of $O(N^2)$ comparisons, `find_jaccard_candidates` implements the **Bayardo (WWW 2007) / SourcererCC (ICSE 2016)** exact set-similarity join:

1. Compute global document frequency `df[h]` for every 64-bit fingerprint.
2. Order each unit's fingerprints by ascending `(df[h], h)` (rarest first) and sort units by ascending length.
3. Sweep units in ascending length order, probing and indexing only each unit's **rarest prefix** of length $|A| - \lceil \tau |A| \rceil + 1$. By the pigeonhole principle, any pair with $J(A, B) \ge \tau$ must share at least one hash in that prefix; the highest-frequency boilerplate hashes sit at the back and are rarely indexed (at $\tau = 0.50$, roughly the rarest half of each set is indexed).
4. Filter candidates by the length bound $|B| \ge \lceil \tau |A| \rceil$ and verify via early-stopping sorted intersection against the exact overlap requirement $t_{\text{ov}} = \lceil \frac{\tau}{1 + \tau}(|A| + |B|) \rceil$.

Because the final blended score is $S = w_J J + w_T T$ with $T \le 1$, any pair reaching $S \ge \theta$ must satisfy:

$$J \ge \frac{\theta - w_T}{w_J}$$

With default weights ($w_J = 0.30$, $w_T = 0.70$), this exact bound yields $\tau = 0.50$ at $\theta = 0.85$ and $\tau = 0.60$ at $\theta = 0.88$, preserving **zero false negatives** relative to the blended score (for example, a deep single-operator mutation can drop Jaccard to $\approx 0.58$ while TSED remains $\approx 0.97$, giving $S \approx 0.855 \ge 0.85$). `min_candidate_jaccard = 0.35` floors $\tau$ as a recall-for-speed guard; note that this floor engages whenever $\theta < 0.70 + 0.30 \times 0.35 = 0.805$, trading the zero-false-negative guarantee for speed at custom thresholds below `0.805`.

### Stage 3: Weighted Zhang-Shasha TSED Verification (`src/ast/similarity/ted.rs`, `src/ast/similarity/mod.rs`)

Stage 2's deduplicated subtree set can over-score pairs whose shared sub-expressions appear in different control-flow skeletons. Stage 3 runs in parallel across surviving Stage 2B candidates via `rayon` (using `map_init` for per-job reusable `TedScratch` DP buffers):

1. **Work cap & weight-ratio pruning:** If the DP matrix size `left.len() * right.len()` exceeds `max_ted_work` (`160_000`, ~400×400 nodes), Stage 3 skips both weight-ratio pruning and DP and falls back directly to `J`, annotating the diagnostic with `(Merkle subtree estimate)`. For pairs within budget, compute $\text{max\_tsed} = 1 - \frac{|W(A) - W(B)|}{\max(W(A), W(B))}$; if $w_J J + w_T \cdot \text{max\_tsed} < \theta$, reject immediately without running DP.
2. **Weighted Zhang-Shasha (1989) with TSED normalization (Song et al., ACL 2024):**
   $$\text{TSED}(A, B) = \max\left(1 - \frac{\Delta_W(A, B)}{\max(W(A), W(B))},\; 0\right)$$
   `LabelWeights` encodes semantic edit costs:
   - `exact = 2.0`: Swapping a callee, operator, or constructor costs double a structural node.
   - `structural = 1.0`: Structural containers, symbols, and literals.
   - `symbol_relabel = 0.25`: Relabeling two symbols in the **same `NameSpace`** with different global Baker `prev` back-distances. Inserting a Type-3 statement mid-function shifts global `prev` distances for every live variable crossing the insertion; a small same-namespace relabel cost keeps that edit localized in Stage 3 while Stage 2A's subtree-local LCA hashes enforce exact variable co-reference.
3. **Blended score:** $S = w_J J + w_T \cdot \text{TSED}$.

### Rule-Level Partitioning (`src/rules/similarity.rs`)

`SimilarityRule` partitions functions by opaque `LanguageId` into separate production (`duplicate-function`) and test (`overlapping-tests`) corpora. Non-test helper functions inside test modules or test files are excluded from both corpora so test utilities neither pollute production duplication nor collide with test cases. Polyglot repositories never emit cross-language matches, and `overlapping-tests` diagnostics list shared exercised callees to aid consolidation.

---

## 5. Why Program Dependence Graphs (PDGs) Were Deferred

We evaluated adding a PDG verifier and split the trade-off into two designs:

1. **Full compiler PDG (Komondoor-Horwitz / GPLAG):** Requires type checking, overload/trait resolution, and alias/escape analysis (`rustc` MIR, `tsc`, `swiftc` SIL), forcing build toolchains and SDKs into `rqcode` and breaking its zero-build millisecond CLI design.
2. **Lightweight syntactic PDG:** An intra-procedural lexical def-use and control-dependence graph (scored e.g. via Weisfeiler-Lehman graph similarity) layered over AST bindings.

During v0.2 design evaluation across 11 Rust, TypeScript, and Swift repositories (54–941 ms per repo), we deferred even the syntactic PDG for four reasons:

- **Precision was the actual problem:** Stages 0–3 cut the 87-file Swift/SwiftUI benchmark from 518 noisy hits to the 4 genuine copy-pasted mock methods (in 54.5 ms) and caught real Type-1, Type-2, and localized Type-3 clones across all three languages.
- **Reordering trade-off is bounded and understood:** Swapping independent statements leaves the moved statements' own and inner subtree hashes intact (while changing ancestors along the path to the root). Because Stage 3 ordered TSED carries `70%` of the score and moving a subtree of weight $w$ can cost up to $2w$ (delete + insert) plus `0.25` per later symbol whose global `prev` distance shifts, heavy statement permutations can drop below `0.85`, an acceptable trade-off until real missed clones justify a reordering-tolerant verifier.
- **Per-language scoping semantics are subtle:** Accurate lexical def-use requires tracking Rust pattern destructuring/`&mut`, TypeScript `var` hoisting vs. `let`/`const` block scope, and Swift `guard let` (binds into the *outer* scope) and `inout`. `NormalizedTree` intentionally erases several of those distinctions (such as collapsing `var`/`let`/`const` into `Bind` and Swift `guard` into `If`, even though `&mut` survives as `Exact("&mut")`), so a *faithful* syntactic PDG would require enriching `NormalizedTree` and every language dialect, whereas an approximate PDG on the current tree would silently miswire edges in those constructs.
- **Higher-leverage improvements came first in empirical data:** Repositories with many parallel adapters (e.g., dozens of similar engine adapters) suffer from $O(k^2)$ pairwise diagnostic noise, which is best addressed by **clone-cluster deduplication** (connected components), not a PDG.

**Re-evaluation trigger:** If future benchmarks surface genuine missed **reordered Moderate Type-3 clones** (independent statements heavily shuffled or branches inverted in larger functions), an optional Stage 3b syntactic PDG verifier can be added once scoping/control-exit metadata is modeled cleanly in `NormalizedTree`.

---

## 6. Code Map & Tuning Reference

| Responsibility | Module | Key Symbols & Defaults |
| :--- | :--- | :--- |
| Pipeline orchestration & defaults | `src/ast/similarity/mod.rs` | `SimilarityCorpus`, `EligibilityConfig` (`min_nodes = 25`, `min_distinct_kinds = 4`), `SimilarityParams` (`max_identical_siblings = 2`, `jaccard_weight = 0.30`, `tsed_weight = 0.70`, `min_candidate_jaccard = 0.35`, `max_ted_work = 160_000`, `eps = 1e-12`) |
| Normalized AST & Stage 0 folding | `src/ast/similarity/tree.rs` | `NormalizedTree`, `TreeBuilder`, `AstKind`, `NodeLabel`, `NameSpace`, `LitKind`, `EligibilityStats` |
| Stage 2A fingerprints & canonical tree | `src/ast/similarity/fingerprint.rs` | `LabelWeights` (`exact = 2.0`, `structural = 1.0`, `symbol_relabel = 0.25`), `SubtreeHasher`, `CanonicalTree` |
| Stage 2B prefix-filtered index | `src/ast/similarity/index.rs` | `find_jaccard_candidates` |
| Stage 3 weighted Zhang-Shasha TSED | `src/ast/similarity/ted.rs` | `TedScratch::weighted_tsed` |
| Shared CST normalization driver | `src/lang/common.rs` | `CstNormalizer` (`max_depth = 512`), `CstNormalizeDialect`, `extract_body_with_dialect` |
| Language dialects | `src/lang/{rust,typescript,swift}/normalize.rs` | `RustNormalizeDialect` (+ helpers in `rust/macro_tokens.rs`), `TsNormalizeDialect`, `SwiftNormalizeDialect` |
| Diagnostic emission & partitioning | `src/rules/similarity.rs` | `SimilarityRule`, `SimilarityRuleId` (`duplicate-function`, `overlapping-tests`) |
| User configuration (`[similarity]`) | `src/config.rs` | `SimilarityConfig` (`function_threshold = 0.85`, `test_threshold = 0.88`, `min_tokens = 25`, `min_distinct_kinds = 4`, `severity = "error"`) |
| End-to-end tests | `tests/similarity_pipeline.rs`, `tests/swift_normalize.rs` | Polyglot clones, dialect edge cases, enum/primitive pinning, cross-language isolation |

### References

- Baker, B. S. (1993/1996). *A Theory of Parameterized Pattern Matching* (STOC 1993) / *Parameterized Pattern Matching: Algorithms and Applications* (JCSS 1996); *On Finding Duplication and Near-Duplication in Large Software Systems* (WCRE 1995).
- Baxter, I. D., Yahin, A., Moura, L., Sant'Anna, M., & Bier, L. (1998). *Clone Detection Using Abstract Syntax Trees* (ICSM, CloneDR).
- Bayardo, R. J., Ma, Y., & Srikant, R. (2007). *Scaling Up All Pairs Similarity Search* (WWW).
- Falleri, J.-R., Morandat, F., Blanc, X., Martinez, M., & Monperrus, M. (2014). *Fine-Grained and Accurate Source Code Differencing* (ASE, GumTree).
- Feng, Z. et al. (2020). *CodeBERT: A Pre-Trained Model for Programming and Natural Languages* (Findings of EMNLP).
- Jiang, L., Misherghi, G., Su, Z., & Glondu, S. (2007). *DECKARD: Scalable and Accurate Tree-Based Detection of Code Clones* (ICSE).
- Kamiya, T., Kusumoto, S., & Inoue, K. (2002). *CCFinder: A Multilinguistic Token-Based Code Clone Detection System for Large Scale Source Code* (IEEE TSE); Kamiya, T., *CCFinderX* (`TKS` metric).
- Komondoor, R., & Horwitz, S. (2001). *Using Slicing to Identify Duplication in Source Code* (SAS, PDG-DUP).
- Liu, C., Chen, C., Han, J., & Yu, P. S. (2006). *GPLAG: Detection of Software Plagiarism by Program Dependence Graph Analysis* (KDD).
- Mayrand, J., Leblanc, C., & Merlo, E. (1996). *Experiment on the Automatic Detection of Function Clones in a Software System Using Metrics* (ICSM).
- Murakami, H., Hotta, K., Higo, Y., Igaki, H., & Kusumoto, S. (2012). *Folding Repeated Instructions for Improving Token-Based Code Clone Detection* (IEEE SCAM, FRISC).
- Nakagawa, T., Higo, Y., & Kusumoto, S. (2021). *NIL: Large-Scale Detection of Large-Variance Clones* (ESEC/FSE).
- Pawlik, M., & Augsten, N. (2016). *Tree Edit Distance: Robust and Memory-Efficient* (Information Systems, APTED).
- Roy, C. K., & Cordy, J. R. (2007/2008). *A Survey on Software Clone Detection Research* (Queen's Univ. TR 2007-541); *NICAD: Accurate Detection of Near-Miss Intentional Clones Using Flexible Pretty-Printing and Code Normalization* (ICPC 2008).
- Roy, C. K., Cordy, J. R., & Koschke, R. (2009). *Comparison and Evaluation of Code Clone Detection Techniques and Tools: A Qualitative Approach* (Science of Computer Programming).
- Saini, V., Farmahinifarahani, F., Lu, Y., Baldi, P., & Lopes, C. V. (2018). *Oreo: Detection of Clones in the Twilight Zone* (ESEC/FSE).
- Sajnani, H., Saini, V., Svajlenko, J., Roy, C. K., & Lopes, C. V. (2016). *SourcererCC: Scaling Code Clone Detection to Big-Code* (ICSE).
- Schleimer, S., Wilkerson, D. S., & Aiken, A. (2003). *Winnowing: Local Algorithms for Document Fingerprinting* (SIGMOD, MOSS).
- Song, Y., Lothritz, C., Tang, D., Bissyandé, T. F., & Klein, J. (2024). *Revisiting Code Similarity Evaluation with Abstract Syntax Tree Edit Distance* (ACL Short Papers, TSED).
- Svajlenko, J., Islam, J. F., Keivanloo, I., Roy, C. K., & Mia, M. M. (2014). *Towards a Big Data Curated Benchmark of Inter-Project Clones* (ICSME, BigCloneBench).
- Zhang, K., & Shasha, D. (1989). *Simple Fast Algorithms for the Editing Distance Between Trees and Related Problems* (SIAM J. Comput.).
