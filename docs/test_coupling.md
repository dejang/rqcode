# Test–Implementation Coupling Architecture & Design Rationale

This document explains **why** `rqcode`'s `test-coupling` rule family works the way it does: what test-engineering literature and empirical anti-patterns establish about tests coupled to implementation details, why visibility-only or regex linting fails, how the three-phase polyglot analysis pipeline operates across Rust, TypeScript, and Swift, and which patterns are deliberately excluded to preserve high precision.

---

## 1. Problem Statement, Constraints & Why Naive Approaches Fail

`rqcode` is a zero-build, offline, single-binary polyglot quality gate (`Rust`, `TypeScript`, `Swift`, plus injectable custom `LanguageFrontend`s). Its test-coupling engine must satisfy five constraints:

1. **Zero compiler/build requirement:** Analyze whole repositories directly from `tree-sitter` CSTs (`ast-grep-core`) without `rustc`, `tsc`, or `swiftc`.
2. **Strict language isolation:** `src/ast/` and `src/rules/` must remain 100% language-agnostic (enforced by `tests/architecture_guardrails.rs`). No rule may inspect syntax strings, file extensions, or language names.
3. **Sub-second latency:** Whole-repository coupling analysis (including reachability, public-surface fixed points, and mocking boundary propagation) must complete in milliseconds.
4. **Precision over recall on ambiguous idioms:** Legitimate testing practices—callback dependency injection, external I/O boundary mocks, pure DTOs, public extension points, and debug-only diagnostics—must never trigger false positives.
5. **Bidirectional coupling coverage:** Detect *both* directions of coupling:
   - **Tests reaching into internals** (private APIs/fields, bypassing package surfaces, reflection/subclass escapes, spying on the SUT, mocking internal collaborators, asserting derived debug layouts).
   - **Production reshaped for tests** (test-only production APIs/fields, APIs widened exclusively for tests, `#if DEBUG` / `NODE_ENV === "test"` / `#[cfg(test)]` hooks, single-implementation mock-only abstractions).

### Why Visibility-Only and Regex Linters Fail

Traditional test-smell linters rely on local syntax patterns or raw declaration visibility. That approach breaks down in four ways:

- **Declaration visibility $\neq$ effective public surface:** A TypeScript `export function helper()` in `src/internal/helper.ts` that is *not* re-exported from `src/index.ts`, or a Rust `pub fn` inside a private `mod internal`, looks "public" locally even though external consumers cannot reach it. Conversely, Swift `@testable import` lets external test targets invoke `internal` APIs that are invisible to production callers.
- **Single-hop caller checks miss helper islands:** Production code reshaped for tests often forms multi-hop islands (`test -> reset_for_tests -> clear_cache -> reset_counters`). Checking only direct callers flags `reset_for_tests` while missing the private/internal helpers beneath it—or misclassifies a helper called only by a dead test-only root as "used by production."
- **Naive mock bans break real integration boundaries:** Flagging every `vi.mock`, `jest.mock`, or `spyOn` destroys legitimate mocking of external I/O boundaries (`fetch`, databases, `std::fs`, `URLSession`, third-party SDKs) and fault injection on injected collaborators.
- **Syntactic escape hatches slip past member-visibility checks:** TypeScript `(sut as any).secret`, `sut["secret"]`, `vi.spyOn(SUT.prototype, "step")`, Swift `Mirror(reflecting: sut)`, test-only subclasses of closed concrete classes, and Rust `assert_eq!(format!("{:?}", val), "...")` on derived `Debug` structs all bypass visibility without ever triggering a naive private-member check.

---

## 2. Conceptual Model & 13-Rule Taxonomy

Good tests verify **observable behavior through the system's public contract** (Khorikov's *resistance to refactoring*; Winters, Manshreck, & Wright's *test behavior, not implementation*; Meszaros's *overspecified software* and *test logic in production* smells). Coupling occurs whenever a refactor that preserves external behavior breaks tests, or whenever production code carries structure whose sole consumer is the test suite.

`rqcode` organizes detection into **13 diagnostic IDs** built incrementally across three architectural phases:

