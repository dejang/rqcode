use super::TsNode;
use super::body::TsBodyWalker;
use super::coupling;
use super::expr::TsBodyContext;
use super::props::{self, ExtractedParams};
use super::syntax;
use crate::ast::item::{
    ConditionalGuard, ExportBinding, FieldDecl, FunctionDecl, FunctionRole, InterfaceImpl,
    ParamAccess, ParamDecl, SubclassPolicy, TypeDecl, TypeKind, TypeRef, Visibility,
};
use crate::lang::common::FunctionHeader;

#[derive(Clone, Copy)]
pub(super) struct DeclMeta {
    pub(super) visibility: Visibility,
    pub(super) is_default_export: bool,
    pub(super) in_test_context: bool,
}

pub(super) struct ExtractedClass {
    pub(super) decl: TypeDecl,
    pub(super) methods: Vec<FunctionDecl>,
    pub(super) interface_impls: Vec<InterfaceImpl>,
}

struct ClassFields {
    fields: Vec<FieldDecl>,
    all_readonly: bool,
}

impl ClassFields {
    fn new() -> Self {
        Self {
            fields: Vec::new(),
            all_readonly: true,
        }
    }

    fn push(&mut self, field: FieldDecl, is_readonly: bool) {
        if !is_readonly {
            self.all_readonly = false;
        }
        self.fields.push(field);
    }

    fn is_immutable_record(&self) -> bool {
        !self.fields.is_empty() && self.all_readonly
    }
}

pub(super) struct TsMemberExtractor<'a, 'b> {
    ctx: &'b TsBodyContext<'a, 'b>,
}

impl<'a, 'b> TsMemberExtractor<'a, 'b> {
    pub(super) fn new(ctx: &'b TsBodyContext<'a, 'b>) -> Self {
        Self { ctx }
    }

    pub(super) fn extract_class(
        &self,
        node: &TsNode<'a>,
        meta: DeclMeta,
        outer_annotations: &[String],
    ) -> Option<ExtractedClass> {
        let name = node.field("name").map(|n| n.text().into_owned())?;
        let mut class_fields = ClassFields::new();
        let mut methods = Vec::new();
        let mut annotations = outer_annotations.to_vec();
        syntax::collect_decorators(node, &mut annotations);

        let (base_types, interface_impls) =
            coupling::extract_class_heritage(node, &name, meta.in_test_context, self.ctx.tree());
        let is_react_component = base_types.iter().any(|ty| {
            matches!(
                ty,
                TypeRef::Named(base) if self.ctx.conventions().is_component_base_class(base)
            )
        });

        if let Some(body) = node.field("body") {
            for child in body.children() {
                match child.kind().as_ref() {
                    "public_field_definition" | "field_definition" => {
                        if syntax::has_modifier_token(&child, "static") {
                            continue;
                        }
                        if let Some(field_name) = child.field("name").map(|n| n.text().into_owned())
                        {
                            class_fields.push(
                                FieldDecl {
                                    name: field_name,
                                    type_name: child
                                        .field("type")
                                        .and_then(|t| syntax::extract_type_ref(&t)),
                                    visibility: syntax::extract_visibility(&child),
                                    explicit_visibility: syntax::has_explicit_visibility(&child),
                                    is_mut_ref: false,
                                    conditional_guard: ConditionalGuard::None,
                                    span: self.ctx.tree().node_span(&child),
                                },
                                syntax::has_modifier_token(&child, "readonly"),
                            );
                        }
                    }
                    "method_definition" => {
                        if let Some(method) = self.extract_method(
                            &child,
                            &name,
                            meta.in_test_context,
                            is_react_component,
                            &mut class_fields,
                        ) {
                            methods.push(method);
                        }
                    }
                    _ => {}
                }
            }
        }

        let subclass_policy = if node.kind() == "abstract_class_declaration" {
            SubclassPolicy::Open
        } else {
            SubclassPolicy::Closed
        };
        let is_immutable_record = class_fields.is_immutable_record();
        let decl = TypeDecl {
            name,
            kind: TypeKind::Class,
            visibility: meta.visibility,
            in_test_module: meta.in_test_context,
            export_binding: ExportBinding::from_is_default(meta.is_default_export),
            has_implicit_field_readers: false,
            is_immutable_record,
            subclass_policy,
            base_types,
            fields: class_fields.fields,
            annotations,
            span: self.ctx.tree().node_span(node),
        };
        Some(ExtractedClass {
            decl,
            methods,
            interface_impls,
        })
    }

