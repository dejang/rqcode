# rqcode

**Language-agnostic static analysis and architectural quality gate for coding agents.**

`rqcode` catches structural decay and LLM anti-patterns that compilers and standard linters miss: scattered domain concepts, leaky module boundaries, cyclic module dependencies, stateless methods attached to types, tests coupled to private internals, near-duplicate functions, overlapping tests, and redundant forwarding wrappers.

It runs as a fast single-binary CLI (with text and JSON output) or as an embeddable Rust library, and supports both **Rust** and **TypeScript** out of the box through a strictly isolated, injectable language-frontend architecture.

---

## Installation

### Prebuilt binary (macOS — Apple Silicon & Intel)

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

- **`LanguageGrammar`** (`src/ast/language.rs`): supplies the tree-sitter grammar (`ast-grep`), language ID/aliases, file extensions, `CommentStyle` (for suppression parsing), member display separator (`::` for Rust, `.` for TypeScript), and field-initializer wording.
- **`LanguageFrontend`** (`src/lang/mod.rs`): extracts a normalized `FileAst` (types, fields, functions, calls, imports, re-exports, normalized `NormToken` body streams, `FileRole`, `ModulePath`) and provides language-specific default excludes (e.g., `target` for Rust, `node_modules`/`dist` for TypeScript).

`src/ast/` and `src/rules/` are strictly language-agnostic—they never import `ast_grep_language` or any concrete frontend. This invariant and end-to-end custom-language injection are continuously verified by `tests/architecture_guardrails.rs`.

### Built-in Frontends

- **Rust** (`src/lang/rust/`): `.rs` — structs, enums, traits, inherent/trait `impl`s, `#[cfg(test)]` modules, macro token extraction, `use` / `pub use` re-exports, crate/module path resolution.
- **TypeScript** (`src/lang/typescript/`): `.ts`, `.tsx`, `.mts`, `.cts` — classes, interfaces, type aliases, parameter properties, decorators, constructors/static/instance methods, ES imports, default/namespace/barrel re-exports (`export *`, `export * as ns`, import-then-export), and `describe`/`it`/`test` suites.

---

## Rules

### Core Polyglot Rules (`src/rules/`)

Core rules operate exclusively on the resolved language-agnostic `Codebase` graph and work identically across Rust, TypeScript, and any custom `LanguageFrontend`.

#### 1. Cohesion & Module Boundaries (`cohesion`)
| Diagnostic ID | What it catches |
| ------------- | --------------- |
| `scattered-concept` | Stateful types manipulated by free functions scattered across multiple external modules instead of cohesive methods on the type |
| `external-field-mutation` | Direct mutation of a type's fields from outside its defining module |
| `split-impl` | Inherent methods for a type split across multiple modules |
| `scattered-tests` | Unit tests for a type scattered outside its defining module |
| `cyclic-module-dependency` | Circular import dependencies between modules (reported as SCC cycles) |
| `leaky-module-boundary` | Parent/child or sibling modules reaching directly into another module's internal fields/types (e.g., mutable-reference bag structs) |
| `stateless-impl-method` | Methods placed on a type that take no instance receiver (`self`/`this`) and do not return the owner type (constructors/factories returning `Self` are allowed) |

#### 2. Test Coupling (`test-coupling`)
| Diagnostic ID | What it catches |
| ------------- | --------------- |
| `test-only-production-api` | Production functions/methods that are only referenced by tests (configurable via `public_api_scope`) |
| `test-calls-private-api` | Tests directly invoking private functions or private methods instead of testing through the public surface |
| `test-accesses-private-field` | Tests directly reading, mutating, or initializing private fields |

#### 3. Structural Similarity (`similarity`)
| Diagnostic ID | What it catches |
| ------------- | --------------- |
| `duplicate-function` | Production functions with near-identical normalized AST structure (TF-IDF n-gram cosine similarity over canonical `NormToken` streams; invariant to variable renaming) |
| `overlapping-tests` | Redundant tests exercising largely the same operations |

### Embedded & Custom `ast-grep` YAML Rules (`rules/`)

`build.rs` embeds all YAML rules under `rules/` directly into the binary at compile time. You can layer project-specific YAML rules on top via `ast_grep.rules_dir` (same rule ID overrides the built-in) or disable individual/all built-ins.

Built-in Rust YAML rules (`rules/rust/*.yml`):

- **Wrappers & indirection:** `no-redundant-wrapper`, `no-private-forwarder`, `single-use-private-helper`, `no-free-function-with-local-type`
- **Configuration & constants:** `module-const`, `local-const`, `const-fn-config`
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
ngram_size = 3
severity = "error" # "error" | "warning"

[cohesion]
enabled = true
min_external_functions = 2
min_external_modules = 2
flag_external_field_mutation = true
flag_test_scatter = true
severity = "error"

[cohesion.boundaries]
flag_cyclic_modules = true
flag_leaky_boundaries = true
flag_stateless_impl_methods = true

[test_coupling]
enabled = true
flag_test_only_production_api = true
flag_private_test_coupling = true
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