| Phase | Diagnostic ID | Direction | Coupling Signal | Key Precision Guards & Exemptions |
| :--- | :--- | :--- | :--- | :--- |
| **1** | `test-calls-private-api` | Test $\to$ Prod | Test or test helper directly calls a `Visibility::Private` production function/method | Requires unambiguous single resolved target; skips trait/interface impls of public traits |
| **1** | `test-accesses-private-field` | Test $\to$ Prod | Test reads, mutates, or initializes (struct literal / destructure) a private production field | Exempts public fields and pure DTOs; covers test helpers and chained destructuring |
| **1** | `test-only-production-api` | Prod $\to$ Test | Production function/method is unreachable from any production root and exercised only by tests | Fixed-point reachability from production roots; `public_api_scope` (`binary-only` default vs. `all`); exempts public APIs in libraries, trait/interface impls, and entrypoints |
| **1** | `test-only-production-field` | Prod $\to$ Test | Production field written in production but read *only* by tests (never read in production) | Exempts `#[derive(Debug)]` / implicit field readers and public fields in library crates |
| **1** | `test-encapsulation-escape` | Test $\to$ Prod | Test uses reflection (`Mirror`, `Reflect.get`), bracket/`as any` escapes, or subclasses a closed concrete production class | Exempts unresolved/external reflection targets, `abstract`/`open` base classes, and concrete classes that already have production subclasses |
| **1** | `production-test-hook` | Prod $\to$ Test | Production code branches on test-only guards (`#[cfg(test)]`, `NODE_ENV === "test"`, `#if DEBUG` / `#if !DEBUG ... #else` hooks) | Exempts `*.config.*` files, compound/unrelated guards (`DEBUG_MODE`, `DEBUG \|\| BETA`), and non-test dev logging |
| **2** | `test-bypasses-public-surface` | Test $\to$ Prod | Test imports, calls, or accesses fields on package-internal symbols past an enclosing public surface (barrel, `lib.rs`, `@testable import`) | Only activates when a language partition defines a public surface; exempts same-module production collaborators and types returned by public factories |
| **2** | `overexposed-for-tests` | Prod $\to$ Test | Production item has explicit widened visibility (e.g., `pub(crate)` / `public` / `export`) whose *only* cross-module callers are tests | Skips items already flagged as `test-only-production-api`, public library surface APIs, and overloaded Swift initializers |
| **3** | `test-mocks-internal-collaborator` | Test $\to$ Prod | Test mocks an internal project module (`vi.mock`, `vi.doMock`, `jest.mock`, `jest.unstable_mockModule`, `vi.mock(import(...))`) | Exempts external libraries (`ImportOrigin::ExternalBoundary`), modules with external imports or ambient I/O (`fetch`, `std::fs`, `URLSession`), and re-export barrels wrapping external boundaries |
| **3** | `test-spies-on-sut` | Test $\to$ Prod | Test spies on a private method or a `this`/`self`-reachable internal method of the system under test (including `as any` and `Prototype` spies) | Exempts external boundary types and fault-injection spies on injected collaborators not reached via self-calls on the spied receiver |
| **3** | `interaction-only-test` | Test $\to$ Prod | Test using internal mocks or SUT spies has $\ge 1$ interaction assertion (`toHaveBeenCalled*`, etc.) and $0$ behavioral assertions | Requires positive coupling signal (exempts callback DI tests); propagates behavioral assertions across same-file test helpers via BFS; counts `expect.soft`, `expect.poll`, Chai, and `assert.*` |
| **3** | `mock-only-abstraction` | Prod $\to$ Test | Internal interface/trait/protocol has $\le 1$ production implementation and $\ge 1$ test-only fake/mock implementation | Exempts public extension points, sealed traits/interfaces (`effective_vis(base) < effective_vis(ty)`), external-boundary abstractions, blanket/unresolved sole impls, and transitively counts subclasses + distinct stdlib impls |
| **3** | `test-asserts-internal-representation` | Test $\to$ Prod | Test asserts exact equality on derived debug formatting (`assert_eq!(format!("{:?}", val), "...")`) of a type with private fields | Exempts custom `Debug` impls (prod-side only), `assert_ne!`, failure message args, both-sides `format!`, chained calls (`.contains`), compound expressions, and types with no private fields |

---

## 3. Architecture & Analysis Pipeline

All 13 rules execute inside a single `TestCouplingRule` pass over the bound, language-agnostic `Codebase`:

