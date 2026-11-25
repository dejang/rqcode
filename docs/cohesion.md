# Code Cohesion Architecture & Design Rationale

This document specifies **how** `rqcode`'s `cohesion` rule family operates and **why** it is designed this way: what empirical software-engineering literature establishes about reliable cohesion detection, why standalone structural cohesion metrics produce unacceptable noise, how the unified multi-signal analysis pipeline operates across Rust, TypeScript, and Swift, and which approaches are deliberately excluded to preserve high precision.

---

## 1. Problem Statement, Constraints & Literature Foundation

`rqcode` is a zero-build, offline, single-binary polyglot quality gate (`Rust`, `TypeScript`, `Swift`, plus injectable custom `LanguageFrontend`s). Its cohesion engine satisfies five architectural constraints:

1. **Zero compiler/build requirement:** Analyze whole repositories directly from `tree-sitter` CSTs (`ast-grep-core`) without `rustc`, `tsc`, or `swiftc`.
2. **Strict language isolation:** `src/ast/` and `src/rules/` remain language-agnostic (zero concrete language imports or grammar dependencies, enforced by `tests/architecture_guardrails.rs`). Rules operate exclusively over typed IR flags and enums rather than inspecting syntax strings, file extensions, or language names.
3. **Sub-second latency:** Whole-repository cohesion analysis (type binding, field-access resolution, module-graph Tarjan SCC, fan-in/fan-out quantiles, and TF-IDF conceptual confirmation) completes in milliseconds.
4. **Precision over recall on ambiguous idioms:** Pure read-only DTOs/value objects, immutable read-only query types, trivial getters/setters, constructors/destructors, private same-module scratch buffers, multi-collaborator orchestrators/facades, foundational leaf utilities, and idiomatic language module roots (`lib.rs`, `mod.rs`) are explicitly exempted to minimize false positives.
5. **Two-tier enforcement semantics:** Structural encapsulation, boundary integrity, and module-hygiene violations fail the quality gate by default (`error`), while heuristic cohesion smells requiring multi-signal judgment emit advisory diagnostics (`warning`, promotable via `--fail-on-warning` or config).

### Why Standalone Structural Metrics Fail

Three decades of empirical cohesion research converge on a clear negative result: **no single structural cohesion metric is reliable on its own**:

- **Severe inter-metric disagreement:** Ó Cinnéide et al. (ESEM 2012, >3,000 refactorings across 8 OSS systems) found that five standard structural metrics (`TCC`, `LCOM5`, `CC`, `SCOM`, `LSCC`) **disagreed in 55% of cases** on whether a refactoring improved or degraded cohesion.
- **Weak standalone predictive power:** Briand et al. (JSS 2000) found only **2 of 10 (20%)** valid cohesion metrics statistically significant at $\alpha = 0.05$. In systematic reviews, Radjenović et al. (IST 2013, 106 studies) classified `LCOM` as weak for fault prediction, and Jabangwe et al. (EMSE 2015, 99 studies) found only `LCOM2` and `LCOM5` passed a >50% vote-count threshold for reliability. Al Dallal (JSS 2012) measured `LCOM3` and `LCOM4` ROC-AUC at **51.0%–53.6%** (near random), and SonarQube removed `LCOM4` in v4.1 (`SONAR-4853`).
- **Role and special-method bias:** Aniche et al. (SCAM 2016, >400 projects) demonstrated that applying global `LCOM` thresholds flags **>50% of Entities and DTOs** as false positives. Al Dallal (JSS 2012), Chae et al. (2000), and Bavota et al. (2011) showed that constructors and trivial accessors distort structural graphs: glue constructors mask genuinely disjoint responsibilities, while trivial getters/setters inflate disconnected-component counts.
- **Package cohesion alone diverges from developer intent:** Candela, Bavota et al. (ACM TOSEM 2016, 100 OSS systems + 29 developers) found that **83%** of developers organize packages using criteria other than structural cohesion/coupling (**52%** architectural layering, **38%** package cohesion, **28%** low coupling, **21%** co-change).

### What Empirically Works: Multi-Signal Structural + Lexical Confirmation & Architectural Anti-Patterns

`rqcode` adopts only mechanisms with strong empirical validation and developer agreement:

