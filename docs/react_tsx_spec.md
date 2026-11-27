# React / TSX Support Specification & Action Plan (v4)

## 1. Goals, Scope & Architectural Invariants

### Goals
Deliver first-class React and TSX (`.tsx`) support in `rqcode` with full behavioral parity across all four rule families (`cohesion`, `test-coupling`, `similarity`, and `ast-grep`), including mixed `.ts` + `.tsx` repositories (plus `.mts` and `.cts` module extensions).

### Non-Negotiable Architectural Invariants
1. **Strict language isolation (`tests/architecture_guardrails.rs`):** `src/ast/` and `src/rules/` must never import `ast_grep_language` or any concrete frontend (`crate::lang::*`), and must never contain language-specific branches or strings.
2. **Zero new dependencies & zero core IR breaking changes:** Reuse `ast_grep_language::Tsx` (`tree_sitter_typescript::LANGUAGE_TSX`), already compiled in via `features = ["tree-sitter-typescript"]` in `Cargo.toml`. Reuse existing IR structs (`LocalBinding`, `ParamDecl`, `ImportDecl`, `ExportDecl`, `CallSite`, `FunctionRole`, `AstKind`, `NodeLabel`) without modifying their fields.
3. **Backwards-compatible public traits & explicit frontend migration:**
   - Additions to `LanguageGrammar` (`src/ast/language.rs`) and `LanguageFrontend` (`src/lang/mod.rs`) MUST provide defaults so third-party / custom frontends (including `CustomScriptFrontend` in `tests/architecture_guardrails.rs`) continue to compile and work unchanged.
   - `TypeScriptFrontend` transitions from a unit struct to a configurable struct (`TypeScriptFrontend::new()` / `TypeScriptFrontend::with_conventions(TypeScriptConventions)`, mirroring `SwiftFrontend` in `src/lang/swift/mod.rs`). All 10 existing internal/test files constructing `TypeScriptFrontend` directly (including `src/lang/typescript/mod.rs:173,179,198`) are explicitly migrated in Point 2.
4. **No hardcoded framework/package lists in logic & no global mutable/OnceLock statics:** All configurable extensions, test directories/suffixes/support files, entrypoints, library roots, component/type wrappers, lifecycle methods, state hooks, external-boundary packages, and asset extensions live in an injectable `TypeScriptConventions` struct with sensible defaults (matching `SwiftConventions`), with zero global `OnceLock` statics (respecting `rules/rust/module-const.yml`).
5. **Repository quality gates:** All changes must pass `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `rqcode check .` with 0 errors and 0 warnings (respecting `clippy::pedantic`, `clippy::cognitive_complexity`, `clippy::self_named_module_files`, `unreachable_pub`, and the 599-line hard cap enforced by `rules/rust/max-file-lines.yml` regex `(?:[^\n]*\n){600}`). Verified: `rqcode.toml` already contains `exclude = ["target", ".git", "tests/fixtures"]`, so intentional violations in `tests/fixtures/react/` will not break `rqcode check .`.

### Deliberate Non-Goals (Out of Scope for This Spec)
- Untyped `.js` / `.jsx` JavaScript frontend support (can follow separately).
- Built-in React-specific lint YAML rules (e.g., Rules of Hooks); users can supply them via custom `ast-grep` YAML rules with `language: Tsx`.
- `tsconfig.json` custom `paths` resolution beyond existing relative paths and `@/` / `~/` / `#` project-import classification.
- Full TypeScript type inference across arbitrary conditional/mapped types or cross-file prop type-alias resolution beyond `TypeResolver` and file-local prop shapes.
- Changing `Codebase::is_binary_only_package` (`src/ast/codebase/mod.rs:126`) from global to per-language (pre-existing polyglot limitation: a repository containing both a Rust library root and a React `main.tsx` disables binary-only public-API checking globally; documented as a known limitation).

---

## 2. Why Naive TSX Support Breaks Mixed Repositories

Two constraints conflict if `.tsx` is naively registered as an independent `"tsx"` language frontend:

1. **Tree-sitter grammar IDs must remain distinct:** In `tree-sitter-typescript` 0.23.2, `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX` assign different `TSSymbol` IDs to shared grammar nodes (for example, `sym_call_expression` is `231` in TypeScript vs `245` in TSX because `sym_jsx_element` at `225` shifts subsequent IDs). Because `ast_grep_config::RuleCollection<LanguageSpec>` groups compiled rules by comparing `LanguageSpec` equality (`PartialEq` on `LanguageSpec::id()`), `TypeScript` (`id = "typescript"`) and `Tsx` (`id = "tsx"`) MUST have distinct grammar IDs so `ast-grep` never runs a rule compiled with TypeScript symbol IDs against a TSX CST.
2. **Core semantic analysis must unify `.ts` and `.tsx`:**
   - `ModuleGraph` (`src/rules/cohesion/graph.rs`) groups modules and resolves import/export edges per `FileAst::language` (`LanguageId`). Separate IDs drop `.tsx -> .ts` imports and `index.ts -> Component.tsx` barrel edges, breaking `cyclic-module-dependency` and `architectural-crossing`.
   - `SurfaceBuilder` (`src/ast/codebase/surface_build.rs`) tracks `lib_root_languages`, `bin_entry_languages`, and `active_languages` per `LanguageId`. Separate IDs disconnect `src/main.tsx` from `.ts` modules, breaking `public_api_scope = "binary-only"` and `test-only-production-api`.
   - `SimilarityRule` (`src/rules/similarity.rs`) partitions corpora by `LanguageId`. Separate IDs miss duplicate functions shared between `.ts` and `.tsx` files.

---

## 3. Action Plan (Ordered Implementation Points)

### Point 1: Separate Grammar Dialect ID from Semantic Language ID
Files: `src/ast/language.rs`, `src/ast/mod.rs`, `src/lang/mod.rs`, `src/source.rs`, `src/lib.rs`, `src/lang/rust/extract.rs`, `src/lang/swift/extract.rs`, `src/lang/typescript/extract.rs`

1. Add a defaulted method to `LanguageGrammar` (`src/ast/language.rs`):
   ```rust
   fn semantic_id(&self) -> LanguageId {
       self.id()
   }
   ```
   Expose `LanguageSpec::semantic_id(&self) -> LanguageId` and `SyntaxTree::semantic_language_id(&self) -> LanguageId`.
   - `LanguageSpec`'s `PartialEq` and `Hash` continue to key on `self.id()` (the grammar/dialect ID), preserving exact Tree-sitter symbol-table isolation in `ast-grep`.
   - `FileAst::language` uses `tree.semantic_language_id()` centrally across ALL three frontends (`rust/extract.rs:54`, `swift/extract.rs:93`, `typescript/extract.rs:104`) so multi-grammar languages unify in `ModuleGraph`, `PublicSurface`, and `SimilarityRule`.
2. Add two defaulted methods to `LanguageFrontend` (`src/lang/mod.rs`):
   ```rust
   /// Returns the Tree-sitter grammar for a specific source file path.
   /// Multi-dialect frontends (such as TypeScript + TSX) override this to select
   /// the matching dialect grammar instead of the default [`Self::grammar`].
   fn grammar_for_path(&self, _path: &Path) -> LanguageSpec {
       self.grammar()
   }

   /// Additional dialect grammars owned by this frontend (for `ast-grep`
   /// rule resolution and ID lookup).
   fn additional_grammars(&self) -> Vec<LanguageSpec> {
       Vec::new()
   }
   ```
   Document `LanguageFrontend::grammar()` as returning the frontend's default dialect grammar and directing path-aware callers to `grammar_for_path(path)`.