```text
  tree-sitter CST (per source file)
         │
         ▼
  Stage 1: Language Frontends (src/lang/{rust,typescript,swift}/)
    Extract FileAst: types, fields, visibility, imports/exports, interface_impls,
    calls, field accesses/inits, test_hooks, module_mocks, FunctionCouplingFacts
         │
         ▼
  Stage 2: Language-Agnostic Binder (src/ast/codebase/bind.rs)
    Resolve call targets, field accesses/inits, interface impls (resolved + target_type_name),
    base classes, encapsulation escape targets, and repr-assertion targets
         │
         ▼
  Stage 3: Effective Public Surface (src/ast/codebase/surface_build.rs, surface.rs)
    Per-language surface detection (crate roots, outermost barrels, Swift targets)
    + transitive visibility propagation (re-exports, public factory return types)
         │
         ▼
  Stage 4: TestCouplingIndex + 7 Focused Analyzers (src/rules/test_coupling/)
    1. private_access.rs  -> test-calls-private-api, test-accesses-private-field, CallerMap
    2. reachability.rs    -> fixed-point prod-root BFS -> test-only-production-{api,field}
    3. surface.rs         -> test-bypasses-public-surface, overexposed-for-tests
    4. hooks.rs           -> production-test-hook
    5. escapes.rs         -> test-encapsulation-escape
    6. mocking.rs         -> test-mocks-internal-collaborator, test-spies-on-sut,
                             interaction-only-test, mock-only-abstraction
    7. representation.rs  -> test-asserts-internal-representation
```

### Stage 1 & 2: Language-Agnostic IR & Resolution (`src/ast/item.rs`, `src/ast/codebase/bind.rs`)

Rather than letting rules inspect AST nodes, frontends distill coupling-relevant facts into plain IR structs:

- **`FunctionCouplingFacts`:** Captures per-function `spy_sites`, `encapsulation_escapes`, `repr_assertions`, `test_hooks`, `interaction_assertions`, `behavioral_assertions`, and `has_ambient_io` (e.g., `fetch`, `WebSocket`, `localStorage`, `std::fs`/`std::net`/`std::process`/`std::env`, `URLSession`, `FileManager`, `UserDefaults`, `Process`).
- **`ModuleMock`:** Stores the mocked specifier, resolved internal `ModulePath` (if any), and language-agnostic **`ImportOrigin`** (`Project`, `StandardValue`, `ExternalBoundary`) so `mocking.rs` never hardcodes language-specific path prefixes (`@/`, `~/`, `#`, `std::`).
- **`InterfaceImpl` & `ResolvedInterfaceImpl`:** Tracks both the resolved `target_id: Option<TypeId>` *and* canonical `target_type_name: String` (plus optional `module_path` override and `InterfaceImplRole::CustomDebugRepr`). Separating resolved vs. unresolved deduplication ensures distinct non-project implementations (such as `impl Codec for String` and `impl Codec for PathBuf`) never collapse into a single count, while `interface_id` is filtered strictly to `TypeKind::Interface | TypeKind::Class`.
- **`ConditionalGuard`:** Normalizes language-specific conditional compilation (`#[cfg(test)]`, `#if DEBUG`, `#if !DEBUG ... #else`) into `None | TestOnly | DebugOnly`.

### Stage 3: Effective Public Surface (`src/ast/codebase/surface_build.rs`, `src/ast/codebase/surface.rs`)

Effective visibility (`EffectiveVisibility`: `Private < Internal < Public`) is computed **independently per `LanguageId`** so a Rust `src/lib.rs` in a polyglot repo never classifies TypeScript files without a barrel as internal:

1. **Boundary discovery:** Identifies library roots (`lib.rs`), binary entrypoints (`main.rs`, CLI roots), TypeScript **outermost re-exporting barrels** (ignoring non-re-exporting `index.ts` files and keeping nested barrels subordinate to the outermost boundary), and Swift target boundaries (`Sources/<Target>` vs. `Tests/<Target>Tests`). Hybrid Rust crates containing both `src/lib.rs` and `src/main.rs` isolate binary-only modules from the library surface.
2. **Transitive exposure:** Propagates `Public` visibility through `pub use` / `export { ... }` / `export *` / two-step namespace re-exports (`import * as ns; export { ns }`), without letting TypeScript `export *` accidentally expose default exports.
3. **Public-contract type closure:** Types returned from or accepted by effectively public factories/APIs become effectively public (`EffectiveVisibility::Public`), preventing false positives when tests exercise an object obtained through a public factory.
4. **Sealed interface detection:** Because `EffectiveVisibility` is totally ordered (`Private < Internal < Public`), any interface/trait inheriting from a super-interface with strictly lower effective visibility (`effective_type_visibility(base) < effective_type_visibility(ty)`) is recognized as sealed—working uniformly in both library and binary-only crates.

### Stage 4: Core Algorithms Across the 7 Rule Modules

