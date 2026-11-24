use super::SwiftNode;
use crate::ast::item::{
    ExportDecl, ImportDecl, ImportKind, ParamAccess, ParamDecl, TypeRef, Visibility,
};
use crate::ast::{ModulePath, SyntaxTree};
use crate::lang::common;

pub(super) struct ParsedImport {
    pub(super) import: ImportDecl,
    pub(super) reexport: Option<ExportDecl>,
}

pub(super) struct SwiftPropertyBinding<'a> {
    pub(super) pattern: SwiftNode<'a>,
    pub(super) type_annotation: Option<SwiftNode<'a>>,
    pub(super) value: Option<SwiftNode<'a>>,
    pub(super) computed_value: Option<SwiftNode<'a>>,
    pub(super) observer_block: Option<SwiftNode<'a>>,
}

pub(super) fn collect_leaf_idents(node: &SwiftNode<'_>, out: &mut Vec<String>) {
    common::collect_leaf_idents(node, "simple_identifier", out);
}

pub(super) fn is_uppercase_ident(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

/// Map Swift shorthand collection CST node kinds (`[T]`, `[K: V]`) to their canonical stdlib type.
pub(super) fn shorthand_collection_type(kind: &str) -> Option<&'static str> {
    match kind {
        "array_type" | "array_literal" => Some("Array"),
        "dictionary_type" | "dictionary_literal" => Some("Dictionary"),
        _ => None,
    }
}

/// In `tree-sitter-swift`, parenthesized expressions `(expr)` are parsed as a single-element
/// unlabeled `tuple_expression` (there is no `parenthesized_expression` node kind).
pub(super) fn unwrap_parenthesized_tuple<'a>(node: &SwiftNode<'a>) -> Option<SwiftNode<'a>> {
    if node.kind() != "tuple_expression" {
        return None;
    }
    if node.field("name").is_some() {
        return None;
    }
    let mut named = node
        .children()
        .filter(|c| c.is_named() && !common::is_comment_kind(c.kind().as_ref()));
    let first = named.next()?;
    named.next().is_none().then_some(first)
}

/// Locate the return-type node of a `function_declaration` or `subscript_declaration`,
/// handling both the `return_type` field name and `-> <type>` sibling layout.
pub(super) fn return_type_node<'a>(node: &SwiftNode<'a>) -> Option<SwiftNode<'a>> {
    if let Some(rt) = node.field("return_type") {
        return Some(rt);
    }
    let mut after_arrow = false;
    for child in node.children() {
        if !child.is_named() && child.text().trim() == "->" {
            after_arrow = true;
            continue;
        }
        if after_arrow && child.is_named() && !common::is_comment_kind(child.kind().as_ref()) {
            return Some(child);
        }
    }
    None
}

/// True when a declaration omits a return type or explicitly returns `Void` / `()`.
pub(super) fn is_void_return_type(node: &SwiftNode<'_>) -> bool {
    let Some(ret) = return_type_node(node) else {
        return true;
    };
    let text = ret.text();
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    compact == "Void" || compact == "()" || compact == "Swift.Void"
}

/// Extract every binding from a `property_declaration`, supporting multi-binding declarations
/// such as `var x: Int = 0, y = 1` or `let lo = a.min(), hi = a.max()`, plus `willSet`/`didSet`
/// property observers.
pub(super) fn extract_property_bindings<'a>(node: &SwiftNode<'a>) -> Vec<SwiftPropertyBinding<'a>> {
    let mut bindings: Vec<SwiftPropertyBinding<'a>> = Vec::new();
    let mut after_eq = false;

    for child in node.children() {
        match child.kind().as_ref() {
            "pattern" => {
                bindings.push(SwiftPropertyBinding {
                    pattern: child,
                    type_annotation: None,
                    value: None,
                    computed_value: None,
                    observer_block: None,
                });
                after_eq = false;
            }
            "type_annotation" => {
                if let Some(last) = bindings.last_mut() {
                    last.type_annotation = Some(child);
                }
            }
            "computed_property" => {
                if let Some(last) = bindings.last_mut() {
                    last.computed_value = Some(child);
                }
            }
            "willset_didset_block" => {
                if let Some(last) = bindings.last_mut() {
                    last.observer_block = Some(child);
                }
            }
            _ if !child.is_named() => match child.text().trim() {
                "=" => after_eq = true,
                "," => after_eq = false,
                _ => {}
            },
            _ if after_eq
                && child.is_named()
                && !common::is_comment_kind(child.kind().as_ref()) =>
            {
                if let Some(last) = bindings.last_mut()
                    && last.value.is_none()
                {
                    last.value = Some(child);
                }
                after_eq = false;
            }
            _ => {}
        }
    }
    bindings
}

