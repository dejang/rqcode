use super::TsNode;
use crate::ast::item::{CallReceiver, ParamAccess, TypeRef};

pub(super) fn collect_leaf_idents(node: &TsNode<'_>, out: &mut Vec<String>) {
    crate::lang::common::collect_leaf_idents(node, "identifier", out);
}

pub(super) fn extract_pattern_ident(node: &TsNode<'_>) -> Option<String> {
    match node.kind().as_ref() {
        "identifier" | "self" => Some(node.text().into_owned()),
        "mut_pattern" => node.children().find_map(|c| extract_pattern_ident(&c)),
        _ => None,
    }
}

pub(super) fn rust_type_ref(raw: impl Into<String>) -> TypeRef {
    let name = raw.into();
    if name == "Self" {
        TypeRef::SelfType
    } else {
        TypeRef::Named(name)
    }
}

pub(super) fn rust_scoped_receiver(ty_path: String) -> Option<CallReceiver> {
    if ty_path.is_empty() {
        None
    } else if ty_path == "Self" {
        Some(CallReceiver::SelfType)
    } else {
        Some(CallReceiver::Named(ty_path))
    }
}

/// Split a Rust `scoped_identifier` / `scoped_type_identifier` into `(prefix_segments, name)`
/// using the CST `path`/`name` fields so turbofish type arguments (`Vec::<std::string::String>::new`)
/// and qualified trait paths (`<T as std::ops::Add>::add`) are never corrupted by `"::"` splitting.
pub(super) fn split_scoped_path(node: &TsNode<'_>) -> Option<(Vec<String>, String)> {
    if !matches!(
        node.kind().as_ref(),
        "scoped_identifier" | "scoped_type_identifier"
    ) {
        return None;
    }
    let name = node.field("name")?.text().trim().to_string();
    if name.is_empty() {
        return None;
    }
    let mut prefix = Vec::new();
    if let Some(path) = node.field("path") {
        collect_path_segments(&path, &mut prefix);
    }
    Some((prefix, name))
}

fn collect_path_segments(node: &TsNode<'_>, out: &mut Vec<String>) {
    match node.kind().as_ref() {
        "scoped_identifier" | "scoped_type_identifier" => {
            if let Some(path) = node.field("path") {
                collect_path_segments(&path, out);
            }
            if let Some(name) = node.field("name") {
                let text = name.text();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
        "generic_type" => {
            if let Some(ty) = node.field("type") {
                collect_path_segments(&ty, out);
            }
        }
        "bracketed_type" => {}
        _ => {
            let text = node.text();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
}

pub(super) fn extract_base_type(node: &TsNode<'_>) -> (Option<TypeRef>, ParamAccess) {
    match node.kind().as_ref() {
        "reference_type" => {
            let is_mut = node.children().any(|c| c.kind() == "mutable_specifier");
            let access = if is_mut {
                ParamAccess::MutableRef
            } else {
                ParamAccess::SharedRef
            };
            let inner_name = node.field("type").and_then(|t| extract_base_type(&t).0);
            (inner_name, access)
        }
        "type_identifier" | "scoped_type_identifier" => (
            Some(rust_type_ref(node.text().into_owned())),
            ParamAccess::Owned,
        ),
        "generic_type" => {
            let outer = node.field("type").map(|t| t.text().into_owned());
            let simple_outer = outer
                .as_deref()
                .and_then(|s| s.rsplit("::").next())
                .unwrap_or("");

            // Unwrap standard container wrappers so `Option<&mut Order>` resolves to `Order`.
            if matches!(
                simple_outer,
                "Option" | "Vec" | "Box" | "Arc" | "Rc" | "Mutex" | "RwLock" | "RefCell" | "Result"
            ) && let Some(args) = node.field("type_arguments")
            {
                for arg in args.children() {
                    if arg.is_named() && arg.kind() != "lifetime" {
                        return extract_base_type(&arg);
                    }
                }
            }
            (outer.map(rust_type_ref), ParamAccess::Owned)
        }
        "mutable_specifier" => (None, ParamAccess::MutableRef),
        _ => (None, ParamAccess::Unknown),
    }
}