- **Production-root fixed-point reachability (`reachability.rs`):** Seeds a worklist with true production roots—entrypoints, effectively public library APIs (under `PublicApiScope::BinaryOnly`), and external interface/trait implementations (evaluated *before* `DebugOnly` suppression so production entrypoints are never dropped)—then propagates reachability through resolved call edges across production functions. Any production function outside the reachable set that has direct test callers is flagged as `test-only-production-api`, cleanly catching multi-hop test-only helper islands.
- **External-boundary fixed-point propagation (`mocking.rs`):** Marks any production file containing an `ImportOrigin::ExternalBoundary` import or non-test `has_ambient_io` as an external boundary, then runs a fixed-point closure through re-export barrels (`src/gateway/index.ts` re-exporting `src/gateway/http.ts`). This ensures mocking a gateway barrel is exempted just like mocking the leaf HTTP client, while mocking pure domain modules (`./pricing`, `./tax`) is flagged.
- **SUT spy reachability (`mocking.rs`):** Distinguishes coupling spies from collaborator fault injection. A `spyOn(target, method)` is flagged only if `target` is a non-external-boundary production type *and* either (a) `method` is non-public (`as any` escape) or (b) the test invokes another method on that same receiver (or class for `Prototype` spies) that transitively reaches `method` via `this`/`self` calls.
- **Helper-propagated assertion counting (`mocking.rs`):** For `interaction-only-test`, behavioral assertions (`expect(x).toBe(...)`, `expect.soft(...)`, `expect.poll(...)`, Chai `.to`/`.be`, `assert.*(...)`) propagate backward across same-file test helpers via BFS. A test is flagged only when it has interaction assertions, zero direct or transitive behavioral assertions, **and** a positive coupling signal (an internal module mock in the file or a SUT spy in the test).
- **Transitive implementation counting (`mocking.rs`):** For `mock-only-abstraction`, production implementations of an interface/trait are closed transitively over production class subclasses (`BaseRepo implements Repo`, `CachedRepo extends BaseRepo` $\implies 2$ production implementations), combined with distinct unresolved non-project `target_type_name`s (skipping sole unresolved/blanket generic targets like `impl<F: Fn()> Handler for F`).

---

## 4. Per-Language Semantics Without Leaking Language Into Core

Each frontend translates language-specific idioms into the shared IR while `tests/architecture_guardrails.rs` guarantees zero cross-contamination into `src/ast/` or `src/rules/`:

| Concept | Rust (`src/lang/rust/`) | TypeScript (`src/lang/typescript/`) | Swift (`src/lang/swift/`) |
| :--- | :--- | :--- | :--- |
| **Public surface boundary** | `src/lib.rs` (library) vs. `src/main.rs` (binary), `pub use` chains, `pub(crate)`/`pub(super)`/`pub(self)` | Outermost re-exporting barrel (`index.ts`), `export { ... }`, `export *`, namespace re-exports | `Sources/<Target>` `public`/`open` vs. `internal` accessed via `@testable import` |
| **Test code & helpers** | `#[test]`, `#[cfg(test)] mod`, item-level `#[cfg(test)]` on structs/traits/impls/fns, `tests/*.rs` | `*.test.*`, `*.spec.*`, `__tests__/**`, `test`/`it`/`describe` callbacks + helper functions | `Tests/**`, `*Tests.swift`, `XCTestCase` methods (`test*`), Swift Testing `@Test` |
| **Test hooks & guards** | `#[cfg(test)]` items/branches inside production files | `process.env?.NODE_ENV === "test"`, `import.meta.env?.MODE === "test"` (skips `*.config.*`) | `DebugGuardStack` tracking `#if DEBUG` and `#if !DEBUG ... #else` (rejects `DEBUG_MODE`, `DEBUG \|\| BETA`), propagated into types, extensions, methods, `init`, `deinit`, `subscript` |
| **Encapsulation escapes** | Private field reads/writes/struct literals/destructuring in same-module `#[cfg(test)]` | `(sut as any).priv`, `sut["priv"]`, `Reflect.get`, test subclasses of non-abstract concrete classes | `Mirror(reflecting: ...)` / `Swift.Mirror(reflecting: ...)` (unwrapping `!`, `?`, parens, `self.prop`), test subclasses of closed classes |
| **Mocking & spying** | Trait fakes (`impl super::Trait for Fake` normalized via `normalize_rust_type_ref`) | `vi.mock`, `vi.doMock`, `jest.mock`, `jest.doMock`, `jest.unstable_mockModule`, `vi.mock(import(...))`, `vi.spyOn`/`jest.spyOn` (unwrapping `as`/`satisfies`/`!`/parens/`Prototype`) | Extension conformances (`extension Live: Proto` via `inherits_from`), test protocol fakes |
| **Ambient I/O & value types** | `std::{fs,net,process,env}` $\to$ ambient I/O; `Duration`, `IpAddr`, `SocketAddr`, `io::{Result,Error,ErrorKind,Cursor}` $\to$ `StandardValue` | `fetch`, `WebSocket`, `XMLHttpRequest`, `localStorage`, `sessionStorage`, `indexedDB` $\to$ ambient I/O; `lodash`, `zod`, `node:path` $\to$ `StandardValue`; `@/`, `~/`, `#` $\to$ `Project` | `URLSession`, `FileManager`, `UserDefaults`, `Process` $\to$ ambient I/O; Foundation/SwiftUI value types $\to$ `StandardValue` |
| **Representation assertions** | `assert_eq!` / `debug_assert_eq!` with standalone `format!("{:?}", x)` / `{0:?}` / positional mapping (ignores `assert_ne!`, msg args, chained calls, compound exprs) | *(Not applicable; no derived `Debug` macro)* | *(Not applicable; `Mirror` covered under escapes)* |