3. Update `LanguageRegistry` (`src/lang/mod.rs`):
   - `frontend_for_id(id)` checks `f.id() == id || f.additional_grammars().iter().any(|g| g.id() == id)`.
   - `resolve_language(name)` checks `f.grammar()` and each grammar in `f.additional_grammars()`.
4. Update `Analyzer::parse_codebase` (`src/lib.rs:118`):
   - Look up the frontend via `self.languages.frontend_for_id(file.language().semantic_id())` so per-file extraction hits `f.id()` directly without scanning `additional_grammars()` per file.
5. Update `SourceTree::discover` (`src/source.rs:114,181`):
   - Use `frontend.grammar_for_path(path)` instead of `frontend.grammar()` for both single-file and directory discovery.

### Point 2: `TypeScriptConventions`, Parameterized TSX Grammar, Module Paths & File Roles
Files: `src/lang/typescript/grammar.rs` (new), `src/lang/typescript/conventions.rs` (new), `src/lang/typescript/mod.rs`, `src/lang/mod.rs`, plus call-site migration in `tests/`

1. **Parameterized dialect grammar (`src/lang/typescript/grammar.rs`, ~95 lines):**
   - Implement a single `TsDialectGrammar` parameterized by `TsDialect` (`TypeScript` vs `Tsx`) to avoid duplicating `LanguageGrammar` methods.
   - Construct specs directly via `LanguageSpec::new(Arc::new(TsDialectGrammar::...))` in `pub fn typescript_language_spec() -> LanguageSpec` and `pub fn tsx_language_spec() -> LanguageSpec` (matching `typescript/mod.rs:71-73` with zero global `OnceLock` statics).
   - Re-export `pub use grammar::{tsx_language_spec, typescript_language_spec};` and `pub use conventions::TypeScriptConventions;` from `src/lang/typescript/mod.rs` so all 8 existing test files importing `rqcode::lang::typescript::typescript_language_spec` remain source-compatible.
   - TypeScript dialect: `id = "typescript"`, `semantic_id = "typescript"`, `aliases = &["typescript", "ts"]`, `extensions = &["ts", "mts", "cts"]`.
   - TSX dialect: `id = "tsx"`, `semantic_id = "typescript"`, `aliases = &["tsx"]`, `extensions = &["tsx"]`, backed by `ast_grep_language::Tsx`.
   - Both dialects share `CommentStyle::new("//", None, Some(("/*", "*/")), &[])` (block comment scanning already extracts JSX `{/* rqcode:allow(...) */}` and `{/* rqcode:allow-file(...) */}` directives) and `member_separator = "."`.
2. **Injectable `TypeScriptConventions` (`src/lang/typescript/conventions.rs`, ~220 lines including unit tests) & `TsBodyContext` wiring:**
   - Mirror `SwiftConventions` (`src/lang/swift/mod.rs:77-147`) with `#[derive(Debug, Clone)]` and `impl Default`:
     - `extensions`: `["ts", "tsx", "mts", "cts"]`
     - `tsx_extensions`: `["tsx"]`
     - `ambient_declaration_suffixes`: `[".d.ts", ".d.mts", ".d.cts"]`
     - `top_level_integration_test_dirs`: `["tests", "__tests__", "e2e"]`
     - `nested_unit_test_dirs`: `["__tests__", "__mocks__"]`
     - `unit_test_stem_suffixes`: `[".test", ".spec", ".stories"]`
     - `test_support_file_stems`: `["test-utils", "testUtils", "setupTests", "setup-tests", "vitest.setup", "jest.setup"]`
     - `entrypoint_stems`: `["main", "cli"]`
     - `library_root_stems`: `["index"]`
     - `component_wrapper_functions`: `["memo", "forwardRef"]`
     - `lazy_wrapper_functions`: `["lazy"]`
     - `component_type_wrappers`: `["FC", "FunctionComponent", "VFC", "NamedExoticComponent", "MemoExoticComponent", "ForwardRefExoticComponent"]`
     - `props_type_wrappers`: `["PropsWithChildren", "PropsWithRef", "PropsWithoutRef", "Readonly", "Required", "Partial"]`
     - `component_base_classes`: `["Component", "PureComponent", "React::Component", "React::PureComponent"]`
     - `component_lifecycle_methods`: `["render", "componentDidMount", "componentDidUpdate", "componentWillUnmount", "componentDidCatch", "getDerivedStateFromError", "shouldComponentUpdate", "getSnapshotBeforeUpdate"]`
     - `state_hook_names`: `["useState"]`
     - `external_boundary_packages`: explicit I/O, network, database, storage, messaging, and cloud SDK packages/prefixes (`fs`, `http`, `https`, `http2`, `net`, `tls`, `dgram`, `dns`, `child_process`, `worker_threads`, `cluster`, `sqlite`, `axios`, `got`, `undici`, `node-fetch`, `ky`, `superagent`, `nodemailer`, `pg`, `mysql`, `mysql2`, `sqlite3`, `better-sqlite3`, `mongodb`, `mongoose`, `redis`, `ioredis`, `prisma`, `@prisma/`, `drizzle-orm`, `typeorm`, `knex`, `sequelize`, `kafkajs`, `amqplib`, `@aws-sdk/`, `aws-sdk`, `@google-cloud/`, `@azure/`, `stripe`, `twilio`, `@sendgrid/`, `firebase`, `@firebase/`, `@supabase/`, `ws`, `socket.io`, `socket.io-client`, `grpc`, `@grpc/`). In `TypeScriptConventions::is_external_boundary_package`, strip an optional leading `"node:"` prefix before matching exact package name or `<pkg>/` subpath prefix so `node:fs`, `node:fs/promises`, `node:http`, `node:net`, `node:child_process`, etc., are all covered uniformly.
     - `asset_extensions`: `[".css", ".scss", ".sass", ".less", ".styl", ".svg", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".ico", ".woff", ".woff2", ".ttf", ".eot", ".mp3", ".mp4", ".webm", ".wasm", ".json"]`
     - `asset_component_queries`: `["react"]` (preserves `./icon.svg?react` as a component import)
   - Thread `&'a TypeScriptConventions` from `TypeScriptFrontend::extract_module` into `TypeScriptExtractor`, `ModuleImportParser`, `TsMemberExtractor`, `TsCouplingInspector`, and a new `pub(super) struct TsBodyContext<'a>` (mirroring `SwiftBodyContext` in `src/lang/swift/body.rs:22-27`) carrying `tree`, `base_dir`, `namespace_imports`, `conventions`, and `prop_shapes`. This avoids parameter bloat (`rules/rust/too-many-params.yml`) and lets sibling modules (`expr.rs`, `jsx.rs`, `props.rs`) share immutable context cleanly.
   - Implement `TypeScriptFrontend { conventions: TypeScriptConventions }` with `Default`, `TypeScriptFrontend::new()`, and `TypeScriptFrontend::with_conventions(TypeScriptConventions)`.
   - Migrate ALL existing construction sites to `TypeScriptFrontend::new()` (verified via `rg '\bTypeScriptFrontend\b' src tests` showing only the struct/impl definition and `::new()` / `::with_conventions(...)`):
     - `src/lang/mod.rs`, `src/lang/typescript/mod.rs` (lines 173, 179, 198)
     - `tests/architecture_guardrails.rs`, `tests/phase2_surface.rs`, `tests/phase3_coupling.rs`, `tests/review_fixes.rs`, `tests/cohesion_module_graph.rs`, `tests/cohesion_lexical.rs`, `tests/cohesion_barrel_ir.rs`, `tests/cohesion_ir_flags.rs`
