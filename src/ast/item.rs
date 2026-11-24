use crate::ast::ModulePath;
use crate::ast::similarity::NormalizedTree;
use crate::diagnostic::Span;

/// Language-agnostic reference to a type in declarations and expressions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeRef {
    /// The enclosing owner type (e.g., Rust `Self`, TypeScript polymorphic `this`).
    SelfType,
    /// A named or module-qualified type (e.g., `Order`, `models::Order`).
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Public,
    /// Package-, crate-, or hierarchy-internal visibility (e.g., Rust `pub(crate)`/`pub(super)`, TS `protected`).
    Internal,
    Private,
}

impl Visibility {
    #[must_use]
    pub fn is_externally_visible(self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Struct,
    Enum,
    Class,
    Interface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamAccess {
    SharedRef,
    MutableRef,
    Owned,
    Unknown,
}

impl ParamAccess {
    #[must_use]
    pub fn is_mutable(self) -> bool {
        matches!(self, Self::MutableRef)
    }
}

/// Conditional compilation guard applied to a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConditionalGuard {
    #[default]
    None,
    /// Declaration is gated behind a debug/non-release conditional compilation block.
    DebugOnly,
}

/// Whether a nominal class type is explicitly designed for external subclassing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SubclassPolicy {
    #[default]
    Closed,
    /// Explicitly extensible base class (e.g., abstract or open class).
    Open,
}

/// Semantic role of an interface implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InterfaceImplRole {
    #[default]
    Normal,
    /// Explicit custom debug/internal representation formatting implementation.
    CustomDebugRepr,
}

/// Semantic origin of an import declaration for external-boundary classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImportOrigin {
    #[default]
    Project,
    /// Standard library value/collection types or test-framework utilities.
    StandardValue,
    /// Third-party package or I/O/system/network/filesystem boundary.
    ExternalBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub name: String,
    pub type_name: Option<TypeRef>,
    pub visibility: Visibility,
    pub explicit_visibility: bool,
    pub is_mut_ref: bool,
    pub conditional_guard: ConditionalGuard,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    pub kind: TypeKind,
    pub visibility: Visibility,
    pub in_test_module: bool,
    pub is_default_export: bool,
    /// Whether generated/compiler-synthesized code (e.g., serialization or structural equality)
    /// implicitly reads all fields of this type.
    pub has_implicit_field_readers: bool,
    pub subclass_policy: SubclassPolicy,
    pub base_types: Vec<TypeRef>,
    pub fields: Vec<FieldDecl>,
    /// Type-level annotations, attributes, or decorators (e.g., derives, class decorators).
    pub annotations: Vec<String>,
    pub span: Span,
}

impl TypeDecl {
    #[must_use]
    pub fn is_stateful_type(&self) -> bool {
        matches!(self.kind, TypeKind::Struct | TypeKind::Class) && !self.fields.is_empty()
    }

    #[must_use]
    pub fn exposed_field_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|f| f.visibility.is_externally_visible())
            .count()
    }

    #[must_use]
    pub fn has_annotation(&self, name: &str) -> bool {
        self.annotations.iter().any(|a| a == name)
    }
}