    fn extract_method(
        &self,
        node: &TsNode<'a>,
        owner_name: &str,
        in_test_context: bool,
        is_react_component: bool,
        class_fields: &mut ClassFields,
    ) -> Option<FunctionDecl> {
        let name = node.field("name")?.text().into_owned();
        let is_ctor = name == "constructor";
        let is_static = syntax::has_modifier_token(node, "static");
        let is_component_lifecycle =
            is_react_component && self.ctx.conventions().is_component_lifecycle_method(&name);
        let self_receiver = (!is_ctor && !is_static).then_some(ParamAccess::MutableRef);
        let return_type = if is_ctor {
            Some(TypeRef::SelfType)
        } else {
            node.field("return_type")
                .and_then(|t| syntax::extract_type_ref(&t))
        };
        let extracted =
            node.field("parameters")
                .map_or_else(ExtractedParams::default, |param_list| {
                    self.extract_param_list(&param_list, is_ctor.then_some(class_fields), None)
                });
        let role = if in_test_context {
            FunctionRole::TestHelper
        } else {
            FunctionRole::Production
        };
        let body = TsBodyWalker::new(self.ctx).extract_with_seeds(
            node.field("body"),
            extracted.local_bindings,
            &extracted.default_exprs,
        );

        Some(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: Some(owner_name.to_string()),
                is_interface_impl: is_component_lifecycle,
                interface_name: None,
                self_receiver,
                visibility: syntax::extract_visibility(node),
                explicit_visibility: syntax::has_explicit_visibility(node),
                export_binding: ExportBinding::Standard,
                is_lifecycle: is_ctor || is_component_lifecycle,
                role,
                params: extracted.params,
                return_type,
            },
            self.ctx.tree().node_span(node),
        ))
    }

    pub(super) fn extract_interface(&self, node: &TsNode<'a>, meta: DeclMeta) -> Option<TypeDecl> {
        let name = node.field("name").map(|n| n.text().into_owned())?;
        let mut fields = Vec::new();
        if let Some(body) = node.field("body") {
            for child in body.children() {
                if child.kind() == "property_signature"
                    && let Some(prop_name) = child.field("name").map(|n| n.text().into_owned())
                {
                    fields.push(FieldDecl {
                        name: prop_name,
                        type_name: child
                            .field("type")
                            .and_then(|t| syntax::extract_type_ref(&t)),
                        visibility: Visibility::Public,
                        explicit_visibility: false,
                        is_mut_ref: false,
                        conditional_guard: ConditionalGuard::None,
                        span: self.ctx.tree().node_span(&child),
                    });
                }
            }
        }
        Some(TypeDecl {
            name,
            kind: TypeKind::Interface,
            visibility: meta.visibility,
            in_test_module: meta.in_test_context,
            export_binding: ExportBinding::from_is_default(meta.is_default_export),
            has_implicit_field_readers: false,
            is_immutable_record: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields,
            annotations: Vec::new(),
            span: self.ctx.tree().node_span(node),
        })
    }

    pub(super) fn extract_enum(&self, node: &TsNode<'a>, meta: DeclMeta) -> Option<TypeDecl> {
        let name = node.field("name").map(|n| n.text().into_owned())?;
        Some(TypeDecl {
            name,
            kind: TypeKind::Enum,
            visibility: meta.visibility,
            in_test_module: meta.in_test_context,
            export_binding: ExportBinding::from_is_default(meta.is_default_export),
            has_implicit_field_readers: false,
            is_immutable_record: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields: Vec::new(),
            annotations: Vec::new(),
            span: self.ctx.tree().node_span(node),
        })
    }

    pub(super) fn extract_callable_params(
        &self,
        node: &TsNode<'a>,
        prop_type_hint: Option<&TsNode<'a>>,
    ) -> ExtractedParams<'a> {
        if let Some(param_list) = node.field("parameters") {
            return self.extract_param_list(&param_list, None, prop_type_hint);
        }
        if let Some(single) = node.field("parameter")
            && single.kind() == "identifier"
        {
            let type_name = prop_type_hint.and_then(syntax::extract_type_ref);
            return ExtractedParams {
                params: vec![ParamDecl {
                    name: single.text().into_owned(),
                    type_name,
                    access: ParamAccess::Owned,
                }],
                local_bindings: Vec::new(),
                default_exprs: Vec::new(),
            };
        }
        ExtractedParams::default()
    }

    fn extract_param_list(
        &self,
        param_list: &TsNode<'a>,
        mut ctor_fields: Option<&mut ClassFields>,
        prop_type_hint: Option<&TsNode<'a>>,
    ) -> ExtractedParams<'a> {
        let mut out = ExtractedParams::default();
        let mut param_idx = 0usize;

        for child in param_list.children() {
            if !matches!(
                child.kind().as_ref(),
                "required_parameter" | "optional_parameter"
            ) {
                continue;
            }
            let external_hint = if param_idx == 0 { prop_type_hint } else { None };
            param_idx += 1;

            let pattern = child.field("pattern").or_else(|| child.field("name"));
            if let Some(pat) = pattern
                && pat.kind() == "object_pattern"
            {
                let type_node = child.field("type");
                let param_ty = type_node
                    .as_ref()
                    .and_then(syntax::extract_type_ref)
                    .or_else(|| external_hint.and_then(syntax::extract_type_ref));
                let shape = props::resolve_param_prop_shape(
                    type_node.as_ref(),
                    external_hint,
                    self.ctx.prop_shapes(),
                    self.ctx.conventions(),
                );
                shape.extract_bindings(&pat, param_ty.as_ref(), self.ctx.prop_shapes(), &mut out);
                if let Some(default_val) = child.field("value") {
                    out.default_exprs.push(default_val);
                }
                out.params.push(ParamDecl {
                    name: String::new(),
                    type_name: param_ty,
                    access: ParamAccess::Owned,
                });
                continue;
            }

            let Some(param_name) = syntax::extract_param_name(&child) else {
                continue;
            };
            let type_name = child
                .field("type")
                .and_then(|t| syntax::extract_type_ref(&t))
                .or_else(|| external_hint.and_then(syntax::extract_type_ref));
            if let Some(class_fields) = ctor_fields.as_deref_mut()
                && syntax::is_parameter_property(&child)
            {
                class_fields.push(
                    FieldDecl {
                        name: param_name.clone(),
                        type_name: type_name.clone(),
                        visibility: syntax::extract_visibility(&child),
                        explicit_visibility: syntax::has_explicit_visibility(&child),
                        is_mut_ref: false,
                        conditional_guard: ConditionalGuard::None,
                        span: self.ctx.tree().node_span(&child),
                    },
                    syntax::has_modifier_token(&child, "readonly"),
                );
            }
            if let Some(default_val) = child.field("value") {
                out.default_exprs.push(default_val);
            }
            out.params.push(ParamDecl {
                name: param_name,
                type_name,
                access: ParamAccess::Owned,
            });
        }
        out
    }
}