---

## 5. Deliberate Non-Goals, False-Positive Boundaries & Deferred Ideas

Every false-positive boundary below was validated against the 19-case benchmark suite and dual independent council reviews:

1. **Callback dependency injection is never flagged as `interaction-only-test`:** Tests like `button.click(onClick); expect(onClick).toHaveBeenCalledOnce()` have no internal module mock and no SUT spy; verifying that a public callback parameter was invoked *is* testing observable public behavior.
2. **External I/O boundaries are exempt from mocking & mock-only-abstraction rules:** Mocking HTTP gateways, filesystems, databases, or third-party SDKs—even through a re-export barrel—is healthy test isolation, not internal coupling.
3. **Public extension points and sealed traits are exempt from `mock-only-abstraction`:** A `pub trait` in a library crate exists for downstream implementors, and a sealed trait (`pub trait Api: private::Sealed`) exists for API evolution/abstraction safety, not for test mocking.
4. **Concrete base classes with production subclasses are exempt from `test-encapsulation-escape`:** If production code already subclasses `BaseHandler` (e.g., `JsonHandler extends BaseHandler`), inheritance is an established extension mechanism rather than a test-only escape hatch.
5. **Public-field DTOs with convenience constructors are intentionally not flagged:** When a Rust struct exposes all fields as `pub` (without `#[non_exhaustive]`), struct-literal construction is part of its public contract (e.g., `Config { ..Default::default() }`, `Money { cents, currency }`, IR structs). Private and package-internal field literals are already enforced by `test-accesses-private-field` and `test-bypasses-public-surface`.
6. **Runtime/compiler-only techniques are deliberately deferred:** Mutation testing, dynamic coverage, and full compiler type-checked alias analysis (e.g., resolving dynamically aliased functions or higher-order DI containers across arbitrary closures) require full builds or test execution and violate `rqcode`'s zero-build millisecond CLI design.

---

## 6. Code Map, Configuration & Verification Reference

### Module & Symbol Map

| Responsibility | Module | Key Symbols |
| :--- | :--- | :--- |
| Rule entry, shared index & diagnostic IDs | `src/rules/test_coupling/mod.rs` | `TestCouplingRule`, `TestCouplingRuleId` (13 variants), `TestCouplingIndex` |
| Direct private calls & field accesses/inits | `src/rules/test_coupling/private_access.rs` | `TestUsageCollector`, `TestUsageSummary` |
| Production-root fixed-point reachability | `src/rules/test_coupling/reachability.rs` | `ReachabilityChecker`, `ReachabilityOutcome` |
| Public surface bypass & overexposure | `src/rules/test_coupling/surface.rs` | `SurfaceChecker` |
| Production test hooks (`#if DEBUG`, `NODE_ENV`, `cfg(test)`) | `src/rules/test_coupling/hooks.rs` | `check_production_test_hooks` |
| Reflection & closed-class subclass escapes | `src/rules/test_coupling/escapes.rs` | `check_encapsulation_escapes` |
| Internal mocks, SUT spies, interaction-only, mock-only abstractions | `src/rules/test_coupling/mocking.rs` | `MockingChecker`, `BfsScratch`, `SpyReachability`, `ProdImplScratch` |
| Derived debug representation equality assertions | `src/rules/test_coupling/representation.rs` | `check_repr_assertions` |
| Language-agnostic coupling IR | `src/ast/item.rs` | `FunctionCouplingFacts`, `ModuleMock`, `InterfaceImpl`, `ConditionalGuard`, `ImportOrigin`, `SubclassPolicy` |
| Cross-file binding & impl deduplication | `src/ast/codebase/bind.rs`, `src/ast/codebase/model.rs` | `Codebase::build`, `ResolvedInterfaceImpl` |
| Per-language effective public surface | `src/ast/codebase/surface_build.rs`, `src/ast/codebase/surface.rs` | `PublicSurface`, `EffectiveVisibility` |
| Frontend coupling extractors | `src/lang/rust/{extract,body,macro_tokens,syntax}.rs`, `src/lang/typescript/{coupling,body,extract}.rs`, `src/lang/swift/{coupling,extract}.rs` | `TsCouplingInspector`, `DebugGuardStack`, `detect_debug_repr_assertions`, `classify_import_origin` |
| User configuration (`[test_coupling]`) | `src/config.rs` | `TestCouplingConfig`, `PublicApiScope` (`binary-only` default \| `all`) |

