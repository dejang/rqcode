# rqcode

**Language-agnostic static analysis and architectural quality gate for coding agents.**

`rqcode` catches structural decay and LLM anti-patterns that compilers and standard linters miss: scattered domain concepts, leaky module boundaries, barrel re-export modules, cyclic module dependencies, low-cohesion types, feature envy, architectural crossings, stateless methods attached to types, tests coupled to private internals, near-duplicate functions, overlapping tests, and redundant forwarding wrappers.

It runs as a fast single-binary CLI (with text and JSON output) or as an embeddable Rust library, and supports **Rust**, **TypeScript**, and **Swift** out of the box through a strictly isolated, injectable language-frontend architecture.

## Why `rqcode`?

Observed failure modes across coding agents today:

- **Duplicate utilities:** Agents forget an existing helper already does the job and write a new one.
- **Additive bias:** Agents prefer writing new parallel code over extending existing abstractions.
- **Implementation-locked tests:** Agents write tests that validate how they reasoned through their implementation rather than the behavior itself. Changing the implementation without changing behavior then forces changing the tests, breaking the trust that tests are supposed to provide as a safe refactoring guardrail.
- **Instruction drift:** Agents selectively apply skills and prompt guidelines over long sessions, so telling an agent to reuse code or follow architecture rules is not a guarantee.
- **Architectural blind spots:** Agents lack a persistent sense of system architecture, often producing tightly coupled modules, leaky boundaries, and circular dependencies.
- **Gate-driven loops:** When a gate exists, agents will hammer it repeatedly (tests, lints, formatters) because passing the gate *is* the goal, which is why deterministic tooling must run in milliseconds.

We cannot build reliable automated software factories, or remove the human verification bottleneck, if we cannot trust the agents writing the code. `rqcode` addresses this by using fast, deterministic checks to steer agents toward higher-quality structure. It does not prove functional correctness directly; instead, it combines lint rules, architectural guardrails, and behavior-focused test checks so refactors and automated changes stay trustworthy.

---

## Installation

### Prebuilt binary (macOS & Linux, x86_64 & arm64)

Install without the Rust/Cargo toolchain:

```bash
curl -fsSL https://github.com/dejang/rqcode/releases/latest/download/install.sh | sh
```

The installer verifies the SHA-256 checksum and places `rqcode` in `~/.local/bin` by default. You can customize the destination or pin a release with environment variables:

```bash
curl -fsSL https://github.com/dejang/rqcode/releases/latest/download/install.sh \
  | RQCODE_INSTALL_DIR=/usr/local/bin RQCODE_VERSION=v0.1.0 sh
```

### Via Cargo (all platforms)

```bash
cargo install rqcode
```

---

## Quick Start

```bash
# Check the current repository
rqcode check .

# Check a specific directory or file with JSON output
rqcode check path/to/project --format json

# Treat warnings as gate failures and use an explicit config file
rqcode check path/to/project --config custom-rqcode.toml --fail-on-warning

# List all active rules (core + embedded + configured custom rules)
rqcode rules
```

### Exit Codes

| Code | Meaning |
| ---- | ------- |
| `0`  | Passed (no errors, and no warnings when `--fail-on-warning` is set) |
| `1`  | Quality gate failed (one or more diagnostics violated the gate) |
| `2`  | Configuration, rule-loading, or I/O error |

---

## Architecture

`rqcode` runs a deterministic, parallel (`rayon`) pipeline:

```text
SourceTree ──▶ LanguageFrontend ──▶ SyntaxTree + FileAst ──▶ Codebase ──▶ RuleRegistry ──▶ Report
  (ignore)      (src/lang/<lang>)     (CST + semantic IR)     (resolved     (core +         (text /
                                                               graph)        ast-grep)       JSON)
```

### Strict Language Isolation

All language-specific logic is confined to `src/lang/<language>/` and injected via two traits:

- **`LanguageGrammar`** (`src/ast/language.rs`): supplies the tree-sitter grammar (`ast-grep`), language ID/aliases, file extensions, `CommentStyle` (for suppression parsing), member display separator (`::` for Rust, `.` for TypeScript and Swift), and field-initializer wording.
- **`LanguageFrontend`** (`src/lang/mod.rs`): extracts a normalized `FileAst` (types, fields, functions, calls, imports, re-exports, normalized `NormalizedTree` body ASTs, `FileRole`, `ModulePath`) and provides language-specific default excludes (e.g., `target` for Rust, `node_modules`/`dist` for TypeScript, `.build`/`.swiftpm`/`DerivedData`/`Pods`/`Carthage` for Swift).

`src/ast/` and `src/rules/` are strictly language-agnostic: they never import `ast_grep_language` or any concrete frontend. This invariant and end-to-end custom-language injection are continuously verified by `tests/architecture_guardrails.rs`.

### Built-in Frontends

- **Rust** (`src/lang/rust/`): `.rs` - structs, enums, traits, inherent/trait `impl`s, `#[cfg(test)]` modules, macro token extraction, `use` / `pub use` re-exports, crate/module path resolution.
- **TypeScript & React/TSX** (`src/lang/typescript/`): `.ts`, `.tsx`, `.mts`, `.cts` - classes (including `React.Component`/`PureComponent` lifecycle methods), interfaces, file-local prop-shape type alias resolution (`PropsWithChildren`, `Readonly`, `Partial`, `Required`, intersections), destructured props & `useState<T>` hooks, `React.memo`/`forwardRef`/`React.lazy`/HOC components, JSX tag/component extraction and CST normalization, parameter properties, decorators, constructors/static/instance methods, ES imports, relative asset imports (`.css`, `.svg`, etc.), default/namespace/barrel re-exports (`export *`, `export * as ns`, import-then-export), and `describe`/`it`/`test` suites. Conventions are injectable via `TypeScriptConventions`; third-party I/O SDKs outside the default `external_boundary_packages` allowlist (e.g., `openai`, `@octokit/rest`, `@apollo/client`) default to `StandardValue` unless added via `TypeScriptConventions::external_boundary_packages` (ambient `fetch`/`WebSocket`/`localStorage` calls remain automatically detected as external boundaries).
- **Swift** (`src/lang/swift/`): `.swift` - structs, classes, actors, enums, protocols, inherent vs. protocol-conformance `extension`s, `init`/`deinit`, stored & computed properties, access levels (`open`/`public`/`internal`/`package`/`fileprivate`/`private`/`private(set)`), attributes (`@MainActor`, `@main`), `inout` parameters, `@testable`/`@_exported`/scoped imports, SwiftPM & Xcode target layouts, and `XCTestCase` / Swift Testing (`@Suite`, `@Test`) suites.

---

## Rules

### Core Polyglot Rules (`src/rules/`)

Core rules operate exclusively on the resolved language-agnostic `Codebase` graph and work identically across Rust, TypeScript, Swift, and any custom `LanguageFrontend`.

#### 1. Cohesion & Module Boundaries (`cohesion`)
| Diagnostic ID | What it catches |
| ------------- | --------------- |
| `scattered-concept` | Stateful types manipulated by free functions scattered across multiple external modules instead of cohesive methods on the type |
| `external-field-mutation` | Direct mutation of a type's fields from outside its defining module |
| `split-impl` | Inherent methods for a type split across multiple modules |
| `scattered-tests` | Unit tests for a type scattered outside its defining module |
| `cyclic-module-dependency` | Circular import and re-export dependencies between modules (reported as SCC cycles) |
| `leaky-module-boundary` | Parent/child or sibling modules reaching directly into another module's internal fields/types (e.g., mutable-reference bag structs) |
| `stateless-impl-method` | Methods placed on a type that take no instance receiver (`self`/`this`) and do not return the owner type (constructors/factories returning `Self` are allowed) |
| `no-barrel-module` | Re-export barrel files (`index.ts` or pure re-export files) that obscure module boundaries and inflate transitive coupling (error by default; opt-in root exemption via `allow_library_root_barrel`) |
| `low-cohesion-type` | Stateful types splitting into multiple disjoint behavioral clusters (`TCC` + connected components + TF-IDF conceptual dissimilarity; advisory warning by default) |
| `feature-envy` | Methods or functions accessing another behavioral type's members far more than their own (`ATFD`, `FDP`, `LAA` + TF-IDF conceptual affinity; advisory warning by default) |
| `architectural-crossing` | Non-entrypoint, non-barrel modules sitting simultaneously in the upper quantile of both fan-in and fan-out within their language graph (advisory warning by default) |