pub(super) fn extract_navigation_member(nav: &SwiftNode<'_>) -> Option<String> {
    let ident = nav.field("suffix")?.field("suffix")?;
    let trimmed = ident.text();
    let trimmed = trimmed.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn extract_visibility_explicit(
    node: &SwiftNode<'_>,
    default_vis: Visibility,
) -> (Visibility, bool) {
    for child in node.children() {
        if child.kind() == "visibility_modifier"
            && let Some(vis) = parse_visibility_modifier(&child)
        {
            return (vis, true);
        }
        if child.kind() == "modifiers"
            && let Some(vis) = child
                .children()
                .filter(|m| m.kind() == "visibility_modifier")
                .find_map(|m| parse_visibility_modifier(&m))
        {
            return (vis, true);
        }
    }
    (default_vis, false)
}

pub(super) fn extract_visibility(node: &SwiftNode<'_>, default_vis: Visibility) -> Visibility {
    extract_visibility_explicit(node, default_vis).0
}

fn parse_visibility_modifier(node: &SwiftNode<'_>) -> Option<Visibility> {
    let text = node.text();
    let trimmed = text.trim();
    if trimmed.contains("(set)") {
        return None;
    }
    match trimmed {
        "open" | "public" => Some(Visibility::Public),
        "private" | "fileprivate" => Some(Visibility::Private),
        "internal" | "package" => Some(Visibility::Internal),
        _ => None,
    }
}

pub(super) fn is_static_or_class_member(node: &SwiftNode<'_>) -> bool {
    for child in node.children() {
        if !child.is_named() && matches!(child.kind().as_ref(), "static" | "class") {
            return true;
        }
        if child.kind() == "modifiers"
            && child.children().any(|m| {
                m.kind() == "property_modifier" && matches!(m.text().trim(), "static" | "class")
            })
        {
            return true;
        }
    }
    false
}

pub(super) fn is_mutating(node: &SwiftNode<'_>) -> bool {
    node.children().any(|child| {
        child.kind() == "modifiers"
            && child
                .children()
                .any(|m| m.kind() == "mutation_modifier" && m.text().trim() == "mutating")
    })
}

pub(super) fn collect_attributes(node: &SwiftNode<'_>, out: &mut Vec<String>) {
    for child in node.children() {
        match child.kind().as_ref() {
            "attribute" => {
                if let Some(name) = extract_attribute_name(&child) {
                    out.push(name);
                }
            }
            "modifiers" => {
                for m in child.children() {
                    if m.kind() == "attribute"
                        && let Some(name) = extract_attribute_name(&m)
                    {
                        out.push(name);
                    }
                }
            }
            _ => {}
        }
    }
}

pub(super) fn has_any_attribute(node: &SwiftNode<'_>, expected: &[String]) -> bool {
    expected.iter().any(|e| has_attribute(node, e))
}

pub(super) fn has_attribute(node: &SwiftNode<'_>, expected: &str) -> bool {
    for child in node.children() {
        match child.kind().as_ref() {
            "attribute" => {
                if extract_attribute_name(&child).as_deref() == Some(expected) {
                    return true;
                }
            }
            "modifiers" => {
                for m in child.children() {
                    if m.kind() == "attribute"
                        && extract_attribute_name(&m).as_deref() == Some(expected)
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn extract_attribute_name(attr: &SwiftNode<'_>) -> Option<String> {
    let user_ty = attr.children().find(|c| c.kind() == "user_type")?;
    user_ty
        .children()
        .filter(|c| c.kind() == "type_identifier")
        .last()
        .map(|c| c.text().into_owned())
        .or_else(|| {
            let trimmed = user_ty.text();
            let trimmed = trimmed.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

pub(super) fn has_inheritance_specifier(node: &SwiftNode<'_>) -> bool {
    node.children().any(|c| c.kind() == "inheritance_specifier")
}

pub(super) fn inherits_any(node: &SwiftNode<'_>, targets: &[String]) -> bool {
    node.children()
        .filter(|c| c.kind() == "inheritance_specifier")
        .filter_map(|c| c.field("inherits_from"))
        .filter_map(|base| extract_nominal_type_ref(&base))
        .any(|ty| match ty {
            TypeRef::Named(name) => name
                .rsplit("::")
                .next()
                .is_some_and(|simple| targets.iter().any(|t| t == simple)),
            TypeRef::SelfType => false,
        })
}

pub(super) fn extract_parameters(
    func_node: &SwiftNode<'_>,
    transparent_wrappers: &[String],
) -> Vec<ParamDecl> {
    func_node
        .children()
        .filter(|child| child.kind() == "parameter")
        .filter_map(|param| {
            let name = param.field("name")?.text().into_owned();
            let type_name = param
                .field("type")
                .and_then(|t| extract_type_ref(&t, transparent_wrappers));
            let access = param
                .children()
                .filter(|c| c.kind() == "parameter_modifiers")
                .find_map(|mods| {
                    mods.children()
                        .filter(|m| m.kind() == "parameter_modifier")
                        .find_map(|m| match m.text().trim() {
                            "inout" => Some(ParamAccess::MutableRef),
                            "borrowing" => Some(ParamAccess::SharedRef),
                            "consuming" => Some(ParamAccess::Owned),
                            _ => None,
                        })
                })
                .unwrap_or(ParamAccess::Owned);
            Some(ParamDecl {
                name,
                type_name,
                access,
            })
        })
        .collect()
}

/// Extract a parameter/property/return [`TypeRef`], unwrapping optional/array/configured
/// transparent container wrappers so e.g. `Array<Order>` or `Order?` resolves to `Order`.
pub(super) fn extract_type_ref(
    node: &SwiftNode<'_>,
    transparent_wrappers: &[String],
) -> Option<TypeRef> {
    match node.kind().as_ref() {
        "type_annotation" => node
            .field("type")
            .and_then(|t| extract_type_ref(&t, transparent_wrappers)),
        "optional_type" => node
            .field("wrapped")
            .and_then(|t| extract_type_ref(&t, transparent_wrappers)),
        "array_type" => node
            .field("element")
            .and_then(|t| extract_type_ref(&t, transparent_wrappers)),
        "type_identifier" | "simple_identifier" => match node.text().trim() {
            "Self" => Some(TypeRef::SelfType),
            "" => None,
            other => Some(TypeRef::Named(other.to_string())),
        },
        "user_type" => {
            let mut segments = Vec::new();
            let mut type_args = None;
            for child in node.children() {
                if child.kind() == "type_identifier" {
                    segments.push(child.text().into_owned());
                } else if child.kind() == "type_arguments" {
                    type_args = Some(child);
                }
            }
            let last_seg = segments.last()?;
            if segments.len() == 1 && last_seg == "Self" {
                return Some(TypeRef::SelfType);
            }
            let is_transparent = transparent_wrappers.iter().any(|w| w == last_seg);
            if is_transparent
                && let Some(args) = type_args
                && let Some(inner) = args
                    .children()
                    .filter(|c| c.is_named() && !common::is_comment_kind(c.kind().as_ref()))
                    .find_map(|arg| extract_type_ref(&arg, transparent_wrappers))
            {
                return Some(inner);
            }
            Some(TypeRef::Named(segments.join("::")))
        }
        "tuple_type" | "tuple_type_item" | "implicitly_unwrapped_optional" => {
            let mut named = node
                .children()
                .filter(|c| c.is_named() && !common::is_comment_kind(c.kind().as_ref()));
            let first = named.next()?;
            named
                .next()
                .is_none()
                .then(|| extract_type_ref(&first, transparent_wrappers))?
        }
        _ => None,
    }
}

/// Extract a nominal [`TypeRef`] WITHOUT unwrapping container wrappers.
///
/// Used for constructor expressions (`[Order]()` constructs `Array`, not `Order`; `Set<Order>()`
/// constructs `Set`, not `Order`) and `extension` targets (`extension Array<Order>` extends
/// `Array`, not `Order`).
pub(super) fn extract_nominal_type_ref(node: &SwiftNode<'_>) -> Option<TypeRef> {
    let kind = node.kind();
    if let Some(ty) = shorthand_collection_type(kind.as_ref()) {
        return Some(TypeRef::Named(String::from(ty)));
    }
    match kind.as_ref() {
        "type_annotation" => node
            .field("type")
            .and_then(|t| extract_nominal_type_ref(&t)),
        "optional_type" | "implicitly_unwrapped_optional" => {
            Some(TypeRef::Named(String::from("Optional")))
        }
        "type_identifier" | "simple_identifier" => match node.text().trim() {
            "Self" => Some(TypeRef::SelfType),
            "" => None,
            other => Some(TypeRef::Named(other.to_string())),
        },
        "user_type" => {
            let segments: Vec<String> = node
                .children()
                .filter(|c| c.kind() == "type_identifier")
                .map(|c| c.text().into_owned())
                .collect();
            let last_seg = segments.last()?;
            if segments.len() == 1 && last_seg == "Self" {
                return Some(TypeRef::SelfType);
            }
            Some(TypeRef::Named(segments.join("::")))
        }
        "tuple_type" | "tuple_type_item" => {
            let mut named = node
                .children()
                .filter(|c| c.is_named() && !common::is_comment_kind(c.kind().as_ref()));
            let first = named.next()?;
            named
                .next()
                .is_none()
                .then(|| extract_nominal_type_ref(&first))?
        }
        _ => None,
    }
}

pub(super) fn extract_extension_target_name(node: &SwiftNode<'_>) -> Option<String> {
    let name_node = node.field("name")?;
    match extract_nominal_type_ref(&name_node)? {
        TypeRef::SelfType => None,
        TypeRef::Named(name) => Some(name),
    }
}

pub(super) fn parse_import_declaration(
    node: &SwiftNode<'_>,
    tree: &SyntaxTree,
    in_test_file: bool,
) -> Option<ParsedImport> {
    let ident_node = node.children().find(|c| c.kind() == "identifier")?;
    let mut segments: Vec<String> = ident_node
        .children()
        .filter(|c| c.kind() == "simple_identifier")
        .map(|c| c.text().into_owned())
        .collect();
    if segments.is_empty() {
        segments.extend(
            ident_node
                .text()
                .split('.')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
        );
    }
    if segments.is_empty() {
        return None;
    }

    let has_import_kind = node.children().any(|c| {
        !c.is_named()
            && matches!(
                c.kind().as_ref(),
                "typealias" | "struct" | "class" | "enum" | "protocol" | "let" | "var" | "func"
            )
    });
    let is_exported = has_attribute(node, "_exported");
    let bypasses_visibility = has_attribute(node, "testable");
    let span = tree.node_span(node);

    if has_import_kind && segments.len() >= 2 {
        let symbol = segments.pop()?;
        let resolved = ModulePath::new(segments.clone());
        let reexport = is_exported.then(|| ExportDecl {
            source_module: Some(resolved.clone()),
            symbol: symbol.clone(),
            exported_name: symbol.clone(),
            is_default: false,
            is_wildcard: false,
            visibility: Visibility::Public,
        });
        let origin = super::coupling::classify_import_origin(
            segments.first().map_or("", String::as_str),
        );
        return Some(ParsedImport {
            import: ImportDecl {
                resolved_module: Some(resolved),
                module_segments: segments,
                kind: ImportKind::Named,
                symbol,
                alias: None,
                in_test_module: in_test_file,
                bypasses_visibility,
                origin,
                span,
            },
            reexport,
        });
    }

    let resolved = ModulePath::new(segments.clone());
    let reexport = is_exported.then(|| ExportDecl {
        source_module: Some(resolved.clone()),
        symbol: String::new(),
        exported_name: String::new(),
        is_default: false,
        is_wildcard: true,
        visibility: Visibility::Public,
    });
    let origin = super::coupling::classify_import_origin(
        segments.first().map_or("", String::as_str),
    );
    Some(ParsedImport {
        import: ImportDecl {
            resolved_module: Some(resolved),
            module_segments: segments,
            kind: ImportKind::Wildcard,
            symbol: String::from("*"),
            alias: None,
            in_test_module: in_test_file,
            bypasses_visibility,
            origin,
            span,
        },
        reexport,
    })
}