- **Structural + Lexical/Conceptual confirmation (`C3` / `TACO`):** Marcus, Poshyvanyk, & Ferenc (IEEE TSE 2008) showed that conceptual cohesion (`C3`, cosine similarity of identifier-token vectors) is orthogonal to structural metrics (PCA loading **0.996**) and pairing `C3` with structural connectivity boosts fault-prediction $R^2$ by **+50%** (`0.107 -> 0.160`, #1 of 45 metric pairs). Palomba et al. (ICPC 2016 / IEEE TSE 2018, `TACO`) achieved **70% P / 79% R / 74% F1** on Blob/God Classes and **67% P / 72% R / 70% F1** on Feature Envy by combining structural signals with textual/conceptual similarity. Bavota et al. (ICSE 2013) confirmed semantic similarity (`MoJoFM = 0.60`) outperforms purely structural (`0.53`) and dynamic (`0.43`) signals for clustering.
- **Developer-perceived smells:** Palomba et al. (ICSME 2014, 34 developers) established that **God/Blob Class** is recognized by **100%** of original and industrial developers (severity 5/5) and **Feature Envy** by **70%** of original / **50%** of industrial developers (severity 4–5/5), whereas passive visibility-only smells are rarely considered severe.
- **Deterministic heuristics beat ML under real-world skew:** While balanced-dataset ML studies reported >95% F1 (Fontana et al., EMSE 2016), Di Nucci et al. (SANER 2018) and Pecorelli et al. (ICPC 2019) proved that under realistic class imbalance (<1% God Class, ~2.3% Feature Envy), well-calibrated deterministic heuristics still outperform ML classifiers—while remaining fast, deterministic, and explainable.
- **Architectural structural knots:** Mo et al. (IEEE TSE 2021, 19 industrial and OSS systems), Oyetoyan et al. (JSS 2013; SANER 2015), and Melton & Tempero (EMSE 2007) proved that dependency cycles (`Clique`) and high-fan-in + high-fan-out hubs (`Crossing`) are among the strongest architectural drivers of defects and churn.

---

## 2. Conceptual Model & 11-Rule Taxonomy

Cohesion measures whether **state, behavior, and module boundaries align around single, well-encapsulated concepts**. `rqcode` models cohesion across **11 diagnostic IDs** organized into two enforcement tiers:

### Tier 1: Structural Encapsulation & Boundary Integrity (`severity = "error"`, fails gate by default)

| Diagnostic ID | Scope | Signal | Key Precision Guards & Exemptions |
| :--- | :--- | :--- | :--- |
| `scattered-concept` | Type | Stateful type has behavior scattered across relevant external free functions (`>= min_external_functions` across `>= min_external_modules`, or anemic type with `has_external_mutation` where total relevant external functions `>= min_external_functions` outnumber inherent behavioral methods) | Exempts pure read-only DTOs/value objects (`has_implicit_field_readers \|\| is_immutable_record`, all declared fields externally visible, `0` inherent behavioral methods, `0` relevant external mutators); ignores multi-type coordinators (via parameters or inferred locals) unless mutating |
| `external-field-mutation` | Function / Method | Non-test production function *or method* outside a stateful type's defining module directly mutates a declared field of that type | Uses unified binder `resolved_field_accesses` (parameters, typed expressions, inferred factory locals); filters to declared fields of `ty`; excludes owner/self accesses (`!access.receiver_is_self && func.resolved_owner != Some(ty.id)`), same-module writes, and test code |
| `split-impl` | Type | Inherent methods for a stateful type are defined outside the type's defining module | Excludes trait/interface/protocol implementations (`is_interface_impl == true`) |
| `scattered-tests` | Type | Stateful type has `0` unit tests in its defining module while unit tests exercising it are scattered across `>= 2` other modules (or `>= 1` when external helpers exist) | Never counts `FileRole::IntegrationTest` files; skips once at least one unit test is co-located |
| `cyclic-module-dependency` | Module Graph | Strongly connected component of size `>= 2` among non-test, non-ancestor modules within the same `LanguageId` | Excludes parent/child ancestor relationships; merges multi-file `ModulePath`s and tracks both import and resolved re-export edges with submodule-first resolution |
| `leaky-module-boundary` | Type | Stateful type exposes `>= 2` mutable-reference fields across a module boundary, or has `>= 1` inherent method whose explicitly restricted (`Visibility::Internal && explicit_visibility`) fields are accessed (including destructuring) or initialized outside its module | Uses unified `resolved_field_accesses` and `resolved_field_inits`; excludes owner/self accesses and owner field initializers (`func.resolved_owner != Some(ty.id)`), unannotated language-default internal fields, test code, and non-stateful types |
| `stateless-impl-method` | Method | Inherent method neither takes an instance receiver (`self`/`this`) nor returns an instance of its owner type | Recognizes wrapped owner returns (`Option<Self>`, `Result<Self, E>`, `Box<Self>`, `Arc<Self>`, etc.); excludes interface impls and test code |
| `no-barrel-module` | File | Non-test production file acts as a barrel module (`BarrelKind::DirectoryIndex` re-exporting index file or `BarrelKind::PureReexport` re-export-only module) | Non-re-exporting `index` entrypoints (`BarrelKind::None`), `.d.ts` ambient files, test files, and module-tree languages (`Rust`, `Swift`) are exempt; optional `allow_library_root_barrel` permits `FileRole::LibraryRoot` entrypoints |

### Tier 2: High-Precision Multi-Signal Cohesion Smells (`advisory_severity = "warning"`)

| Diagnostic ID | Scope | Signal | Key Precision Guards & Exemptions |
| :--- | :--- | :--- | :--- |
| `low-cohesion-type` | Type | Stateful type splits into `>= 2` substantial structural components (`TCC < max_tcc` over field-using methods) that remain disconnected after TF-IDF lexical bridging (`cos < max_conceptual_similarity`) | Excludes constructors, lifecycle destructors, interface impls, and trivial getters/setters (while propagating `F*(m)` through excluded accessors, property-syntax getters, and property observers); requires size gates (`>= 4` fields, `>= 5` eligible methods, `>= 4` field-using methods) and `>= 2` substantial components (each `>= min_component_methods` methods touching `>= 1` field) |
| `feature-envy` | Method | Inherent instance method accesses `>= min_foreign_accesses` distinct declared fields/methods of one dominant foreign behavioral type $T^*$ (`FDP <= max_foreign_providers`, `LAA < max_laa`) AND has higher TF-IDF conceptual affinity with $T^*$ than with its own type | Counts only declared fields and declared inherent instance methods of $T^*$; exempts pure exposed-field DTOs (0 instance methods), read-only query/accessor value objects (0 non-trivial behavioral methods across both `Struct` and `Class`), private same-module helper/scratch types, interface impls, lifecycle/factory methods, and `(func, target)` pairs already flagged by `external-field-mutation` |
| `architectural-crossing` | Module Graph | Non-exempt production module simultaneously sits at or above both effective fan-in and effective fan-out thresholds (`>= max(min_fan_in, Q_p)` and `>= max(min_fan_out, Q_p)`) | Partition gate (`>= min_graph_modules`, default `12`) and quantile floor ($p = \lfloor q \cdot (n - 1) \rfloor$, default $q = 0.90$) computed strictly over non-exempt candidate modules; skips `LibraryRoot`, `BinaryEntrypoint`, all-barrel modules, pure empty/facade modules, foundational leaf utilities (low fan-out), and top-level orchestrators (low fan-in) |

---

## 3. Architecture & Unified Analysis Pipeline

All 11 rules execute inside a single `CohesionRule` pass over the bound, language-agnostic `Codebase`:

```text
  tree-sitter CST (per source file)
         │
         ▼
  Stage 1: Language Frontends (src/lang/{rust,typescript,swift}/)
    Extract FileAst: types (has_implicit_field_readers, is_immutable_record),
    fields (visibility, explicit_visibility), barrel_kind, imports,
    exports (origin + span), functions (is_lifecycle, body_tree,
    field_accesses, calls, field_inits, referenced_idents)
         │
         ▼
  Stage 2: Language-Agnostic Binder (src/ast/codebase/bind.rs)
    Resolve owners, return types, params, local bindings (explicit, bounded
    4-pass factory inference & call-site helper param inference), type-name
    receiver fallbacks, resolved_field_accesses, resolved_call_receivers,
    resolved_field_inits
         │
         ▼
  Stage 3: Shared Cohesion Indices (src/rules/cohesion/)
    - ModuleGraph (partitioned by LanguageId, multi-file ModulePath nodes,
      non-ancestor import + resolved re-export edges)
    - LexicalCorpus (codebase-wide identifier tokenization + smoothed IDF weights)
         │
         ▼
  Stage 4: Focused Cohesion Analyzers (src/rules/cohesion/)
    1. mod.rs           -> scattered-concept, external-field-mutation,
                           split-impl, scattered-tests
    2. graph.rs         -> ModuleGraph, TarjanScc, cyclic-module-dependency,
                           architectural-crossing
    3. boundaries.rs    -> leaky-module-boundary, stateless-impl-method,
                           no-barrel-module
    4. type_cohesion.rs -> low-cohesion-type, shared is_trivial_accessor predicate
    5. feature_envy.rs  -> feature-envy
    6. lexical.rs       -> identifier splitting, stopword filtering, TF-IDF cosine
```

### Shared IR Contract (`src/ast/`)

To maintain strict language isolation, frontends distill cohesion-relevant facts into typed IR fields:

- **`BarrelKind` on `FileAst`** (`None | DirectoryIndex | PureReexport`):
  - `FileAst::is_directory_barrel()` (`matches!(barrel_kind, BarrelKind::DirectoryIndex)`) identifies index-file barrel candidates for `PublicSurface` (`src/ast/codebase/surface_build.rs`), which constructs a `BarrelBoundary` **only** when the file contains at least one project-internal re-export (preventing external-only barrels from creating empty boundaries that hide sibling modules).
  - `FileAst::is_barrel()` (`barrel_kind != BarrelKind::None`) drives `no-barrel-module`, `architectural-crossing` exemptions, and (alongside `file.types.is_empty() && file.functions.is_empty()`) external-boundary propagation in `src/rules/test_coupling/mocking.rs`.
- **`ExportOrigin` and `ExportDecl::span: Span`:** Every `ExportDecl` records its source `Span` and an explicit `origin: ExportOrigin`:
  - `ExportOrigin::Local`: local declaration or `export { localIdent }` defined in this file.
  - `ExportOrigin::ImportedBinding`: `export { X }` / `export default X` re-exporting an identifier matching a non-test `ImportDecl::local_name()` in the same file (whether project-internal or external).
  - `ExportOrigin::DirectInternal`: direct re-export (`export ... from "..."`, `pub use ...`, `@_exported import ...`) whose target path was resolved by the frontend (`source_module: Some(ModulePath)`); unmatched non-project paths (e.g., Swift system framework `@_exported import Foundation`) harmlessly fail to match any project node in `ModuleGraph`/`PublicSurface`.
  - `ExportOrigin::DirectExternal`: direct re-export (`export ... from "..."`) targeting an unresolved external package or path alias (`source_module: None`).
  - `ExportDecl::is_reexport()` returns `!matches!(self.origin, ExportOrigin::Local)`.
  - `TypeResolver` and `PublicSurface` ignore `ExportOrigin::DirectExternal` entirely and resolve `ImportedBinding` through `file.imports`, keeping external re-exports cleanly isolated from internal symbol/surface resolution while giving `no-barrel-module` exact spans for every re-export form.
- **`TypeDecl::is_immutable_record: bool`:** Orthogonal to `has_implicit_field_readers` (which specifically means compiler-synthesized code reads every field and is consumed by `test-only-production-field`). Frontends set `is_immutable_record = true` for nominal record types without derive/conformance DTO idioms whose declared instance fields are all immutable/read-only (populated by the TypeScript frontend for classes with $\ge 1$ non-`static` declared field—including constructor parameter properties—where every non-`static` declared field is `readonly`; Rust and Swift rely on explicit derive/conformance signals via `has_implicit_field_readers`), enabling `scattered-concept`'s pure DTO exemption without disabling `test-only-production-field`.
- **`FunctionDecl::is_lifecycle: bool`:** Marks language-defined constructors and destructors (TypeScript `constructor`, Swift `init` and `deinit`) so `low-cohesion-type`, `feature-envy`, and the DTO exemption exclude lifecycle hooks without matching language-specific names in `src/rules/`.
- **Binder receiver scope (`bind.rs`):** Stage 2 resolves types for parameters, explicitly typed locals, bounded fixed-point (up to 4 iterations) factory-inferred locals and call-site-inferred helper parameters, and type-name call receiver fallbacks. Chained member receivers (`self.<field>.member` / `this.<field>.member`) are intentionally left unbound by Stage 2, keeping zero-build resolution sound and naturally excluding transparent composition/wrapper delegation (`self.inner.foo()`) from `feature-envy`.

---

## 4. Detailed Rule Specifications

### 4.1 Unified Field-Access Resolution (`external-field-mutation`, `leaky-module-boundary`, `scattered-concept`)

All field-access checks consume the binder's canonical `func.resolved_field_accesses` vector zipped with `func.decl.field_accesses`, ensuring consistent coverage across parameters, typed expressions, struct-pattern destructuring, and factory-inferred locals:

- **`external-field-mutation`:** Iterates over all non-test production functions **and methods** (`!func.decl.is_test_code() && !file.is_test_file()`). For each `(access, Some(type_id))` pair where:
  1. `access.is_write` is `true`,
  2. `!access.receiver_is_self && func.resolved_owner != Some(type_id)` (excluding methods/extensions/trait impls mutating their own type, which belong to `split-impl` or valid protocol conformances),
  3. `ty` is stateful, non-test, and `func.module_path != ty.module_path`, and
  4. `access.field_name` matches a declared field of `ty` (guarding against deref coercions or shadowed locals),
  the mutated field name is recorded under `(func.id, type_id)`. Each `(func.id, type_id)` pair with $\ge 1$ mutated field produces one diagnostic on `func.decl.span` (displaying `func.display_name()`, supporting both free functions and methods on foreign types) with a related location on `ty.decl.span`.
- **`leaky-module-boundary`:** `BoundaryIndex::build` collects external accesses and initializers uniformly while respecting type ownership and explicit visibility:
  - For `resolved_field_accesses` zipped with `decl.field_accesses`: skip accesses where `access.receiver_is_self || func.resolved_owner == Some(type_id)`.
  - For `resolved_field_inits`: skip initializers where `func.resolved_owner == Some(init.type_id)` (so cross-module trait impls such as `impl From<X> for Order { Self { ... } }` and split constructors are not double-flagged alongside `split-impl`).
  - Restricted internal fields require **both** `field.visibility == Visibility::Internal && field.explicit_visibility` (targeting explicit crate/module-internal fields such as Rust `pub(crate)`/`pub(super)` and TypeScript `protected`, while avoiding false positives on unannotated Swift properties whose language default is `internal`).
  - Direct reads, writes, struct-pattern destructuring, and struct-literal initializations of restricted fields outside `ty.module_path` are all captured uniformly; callers are expected to use the type's inherent methods or owned conversion helpers rather than destructuring or mutating internal fields across module boundaries.
- **`scattered-concept`:** `Codebase::external_free_functions_for` considers non-test external free functions (`func.decl.is_free_function() && func.module_path != ty.module_path`) that either take `ty` as a resolved parameter **or** access a declared field of `ty` via `resolved_field_accesses` (e.g., through an inferred factory local). Field reads and writes are collected by zipping `func.decl.field_accesses` with `func.resolved_field_accesses` (filtered to declared fields of `ty`). Let `other_project_types` be the number of distinct project types other than `ty.id` appearing across `func.resolved_params` and `func.resolved_field_accesses` (so multi-type coordinators are identified uniformly whether they accept parameters or load types into locals). A usage is retained in `relevant_externals` iff:
  - it mutates fields (`!direct_field_writes.is_empty()`), OR
  - `takes_mut_ref && other_project_types == 0`, OR
  - `!direct_field_reads.is_empty() && other_project_types == 0`.
  Let `has_external_mutation` be whether any retained usage in `relevant_externals` has `!direct_field_writes.is_empty() || takes_mut_ref`, `external_modules` be the distinct `ModulePath`s of `relevant_externals`, and `inherent_behavioral_methods` be the count of non-test, non-lifecycle, non-factory inherent instance methods on `ty` (excluding `is_test_code()`, `is_lifecycle`, `!has_self_receiver()`, and `returns_self_type()` / `resolved_return_type == Some(ty.id)`). Unless exempted as a pure read-only DTO (Section 4.2), `scattered-concept` fires iff:
  1. **Module scatter:** `relevant_externals.len() >= min_external_functions` AND `external_modules.len() >= min_external_modules`, OR
  2. **Anemic type with external mutation:** `has_external_mutation` AND `relevant_externals.len() >= min_external_functions` AND `relevant_externals.len() > inherent_behavioral_methods` (reporting `inherent_behavioral_methods` consistently in the diagnostic message).

### 4.2 Pure DTO / Value-Object Exemption (`scattered-concept`)

To eliminate Aniche et al.'s (SCAM 2016) >50% Entity/DTO false-positive rate without weakening detection of anemic domain objects, `scattered-concept` defines a **pure read-only DTO** as a stateful type satisfying **all four** conditions:

1. `ty.decl.has_implicit_field_readers || ty.decl.is_immutable_record` (Rust `#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]`, Swift `Codable`/`Encodable`/`Decodable`/`Equatable`/`Hashable`, or TypeScript classes with $\ge 1$ non-`static` declared field where every non-`static` declared field is `readonly`; TypeScript `interface`/`type` DTOs are non-stateful and already exempt),
2. All declared fields are externally visible (`ty.decl.exposed_field_count() == ty.decl.fields.len()`, where `Visibility::is_externally_visible` covers `Public | Internal`),
3. `inherent_behavioral_methods == 0` (explicit `new`/`constructor`/`init` factories and test helpers never disqualify a DTO), and
4. `!has_external_mutation` (zero retained relevant external free functions mutate `ty`'s fields or take `&mut ty` with `other_project_types == 0`).

When all four hold, `scattered-concept` is suppressed.

### 4.3 Multi-File, Language-Partitioned `ModuleGraph` (`cyclic-module-dependency`, `architectural-crossing`)

`ModuleGraph` models logical modules partitioned strictly by `LanguageId`:

- **Nodes:** Each unique `(LanguageId, ModulePath)` appearing among non-test production files forms one node `ModuleNodeId`, owning a `Vec<FileId>` of all non-test production files belonging to that module (sorted deterministically by `relative_path`). Multiple files sharing a `(LanguageId, ModulePath)` merge into one node.
- **Submodule-first target resolution:** For both `ImportDecl` (`kind == ImportKind::Named && !symbol.is_empty()`) and `ExportDecl` (`origin == ExportOrigin::DirectInternal && !is_wildcard && !symbol.is_empty()`), target lookup first checks `(language, target_module + symbol)` before falling back to `(language, target_module)`. This matches import resolution semantics so re-exports such as Rust `pub use crate::a::b;` resolve directly to `crate::a::b` without adding a spurious edge to `crate::a`.
- **Edges:** For every non-test file in node `u`:
  - Each non-test `ImportDecl` resolving to a target node `v` in the same `LanguageId`, AND
  - Each `ExportDecl` with `origin == ExportOrigin::DirectInternal` (`source_module: Some(target)`) resolving to a target node `v` in the same `LanguageId`
  adds a directed edge `(u -> v, span)` provided `u != v` and neither `module_path` is an ancestor of the other (`!u.is_same_or_child_of(v) && !v.is_same_or_child_of(u)`). Parallel edges between the same `(u, v)` pair are deduplicated (retaining the first deterministic span).
- **Cycles (`cyclic-module-dependency`):** Runs Tarjan's SCC algorithm over the module graph and emits one diagnostic per SCC of size $\ge 2$, sorted deterministically by primary file `relative_path`.

### 4.4 Barrel Module Policy (`no-barrel-module`)

In module systems with direct file-level imports (such as TypeScript/ESM), barrel modules create artificial fan-in/fan-out hubs, obscure true module coupling, trigger circular-dependency initialization failures, and degrade tree-shaking and test startup.

- **Detection:** When `cfg.boundaries.barrels.flag_barrel_modules` is `true`, for every non-test production file:
  - Skip if `file.barrel_kind == BarrelKind::None`.
  - Skip if `cfg.boundaries.barrels.allow_library_root_barrel && file.role == FileRole::LibraryRoot`.
  - Skip if the file has no `ExportDecl` with `is_reexport() == true`.
  - Otherwise emit `no-barrel-module` at `cfg.severity` (`Severity::Error` by default, failing the gate), anchored at the first `ExportDecl` with `is_reexport() == true`, attaching up to `max_barrel_related` (default `3`) distinct additional re-export spans (deduplicated by span so multi-specifier statements such as `export { A, B, C } from './x'` do not repeat the anchor) as related locations, and suggesting direct imports from the defining source modules.
- **Frontend classification:**
  - **TypeScript:**
    - Ambient declaration files (`relative_path` file name ending with `.d.ts`) always receive `BarrelKind::None`.
    - `BarrelKind::DirectoryIndex`: filename stem is `"index"` (non-`.d.ts`) and the file contains at least one `ExportDecl` with `is_reexport() == true`—covering direct `export ... from "..."` (`DirectInternal` or `DirectExternal`) and import-then-export (`ImportedBinding`). An `index.ts` that defines its own code and contains **zero** re-exports is classified as `BarrelKind::None`.
    - `BarrelKind::PureReexport`: non-`index` production `.ts` file (non-`.d.ts`) inspected directly at the CST root level whose top-level non-comment statements consist **exclusively** of `import_statement` and re-export `export_statement` nodes, with at least one `is_reexport() == true` declaration and zero local declarations or executable statements (no classes, interfaces, enums, `type_alias_declaration`, functions, variables, or expression statements).
    - `FileRole::LibraryRoot` applies to repository-root `index.ts` and `src/index.ts`; setting `allow_library_root_barrel = true` exempts those root entrypoints while still rejecting all nested directory and pure re-export barrels.
  - **Rust & Swift:** Emit `BarrelKind::None`. Rust requires `lib.rs` / `mod.rs` to form the crate module tree (`mod foo;`), and Swift compiles all files in a target into a single module without file-level import barrels.
- **Interaction with `test-coupling` rules:**
  - `PublicSurface` (`src/ast/codebase/surface_build.rs`) builds a `BarrelBoundary` only for `BarrelKind::DirectoryIndex` files that contain at least one resolved project-internal re-export (`DirectInternal`, or `ImportedBinding` resolving through a non-test `ImportDecl` with `resolved_module.is_some()`—excluding unresolved path aliases such as `@/...` as well as external packages), keeping `test-bypasses-public-surface` accurate whenever barrels are suppressed or permitted via `allow_library_root_barrel = true`.
  - `MockingChecker` (`src/rules/test_coupling/mocking.rs`) propagates external-boundary status through files satisfying `file.is_barrel() || (file.types.is_empty() && file.functions.is_empty())` across both direct re-exports (`source_module`) and `ImportedBinding` re-exports (resolved through matching non-test `ImportDecl`s), so both TypeScript barrels and pure Rust `mod.rs` re-export facades remain covered.

### 4.5 Shared Lexical Cohesion Engine (`src/rules/cohesion/lexical.rs`)

Both `low-cohesion-type` and `feature-envy` share a lightweight, deterministic, zero-dependency TF-IDF cosine engine built once per `Codebase` pass. Its parameters (`LexicalParams`: `min_token_len = 3` and stopword list) are injectable via the Rust API with documented defaults (not exposed as user-facing `rqcode.toml` knobs):

1. **Identifier tokenization:** Split camelCase, PascalCase, and snake_case tokens (`calculateOrderTax` / `calculate_order_tax` $\to$ `["calculate", "order", "tax"]`; handles acronym boundaries such as `parseHTTPHeader` $\to$ `["parse", "http", "header"]`). Lowercase all tokens, drop tokens shorter than `min_token_len` (default `3`), and drop configured generic programming stopwords (default: `self`, `this`, `get`, `set`, `new`, `from`, `into`, `with`, `val`, `value`, `item`, `data`, `list`, `vec`, `map`, `str`, `string`, `int`, `bool`, `void`, `async`, `await`, `return`, `result`, `option`, `error`, `err`, `true`, `false`, `null`, `undefined`, `len`, `push`, `pop`, `iter`, `clone`, `default`, `index`, `count`, `size`, `key`, `name`, `type`, `kind`, `span`, `node`).
2. **Deterministic, symmetric document token streams:**
   - **Method document $T(m)$:** Concatenate tokenized strings from five structured IR sources in fixed order: (1) method `decl.name`, (2) parameter names and parameter type names (`decl.params`), (3) all resolved field-access names (`field_name` for every `FieldAccess` with `resolved_field_accesses[i].is_some()`, treating `self` and foreign fields symmetrically), (4) called callee names (`decl.calls`), and (5) body `decl.referenced_idents`. Because `BodyAccumulator` deduplicates `referenced_idents` per function, body-only identifiers contribute $\text{tf} = 1$, while identifiers emphasized across both structured signatures/accesses and the body receive a modest sublinear TF boost.
   - **Type document $T(\tau)$:** Concatenate tokenized strings from `ty.decl.name`, all declared field names (`ty.decl.fields`), and the pooled method documents of all non-test inherent methods of $\tau$ (excluding method $m$ when computing $\vec{v}_{\text{own}}$ for `feature-envy` so $m$ never correlates with itself).
   - **Cluster document $T(C_a)$:** Concatenate the pooled method documents $T(m)$ for all $m \in C_a$ plus the tokenized declared field names in $\bigcup_{m \in C_a} F^*(m)$.
3. **Codebase-wide smoothed IDF:** Across all $N$ non-test production functions in the `Codebase`, compute document frequency $\text{df}(t)$ (number of distinct functions whose document $T(m)$ contains token $t$) and weight:
   $$\text{idf}(t) = \ln\!\left(\frac{N + 1}{\text{df}(t) + 1}\right) + 1$$
   Codebase-wide IDF ensures domain terms shared across two methods of the same concept (e.g., `invoice`, `ledger`, `smtp`) retain high weight, whereas codebase-ubiquitous verbs (`build`, `check`, `parse`) are down-weighted.
4. **Sublinear TF-IDF cosine similarity:** For any token bag $D$, count occurrences $\text{tf}(t)$ of each token $t \in D$ and build sparse vector $w(t) = (1 + \ln \text{tf}(t)) \cdot \text{idf}(t)$. Cosine similarity between two sparse vectors $\vec{u}, \vec{v}$ is:
   $$\cos(\vec{u}, \vec{v}) = \frac{\vec{u} \cdot \vec{v}}{\lVert \vec{u} \rVert_2 \cdot \lVert \vec{v} \rVert_2} \in [0, 1]$$
   (defined as `0.0` if either vector has zero norm).

### 4.6 Type Cohesion (`low-cohesion-type`)

Detects stateful types (`Struct` / `Class`) that bundle two or more structurally **and** conceptually unrelated responsibilities.

1. **Shared trivial accessor predicate (`is_trivial_accessor`):**
   A non-test inherent instance method $m$ of `ty` is a **trivial accessor** iff **all** of the following hold (purely structural, zero name heuristics):
   - `m.decl.calls.is_empty()` and `m.decl.body_tree` contains no `AstKind::Call` (so macro invocations such as Rust `format!` and `.clone()` calls are retained as non-trivial),
   - `m.decl.body_tree.len() <= max_accessor_nodes` (default `10`),
   - `m.decl.body_tree` contains **none** of `AstKind::{If, Branch, Loop, Try, Closure, Binary, Index, Construct, Bind, Call}`,
   - `m.decl.field_accesses.len() == 1`, and that single `FieldAccess` resolves to `Some(ty.id)` with `receiver_is_self == true` and targets a declared field of `ty` (unambiguously excluding multi-access tuples, chained reads, and foreign accesses), and
   - Either:
     - **Pure getter (including shared getters, Swift read-only computed properties, and Rust `&mut self.x` reference accessors):** `m.decl.params.is_empty()`, `m.decl.body_tree` contains no `AstKind::Assign`, and the single field access has `access.is_read == true`, OR
     - **Pure setter:** `m.decl.params.len() == 1`, the single field access has `access.is_write && !access.is_read` (simple assignment `self.x = v`, excluding compound `+=` which sets `is_read && is_write`), and `m.decl.body_tree` contains at most 1 `AstKind::Assign`.
2. **Eligible method filtering ($M_{\text{elig}}$):**
   Collect all non-test inherent instance methods (`is_inherent_method() && has_self_receiver() && !is_test_code() && !file.is_test_file()`) of `ty`, excluding:
   - Lifecycle hooks and factories: `func.decl.is_lifecycle`, `func.decl.returns_self_type()`, or `func.resolved_return_type == Some(ty.id)`.
   - Trivial accessors satisfying `is_trivial_accessor(func, ty, max_accessor_nodes)`.
   *(Note: `is_interface_impl` reflects each frontend's syntax—explicit trait impl blocks in Rust and protocol extensions in Swift set `is_interface_impl: true`, while inline class methods in TypeScript and inline Swift conformances remain inherent methods.)*
3. **Direct fields $F(m)$, calls $C(m)$, and transitive closure $F^*(m)$:**
   Let $M_{\text{inst}}$ be **all** non-test inherent instance methods of `ty` (including excluded trivial accessors, property-syntax getters, computed properties, and Swift `didSet`/`willSet` property observers). For each $m \in M_{\text{inst}}$:
   - **Property-observer field seed:** If `m.decl.name` matches a declared field name of `ty` (e.g., a Swift `didSet`/`willSet` observer on stored property `x`), seed $F(m)$ with `m.decl.name` so the observer remains connected to its own property even when its body does not mention `self.x`.
   - For each `(access, Some(tid))` where `tid == ty.id && access.receiver_is_self`:
     - If `access.field_name` is a declared field of `ty`: insert `access.field_name` into $F(m)$. Additionally, if `access.is_write`, insert all methods in $M_{\text{inst}}$ whose name equals `access.field_name` (e.g., Swift `didSet`/`willSet` observers) into $C(m)$.
     - Else insert all methods in $M_{\text{inst}}$ whose name equals `access.field_name` (property-syntax getters/setters or computed properties such as TypeScript `get balance()`/`set balance()` or Swift `var balance: Int`) into $C(m)$.
   - For each `call` with `call.receiver == Some(CallReceiver::SelfInstance)`: insert all methods in $M_{\text{inst}}$ whose name equals `call.callee` into $C(m)$ (covering overloaded method names uniformly).
   - Compute the transitive field-access closure $F^*(m)$ over $C(m)$ across all $M_{\text{inst}}$ (Bieman & Ott 1994). Propagating $F^*$ through both call-syntax (`self.balance()`) and property-syntax (`self.balance`) accessors keeps methods sharing the same underlying backing field connected while excluding trivial accessors from vertex counts and `TCC` denominators.
4. **Size eligibility gate:**
   Let $M_{\text{field}} = \{m \in M_{\text{elig}} : F^*(m) \neq \emptyset\}$. Skip `ty` unless:
   - `ty.decl.fields.len() >= min_fields` (default `4`),
   - `|M_elig| >= min_methods` (default `5`), and
   - `|M_field| >= min_field_using_methods` (default `4`).
5. **Structural graph, `TCC`, and substantial components:**
   - Compute Tight Class Cohesion over field-using eligible method pairs (so stateless helper methods on `ty` do not artificially dilute `TCC`):
     $$\text{TCC} = \frac{|\{\{m_i, m_j\} \subset M_{\text{field}} : F^*(m_i) \cap F^*(m_j) \neq \emptyset\}|}{\binom{|M_{\text{field}}|}{2}}$$
   - Build the undirected structural interaction graph $G = (M_{\text{elig}}, E)$ with edge $(m_i, m_j) \in E$ iff $F^*(m_i) \cap F^*(m_j) \neq \emptyset$ or $m_i$ calls $m_j$ or $m_j$ calls $m_i$ (Hitz & Montazeri `LCOM4`).
   - Extract connected components of $G$ and retain **substantial components**: components containing $\ge \text{min\_component\_methods}$ (default `2`) methods in $M_{\text{elig}}$ whose union of transitive declared fields $\bigcup_{m \in C} F^*(m)$ is non-empty ($\ge 1$ field). Single orphan helper methods do not create spurious splits.
   - Structural trigger: `TCC < max_tcc` (default `0.50`) AND `substantial_components.len() >= 2`.
     *(For a balanced 2-way split of $2n$ field-using methods into two disjoint halves of size $n$, $\text{TCC} = \frac{n - 1}{2n - 1} < 0.50$—e.g., a 3+3 split has $\text{TCC} = 6/15 = 0.40$—and balanced $k$-way splits ($k \ge 2$) always satisfy $\text{TCC} < 0.50$, whereas a heavily dominant cohesive core paired with small auxiliary clusters—e.g., 8+2 at $\text{TCC} = 29/45 = 0.644$ or 20+2+2 at $\text{TCC} = 192/276 = 0.696$—is intentionally not flagged.)*
6. **Conceptual confirmation (TF-IDF cluster bridging):**
   - For each substantial structural component $C_a$, build its pooled cluster TF-IDF vector $\vec{v}_{C_a}$ from $T(C_a)$ (Section 4.5).
   - Add a conceptual bridge edge between $C_a$ and $C_b$ whenever $\cos(\vec{v}_{C_a}, \vec{v}_{C_b}) \ge \text{max\_conceptual\_similarity}$ (default `0.20`).
   - Compute connected components of the bridged component graph (single-linkage transitivity intentionally biases toward precision so related multi-step phases merge rather than false-positive).
   - **Emit `low-cohesion-type` (`advisory_severity`, default `warning`)** iff **$\ge 2$ conceptually disjoint substantial clusters** remain.
   - The diagnostic reports `TCC`, cluster count, and attaches each disjoint cluster's representative methods and field sets as related locations with a concrete split suggestion.

### 4.7 Method-Level Feature Envy (`feature-envy`)

Detects an inherent instance method placed on the wrong stateful type—operating heavily on one foreign behavioral type bound via parameters or local variables rather than on its own state.

1. **Eligibility & exemptions:**
   - Applies to non-test inherent instance methods (`is_inherent_method() && has_self_receiver() && !is_test_code() && !file.is_test_file()`) on stateful types (`Struct` / `Class`).
   - Skips interface/trait/protocol implementations (`is_interface_impl`), lifecycle hooks (`is_lifecycle`), and constructors/factories (`returns_self_type()` or `resolved_return_type == Some(owner_id)`).
2. **Member-validated access accounting:**
   For any stateful type $\tau$, let $\text{Fields}(\tau)$ be its declared field names and $\text{Methods}(\tau)$ be the names of its non-test inherent instance methods.
   - **Self accesses ($A_{\text{self}}$):** count of distinct member names $x$ accessed on `self`/`this` via either:
     - `FieldAccess` with `resolved_field_accesses[i] == Some(owner_id) && access.receiver_is_self` where $x \in \text{Fields}(T_{\text{own}}) \cup \text{Methods}(T_{\text{own}})$ (covering both stored fields and property-syntax getters/computed properties), OR
     - `CallSite` with `call.receiver == Some(CallReceiver::SelfInstance)` where $x \in \text{Methods}(T_{\text{own}})$.
   - **Foreign accesses ($A_{\text{foreign}}(T)$):** for each foreign project-defined stateful type $T \neq T_{\text{own}}$, count distinct member names $x$ accessed on $T$ via either:
     - `FieldAccess` with `resolved_field_accesses[i] == Some(T)` where $x \in \text{Fields}(T) \cup \text{Methods}(T)$, OR
     - `CallSite` with `resolved_call_receivers[i] == Some(T) && call.kind == CallKind::Method` where $x \in \text{Methods}(T)$.
     *(Requiring $x \in \text{Methods}(T)$ filters out unrelated container/wrapper methods such as `Vec::push`, `Vec::extend`, `Option::is_some`, or `Array::map` on unwrapped container types `Vec<T>` / `T[]`, except in the rare case where `T` itself declares an inherent method of the same name.)*
   - Let $\text{FDP}$ (Foreign Data Providers) be the number of distinct foreign types $T$ with $A_{\text{foreign}}(T) \ge 1$, $A_{\text{foreign\_total}} = \sum_T A_{\text{foreign}}(T)$, and $T^* = \arg\max_T A_{\text{foreign}}(T)$ (with deterministic `TypeId` tie-breaking).
3. **Target & boundary exemptions:**
   - **Non-behavioral target exemption (mutually exclusive cases):** Let $I(T^*)$ be all non-test inherent instance methods of $T^*$.
     - **Case A ($I(T^*)$ is empty — 0 instance methods):** Exempt $T^*$ iff all declared fields are externally visible (`exposed_field_count() == fields.len()`). *(A 0-method type with exposed fields is a passive DTO/record; a 0-method type with private fields is not exempt.)*
     - **Case B ($I(T^*)$ is non-empty — $\ge 1$ instance method):** Exempt $T^*$ iff **every** method $f \in I(T^*)$ is non-behavioral—meaning $f$ is a lifecycle hook (`is_lifecycle`), factory (`returns_self_type()` / `resolved_return_type == Some(T^*)`), trivial accessor (`is_trivial_accessor`, Section 4.6), OR a **0-parameter read-only query method** satisfying all of:
       1. `f.decl.params.is_empty()` and `!f.decl.coupling.has_ambient_io`,
       2. Zero direct or transitive (via `self` calls in $I(T^*)$) field writes (`!is_write`), `AstKind::Assign`, or `AstKind::Loop`, and
       3. **Type-kind mutability check:**
          - For `TypeKind::Struct` (value types with explicit receiver mutability such as Rust `struct` and Swift `struct`): `f.decl.self_receiver == Some(ParamAccess::SharedRef)` (excludes receiver mutation so queries like `CanonicalTree::len(&self)` qualify while `&mut self` / `mutating func` commands like `flush(&mut self)` are retained as behavioral).
          - For `TypeKind::Class` (reference types without receiver-mutability syntax such as TypeScript `class` and Swift `class`/`actor`, whether in class bodies or extensions): the count of `AstKind::Call` nodes in `f.decl.body_tree` equals `f.decl.calls.len()` (preventing uncaptured indirect calls such as `this.handlers[0]()` from passing vacuously) AND every `CallSite` in `f` (if any) targets `CallReceiver::SelfInstance` and resolves only to methods in $I(T^*)$ that also satisfy this read-only query or trivial-accessor check (evaluated with a visited set treating back-edges as read-only so mutually recursive 0-arg queries terminate deterministically), so pure queries like `isZero() { return this.cents === 0; }` or `isEmpty() { return this.items.length === 0; }` are recognized while side-effecting commands like `flush() { this.handle.write(); }` are retained as behavioral.
     Reading fields or read-only queries of a passive DTO, config, IR struct, or immutable value tree (`CanonicalTree`) inside an analyzer, validator, or DP scratchpad is clean separation of data and algorithm, not Feature Envy.
   - **Same-module private helper/scratch exemption:** Skip if $T^*$ has `Visibility::Private` AND resides in the same `ModulePath` as $T_{\text{own}}$ (`target.module_path == owner.module_path && target.decl.visibility == Visibility::Private`). Private co-located scratch buffers and state helpers are intentional intra-module decomposition rather than misplaced domain behavior.
   - **Deduplication with `external-field-mutation`:** Skip if `cfg.flag_external_field_mutation` is enabled and `external-field-mutation` already recorded a diagnostic for the same `(func.id, T^*)` pair (avoiding emitting both an error and a warning for the exact same misplaced mutator; inline `// rqcode:allow(external-field-mutation)` applies post-check during report filtering, so a suppressed mutation diagnostic remains deduplicated).
4. **Structural Marinescu gate (Lanza & Marinescu 2006):**
   Require **all three**:
   - $A_{\text{foreign}}(T^*) \ge \text{min\_foreign\_accesses}$ (default `3`),
   - $\text{FDP} \le \text{max\_foreign\_providers}$ (default `2`, excluding multi-collaborator orchestrators/facades), and
   - Locality of Attribute Access:
     $$\text{LAA} = \frac{A_{\text{self}}}{A_{\text{self}} + A_{\text{foreign\_total}}} < \text{max\_laa} \quad (\text{default } 0.34)$$
     *(With default thresholds: `0` self accesses requires $\ge 3$ accesses on $T^*$ ($\text{LAA} = 0.00$); `1` self access requires $\ge 3$ accesses on $T^*$ ($\text{LAA} \le 0.25$); `2` self accesses requires $\ge 4$ foreign accesses ($\text{LAA} \le 0.333 < 0.34$)).*
5. **Conceptual confirmation (`TACO`, Palomba et al., TSE 2018):**
   Compute $\vec{v}_m$ (method $m$), $\vec{v}_{\text{own}}$ ($T_{\text{own}}$'s name, fields, and *other* non-test inherent methods), and $\vec{v}_{T^*}$ ($T^*$'s name, fields, and non-test inherent methods). Require:
   $$\cos(\vec{v}_m, \vec{v}_{T^*}) > \cos(\vec{v}_m, \vec{v}_{\text{own}}) \quad \text{and} \quad \cos(\vec{v}_m, \vec{v}_{T^*}) \ge \text{min\_conceptual\_affinity} \; (\text{default } 0.15)$$
   Emit `feature-envy` (`advisory_severity`, default `warning`) on `func.decl.span` with a related location on $T^*$'s definition.

### 4.8 Architectural Crossing (`architectural-crossing`)

Detects module-level dependency knots (Mo et al., IEEE TSE 2021) where a single non-entrypoint module acts simultaneously as a heavily depended-on target and a heavy importer of other modules.

1. **Graph construction:** Reuses the language-partitioned, non-ancestor `ModuleGraph` (Section 4.3). For each module node $u$, compute $\text{FanIn}(u)$ (count of distinct incoming module edges) and $\text{FanOut}(u)$ (count of distinct outgoing module edges).
2. **Node exemptions (candidate set $U_{\text{cand}}$):** Exclude module node $u$ from candidate evaluation if:
   - Any file in $u$ has `FileRole::LibraryRoot` or `FileRole::BinaryEntrypoint` (`lib.rs`, `main.rs`, `src/index.ts`, `src/cli.ts`), OR
   - **All** non-test production files in $u$ are barrel files (`files.iter().all(FileAst::is_barrel)`), OR
   - **All** non-test production files in $u$ define zero types and zero functions (`files.iter().all(|f| f.types.is_empty() && f.functions.is_empty())`, e.g., pure Rust `mod.rs` facade modules).
   This ensures:
   - Forbidden TypeScript barrels are flagged once by `no-barrel-module`, never double-flagged as crossings, while a real module sharing a `ModulePath` with a sibling barrel (e.g., `foo.ts` alongside `foo/index.ts`) is not shielded by the barrel.
   - Idiomatic Rust `lib.rs` / `mod.rs` and binary entrypoints never false-positive and never skew the quantile distribution.
3. **Partition size gate:** Skip any `LanguageId` partition whose candidate set satisfies $|U_{\text{cand}}| < \text{min\_graph\_modules}$ (default `12`).
4. **Adaptive dual-floor threshold:**
   Let $n = |U_{\text{cand}}|$ and $q = \text{quantile}$ (default `0.90`, clamped to `[0.0, 1.0]`). Compute the deterministic 0-based floor quantile index $p = \lfloor q \cdot (n - 1) \rfloor$ on the sorted ascending `FanIn` and `FanOut` arrays of $U_{\text{cand}}$ (e.g., at $n = 12$ and $q = 0.90$, $p = 9$, selecting the 10th smallest value):
   $$\tau_{\text{in}} = \max(\text{min\_fan\_in},\; \text{FanIn}_{\text{sorted}}[p]), \qquad \tau_{\text{out}} = \max(\text{min\_fan\_out},\; \text{FanOut}_{\text{sorted}}[p])$$
   with defaults `min_fan_in = 4` and `min_fan_out = 4`.
   Emit `architectural-crossing` (`advisory_severity`, default `warning`) for every $u \in U_{\text{cand}}$ satisfying **both** $\text{FanIn}(u) \ge \tau_{\text{in}}$ AND $\text{FanOut}(u) \ge \tau_{\text{out}}$, anchored at the primary file of $u$ (first non-test file sorted by `relative_path`) and attaching up to `max_related_edges` (default `3`) incoming and up to `max_related_edges` outgoing representative module edges as related locations.
   - Stable foundational utilities (`diagnostic.rs`, `item.rs`: high fan-in, low fan-out) are naturally excluded.
   - Clean top-level orchestrators (`engine.rs`, CLI routers: low fan-in, high fan-out) are naturally excluded.

---

## 5. Per-Language Semantics Without Leaking Language Into Core

Each frontend lowers language-specific constructs into the shared IR while `tests/architecture_guardrails.rs` enforces zero concrete language imports in `src/ast/` or `src/rules/`:

| Concept | Rust (`src/lang/rust/`) | TypeScript (`src/lang/typescript/`) | Swift (`src/lang/swift/`) |
| :--- | :--- | :--- | :--- |
| **Stateful types** | `struct` with fields | `class` with declared/parameter properties | `struct` / `class` / `actor` with stored properties |
| **Pure DTO signals** | `has_implicit_field_readers = true` for `#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]` | `is_immutable_record = true` for classes with $\ge 1$ non-`static` declared field where all non-`static` declared fields are `readonly`; `interface`/`type` DTOs are non-stateful | `has_implicit_field_readers = true` for `Codable`, `Encodable`, `Decodable`, `Equatable`, `Hashable` conformance |
| **Lifecycle methods (`is_lifecycle`)** | `fn new(...) -> Self` (excluded via `self_receiver: None` & `returns_self_type()`); `Drop` (excluded via `is_interface_impl`) | `constructor` (`is_lifecycle: true`, `self_receiver: None`, `return_type: SelfType`) | `init` (`is_lifecycle: true`, `self_receiver: None`, `return_type: SelfType`), `deinit` (`is_lifecycle: true`) |
| **Wrapped `Self` returns** | Unwraps `Option`, `Vec`, `Box`, `Arc`, `Rc`, `Mutex`, `RwLock`, `RefCell`, `Result` | Unwraps `Promise`, `Array`, union with `null`/`undefined` | Unwraps optional `?`/`!`, arrays, `Result` |
| **Barrel classification (`BarrelKind`)** | `None` (`lib.rs` / `mod.rs` are required module-tree nodes) | `DirectoryIndex` (re-exporting non-`.d.ts` `index.ts`), `PureReexport` (non-index, non-`.d.ts` file with only imports & re-exports at CST root) | `None` (target-scoped compilation) |
| **Module graph nodes** | `crate::...` (`lib.rs`/`main.rs`/`mod.rs` normalized to directory module) | Relative directory + stem (`index.ts` normalized to directory module) | Per-file `<Target>[::<Dir>…]::<File>`, where a file stem equal to its parent directory collapses (`Sources/Core/Core.swift` $\to$ `Core`). Target imports `import Core` resolve to that `Core` node when it exists; scoped imports such as `import struct Core.Order` resolve submodule-first to `Core::Order`, falling back to `Core` |

---

## 6. Deliberate Non-Goals & Excluded Approaches

The following cohesion techniques from the literature are intentionally excluded from `rqcode`:

1. **Standalone `LCOM1`–`LCOM5` / `LCOM4` rules:** Empirically near-random standalone (ROC-AUC 51%–53.6%, Al Dallal 2012) and removed from production linters (`SONAR-4853`). Structural components are used *only* inside `low-cohesion-type` when paired with size gates, accessor/lifecycle filtering, `TCC`, and TF-IDF conceptual confirmation.
2. **Global file/directory "promiscuous module" rules without layering metadata:** Candela et al. (TOSEM 2016) proved 83% of developers group files by architectural layer rather than intra-package cohesion; flagging directories or multi-type files solely on internal graph density creates severe false positives on legitimate `types.ts`, `errors.rs`, and layered modules.
3. **Git co-change / evolutionary coupling (`--git-history`):** While valuable in offline research (`TACO`, `DV8`), coupling analysis to `.git` history breaks shallow clones, generated/temp workspaces, single-file checks, and deterministic source-only repeatability.
4. **Supervised ML / neural code-smell classifiers (CodeBERT, Random Forest, etc.):** Heavy model footprint, non-deterministic cross-domain degradation, and empirically outperformed by calibrated heuristics under real-world class imbalance (Di Nucci et al., SANER 2018; Pecorelli et al., ICPC 2019).
5. **Compiler-only inter-procedural alias / chained-field type analysis:** Full alias graphs across closures, trait objects, dynamic dispatch, and arbitrary `self.<field>` chains require `rustc`/`tsc`/`swiftc` builds and violate `rqcode`'s zero-build millisecond design.

---

## 7. Code Map & Configuration Reference

### Module & Symbol Map

| Responsibility | Module | Key Symbols |
| :--- | :--- | :--- |
| Rule entry, diagnostic IDs & concept checks | `src/rules/cohesion/mod.rs` | `CohesionRule`, `CohesionRuleId` (11 variants), `CohesionChecker` |
| Module graph, cycles & crossings | `src/rules/cohesion/graph.rs` | `ModuleGraph`, `TarjanScc`, `check_cyclic_modules`, `check_architectural_crossings` |
| Leaky boundaries, stateless methods & barrels | `src/rules/cohesion/boundaries.rs` | `BoundaryIndex`, `check_leaky_boundaries`, `check_stateless_impl_methods`, `check_barrel_modules` |
| Structural + conceptual type cohesion & trivial-accessor predicate | `src/rules/cohesion/type_cohesion.rs` | `check_low_cohesion_types`, `is_trivial_accessor`, `TypeCohesionAnalyzer` |
| Structural + conceptual feature envy | `src/rules/cohesion/feature_envy.rs` | `check_feature_envy`, `FeatureEnvyAnalyzer` |
| Shared identifier tokenization & TF-IDF cosine | `src/rules/cohesion/lexical.rs` | `LexicalCorpus`, `LexicalParams`, `SparseTfIdf`, `tokenize_identifier`, `cosine_similarity` |
| Language-agnostic cohesion IR | `src/ast/mod.rs`, `src/ast/item.rs` | `BarrelKind`, `ExportOrigin`, `ExportDecl::{origin, span}`, `TypeDecl::is_immutable_record`, `FunctionDecl::is_lifecycle` |
| Unified binder & cross-file resolution | `src/ast/codebase/bind.rs`, `src/ast/codebase/mod.rs` | `IndexedFunc::resolved_field_accesses`, `IndexedFunc::resolved_call_receivers` |
| User configuration (`[cohesion]`) | `src/config.rs` | `CohesionConfig`, `ModuleBoundaryConfig`, `BarrelPolicyConfig`, `TypeCohesionConfig`, `FeatureEnvyConfig`, `CrossingConfig` |

### Configuration (`rqcode.toml`)

```toml
[cohesion]
enabled = true
severity = "error"            # Tier 1: structural encapsulation, boundaries & no-barrel-module
advisory_severity = "warning" # Tier 2: low-cohesion-type, feature-envy, architectural-crossing
min_external_functions = 2
min_external_modules = 2
flag_external_field_mutation = true
flag_test_scatter = true

[cohesion.boundaries]
flag_cyclic_modules = true
flag_leaky_boundaries = true
flag_stateless_impl_methods = true
flag_barrel_modules = true
allow_library_root_barrel = false
max_barrel_related = 3

[cohesion.type_cohesion]
enabled = true
min_fields = 4
min_methods = 5
min_field_using_methods = 4
min_component_methods = 2
max_accessor_nodes = 10
max_tcc = 0.50
max_conceptual_similarity = 0.20

[cohesion.feature_envy]
enabled = true
min_foreign_accesses = 3
max_foreign_providers = 2
max_laa = 0.34
min_conceptual_affinity = 0.15

[cohesion.crossing]
enabled = true
min_graph_modules = 12
min_fan_in = 4
min_fan_out = 4
quantile = 0.90
max_related_edges = 3
```

### References

- Al Dallal, J. (2012). *The Impact of Accounting for Special Methods in the Measurement of Object-Oriented Class Cohesion on Refactoring and Fault Prediction Activities* (JSS).
- Aniche, M., Treude, C., Zaidman, A., van Deursen, A., & Gerosa, M. A. (2016). *SATT: Tailoring Code Metric Thresholds for Different Software Architectures* (IEEE SCAM).
- Bavota, G., De Lucia, A., Marcus, A., & Oliveto, R. (2011/2013). *Identifying Extract Class Refactoring Opportunities Using Structural and Semantic Cohesion Measures* (JSS 2011) / *Using Structural and Semantic Measures to Improve Software Modularization* (ICSE 2013 / EMSE).
- Bieman, J. M., & Ott, L. M. (1994). *Measuring Functional Cohesion* (IEEE TSE); Bieman, J. M., & Kang, B.-K. (1995). *Cohesion and Reuse in an Object-Oriented System* (ACM SSR, `TCC`/`LCC`).
- Briand, L. C., Wüst, J., Daly, J. W., & Porter, D. V. (2000). *Exploring the Relationships Between Design Measures and Software Quality in Object-Oriented Systems* (JSS).
- Candela, I., Bavota, G., Russo, B., & Oliveto, R. (2016). *Using Cohesion and Coupling for Software Remodularization: Is It Enough?* (ACM TOSEM).
- Chae, H. S., Kwon, Y. R., & Bae, D. H. (2000). *A Cohesion Measure for Object-Oriented Classes* (Software: Practice and Experience).
- Di Nucci, D., Palomba, F., Tamburri, D. A., Serebrenik, A., & De Lucia, A. (2018). *Detecting Code Smells Using Machine Learning Techniques: Are We There Yet?* (IEEE SANER).
- Fontana, F. A., Mäntylä, M. V., Zanoni, M., & Marino, A. (2016). *Comparing and Experimenting Machine Learning Techniques for Code Smell Detection* (EMSE).
- Hitz, M., & Montazeri, B. (1995). *Measuring Coupling and Cohesion in Object-Oriented Systems* (Int. Symp. on Applied Corporate Computing, `LCOM4`).
- Jabangwe, R., Börstler, J., Šmite, D., & Wohlin, C. (2015). *Empirical Evidence on the Link Between Object-Oriented Measures and External Quality Attributes: A Systematic Literature Review* (EMSE).
- Lanza, M., & Marinescu, R. (2006). *Object-Oriented Metrics in Practice* (Springer).
- Marcus, A., Poshyvanyk, D., & Ferenc, R. (2008). *Using the Conceptual Cohesion of Classes for Fault Prediction in Object-Oriented Systems* (IEEE TSE, `C3`).
- Melton, H., & Tempero, E. (2007). *An Empirical Study of Cycles Among Classes in Java* (EMSE).
- Mo, R., Cai, Y., Kazman, R., Xiao, L., & Feng, Q. (2021). *Architecture Anti-Patterns: Automatically Detectable Violations of Design Principles* (IEEE TSE, `DV8`).
- Ó Cinnéide, M., Tratt, L., Harman, M., Counsell, S., & Hemati Moghadam, I. (2012). *Experimental Assessment of Software Metrics Using Automated Refactoring* (ACM/IEEE ESEM).
- Oyetoyan, T. D., Conradi, R., & Cruzes, D. S. (2013/2015). *Criticality of Defects in Cyclic Dependent Components* (JSS 2013) / *Circular Dependencies and Change-Proneness: An Empirical Study* (IEEE SANER 2015).
- Palomba, F., Bavota, G., Di Penta, M., Oliveto, R., & De Lucia, A. (2014). *Do They Really Smell Bad? A Study on Developers' Perception of Bad Code Smells* (IEEE ICSME).
- Palomba, F., Panichella, A., De Lucia, A., Oliveto, R., & Zaidman, A. (2016/2018). *A Textual-Based Technique for Smell Detection* (ICPC 2016) / *Toward a Smell-Aware Bug Prediction Model* / `TACO` (IEEE TSE 2018).
- Pecorelli, F., Palomba, F., Di Nucci, D., & De Lucia, A. (2019). *Comparing Heuristic and Machine Learning Approaches for Metric-Based Code Smell Detection* (IEEE ICPC).
- Radjenović, D., Heričko, M., Torkar, R., & Živkovič, A. (2013). *Software Fault Prediction Metrics: A Systematic Literature Review* (IST).
