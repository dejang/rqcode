use crate::ast::ModulePath;
use crate::ast::item::{FunctionDecl, InterfaceImplRole, ParamDecl, TypeDecl};
use crate::diagnostic::Span;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncId(pub usize);

#[derive(Debug, Clone)]
pub struct IndexedType {
    pub id: TypeId,
    pub file_id: FileId,
    pub module_path: ModulePath,
    pub decl: TypeDecl,
    pub resolved_base_types: Vec<TypeId>,
}

#[derive(Debug, Clone)]
pub struct ResolvedInterfaceImpl {
    pub interface_id: Option<TypeId>,
    pub target_id: Option<TypeId>,
    pub target_type_name: String,
    pub file_id: FileId,
    pub in_test_module: bool,
    pub role: InterfaceImplRole,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ResolvedParam {
    pub decl: ParamDecl,
    pub resolved_type: Option<TypeId>,
}

#[derive(Debug, Clone)]
pub struct ResolvedFieldInit {
    pub type_id: TypeId,
    pub field_name: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IndexedFunc {
    pub id: FuncId,
    pub file_id: FileId,
    pub module_path: ModulePath,
    pub member_separator: &'static str,
    pub decl: FunctionDecl,
    pub resolved_owner: Option<TypeId>,
    pub resolved_interface: Option<TypeId>,
    pub resolved_return_type: Option<TypeId>,
    pub resolved_params: Vec<ResolvedParam>,
    pub resolved_constructions: Vec<TypeId>,
    pub resolved_local_types: HashMap<String, TypeId>,
    pub resolved_call_receivers: Vec<Option<TypeId>>,
    pub resolved_field_accesses: Vec<Option<TypeId>>,
    pub resolved_field_inits: Vec<ResolvedFieldInit>,
    pub resolved_escape_targets: Vec<Option<TypeId>>,
    pub resolved_spy_targets: Vec<Option<TypeId>>,
    pub resolved_repr_targets: Vec<Option<TypeId>>,
}

impl IndexedFunc {
    #[must_use]
    pub fn display_name(&self) -> String {
        self.decl.display_name_with_sep(self.member_separator)
    }
}

/// Summary of how an external free function interacts with a specific type.
#[derive(Debug, Clone)]
pub struct ExternalFuncUsage<'a> {
    pub func: &'a IndexedFunc,
    pub takes_mut_ref: bool,
    pub direct_field_reads: Vec<String>,
    pub direct_field_writes: Vec<String>,
    pub other_project_type_params: usize,
}

impl ExternalFuncUsage<'_> {
    #[must_use]
    pub fn accesses_fields(&self) -> bool {
        !self.direct_field_reads.is_empty() || !self.direct_field_writes.is_empty()
    }

    #[must_use]
    pub fn mutates_fields(&self) -> bool {
        !self.direct_field_writes.is_empty()
    }
}

/// Distribution of tests that exercise a given type across the codebase.
#[derive(Debug, Clone)]
pub struct TypeTestProfile<'a> {
    pub in_defining_module: Vec<&'a IndexedFunc>,
    pub scattered_unit_tests: Vec<&'a IndexedFunc>,
    pub integration_tests: Vec<&'a IndexedFunc>,
}
