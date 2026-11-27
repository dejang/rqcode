use super::TsNode;
use super::conventions::TypeScriptConventions;
use super::syntax;
use crate::ast::item::{LocalBinding, ParamDecl, TypeRef};
use crate::lang::common::is_comment_kind;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
struct PropField {
    ty: Option<TypeRef>,
    nested: Option<PropShape>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PropShape {
    fields: HashMap<String, PropField>,
}

pub(super) type PropShapeMap = HashMap<String, PropShape>;

#[derive(Default)]
pub(super) struct ExtractedParams<'a> {
    pub(super) params: Vec<ParamDecl>,
    pub(super) local_bindings: Vec<LocalBinding>,
    pub(super) default_exprs: Vec<TsNode<'a>>,
}

impl PropShape {
    pub(super) fn field_type(&self, field_name: &str) -> Option<&TypeRef> {
        self.fields.get(field_name).and_then(|f| f.ty.as_ref())
    }

    pub(super) fn extract_bindings<'a>(
        &self,
        pattern: &TsNode<'a>,
        whole_type: Option<&TypeRef>,
        shapes: &PropShapeMap,
        out: &mut ExtractedParams<'a>,
    ) {
        for child in pattern.children() {
            match child.kind().as_ref() {
                "shorthand_property_identifier_pattern" => {
                    let name = child.text().into_owned();
                    let type_name = self.fields.get(&name).and_then(|f| f.ty.clone());
                    out.local_bindings.push(LocalBinding {
                        name,
                        type_name,
                        init_receiver: None,
                        init_callee: None,
                    });
                }
                "object_assignment_pattern" => {
                    if let Some(right) = child.field("right") {
                        out.default_exprs.push(right);
                    }
                    if let Some(left) = child.field("left") {
                        match left.kind().as_ref() {
                            "shorthand_property_identifier_pattern" | "identifier" => {
                                let name = left.text().into_owned();
                                let type_name = self.fields.get(&name).and_then(|f| f.ty.clone());
                                out.local_bindings.push(LocalBinding {
                                    name,
                                    type_name,
                                    init_receiver: None,
                                    init_callee: None,
                                });
                            }
                            "object_pattern" => {
                                self.extract_bindings(&left, whole_type, shapes, out);
                            }
                            _ => {}
                        }
                    }
                }
                "pair_pattern" => {
                    let key = child.field("key").map(|k| {
                        syntax::extract_static_string_key(&k)
                            .unwrap_or_else(|| k.text().into_owned())
                    });
                    let prop_field = key.as_deref().and_then(|k| self.fields.get(k));
                    let Some(mut val) = child.field("value") else {
                        continue;
                    };
                    if val.kind() == "assignment_pattern" {
                        if let Some(right) = val.field("right") {
                            out.default_exprs.push(right);
                        }
                        let Some(left) = val.field("left") else {
                            continue;
                        };
                        val = left;
                    }
                    match val.kind().as_ref() {
                        "identifier" => {
                            out.local_bindings.push(LocalBinding {
                                name: val.text().into_owned(),
                                type_name: prop_field.and_then(|f| f.ty.clone()),
                                init_receiver: None,
                                init_callee: None,
                            });
                        }
                        "object_pattern" => {
                            let nested_shape = prop_field
                                .and_then(|f| f.nested.clone())
                                .or_else(|| {
                                    prop_field
                                        .and_then(|f| f.ty.as_ref())
                                        .and_then(|ty| match ty {
                                            TypeRef::Named(name) => shapes.get(name).cloned(),
                                            TypeRef::SelfType => None,
                                        })
                                })
                                .unwrap_or_default();
                            let whole = prop_field.and_then(|f| f.ty.as_ref());
                            nested_shape.extract_bindings(&val, whole, shapes, out);
                        }
                        _ => {}
                    }
                }
                "rest_pattern" => {
                    if let Some(ident) = child.children().find(|c| c.kind() == "identifier") {
                        out.local_bindings.push(LocalBinding {
                            name: ident.text().into_owned(),
                            type_name: whole_type.cloned(),
                            init_receiver: None,
                            init_callee: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

/// Pass 1: pre-scan file-local `interface` and `type` declarations into prop shapes.
pub(super) fn collect_prop_shapes(
    root: &TsNode<'_>,
    conventions: &TypeScriptConventions,
) -> PropShapeMap {
    let mut shapes = PropShapeMap::new();
    let mut type_aliases = Vec::new();

    for child in root.children() {
        let decl = if child.kind() == "export_statement" {
            let Some(inner) = child.field("declaration") else {
                continue;
            };
            inner
        } else {
            child
        };
        match decl.kind().as_ref() {
            "interface_declaration" => {
                if let Some(name) = decl.field("name").map(|n| n.text().into_owned())
                    && let Some(body) = decl.field("body")
                {
                    let shape = extract_object_type_shape(&body, &shapes, conventions, 0);
                    if !shape.fields.is_empty() {
                        shapes.insert(name, shape);
                    }
                }
            }
            "type_alias_declaration" => type_aliases.push(decl),
            _ => {}
        }
    }

    for decl in type_aliases {
        if let Some(name) = decl.field("name").map(|n| n.text().into_owned())
            && let Some(value) = decl.field("value")
        {
            let shape = extract_shape_from_type(&value, &shapes, conventions, 0);
            if !shape.fields.is_empty() {
                shapes.insert(name, shape);
            }
        }
    }

    shapes
}

/// Extract the prop type argument from a declarator annotation such as `React.FC<Props>`.
pub(super) fn extract_component_type_hint<'a>(
    type_ann: &TsNode<'a>,
    conventions: &TypeScriptConventions,
) -> Option<TsNode<'a>> {
    let mut cur = type_ann.clone();
    while matches!(
        cur.kind().as_ref(),
        "type_annotation" | "parenthesized_type" | "readonly_type"
    ) {
        let next = cur
            .children()
            .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))?;
        cur = next;
    }
    if cur.kind() != "generic_type" {
        return None;
    }
    let name_node = cur.field("name")?;
    let name_text = name_node.text();
    let simple = name_text.rsplit('.').next().unwrap_or(name_text.as_ref());
    if !conventions.is_component_type_wrapper(simple) {
        return None;
    }
    let type_args = cur.field("type_arguments")?;
    type_args
        .children()
        .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
}

/// Extract the prop type argument from a wrapper call such as `memo<Props>` or `forwardRef<El, Props>`.
pub(super) fn extract_wrapper_call_prop_hint<'a>(
    call_node: &TsNode<'a>,
    wrapper_name: &str,
    conventions: &TypeScriptConventions,
) -> Option<TsNode<'a>> {
    let type_args = call_node.field("type_arguments")?;
    let target_idx = usize::from(conventions.is_ref_forwarding_wrapper(wrapper_name));
    type_args
        .children()
        .filter(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
        .nth(target_idx)
}

/// Resolve the prop shape for a destructured parameter following spec priority:
/// 1. inline object/intersection/wrapper type on the parameter,
/// 2. named type annotation in the file-local prop-shape map,
/// 3. external hint from `React.FC<Props>` / `memo<Props>` / `forwardRef<El, Props>`.
pub(super) fn resolve_param_prop_shape(
    param_type: Option<&TsNode<'_>>,
    external_hint: Option<&TsNode<'_>>,
    shapes: &PropShapeMap,
    conventions: &TypeScriptConventions,
) -> PropShape {
    if let Some(ty_node) = param_type {
        let shape = extract_shape_from_type(ty_node, shapes, conventions, 0);
        if !shape.fields.is_empty() {
            return shape;
        }
    }
    if let Some(hint) = external_hint {
        return extract_shape_from_type(hint, shapes, conventions, 0);
    }
    PropShape::default()
}

fn extract_shape_from_type(
    node: &TsNode<'_>,
    shapes: &PropShapeMap,
    conventions: &TypeScriptConventions,
    depth: usize,
) -> PropShape {
    if depth > conventions.max_prop_shape_depth {
        return PropShape::default();
    }
    match node.kind().as_ref() {
        "type_annotation" | "readonly_type" | "parenthesized_type" => node
            .children()
            .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
            .map_or_else(PropShape::default, |c| {
                extract_shape_from_type(&c, shapes, conventions, depth + 1)
            }),
        "object_type" | "interface_body" => {
            extract_object_type_shape(node, shapes, conventions, depth + 1)
        }
        "intersection_type" => {
            let mut merged = PropShape::default();
            for child in node.children() {
                if child.is_named() && !is_comment_kind(child.kind().as_ref()) {
                    let sub = extract_shape_from_type(&child, shapes, conventions, depth + 1);
                    merged.fields.extend(sub.fields);
                }
            }
            merged
        }
        "type_identifier" => shapes
            .get(node.text().as_ref())
            .cloned()
            .unwrap_or_default(),
        "generic_type" => {
            let Some(outer) = node.field("name") else {
                return PropShape::default();
            };
            let outer_text = outer.text();
            let simple = outer_text.rsplit('.').next().unwrap_or(outer_text.as_ref());
            if conventions.is_props_type_wrapper(simple)
                && let Some(args) = node.field("type_arguments")
                && let Some(first) = args
                    .children()
                    .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
            {
                return extract_shape_from_type(&first, shapes, conventions, depth + 1);
            }
            PropShape::default()
        }
        _ => PropShape::default(),
    }
}

fn extract_object_type_shape(
    body: &TsNode<'_>,
    shapes: &PropShapeMap,
    conventions: &TypeScriptConventions,
    depth: usize,
) -> PropShape {
    let mut shape = PropShape::default();
    for child in body.children() {
        if child.kind() != "property_signature" {
            continue;
        }
        let Some(name_node) = child.field("name") else {
            continue;
        };
        let prop_name = syntax::extract_static_string_key(&name_node)
            .unwrap_or_else(|| name_node.text().into_owned());
        let Some(type_node) = child.field("type") else {
            continue;
        };
        let ty = syntax::extract_type_ref(&type_node);
        let nested = {
            let candidate = extract_shape_from_type(&type_node, shapes, conventions, depth + 1);
            (!candidate.fields.is_empty()).then_some(candidate)
        };
        if ty.is_some() || nested.is_some() {
            shape.fields.insert(prop_name, PropField { ty, nested });
        }
    }
    shape
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SyntaxTree;
    use crate::lang::typescript::tsx_language_spec;

    #[test]
    fn collects_interface_and_type_alias_prop_shapes_with_wrappers_and_intersections() {
        let src = r"
export interface BaseProps {
    order: Order;
}
type CardProps = BaseProps & PropsWithChildren<{ account: Account; nested: { item: Item } }>;
";
        let tree = SyntaxTree::parse("src/Card.tsx", "src/Card.tsx", tsx_language_spec(), src);
        let conv = TypeScriptConventions::default();
        let shapes = collect_prop_shapes(&tree.root(), &conv);

        let card = shapes.get("CardProps").expect("CardProps shape");
        assert_eq!(
            card.fields["order"].ty,
            Some(TypeRef::Named("Order".into()))
        );
        assert_eq!(
            card.fields["account"].ty,
            Some(TypeRef::Named("Account".into()))
        );
        let nested = card.fields["nested"].nested.as_ref().expect("nested shape");
        assert_eq!(
            nested.fields["item"].ty,
            Some(TypeRef::Named("Item".into()))
        );
    }
}