See [docs/cohesion.md](docs/cohesion.md) for the literature survey, formal definitions, per-language mappings, false-positive boundaries, and design rationale.

#### 2. Test Coupling (`test-coupling`)
| Diagnostic ID | What it catches |
| ------------- | --------------- |
| `test-only-production-api` | Production functions/methods unreachable from production roots and referenced only by tests (configurable via `public_api_scope`) |
| `test-only-production-field` | Production fields written in production but read only by tests |
| `test-calls-private-api` | Tests directly invoking private functions or private methods instead of testing through the public surface |
| `test-accesses-private-field` | Tests directly reading, mutating, or initializing private fields |
| `test-bypasses-public-surface` | Tests importing, calling, or accessing internal symbols past a package's public surface (`lib.rs`, barrel `index.ts`, Swift `@testable`) |
| `overexposed-for-tests` | Internal APIs widened explicitly solely for cross-module test callers |
| `production-test-hook` | Production conditionals or hooks guarded for test/debug environments (`#[cfg(test)]`, `NODE_ENV === "test"`, `#if DEBUG`) |
| `test-encapsulation-escape` | Tests using runtime reflection (`Mirror`, `Reflect.get`), `as any`/bracket escapes, or subclassing closed concrete production classes |
| `test-mocks-internal-collaborator` | Tests mocking internal project modules (exempting external I/O boundaries and re-export barrels wrapping them) |
| `test-spies-on-sut` | Tests spying on private or `this`/`self`-reachable internal methods of the system under test |
| `interaction-only-test` | Tests relying on internal mocks or SUT spies that assert only call interactions with zero behavioral assertions |
| `mock-only-abstraction` | Internal interfaces/traits/protocols with at most one production implementation created solely for test doubles |
| `test-asserts-internal-representation` | Exact-equality assertions on derived debug representations that expose private fields |

See [docs/test_coupling.md](docs/test_coupling.md) for the rule taxonomy, three-phase analysis pipeline, per-language mappings, false-positive boundaries, and design rationale.

Known polyglot limitation: `public_api_scope = "binary-only"` evaluates binary-only package detection globally across the scanned repository, so a mixed repository containing both a library root (for example `src/lib.rs`) and a binary entrypoint in another language (for example `src/main.tsx`) disables binary-only public-API checking globally unless `public_api_scope = "all"` is set.

#### 3. Structural Similarity (`similarity`)
| Diagnostic ID | What it catches |
| ------------- | --------------- |
| `duplicate-function` | Production functions with near-identical normalized AST structure (4-stage pipeline: sibling-folded boilerplate & structural diversity filter, callee/operator-preserving `NormalizedTree` with consistent parameterized renaming, prefix-filtered Merkle subtree Jaccard retrieval, and weighted Zhang-Shasha Tree Edit Distance verification) |
| `overlapping-tests` | Redundant tests exercising largely the same operations |

See [docs/similarity.md](docs/similarity.md) for the literature survey, crates/algorithms evaluated, pipeline architecture, and design rationale.

### Embedded & Custom `ast-grep` YAML Rules (`rules/`)

`build.rs` embeds all YAML rules under `rules/` directly into the binary at compile time. You can layer project-specific YAML rules on top via `ast_grep.rules_dir` (same rule ID overrides the built-in) or disable individual/all built-ins. For custom TypeScript rules, specify `language: Tsx` (or `language = "tsx"`) to target `.tsx` JSX files and `language: TypeScript` to target `.ts`/`.mts`/`.cts` files.