3. **`LanguageFrontend` wiring on `TypeScriptFrontend`:**
   - `extensions(&self)` -> `&["ts", "tsx", "mts", "cts"]`
   - `grammar_for_path(&self, path: &Path)` -> `tsx_language_spec()` when `path.extension()` matches `conventions.tsx_extensions` (ASCII case-insensitive), otherwise `typescript_language_spec()`.
   - `additional_grammars(&self)` -> `vec![tsx_language_spec()]`.
4. **Case-insensitive `module_path` normalization:**
   - Strip matching extension from `conventions.extensions` ASCII case-insensitively (`Foo.TSX` -> `Foo`), then strip matching `conventions.unit_test_stem_suffixes` (`.test`, `.spec`, `.stories`), then omit `"index"`.
   - Examples:
     - `src/components/OrderCard.tsx` -> `src::components::OrderCard`
     - `src/components/OrderCard.test.tsx` -> `src::components::OrderCard`
     - `src/components/OrderCard.TSX` -> `src::components::OrderCard`
     - `src/components/index.tsx` -> `src::components`
5. **Symmetric & React-aware `file_role` classification (in `conventions.rs`, leaving `src/lang/common.rs` untouched):**
   - Check `top_level_integration_test_dirs` (`tests`, `__tests__`, `e2e`) on the first path component -> `FileRole::IntegrationTest` (preserving existing `typescript/mod.rs:201` test).
   - Check `nested_unit_test_dirs` (`__tests__`, `__mocks__`) on any non-first directory component -> `FileRole::UnitTest` (colocated `src/components/__tests__/OrderCard.tsx` and `src/**/__mocks__/*.ts` classify as `UnitTest`, NOT `Module`).
   - Derive file-name matches symmetrically across `{stem} x {extensions}` (ASCII case-insensitive extension stripping):
     - Stem ending with any `unit_test_stem_suffixes` (`.test`, `.spec`, `.stories`) -> `FileRole::UnitTest`.
     - Stem matching any `test_support_file_stems` (`test-utils`, `testUtils`, `setupTests`, `setup-tests`, `vitest.setup`, `jest.setup`) -> `FileRole::UnitTest`.
     - Stem matching any `entrypoint_stems` (`main`, `cli`) across all extensions -> `FileRole::BinaryEntrypoint`.
     - Path matching `[<root_stem>.<ext>]` or `["src", <root_stem>.<ext>]` for `library_root_stems` (`index.*`, `src/index.*`) -> `FileRole::LibraryRoot`.
   - **CRA-style `src/index.tsx` reclassification after extraction (`extract.rs`):**
     - If `role == FileRole::LibraryRoot` and after AST extraction the file has no exports (`exports.is_empty()` and no exported types/functions) AND has top-level side effects (`!top_level_idents.is_empty()`, e.g., `createRoot(...).render(<App />)`), reclassify `FileAst::role` to `FileRole::BinaryEntrypoint`.
     - If `src/index.tsx` DOES export symbols or re-exports (a component library root), it stays `FileRole::LibraryRoot`.
   - Update `classify_barrel_kind` (`imports.rs`) to check all `conventions.ambient_declaration_suffixes` (`.d.ts`, `.d.mts`, `.d.cts`).

### Point 3: React Component Extraction, HOC Unwrapping, Destructured Props & Hook State
Files: `src/lang/typescript/props.rs` (new, ~220 lines), `src/lang/typescript/extract.rs`, `src/lang/typescript/members.rs`, `src/lang/typescript/body.rs`