### Configuration (`rqcode.toml`)

All rule flags are flattened directly under `[test_coupling]` and default to `true` (with `public_api_scope = "binary-only"` and `severity = "error"`):

```toml
[test_coupling]
enabled = true
public_api_scope = "binary-only" # "binary-only" | "all"
severity = "error"               # "error" | "warning"

# Phase 1: Core private access & production reachability
flag_test_only_production_api = true
flag_test_only_production_field = true
flag_private_test_coupling = true
flag_production_test_hook = true
flag_test_encapsulation_escape = true

# Phase 2: Effective public surface
flag_test_bypasses_public_surface = true
flag_overexposed_for_tests = true

# Phase 3: Mocking, spying, interaction-only, abstractions & representation
flag_test_mocks_internal_collaborator = true
flag_test_spies_on_sut = true
flag_interaction_only_test = true
flag_mock_only_abstraction = true
flag_test_asserts_internal_representation = true
```

### Verification & Benchmarks

- **Polyglot benchmark fixtures (`tests/fixtures/test_coupling/benchmarks/`):** 19 multi-file benchmark suites (11 positive coupling cases spanning Phases 1–3 across Rust, TypeScript, and Swift + 8 adversarial negative controls), verified automatically by `tests/fixture_harness.rs`.
- **Phase regression suites:** `tests/phase2_surface.rs` (10 tests covering multi-language surface isolation, nested barrels, hybrid `lib.rs`/`main.rs`, factory return types, Swift overloaded initializers) and `tests/phase3_coupling.rs` (8 tests covering callback DI, helper/soft assertions, SUT vs. collaborator spies, `vi.doMock`/barrel external-boundary propagation, transitive subclass & stdlib impl counting, sealed vs. binary subtraits, Rust `format!` placeholder parsing, `#[cfg(test)]` items, and Swift extensions/`#if !DEBUG ... #else`/`Mirror`).
- **Architectural guardrails & self-dogfooding:** `tests/architecture_guardrails.rs` enforces zero language leakage in `src/ast/` and `src/rules/`, and `tests/integration.rs` (`rqcode_passes_its_own_quality_gate`) runs `rqcode` continuously on its own repository.

### References

- Khorikov, V. (2020). *Unit Testing: Principles, Practices, and Patterns* (Manning) — Four pillars of a good unit test, resistance to refactoring, observable behavior vs. implementation details, classical vs. London-school mocking.
- Meszaros, G. (2007). *xUnit Test Patterns: Refactoring Test Code* (Addison-Wesley) — *Overspecified Software*, *Test Logic in Production*, *Fragile Test*, and *Test-Specific Subclass* smells.
- Winters, T., Manshreck, T., & Wright, H. (2020). *Software Engineering at Google* (O'Reilly), Ch. 12 (*Unit Testing*) & Ch. 13 (*Test Doubles*) — Testing public APIs, preferring real implementations over mocks, avoiding interaction testing of internal collaborators.
- Peruma, A., Almalki, K., Newman, C. D., Mkaouer, M. W., Ouni, A., & Palomba, F. (2019). *On the Distribution of Test Smells in Open Source Android Applications* (CASCON) / Garousi, V. & Küçük, B. (2018). *Smells in Software Test Code: A Survey of Knowledge in Industry and Academia* (JSS).