/// Records that `target_type` implements `interface_name` (e.g., trait impl, `implements`, protocol conformance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceImpl {
    pub interface_name: TypeRef,
    pub target_type: TypeRef,
    pub module_path: Option<ModulePath>,
    pub in_test_module: bool,
    pub role: InterfaceImplRole,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDecl {
    pub name: String,
    pub type_name: Option<TypeRef>,
    pub access: ParamAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldAccess {
    pub receiver_param: String,
    pub receiver_is_self: bool,
    pub receiver_type: Option<TypeRef>,
    pub field_name: String,
    pub is_read: bool,
    pub is_write: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKind {
    Free,
    Method,
    Scoped,
    /// Call explicitly targeting a free function in the caller's logical module (e.g., Rust `super::f()` in `mod tests`).
    SameModule,
}

/// Receiver of a call expression in the language-agnostic IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallReceiver {
    /// Current instance receiver (e.g., Rust `self.m()`, JS/TS/Java `this.m()`).
    SelfInstance,
    /// Enclosing owner type (e.g., Rust `Self::f()`, Python `cls.f()`).
    SelfType,
    /// Named local variable, parameter, module, or type (`order.total()`, `Order::new()`).
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    pub kind: CallKind,
    pub callee: String,
    pub receiver: Option<CallReceiver>,
    pub arg_receivers: Vec<Option<CallReceiver>>,
    pub span: Span,
}

impl CallSite {
    /// Whether this call targets the enclosing owner type or current instance.
    #[must_use]
    pub const fn targets_self_owner(&self) -> bool {
        matches!(
            self.receiver,
            Some(CallReceiver::SelfInstance | CallReceiver::SelfType)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBinding {
    pub name: String,
    pub type_name: Option<TypeRef>,
    pub init_receiver: Option<CallReceiver>,
    pub init_callee: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInit {
    pub type_name: TypeRef,
    pub field_name: String,
    pub span: Span,
}

/// Semantic kind of an import declaration across module systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImportKind {
    #[default]
    Named,
    Default,
    Namespace,
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    /// Canonical module path resolved by the language frontend for project-internal imports.
    pub resolved_module: Option<ModulePath>,
    /// Raw module path segments used as a suffix-matching fallback (e.g., integration-test imports).
    pub module_segments: Vec<String>,
    pub kind: ImportKind,
    pub symbol: String,
    pub alias: Option<String>,
    pub in_test_module: bool,
    /// Whether this import explicitly bypasses normal module/package visibility (e.g., `@testable import`).
    pub bypasses_visibility: bool,
    pub origin: ImportOrigin,
    pub span: Span,
}

/// Submodule declaration linking a parent module to a child module (e.g., `pub mod foo;` or `mod bar;`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDecl {
    pub name: String,
    pub visibility: Visibility,
    pub span: Span,
}

impl ImportDecl {
    #[must_use]
    pub fn local_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.symbol)
    }
}

/// Export or re-export declaration used for cross-module and barrel resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDecl {
    /// Target module re-exported from (if this is a re-export like `export { Foo } from './foo'` or `pub use foo::Foo`).
    pub source_module: Option<ModulePath>,
    /// Original symbol name in `source_module` (empty for wildcard/namespace re-exports or default exports).
    pub symbol: String,
    /// Exported public name (defaults to `symbol`, or `alias` for `as`, `"default"` for default exports, or the namespace name for `export * as ns`).
    pub exported_name: String,
    pub is_default: bool,
    /// When `true` and `exported_name` is empty, re-exports all symbols flatly; when `exported_name` is non-empty, re-exports `source_module` as a namespace.
    pub is_wildcard: bool,
    pub visibility: Visibility,
}

/// Semantic role of a function/method within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FunctionRole {
    /// Regular production function or method.
    #[default]
    Production,
    /// Runtime entrypoint (e.g., `fn main`) exempt from unused/test-only API checks.
    Entrypoint,
    /// Executable test case (`#[test]`, `it(...)`, `test(...)`).
    Test,
    /// Non-test helper defined inside a test module or unit-test file.
    TestHelper,
}

/// Test-environment conditional hook inside production code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestHookSite {
    pub label: String,
    pub span: Span,
}

/// Runtime reflection or encapsulation-escape mechanism used by a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncapsulationEscape {
    pub mechanism: String,
    pub target_receiver: Option<CallReceiver>,
    pub span: Span,
}

/// Module-level mock replacing a dependency in a test file or test body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMock {
    pub resolved_module: Option<ModulePath>,
    pub raw_specifier: String,
    pub origin: ImportOrigin,
    pub in_test_context: bool,
    pub span: Span,
}

/// Spy installed on a receiver method inside a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpySite {
    pub target_receiver: Option<CallReceiver>,
    pub method_name: String,
    pub span: Span,
}

/// Breakdown of behavioral vs mock-interaction assertions inside a test function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssertionSummary {
    pub interaction_count: u32,
    pub behavioral_count: u32,
}

impl AssertionSummary {
    pub const fn record_interaction(&mut self) {
        self.interaction_count += 1;
    }

    pub const fn record_behavioral(&mut self) {
        self.behavioral_count += 1;
    }
}

/// Exact-equality assertion on a value's debug/internal representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReprAssertion {
    pub target_receiver: Option<CallReceiver>,
    pub span: Span,
}

/// Cohesive test-coupling evidence extracted from a function body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FunctionCouplingFacts {
    pub test_hooks: Vec<TestHookSite>,
    pub encapsulation_escapes: Vec<EncapsulationEscape>,
    pub module_mocks: Vec<ModuleMock>,
    pub spy_sites: Vec<SpySite>,
    pub assertions: AssertionSummary,
    pub repr_assertions: Vec<ReprAssertion>,
    pub has_ambient_io: bool,
}

impl FunctionCouplingFacts {
    pub const fn record_interaction_assertion(&mut self) {
        self.assertions.record_interaction();
    }

    pub const fn record_behavioral_assertion(&mut self) {
        self.assertions.record_behavioral();
    }