Built-in Rust YAML rules (`rules/rust/*.yml`):

- **Wrappers & indirection:** `no-redundant-wrapper`, `no-private-forwarder`, `single-use-private-helper`, `no-free-function-with-local-type`
- **Configuration & constants:** `module-const`, `local-const`, `impl-const`, `const-fn-config`
- **Code structure & safety:** `max-file-lines` (600 lines), `too-many-params`, `unwrap-or-expect`, `float-equality-literal`, `no-pub-in-main`, `cfg-test-on-non-mod`
- **Performance:** `collect-in-loop`, `linear-scan-in-loop`
- **Hygiene & unfinished code:** `no-todo-macro`, `no-unimplemented-macro`, `no-dbg-macro`, `allow-clippy-lint`, `allow-unused-or-dead-code`, `no-doc-attribute`

---

## Configuration (`rqcode.toml`)

`rqcode` automatically looks for `rqcode.toml` in the target directory (or the parent directory when checking a single file). All fields are optional and fall back to sensible defaults:

```toml
# Additional paths/globs to exclude (merged with each language frontend's default excludes)
exclude = ["target", ".git", "tests/fixtures"]

[similarity]
enabled = true
function_threshold = 0.85
test_threshold = 0.88
min_tokens = 25
min_distinct_kinds = 4
severity = "error" # "error" | "warning"

[cohesion]
enabled = true
min_external_functions = 2
min_external_modules = 2
flag_external_field_mutation = true
flag_test_scatter = true
severity = "error"
advisory_severity = "warning"

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

[test_coupling]
enabled = true
flag_test_only_production_api = true
flag_test_only_production_field = true
flag_private_test_coupling = true
flag_test_bypasses_public_surface = true
flag_overexposed_for_tests = true
flag_production_test_hook = true
flag_test_encapsulation_escape = true
flag_test_mocks_internal_collaborator = true
flag_test_spies_on_sut = true
flag_interaction_only_test = true
flag_mock_only_abstraction = true
flag_test_asserts_internal_representation = true
public_api_scope = "binary-only" # "binary-only" | "all"
severity = "error"

[ast_grep]
enabled = true
builtin_rules = true
rules_dir = "custom-rules" # resolved relative to rqcode.toml
disabled_rules = ["no-doc-attribute"]

# Optional inline ast-grep pattern rules
[[ast_grep.rules]]
id = "no-eprintln"
language = "rust"
pattern = "eprintln!($$$ARGS)"
message = "Avoid eprintln! in library code"
suggestion = "Use structured diagnostics or logging instead."
severity = "warning"
skip_tests = true
```

---

## Inline Suppressions

Diagnostics can be suppressed using the target language's comment syntax (configured via `CommentStyle`):

- **Same line or next line:** `// rqcode:allow(rule-id)` (or comma-separated IDs / `*` wildcard)
- **Whole file:** `// rqcode:allow-file(rule-id)` (or `//! rqcode:allow(rule-id)` in Rust)

---

## Library Usage

`rqcode` can be embedded directly in tooling or extended with custom `LanguageFrontend` and `Rule` implementations:

```rust
use rqcode::Analyzer;
use rqcode::config::Config;
use std::path::Path;

let root = Path::new(".");
let config = Config::load_from_dir(root)?;
let analyzer = Analyzer::from_config(&config, root)?;
let report = analyzer.check_path(root, &config)?;

print!("{}", report.render_text());
assert!(report.is_clean());
```

To plug in a custom language without modifying core analysis code, register your `LanguageFrontend` into a `LanguageRegistry`, build a `RuleRegistry::from_config(&config, root, &languages)?`, and construct `Analyzer::new(languages, rules)`.

---

## Development & Self-Check

`rqcode` enforces strict clippy pedantic lints (`Cargo.toml`) and dogfoods its own quality gate as part of the integration test suite (`tests/integration.rs::rqcode_passes_its_own_quality_gate`):

```bash
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- check .
```

---

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