1. **HOC-wrapped components, hoisted exports, & lazy routes (`extract.rs`):**
   - **Inline wrapped declarations (`const Card = memo((...) => ...)`, `export const Input = forwardRef<El, Props>((...) => ...)`, `export default memo(forwardRef((...) => ...))`):**
     - Recursively unwrap nested `call_expression`s matching `conventions.component_wrapper_functions` (plain `memo(...)` or member `React.memo(...)`) when the first named argument is an `arrow_function` or `function_expression` (or another recognized wrapper call).
     - Extract the inner function via `extract_function_like` using:
       - the outer `variable_declarator` / `export_statement` span (so `// rqcode:allow(...)` placed above multiline wrappers matches `span.start_line()`);
       - the variable name (or for `export default`, the `function_expression`'s own name if present, else `"default"`);
       - prop type arguments collected from the declarator's type annotation (`React.FC<Props>`) or the wrapper call's `type_arguments` (`memo<Props>` takes arg 0; `forwardRef<El, Props>` takes arg 1).
     - Walk any extra arguments on the wrapper call (such as `memo`'s 2nd `areEqual` comparator) so references/calls inside comparators are preserved.
   - **Structural `export default <call_expression>` export propagation (handles `React.memo(OrderCard)`, `observer(OrderCard)`, `connect(mapState)(OrderCard)`, `withRouter(OrderCard)`, etc., including hoisted functions):**
     - Collect pending `(symbol, is_default, export_span)` local-export markers (both from `export { ... }` / `export default Ident` and from structural walking of `export default <call_expression>` chains) and apply `mark_local_exported` in a post-pass after all top-level statements have been extracted (next to `extract.rs:78-88`), so hoisted `export default observer(Foo); function Foo() {}` is handled cleanly.
     - For `export default <call_expression>` that does not unwrap to an inline function: walk the call chain's arguments; for any bare `identifier` matching a locally declared function or class in `self.functions` / `self.types`, emit `ExportDecl { source_module: None, symbol, exported_name: "default", is_default: true, is_wildcard: false, visibility: Visibility::Public, origin: ExportOrigin::Local, span }` and mark it default-exported. **CRITICAL:** Do NOT push the exported component identifier into `top_level_idents` (that would seed an unused default-exported component as a production root and defeat `test-only-production-api`). Collect non-target identifiers (such as `mapState` in `connect(mapState)(OrderCard)`) normally.
   - **Lazy route imports (handles all 3 common React patterns without new IR fields):**
     - Pattern A & B: Top-level non-test (`!in_test_context`) `const OrdersPage = lazy(() => import('./pages/Orders'))` or `export const OrdersPage = React.lazy(() => import('./pages/Orders'))`:
       - Emit a synthetic `ImportDecl` matching `ModuleImportParser`'s default-import layout (`ImportKind::Default`, `resolved_module: Some(target_mod)`, `symbol: declarator_name`, `alias: None`, `origin: ImportOrigin::Project`, `in_test_module: false`). Skipping this when `in_test_context` is true ensures a `lazy(...)` inside a `describe` block in a production file never sets `in_test_module: true` or flips `has_inline_test_imports`.
       - If the declaration is exported (`export const OrdersPage = lazy(...)`), also emit `ExportDecl { source_module: None, symbol: declarator_name, exported_name: declarator_name, origin: ExportOrigin::ImportedBinding, .. }` so importing `OrdersPage` from `routes.tsx` chains via Point 5's `ImportedBinding` traversal to `pages::Orders`'s default export!
       - Return immediately from `extract_variable_declaration` (do NOT fall through to `collect_leaf_idents`, so an unrendered `const DeadPage = lazy(...)` does NOT falsely mark `DeadPage` reachable).
     - Pattern C: Top-level non-test expression/array/object literal containing a dynamic `import('<relative>')` (e.g., `export const routes = [{ element: lazy(() => import('./pages/Orders')) }]`):
       - When falling through in a non-test top-level statement, scan for any relative dynamic `import('<specifier>')` resolving to `Some(target_mod)`. Emit a synthetic `ImportDecl { kind: ImportKind::Default, resolved_module: Some(target_mod), symbol: target_mod.to_string(), alias: None, origin: ImportOrigin::Project, in_test_module: false, .. }` and push `target_mod.to_string()` into `top_level_idents` so `ReachabilityChecker` seeds `prod_default_func_in_mod(target_mod)` cleanly.
2. **Class component lifecycle methods (`members.rs`):**
   - Reorder `extract_class` in `members.rs` so `extract_class_heritage` runs BEFORE iterating class body methods (currently lines 82-120 vs 122-123).
   - When any `base_types` entry matches `conventions.component_base_classes` (`Component`, `PureComponent`, `React::Component`, `React::PureComponent` - verified that `extends React.Component<P, S>` normalizes to `TypeRef::Named("React::Component")`), mark methods matching `conventions.component_lifecycle_methods` with `is_interface_impl: true` and `is_lifecycle: true` (keeping `role: FunctionRole::Production`, `interface_name: None`). Because `resolved_interface` is `None` for external framework overrides, `reachability.rs:189` treats them as production reachability roots and `reachability.rs:433` / `surface.rs:315` exempt them from `test-only-production-api` / overexposed checks WITHOUT setting `has_binary_entry = true` in `src/ast/codebase/mod.rs:78` (preserving library semantics for React component libraries).
3. **Destructured props via `LocalBinding` with explicit plumbing (`props.rs`, `members.rs`, `extract.rs`, `body.rs`):**
   - **Pass 1 prop-shape pre-scan** (runs BEFORE Pass 2 declaration extraction in `TypeScriptExtractor::extract_file`):
     - Inspects top-level statements and unwraps `export_statement.field("declaration")`.
     - Extracts file-local prop shapes (`PropShapeMap = HashMap<String, HashMap<String, TypeRef>>`) from `interface_declaration` and `type_alias_declaration` (supporting `object_type`, `intersection_type`, and configured `props_type_wrappers` such as `PropsWithChildren<T>`). `type_alias_declaration` is NOT emitted as a stateful `TypeDecl`.
   - **Exact return shape & 3-site plumbing:**
     - Define in `props.rs`:
       ```rust
       pub(super) struct ExtractedParams<'a> {
           pub(super) params: Vec<ParamDecl>,
           pub(super) local_bindings: Vec<LocalBinding>,
           pub(super) default_exprs: Vec<TsNode<'a>>,
       }
       ```
     - Return `ExtractedParams<'a>` from `TsMemberExtractor::extract_callable_params` and `extract_param_list` (accepting optional external prop-type hint from `React.FC<Props>` / `memo<Props>` / `forwardRef<El, Props>`).
     - Wire `ExtractedParams<'a>` at ALL THREE callable extraction sites:
       1. `extract.rs:319` (`extract_function_like`)
       2. `members.rs:168-172` (`extract_method`)
       3. `extract.rs:409` (test callbacks)
     - At each site, seed `TsBodyWalker` with `extracted.local_bindings` and walk every node in `extracted.default_exprs` in `AccessMode::Read` BEFORE walking `node.field("body")`.
     - Also update the nested-callback parameter arm in `body.rs:109-118` (`required_parameter | optional_parameter` with `object_pattern`) to emit `LocalBinding`s via the same `props.rs` helper so `items.forEach(({ order }: Row) => { order.total = 0 })` is detected consistently alongside top-level components.
   - **Preserve exact positional arity with empty placeholder `ParamDecl.name`:**
     - For each `object_pattern` parameter, push **exactly one** `ParamDecl { name: String::new(), type_name: param_ty, access: ParamAccess::Owned }` at its true positional index.
     - Note on downstream behavior: previously destructured parameters were silently dropped (`members.rs:285-287`). Keeping one positional `ParamDecl` with `name: String::new()` improves accuracy for `bind.rs:307-343`, `type_cohesion.rs:64-66`, `feature_envy.rs:343`, `surface_build.rs:489`, and `codebase/mod.rs:94-101,262` (verified against all existing TS fixtures during implementation). If `param_ty` resolves to a declared type (e.g., `interface Props`), `bind.rs:171-174` inserts key `""` in `resolved_local_types`, which is harmless because `bind.rs:382` ignores empty receivers, `lexical.rs:344` emits zero tokens for `""`, and `""` never appears in diagnostics.
     - Emit each destructured binding as `LocalBinding { name, type_name, init_receiver: None, init_callee: None }` (matching exact `src/ast/item/mod.rs:243-248` fields, zero core IR changes).
   - **Prop-shape resolution priority:**
     1. Inline `object_type`, `intersection_type`, or `props_type_wrappers` annotation on the parameter.
     2. Named type annotation looked up in the file-local prop-shape map.
     3. Declarator type annotation matching `conventions.component_type_wrappers` (`React.FC<Props>`, `FC<{ order: Order }>` -> type arg 0).
     4. Wrapper call `type_arguments` (`memo<Props>` -> type arg 0, `forwardRef<El, Props>` -> type arg 1).
   - **Exact `tree-sitter-typescript 0.23.2` `object_pattern` children (recursive):**
     1. `shorthand_property_identifier_pattern` (`{ order }`) -> `LocalBinding` for `"order"`.
     2. `object_assignment_pattern` (`{ count = 0 }`, `{ order = fallback }`) -> recurse on `left` and push `right` into `default_exprs`.
     3. `pair_pattern` (`{ order: activeOrder }`, `{ order: activeOrder = fallback }`, `{ nested: { item } }`) -> look up `key` in prop shape; if `value` is `assignment_pattern`, push `right` into `default_exprs` and unwrap `left`; recurse if nested `object_pattern`, else emit `LocalBinding`.
     4. `rest_pattern` (`{ className, ...rest }`) -> emit `LocalBinding` for `rest` with `type_name: None` (or whole named props `TypeRef` if present).
   - Update `TsBodyWalker::record_object_pattern_reads` (`body.rs:247-274`) to handle `object_assignment_pattern`, `rest_pattern`, and nested `object_pattern` consistently.
4. **Hook state typing (`useState<T>` array destructuring, `body.rs::record_local_binding`):**
   - In `TsBodyWalker::record_local_binding` (`body.rs:203-230`), when `name` is `array_pattern` and `value` is a `call_expression` whose callee matches `conventions.state_hook_names` (`useState` or `React.useState`), emit a `LocalBinding { name, type_name, init_receiver: None, init_callee: None }` for the first `identifier` element typed from the call's first `type_arguments` entry (falling back to `expr::infer_expr_origin(ctx, first_arg)`).

### Point 4: JSX Body Walking, Component Calls, Dynamic `import()` & Top-Level Idents
Files: `src/lang/typescript/jsx.rs` (new, ~160 lines), `src/lang/typescript/expr.rs` (new, ~150 lines), `src/lang/typescript/body.rs`, `src/lang/typescript/syntax.rs`

1. **Module visibility for `expr.rs` and `jsx.rs`:**
   - `expr.rs` provides `pub(super)` free functions taking `&TsBodyContext<'_>` (mirroring `src/lang/swift/expr.rs:390`).
   - `jsx.rs` provides `pub(super) fn is_intrinsic_jsx_tag` and `impl TsBodyWalker<'_>` methods (or `pub(super)` accessors on `TsBodyWalker`'s `ctx` and `acc` fields) so JSX walking cleanly invokes `self.walk(...)` and pushes into `self.acc` without breaking encapsulation.
2. **Intrinsic tag rule matching TypeScript's `isIntrinsicJsxName` (`jsx.rs`):**
   - `pub(super) fn is_intrinsic_jsx_tag(name: &str) -> bool`: returns `true` iff `name.starts_with(|c: char| c.is_ascii_lowercase()) || name.contains('-')`; returns `false` for all component tags.
3. **JSX walking dispatched via a single arm from `TsBodyWalker::walk` (`jsx.rs`):**
   - `jsx_closing_element` (`</OrderCard>`, `</div>`) and `jsx_text`: return immediately without walking children.
   - `jsx_opening_element` and `jsx_self_closing_element`:
     - Inspect `name` field:
       - **`identifier`:**
         - If `is_intrinsic_jsx_tag(name)`: skip completely.
         - Else (component tag): record `CallSite { kind: CallKind::Free, callee: name, receiver: None, arg_receivers: Vec::new(), span }`, push `name` to `referenced_idents`, and push `TypeRef::Named(name)` to `constructed_types` (enables class-component construction tracking / `scattered-tests`, harmless for function components because `TypeResolver` only resolves declared `TypeDecl`s).
       - **`"member_expression" | "nested_identifier"`** (handles both `tree-sitter-typescript`'s aliased `member_expression` and multi-level `nested_identifier` chains like `<UI.OrderCard />`, `<Card.Header />`, `<Dialog.Root />`, `<ThemeContext.Provider />`, `<UI.Forms.Input />`):
         - Call a dedicated `record_jsx_member_call` helper that recursively reads `object` and `property` fields, determines root identifier `root` and leaf property `prop`:
           - If `root` is in `namespace_imports`: emit `CallSite { kind: CallKind::Scoped, callee: prop, receiver: Some(CallReceiver::Named(ns_mod.to_string())), arg_receivers: Vec::new(), span }`.
           - Else if `root` starts with an uppercase ASCII letter: emit `CallSite { kind: CallKind::Scoped, callee: prop, receiver: Some(CallReceiver::Named(root)), arg_receivers: Vec::new(), span }`.
           - Else: emit `CallSite { kind: CallKind::Method, callee: prop, receiver: Some(CallReceiver::Named(root)), arg_receivers: Vec::new(), span }`.
         - Push `prop` to `scoped_ident_refs`, and walk the `object` sub-expression in `AccessMode::Read`.
         - **NEVER** record a `FieldAccess` and **NEVER** call `record_scoped_type_call` / `push_constructed_type` (avoiding bogus constructions for `<Card.Header />`, `<Dialog.Root />`, `<ThemeContext.Provider />`).
       - **`jsx_namespace_name`** (e.g., `<svg:path />`): skip tag name.
     - Walk all attribute and spread children (`jsx_attribute`, `jsx_expression`) in `AccessMode::Read` so prop expressions, inline arrow callbacks (`onClick={() => { order.total = 0; }}`), hooks, spies, and mocks inside JSX are analyzed.
   - Leave `arg_receivers: Vec::new()` on all JSX component `CallSite`s so named JSX props are never matched positionally.
4. **Dynamic `import('<relative>')` inside function bodies (`body.rs::record_call`):**
   - When `call_expression` has `function.kind() == "import"` and its first named argument is a string literal resolving via `resolve_relative_module_path` to `Some(target_mod)`, emit `CallSite { kind: CallKind::Scoped, callee: String::from("default"), receiver: Some(CallReceiver::Named(target_mod.to_string())), arg_receivers: Vec::new(), span }`.
5. **Update `syntax::collect_leaf_idents` (`src/lang/typescript/syntax.rs`):**
   - Skip `jsx_closing_element` and any `jsx_opening_element` / `jsx_self_closing_element` `name` satisfying `jsx::is_intrinsic_jsx_tag`.
   - Result: top-level JSX in `main.tsx` seeds `<App />` as a production root without polluting `top_level_idents` or `prod_unresolved_field_names` (`reachability.rs:513-518`) with `div`, `main`, `label`, `title`, `summary`, etc.

### Point 5: Language-Agnostic Import-Aware Free Call & Default-Export Reachability
Files: `src/rules/test_coupling/imports.rs` (new, ~180 lines), `src/rules/test_coupling/mod.rs`, `src/rules/test_coupling/reachability.rs`

All logic in this point is strictly language-agnostic, operating solely on `ImportDecl`, `ExportDecl`, `CallKind`, and `ModulePath`, and lives in a dedicated `src/rules/test_coupling/imports.rs` submodule:

1. **`TestCouplingIndex::resolve_imported_free_funcs(&self, file_id: FileId, caller_is_test: bool, local_name: &str) -> Option<Vec<FuncId>>` (`test_coupling/imports.rs`):**
   - **4-case import scope predicate (works uniformly for Rust integration tests `tests/*.rs`, TS `.test.tsx`, Swift `Tests/*.swift`, Rust inline `#[cfg(test)] mod tests`, and TS Vitest in-source tests):**
     ```rust
     let file = self.codebase.file(file_id);
     let has_inline_test_imports = !file.is_test_file()
         && file.imports.iter().any(|imp| imp.in_test_module);
     let matches_scope = |imp: &ImportDecl| -> bool {
         if file.is_test_file() {
             true
         } else if !caller_is_test {
             !imp.in_test_module
         } else if has_inline_test_imports {
             imp.in_test_module
         } else {
             !imp.in_test_module
         }
     };
     ```
     - Dedicated test files (`tests/*.rs` where `in_test_module == false`, `.test.tsx` / `Tests/*.swift` where `in_test_module == true`) match all file imports.
     - Production callers (`!caller_is_test`, including `seed_ident_roots` and `compute_prod_caller_scopes`) match only `!imp.in_test_module` (never shadowed by Rust `#[cfg(test)] mod tests`).
     - Rust inline `#[cfg(test)] mod tests` (`has_inline_test_imports == true`) matches only `imp.in_test_module`.
     - TS Vitest in-source tests (`!file.is_test_file() && !has_inline_test_imports`) matches top-level ES imports (`!imp.in_test_module`).
   - Inspect explicit imports matching `matches_scope(imp) && matches!(imp.kind, ImportKind::Default | ImportKind::Named) && ImportDecl::local_name(imp) == local_name`.
   - Returns `None` if no matching explicit import binds `local_name`.
   - Returns `Some(funcs)` when an explicit import binds `local_name`:
     - **`imp.resolved_module == None`:**
       - If `imp.origin == ImportOrigin::StandardValue` (e.g., `import { render } from '@testing-library/react'`, `import { memo } from 'react'`): return `Some(Vec::new())` immediately.
       - Otherwise (`Project` alias like `@/components/Row`, or Rust `tests/integration.rs` `use my_crate::foo`): fall back to unique `prod_free_by_name.get(&imp.symbol)` filtered by `self.is_callable_from(codebase.func(fid), &file.module_path)` and excluding same-module private functions (`!(target.module_path == file.module_path && target.decl.visibility == Visibility::Private)`).
     - **`imp.resolved_module == Some(target_mod)`:**
       - Resolve directly + recursively follow barrel re-exports up to bounded depth 4 (cycle-guarded by visited `(ModulePath, symbol, is_default)` set), handling ALL four barrel forms:
         1. Direct hit: `ImportKind::Default` -> `prod_default_func_in_mod(target_mod)`; `ImportKind::Named` -> `prod_free_in_mod(target_mod, symbol)`.
         2. Named re-export of default (`export { default as OrderCard } from './OrderCard'` where `imports.rs:152-169` emits `exported_name == "OrderCard"`, `symbol == "default"`, `is_default == false`): match `exp.exported_name == symbol` and when `exp.symbol == "default"`, switch to default-export lookup on `source_module`.
         3. Direct named or wildcard re-export (`export { X } from './x'`, `export { X as default } from './x'`, `export * from './x'`): match `exp.is_default` (for default lookup) or `exp.exported_name == symbol` / `exp.is_wildcard` (for named lookup) and follow `source_module`.
         4. Two-step `ExportOrigin::ImportedBinding` (`import X from './x'; export { X }` or `export default X` where `source_module == None`): look up `exp.symbol` in the barrel file's own non-test `ImportDecl`s (`imp.local_name() == exp.symbol`) and follow that import's `resolved_module` and `ImportKind`.
       - If still empty after depth 4 (e.g., Rust `pub use inner as api;` module alias where `target_mod` has no `FileAst`, or deep >4-hop barrel): fall back to unique `prod_free_by_name.get(&imp.symbol)` filtered by `is_callable_from` and strictly excluding same-module private functions (and skipping this fallback for `ImportKind::Default` when `target_mod` is a known module in the codebase).
   - **Shadowing guarantee preserved:** For `OrderList.test.tsx` importing `Row` from `../shared/Row` while `OrderList.tsx` defines private `Row`, the fallback excludes same-module private symbols, returning `Some(Vec::new())` and preventing false `test-calls-private-api`.
2. **Additive reachability helper (`TestCouplingIndex::ident_targets` in `test_coupling/imports.rs`):**
   - Add `pub(super) fn ident_targets(&self, file_id: FileId, caller_is_test: bool, module_path: &'a ModulePath, ident: &'a str, include_methods: bool) -> Vec<FuncId>` that combines:
     - imported targets from `resolve_imported_free_funcs(file_id, caller_is_test, ident)` (if any);
     - PLUS existing `prod_free_in_mod(module_path, ident)` and `prod_free_named(ident)` filtered by `is_callable_from`;
     - PLUS `prod_methods_named(ident)` filtered by `is_callable_from` when `include_methods == true` (preserving exact existing edges at `reachability.rs:158` and `:218`; `collect_ident_edges` passes `include_methods = false` and retains its separate `scoped_ident_refs` loop at `reachability.rs:351-360`).
   - **Explicit merge & scope-classification semantics:** Reachability (`seed_ident_roots`, `collect_ident_edges`, `compute_prod_caller_scopes`) is strictly **ADDITIVE** (adds imported targets to existing name-based edges and never removes existing edges, preserving conservative over-approximation for `ProdReachable`). ONLY `resolve_prod_call` lets an explicit import shadow same-module lookup. In `compute_prod_caller_scopes`, each returned target `fid` is classified by `file.module_path.is_same_or_child_of(&codebase.func(fid).module_path)` into `same_mod_called` vs `external_called` (and recorded in `calls_by_module`).
   - Replacing the duplicated lookup loops in `reachability.rs` (`seed_ident_roots`, `collect_ident_edges`, `compute_prod_caller_scopes`) with calls to `self.index.ident_targets(...)` yields a **net line reduction in `reachability.rs`** (583 -> ~555 lines, well under the 599-line cap).
3. **Update `TestCouplingIndex::resolve_prod_call` (`test_coupling/mod.rs`):**
   - `CallKind::Free`: check `test_free_by_file_name`, then consult `resolve_imported_free_funcs(caller.file_id, self.is_test_func(caller), callee)` BEFORE falling back to `prod_free_in_mod(&caller.module_path, callee)` and the existing glob/unique fallback.
   - `CallKind::Scoped`: if `receiver_ty.is_none()` and `call.callee == "default"` and `CallReceiver::Named(path)` matches a target module, look up `prod_default_func_by_mod` by exact `ModulePath` (`ModulePath::new(path.split("::").map(String::from).collect())`, deterministic O(1) lookup) to connect dynamic `import('./pages/Orders')`.

### Point 6: JSX CST Normalization for Structural Similarity
Files: `src/lang/typescript/normalize.rs`

Add JSX normalization arms to `TsNormalizeDialect` using only existing language-agnostic `AstKind` / `NodeLabel` variants (zero changes to `src/ast/similarity/`):

1. `jsx_element`:
   - Enter `AstKind::Construct` (supports Stage 0 sibling folding for repetitive `<li>`, `<td>`, `<Option>` lists).
   - Emit tag name from `open_tag` (`jsx_opening_element`): `Exact(tag_text)` if `name` field is present, or `Exact("<>")` for JSX fragments (`<>...</>`).
   - Walk attributes on `open_tag` (`jsx_attribute` and spread `jsx_expression`).
   - Walk children of `jsx_element` excluding `open_tag` and `close_tag`.
   - Exit `AstKind::Construct`.
2. `jsx_self_closing_element`:
   - Enter `AstKind::Construct`, emit `Exact(tag_text)` from `name`, walk attributes (`jsx_attribute` and spread `jsx_expression`), exit `AstKind::Construct`.
   - Result: `<Badge />` and `<Badge></Badge>` normalize identically, while `<UserAvatar>` and `<OrderBadge>` remain distinct.
3. `jsx_attribute` (has NO named fields in `tree-sitter-typescript 0.23.2`):
   - Extract attribute name as named non-comment child 0 (`property_identifier` or `jsx_namespace_name`) and optional value as named non-comment child 1.
   - Enter `AstKind::Arg`, emit `Exact(attr_name)` (NOT `NameSpace::Label`, so forms/tables differing only in prop names such as `placeholder`/`aria-label` vs `title`/`name` are NOT alpha-renamed into false `duplicate-function` hits).
   - If a value child is present, walk it; if omitted (boolean shorthand `<Button disabled />`), emit `LitKind::Bool`.
   - Exit `AstKind::Arg`.
4. `jsx_expression`:
   - Walk named non-comment children (`{ ... }` and spread `{...props}`); comment-only `{/* ... */}` emits nothing.
5. `jsx_closing_element`:
   - Skip completely (return `true` in `visit_compound`).
6. `jsx_text`:
   - Emit `LitKind::String` iff `!node.text().trim().is_empty()`.
7. `html_character_reference`:
   - Emit `LitKind::String`.

### Point 7: Import Origin Policy & Relative Asset Imports
Files: `src/lang/typescript/coupling.rs`, `src/lang/typescript/imports.rs`

1. **Relative CSS / asset imports (`imports.rs`):**
   - Check `conventions.is_asset_specifier(&specifier)` (stripping `?inline` / `?url` / `?raw` query suffixes while preserving configured `asset_component_queries` like `.svg?react`) BEFORE calling `resolve_relative_module_path`.
   - For asset specifiers (`./OrderCard.module.css`, `./logo.svg`, `./styles.css?inline`), set `resolved_module = None` and `origin = ImportOrigin::StandardValue`.
2. **Injectable external-boundary package classification (`conventions.rs`, `coupling.rs`):**
   - Classify explicit I/O packages (`conventions.is_external_boundary_package(specifier)`) as `ImportOrigin::ExternalBoundary`, project/alias imports as `ImportOrigin::Project`, and all other non-relative packages (`react`, `react-dom`, `react-router-dom`, `@tanstack/react-query`, `zustand`, `@mui/*`, `@radix-ui/*`, `lucide-react`, `@testing-library/*`, `vitest`, `zod`, `lodash`, etc.) as `ImportOrigin::StandardValue` (mirroring `src/lang/swift/coupling.rs:9-54`).
   - **Verified before-and-after `ImportOrigin` audit across ALL existing TS fixtures & inline tests:**
     | Specifier | Existing fixture(s) / test(s) | Before | After |
     |---|---|---|---|
     | `"vitest"` | `p3_ts_barrel_mock_spy_interaction`, `neg_ts_colocated_import`, `neg_dev_guards`, `neg_ts_abstract_subclass`, `neg_ts_external_boundary_mock`, `tests/phase3_coupling.rs`, `tests/cohesion_barrel_policy.rs` | `StandardValue` | `StandardValue` (unchanged) |
     | `"nodemailer"` | `neg_ts_external_boundary_mock/src/gateways/smtpMailer.ts` | `ExternalBoundary` | `ExternalBoundary` (unchanged) |
     | Relative `./*`, `../*` | All TS fixtures (`cohesion`, `test_coupling`, `benchmarks`) | `Project` | `Project` (unchanged) |
     | `"react"`, `"react-router-dom"`, etc. | New React fixtures | (`ExternalBoundary` - broke mocking check) | `StandardValue` (fixed) |
     | `"./Card.module.css"` | New React fixtures | (`Project` - bogus module) | `StandardValue`, `resolved_module: None` (fixed) |
   - **User-facing behavior note (documented in `README.md`):** Third-party I/O SDKs not in the default `external_boundary_packages` allowlist (e.g., `openai`, `@octokit/rest`, `@apollo/client`) now default to `StandardValue` rather than `ExternalBoundary` unless added via `TypeScriptConventions::external_boundary_packages` (ambient `fetch` / `WebSocket` / `localStorage` calls remain automatically detected as external boundaries).
3. **Behavioral assertions & module mocks (`coupling.rs`):**
   - `@testing-library/jest-dom` matchers already record behavioral assertions via `record_expect`.
   - `vi.mock` / `jest.mock` / `vi.spyOn` / `jest.spyOn` on `.tsx` modules resolve via `resolve_relative_module_path` (stripping `.ts`/`.tsx`/`.mts`/`.cts` case-insensitively).

### Point 8: `ast-grep` Rule Support for TSX
Files: `src/lang/mod.rs`, `src/rules/ast_grep/` (verification only; no language-specific changes)

1. Verified: `rules/` currently contains ONLY Rust embedded rules (`rules/rust/*.yml`, zero embedded `language: TypeScript` rules).
2. Custom `ast-grep` YAML rules with `language: Tsx` (or `language: tsx`) resolve via `LanguageRegistry::resolve_language` to `tsx_language_spec()`, compile against `LANGUAGE_TSX`, and scan `.tsx` files.
3. Inline `[[ast_grep.rules]]` with `language = "tsx"` match `.tsx` trees via `tree.language().matches_name("tsx")`.
4. `metadata.skip_tests: true` suppresses matches inside `.test.tsx` / `.spec.tsx` / `__tests__/` files and `describe`/`it`/`test` blocks.
5. `language: TypeScript` targets `.ts`/`.mts`/`.cts` files (matching `ast-grep` CLI semantics where `TypeScript` and `Tsx` are distinct grammars).

### Point 9: Module Line-Budget Plan (<= 599 lines/file), Harness Split & Documentation Correction
Files: `README.md`, `tests/fixture_harness.rs`, `src/lang/typescript/*`, `src/rules/test_coupling/*`

1. **Verified file line budgets (strictly under the 599-line cap of `rules/rust/max-file-lines.yml`):**
   - `src/lang/typescript/grammar.rs` (NEW): ~95 lines (`TsDialectGrammar`, `typescript_language_spec`, `tsx_language_spec`).
   - `src/lang/typescript/conventions.rs` (NEW): ~220 lines (`TypeScriptConventions`, `Default`, file-role / package / asset helpers + convention unit tests).
   - `src/lang/typescript/expr.rs` (NEW): ~150 lines (extract `infer_expr_origin` and expression helpers from `body.rs:448-590`, mirroring `src/lang/swift/expr.rs`).
   - `src/lang/typescript/jsx.rs` (NEW): ~160 lines (`is_intrinsic_jsx_tag`, JSX body walking, `record_jsx_member_call` + JSX unit tests).
   - `src/lang/typescript/props.rs` (NEW): ~220 lines (Pass 1 prop-shape pre-scan, `ExtractedParams`, recursive `object_pattern` -> `LocalBinding` extraction + prop unit tests).
   - `src/lang/typescript/body.rs`: shrinks from 590 lines to **~460 lines** (after extracting `expr.rs`), leaving ~135 lines of headroom.
   - `src/lang/typescript/mod.rs`: ~490 lines (existing 369-line `#[cfg(test)]` block preserved; `TsDialectGrammar` and `TypeScriptConventions` extracted out; re-exports `typescript_language_spec`, `tsx_language_spec`, `TypeScriptConventions`).
   - `src/lang/typescript/extract.rs`: 432 -> ~510 lines.
   - `src/lang/typescript/members.rs`: 315 -> ~340 lines.
   - `src/lang/typescript/normalize.rs`: 225 -> ~320 lines.
   - `src/rules/test_coupling/imports.rs` (NEW): ~180 lines (`resolve_imported_free_funcs`, 4-form barrel traversal, and `ident_targets`).
   - `src/rules/test_coupling/mod.rs`: 432 -> ~460 lines.
   - `src/rules/test_coupling/reachability.rs`: **shrinks from 583 lines to ~555 lines** (replaces inline lookup loops with `self.index.ident_targets(...)`).
   - `src/rules/test_coupling/mocking.rs` (599 lines): **NOT touched**.
   - `src/lang/common.rs` (597 lines): **NOT touched**.
2. **Fixture harness isolation (`tests/fixture_harness.rs`):**
   - Add a dedicated `#[test] fn react_fixtures_match_expectations()` running `tests/fixtures/react/` and exclude `react/` from `multi_file_fixtures_match_expectations()` so red React parity fixtures during TDD never mask regressions in existing Rust/TS/Swift fixtures, while `cargo test` executes both suites.
3. **`README.md` correction:**
   - Correct line 97 (which currently inaccurately claims `.tsx`/`.mts`/`.cts` and full type-alias extraction) to accurately state supported TypeScript/TSX extensions (`.ts`, `.tsx`, `.mts`, `.cts`), file-local prop-shape type alias resolution, `TypeScriptConventions` (including the `external_boundary_packages` allowlist behavior), and that custom `ast-grep` rules targeting JSX files must specify `language: Tsx` (whereas `language: TypeScript` targets `.ts`/`.mts`/`.cts`).

---

## 4. Verification & Parity Test Strategy

All parity tests will be written and frozen (committed locally) **before** implementation begins.

**Compile-clean TDD constraint:** Frozen tests (`tests/react_tsx.rs`, `tests/react_tsx_rules.rs`, `tests/react_tsx_regressions.rs`, and `tests/fixtures/react/*`) MUST drive ONLY currently-existing public APIs (`SourceTree::discover`, `SyntaxTree::parse`, `Analyzer::from_config`, `Analyzer::check_path`, `LanguageRegistry::with_defaults`, `fixture_harness`) and NEVER call not-yet-existing helpers (`tsx_language_spec`, `grammar_for_path`, `TypeScriptConventions`). Unit tests for new internal/public helpers (`TsDialectGrammar`, `TypeScriptConventions`, `grammar_for_path`, `extends React.Component<P, S>`) are added during Stage 4 implementation.

**Valid TypeScript constraint:** Every `.ts` and `.tsx` fixture file MUST be valid TypeScript/TSX (no syntactically broken code or unexported symbol references that `tsc` would reject).

### End-to-End Fixtures (`tests/fixtures/react/`)
1. `clean_react_app`: Clean Vite-style React + TS app (`src/main.tsx`, anonymous `export default () => ...` in `src/App.tsx`, `React.memo`/`forwardRef` components, `React.lazy(() => import('./pages/Orders'))` with `OrdersScreen` default export, `routes.tsx` lazy re-export with `SettingsScreen` default export, AND a routes-array lazy import for anonymous `Billing`, class error boundary extending `React.Component<P, S>` with `componentDidCatch` + `render`, CSS module import `./OrderCard.module.css`, `src/test-utils.tsx` custom `renderWithProviders`, `src/setupTests.ts`, colocated `src/components/__tests__/OrderCard.tsx`, behavioral React Testing Library tests) -> 0 diagnostics across `cohesion`, `test-coupling`, `similarity`. (Note: 4-form barrel re-exports are exercised in `test_coupling` and `barrel_reexports`, since `no-barrel-module` intentionally flags barrel files when `cohesion` is enabled.)
2. `scattered_concept`: `Order` class in `src/models/order.ts` mutated externally by two React components (`src/components/PricingPanel.tsx` using `interface Props` with `{ order, discount = 5 }` default destructuring and `src/components/ShippingPanel.tsx` using `React.FC<{ backupOrder: Order; order?: Order; rate?: number }>` with renamed `{ backupOrder, order: activeOrder = backupOrder, rate = 0.1 }`) -> `scattered-concept` + `external-field-mutation`.
3. `cyclic_modules`: Cross-extension cycle between `src/AlphaHelper.ts` and `src/BetaCard.tsx` -> `cyclic-module-dependency`.
4. `barrel_reexports`: `src/components/index.ts` re-exporting from `src/components/OrderCard.tsx` and `src/models/order.ts` (with decoy `src/legacy/order.ts` preventing unique-name fallback) -> `no-barrel-module` + `external-field-mutation` across `.ts`/`.tsx`.
5. `duplicate_components`: Near-duplicate React components with renamed variables/props and equivalent JSX structure (a `React.memo` wrapped component vs plain function component pair, plus a cross-extension `.ts`/`.tsx` helper pair), PLUS negative cases that must NOT be flagged: distinct components with different tags (`<UserAvatar>` vs `<OrderBadge>`), two form components differing only in JSX attribute names (`placeholder`/`aria-label` vs `title`/`name`), and repetitive `<li>` list boilerplate -> exact `duplicate-function` hits ONLY on the true duplicate pairs.
6. `test_coupling`: Valid TypeScript Vite React app (`src/main.tsx`, `src/components/*`, `src/models/*`, `src/gateways/*`, tests):
   - Valid TS private-API access: test accessing private `VaultService` method via cast (`(vault as any).rawSecret()`) -> `test-calls-private-api`.
   - Component/helper only rendered in tests (`DebugVaultPanel` and an `export default memo(UnusedBanner)` rendered only by tests) -> `test-only-production-api`.
   - Test mocking internal React child component (`OrderListRows.test.tsx` calling `vi.mock('./ChildRow')` where `ChildRow.tsx` imports `'react'`, `'react-router-dom'`, and `./ChildRow.module.css`) + interaction-only assertion -> `test-mocks-internal-collaborator` + `interaction-only-test`.
   - `process.env.NODE_ENV === "test"` branch inside a React component -> `production-test-hook`.
   - Negative cases (must NOT fire):
     - Colocated `OrderList.test.tsx` explicitly importing `import { Row } from '../shared/Row'` while `OrderList.tsx` defines a private `Row` -> NO `test-calls-private-api`.
     - Component wrapped in an arbitrary HOC (`export default observer(ObservedCard)`, hoisted function form) and two-step barrel (`export { default as ObservedCard }` + `import Badge from './TwoStepBadge'; export { Badge }`) imported via `cardsBarrel.ts`, rendered by `App.tsx`, and tested -> NO `test-calls-private-api` and NO `test-only-production-api`.
     - Test mocking an external I/O module (`apiClient.ts` importing `axios`) with behavioral assertion (`expect(...).toBeInTheDocument()`) -> NO diagnostics.
     - Test importing `render` from `@testing-library/react` while `src/utils/renderHelper.ts` defines an unreachable project function `render` -> NO false collision.
7. `overlapping_tests`: Two redundant `.test.tsx` cases rendering the same component with identical structure -> `overlapping-tests`.

### Focused Integration Tests (`tests/react_tsx.rs`, `tests/react_tsx_rules.rs`, `tests/react_tsx_regressions.rs`)
- Extension discovery (`.ts`, `.tsx`, `.mts`, `.cts`, uppercase `.TSX` verified with TSX JSX call extraction) via `SourceTree::discover`, `extract_file_ast`, and `Analyzer::check_path`.
- Intrinsic HTML/custom-element tags (`<main>`, `<form>`, `<input>`, `<div>`, `<header>`, `<summary>`, `<label>`, `<title>`, `<my-widget>`) do NOT create false reachability edges for `test-only-production-api` or hide `test-only-production-field` (while `Item.describe` stays reachable via `<App />`).
- Namespaced/compound JSX components (`<UI.OrderCard />`, `<Card.Header />`, `<Dialog.Root />`, `<ThemeContext.Provider />`, `<Forms.Fields.Input />`) are tracked as calls without recording field accesses or bogus type constructions.
- `React.memo`, `React.forwardRef`, and arbitrary curried HOC (`connect(...)(BannerView)`) wrapped components are analyzed as functions (live component stays clean, test-only component is flagged), and `// rqcode:allow(duplicate-function)` above the multiline `React.memo` primary wrapper suppresses function-level diagnostics.
- Destructured props (`interface Props`, `type Props = { ... }`, intersection / `PropsWithChildren`, inline `{ order }: { order: Order }`, `React.FC<Props>`, `forwardRef<HTMLDivElement, Props>`, renamed `{ order: activeOrder = fallback }`, defaults `{ count = 0 }`, rest `{ className, ...rest }`, multi-param `function readPair({ first, second }: Pair, order: Order)` preserving positional arity with empty placeholder `params[0].name`, nested callback `rows.forEach(({ order }: Row) => ...)`, and parameter default expression walking) plus `useState<Order>` array destructuring resolve local types accurately.
- CRA-style `src/index.tsx` side-effect entrypoint vs library `src/index.tsx` with exports.
- Paired JSX suppression tests: real diagnostics (`external-field-mutation`, `production-test-hook`, and custom `ast-grep` rule `no-danger-html`) are verified to fire WITHOUT suppressions, to be silenced WITH line-level `{/* rqcode:allow(no-danger-html) */}`, `{/* rqcode:allow(production-test-hook) */}`, and `// rqcode:allow(external-field-mutation)`, and to be silenced WITH file-level `{/* rqcode:allow-file(...) */}` inside JSX.
- Custom `ast-grep` YAML rule (`language: Tsx`) and inline `[[ast_grep.rules]]` (`language = "tsx"`) match JSX patterns in `.tsx` files and respect `skip_tests: true`, while `language: TypeScript` rules continue matching `.ts`/`.mts`/`.cts` files without symbol-ID corruption.
- Cross-extension `.ts` + `.tsx` similarity (duplicate helper shared between a `.ts` and a `.tsx` file detected in the same corpus).
- **Cross-language Point 5 regression tests (`tests/react_tsx_regressions.rs`):**
  - Rust inline `#[cfg(test)] mod tests` with `use crate::other::helper;` resolves test call to `src/other.rs` without shadowing production `worker::run() -> helper()` (flagging only `src/other.rs::helper` as `test-only-production-api`).
  - Rust integration test (`tests/integration.rs`) with `use my_crate::api::run;` (including `pub use inner as api;` module alias and a private `shadow::run`) resolves `run` cleanly.
  - Swift multi-file free-function resolution flags only `debugOnlyScore` in `Sources/App/Scorer.swift`.