    pub const fn mark_ambient_io(&mut self) {
        self.has_ambient_io = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDecl {
    pub name: String,
    pub owner_type: Option<String>,
    pub is_interface_impl: bool,
    pub interface_name: Option<TypeRef>,
    pub self_receiver: Option<ParamAccess>,
    pub visibility: Visibility,
    pub explicit_visibility: bool,
    pub is_default_export: bool,
    pub role: FunctionRole,
    pub conditional_guard: ConditionalGuard,
    pub params: Vec<ParamDecl>,
    pub return_type: Option<TypeRef>,
    pub body_tree: NormalizedTree,
    pub field_accesses: Vec<FieldAccess>,
    pub calls: Vec<CallSite>,
    pub constructed_types: Vec<TypeRef>,
    pub local_bindings: Vec<LocalBinding>,
    pub field_inits: Vec<FieldInit>,
    pub referenced_idents: Vec<String>,
    pub scoped_ident_refs: Vec<String>,
    pub coupling: FunctionCouplingFacts,
    pub span: Span,
}

impl FunctionDecl {
    #[must_use]
    pub fn is_free_function(&self) -> bool {
        self.owner_type.is_none()
    }

    #[must_use]
    pub fn is_inherent_method(&self) -> bool {
        self.owner_type.is_some() && !self.is_interface_impl
    }

    #[must_use]
    pub fn has_self_receiver(&self) -> bool {
        self.self_receiver.is_some()
    }

    #[must_use]
    pub fn returns_self_type(&self) -> bool {
        matches!(self.return_type.as_ref(), Some(TypeRef::SelfType))
            || matches!(
                (&self.owner_type, &self.return_type),
                (Some(owner), Some(TypeRef::Named(ret))) if owner == ret
            )
    }

    #[must_use]
    pub const fn is_test(&self) -> bool {
        matches!(self.role, FunctionRole::Test)
    }

    #[must_use]
    pub const fn is_test_code(&self) -> bool {
        matches!(self.role, FunctionRole::Test | FunctionRole::TestHelper)
    }

    #[must_use]
    pub const fn is_entrypoint(&self) -> bool {
        matches!(self.role, FunctionRole::Entrypoint)
    }

    #[must_use]
    pub fn display_name_with_sep(&self, sep: &str) -> String {
        match &self.owner_type {
            Some(owner) => format!("{owner}{sep}{}", self.name),
            None => self.name.clone(),
        }
    }

    #[must_use]
    pub fn distinct_callees(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.calls.iter().map(|c| c.callee.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }
}

/// Inline suppression directive parsed from source comments, e.g. `// rqcode:allow(rule-id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    rule_id: String,
    line: usize,
    file_level: bool,
}

impl Suppression {
    pub fn line_level(rule_id: impl Into<String>, line: usize) -> Self {
        Self {
            rule_id: rule_id.into(),
            line,
            file_level: false,
        }
    }

    pub fn file_level(rule_id: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.into(),
            line: 1,
            file_level: true,
        }
    }

    #[must_use]
    pub fn matches(&self, rule_id: &str, target_line: usize) -> bool {
        let rule_matches = self.rule_id == "*" || self.rule_id == rule_id;
        if !rule_matches {
            return false;
        }
        if self.file_level {
            return true;
        }
        // Applies to the same line or the immediately following line.
        self.line == target_line || self.line + 1 == target_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppression_matches_same_line_next_line_and_wildcard() {
        let next_line = Suppression::line_level("duplicate-function", 10);
        assert!(next_line.matches("duplicate-function", 10));
        assert!(next_line.matches("duplicate-function", 11));
        assert!(!next_line.matches("duplicate-function", 12));
        assert!(!next_line.matches("scattered-concept", 11));

        let file_wild = Suppression::file_level("*");
        assert!(file_wild.matches("any-rule", 200));
    }

    #[test]
    fn type_decl_identifies_stateful_exposed_fields() {
        let ty = TypeDecl {
            name: "Order".to_string(),
            kind: TypeKind::Struct,
            visibility: Visibility::Public,
            in_test_module: false,
            is_default_export: false,
            has_implicit_field_readers: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields: vec![
                FieldDecl {
                    name: "total".to_string(),
                    type_name: Some(TypeRef::Named("u64".to_string())),
                    visibility: Visibility::Public,
                    explicit_visibility: true,
                    is_mut_ref: false,
                    conditional_guard: ConditionalGuard::None,
                    span: Span::new("src/order.rs", 2, 5, 2, 19),
                },
                FieldDecl {
                    name: "id".to_string(),
                    type_name: Some(TypeRef::Named("u64".to_string())),
                    visibility: Visibility::Private,
                    explicit_visibility: false,
                    is_mut_ref: false,
                    conditional_guard: ConditionalGuard::None,
                    span: Span::new("src/order.rs", 3, 5, 3, 12),
                },
            ],
            annotations: vec!["Debug".to_string()],
            span: Span::new("src/order.rs", 1, 1, 4, 2),
        };

        assert!(ty.is_stateful_type());
        assert_eq!(ty.exposed_field_count(), 1);
        assert!(ty.has_annotation("Debug"));
    }

    #[test]
    fn coupling_facts_track_interaction_and_behavioral_assertions() {
        let mut facts = FunctionCouplingFacts::default();
        facts.record_interaction_assertion();
        facts.record_behavioral_assertion();
        facts.record_interaction_assertion();
        assert_eq!(facts.assertions.interaction_count, 2);
        assert_eq!(facts.assertions.behavioral_count, 1);
    }
}
