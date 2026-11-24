use super::TsNode;
use crate::ast::SyntaxTree;
use crate::ast::item::{
    CallReceiver, ImportOrigin, InterfaceImplRole, ModuleDecl, ParamAccess, ParamDecl, TypeRef,
    Visibility,
};

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

pub(super) fn parse_visibility_modifier(node: &TsNode<'_>) -> Visibility {
    let text = node.text();
    let Some(inner) = text
        .trim()
        .strip_prefix("pub(")
        .and_then(|s| s.strip_suffix(')'))
    else {
        return if text.contains('(') {
            Visibility::Internal
        } else {
            Visibility::Public
        };
    };
    let mut parts = inner.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some("self"), None, _) | (Some("in"), Some("self"), None) => Visibility::Private,
        _ => Visibility::Internal,
    }
}

pub(super) fn has_explicit_visibility(node: &TsNode<'_>) -> bool {
    node.children().any(|c| c.kind() == "visibility_modifier")
}

pub(super) fn extract_visibility(node: &TsNode<'_>) -> Visibility {
    for child in node.children() {
        if child.kind() == "visibility_modifier" {
            return parse_visibility_modifier(&child);
        }
    }
    Visibility::Private
}

pub(super) fn extract_derives(attrs: &[String]) -> Vec<String> {
    let mut derives = Vec::new();
    for attr in attrs {
        if let Some((_, rest)) = attr.split_once("derive(")
            && let Some((inner, _)) = rest.split_once(')')
        {
            for item in inner.split(',') {
                let name = item.trim().rsplit("::").next().unwrap_or("").trim();
                if !name.is_empty() {
                    derives.push(name.to_string());
                }
            }
        }
    }
    derives
}

pub(super) fn has_implicit_field_readers(annotations: &[String]) -> bool {
    annotations.iter().any(|a| {
        matches!(
            a.as_str(),
            "Serialize" | "Deserialize" | "PartialEq" | "Eq" | "PartialOrd" | "Ord" | "Hash"
        )
    })
}

pub(super) fn has_test_attr(attrs: &[String]) -> bool {
    let mut compact = String::new();
    for attr in attrs {
        compact.clear();
        compact.extend(attr.chars().filter(|c| !c.is_whitespace()));
        if compact == "#[test]"
            || compact.ends_with("::test]")
            || compact.contains("::test(")
            || compact.starts_with("#[rstest")
        {
            return true;
        }
    }
    false
}

pub(super) fn extract_function_params(node: &TsNode<'_>) -> (Option<ParamAccess>, Vec<ParamDecl>) {
    let mut self_receiver = None;
    let mut params = Vec::new();
    let Some(param_list) = node.field("parameters") else {
        return (self_receiver, params);
    };
    for child in param_list.children() {
        if child.kind() == "self_parameter" {
            let mut is_ref = false;
            let mut is_mut = false;
            for part in child.children() {
                match part.kind().as_ref() {
                    "&" => is_ref = true,
                    "mutable_specifier" => is_mut = true,
                    _ => {}
                }
            }
            self_receiver = Some(match (is_ref, is_mut) {
                (true, true) => ParamAccess::MutableRef,
                (true, false) => ParamAccess::SharedRef,
                (false, _) => ParamAccess::Owned,
            });
        } else if child.kind() == "parameter" {
            let param_name = child
                .field("pattern")
                .and_then(|p| extract_pattern_ident(&p))
                .unwrap_or_else(|| "_".to_string());
            let (type_name, access) = child
                .field("type")
                .map_or((None, ParamAccess::Unknown), |t| extract_base_type(&t));
            if param_name == "self" {
                self_receiver = Some(access);
            } else {
                params.push(ParamDecl {
                    name: param_name,
                    type_name,
                    access,
                });
            }
        }
    }
    (self_receiver, params)
}

pub(super) fn is_transparent_chain_method(name: &str) -> bool {
    matches!(
        name,
        "unwrap"
            | "expect"
            | "clone"
            | "unwrap_or_default"
            | "unwrap_or_else"
            | "as_ref"
            | "as_mut"
            | "borrow"
            | "borrow_mut"
            | "ok"
            | "deref"
            | "deref_mut"
    )
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

/// Extract the outer target type of a Rust `impl` block without unwrapping containers
/// (`Vec<Foo>` stays `Vec`, `&str` / `()` / `[u8]` retain distinct textual names).
pub(super) fn extract_impl_target_type(node: &TsNode<'_>) -> Option<TypeRef> {
    match node.kind().as_ref() {
        "type_identifier" | "scoped_type_identifier" | "primitive_type" => {
            let text = node.text();
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| rust_type_ref(trimmed))
        }
        "generic_type" => node.field("type").and_then(|t| {
            let text = t.text();
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| rust_type_ref(trimmed))
        }),
        _ => {
            let text = node.text();
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| TypeRef::Named(trimmed.to_string()))
        }
    }
}

/// Normalize relative Rust type paths (`super::T`, `self::T`, `crate::...`) into canonical
/// module-qualified paths so `InterfaceImpl` resolution matches the declaring module scope.
pub(super) fn normalize_rust_type_ref(
    current_module: &crate::ast::ModulePath,
    declared_modules: &std::collections::HashSet<String>,
    ty_ref: TypeRef,
    in_test_module: bool,
) -> TypeRef {
    let TypeRef::Named(raw) = ty_ref else {
        return ty_ref;
    };
    let mut segs: Vec<String> = raw
        .split("::")
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if segs.len() < 2 {
        return TypeRef::Named(raw);
    }
    let Some(name) = segs.pop() else {
        return TypeRef::Named(raw);
    };
    if let Some(resolved_mod) =
        resolve_rust_import_module(current_module, declared_modules, &segs, in_test_module)
    {
        TypeRef::Named(format!("{resolved_mod}::{name}"))
    } else {
        TypeRef::Named(raw)
    }
}

pub(super) fn apply_rust_module_segment(buf: &mut Vec<String>, seg: &str) -> Option<()> {
    match seg {
        "self" => Some(()),
        "super" => {
            if buf.len() <= 1 {
                return None;
            }
            buf.pop();
            Some(())
        }
        other => {
            buf.push(other.to_string());
            Some(())
        }
    }
}

pub(super) fn collect_top_level_mods(tree: &SyntaxTree, root: &TsNode<'_>) -> Vec<ModuleDecl> {
    let mut mods = Vec::new();
    let mut pending_cfg_test = false;
    let mut compact = String::new();
    for child in root.children() {
        match child.kind().as_ref() {
            "attribute_item" => {
                compact.clear();
                compact.extend(child.text().chars().filter(|c| !c.is_whitespace()));
                if compact.contains("cfg(test)") {
                    pending_cfg_test = true;
                }
            }
            "line_comment" | "block_comment" => {}
            "mod_item" => {
                let name = child
                    .field("name")
                    .map(|n| n.text().into_owned())
                    .unwrap_or_default();
                if !pending_cfg_test && name != "tests" && !name.is_empty() {
                    mods.push(ModuleDecl {
                        name,
                        visibility: extract_visibility(&child),
                        span: tree.node_span(&child),
                    });
                }
                pending_cfg_test = false;
            }
            _ => pending_cfg_test = false,
        }
    }
    mods
}

pub(super) fn resolve_rust_import_module(
    current_module: &crate::ast::ModulePath,
    declared_modules: &std::collections::HashSet<String>,
    raw_segments: &[String],
    in_test_module: bool,
) -> Option<crate::ast::ModulePath> {
    let first = raw_segments.first()?.as_str();
    let mut resolved = Vec::new();
    match first {
        "crate" => {
            resolved.push(String::from("crate"));
            for seg in &raw_segments[1..] {
                apply_rust_module_segment(&mut resolved, seg)?;
            }
        }
        _ if matches!(first, "self" | "super") || declared_modules.contains(first) => {
            resolved.extend(current_module.segments().iter().cloned());
            if in_test_module && matches!(first, "self" | "super") {
                resolved.push(String::from("tests"));
            }
            for seg in raw_segments {
                apply_rust_module_segment(&mut resolved, seg)?;
            }
        }
        _ => return None,
    }
    Some(crate::ast::ModulePath::new(resolved))
}

pub(super) fn classify_import_origin(
    raw_segments: &[String],
    resolved: Option<&crate::ast::ModulePath>,
    symbol: &str,
) -> ImportOrigin {
    if resolved.is_some() {
        return ImportOrigin::Project;
    }
    let Some(first) = raw_segments.first().map(String::as_str) else {
        return ImportOrigin::StandardValue;
    };
    if matches!(first, "crate" | "self" | "super") {
        return ImportOrigin::Project;
    }
    if matches!(first, "std" | "core" | "alloc") {
        let is_boundary = match raw_segments.get(1).map(String::as_str) {
            Some("fs" | "process" | "env" | "os") => true,
            Some("io") => !matches!(symbol, "Result" | "Error" | "ErrorKind" | "Cursor"),
            Some("net") => !matches!(
                symbol,
                "IpAddr" | "Ipv4Addr" | "Ipv6Addr" | "SocketAddr" | "SocketAddrV4" | "SocketAddrV6"
            ),
            Some("time") => symbol != "Duration",
            _ => false,
        };
        return if is_boundary {
            ImportOrigin::ExternalBoundary
        } else {
            ImportOrigin::StandardValue
        };
    }
    ImportOrigin::ExternalBoundary
}

pub(super) fn extract_trait_supertraits(node: &TsNode<'_>) -> Vec<TypeRef> {
    let mut bases = Vec::new();
    for child in node.children() {
        if child.kind() != "trait_bounds" {
            continue;
        }
        for bound in child.children() {
            if !bound.is_named() || bound.kind() == "lifetime" {
                continue;
            }
            if let (Some(ty), _) = extract_base_type(&bound) {
                bases.push(ty);
            }
        }
    }
    bases
}

pub(super) fn classify_interface_impl_role(interface_name: &TypeRef) -> InterfaceImplRole {
    match interface_name {
        TypeRef::Named(name) if name.rsplit("::").next() == Some("Debug") => {
            InterfaceImplRole::CustomDebugRepr
        }
        _ => InterfaceImplRole::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::rust::rust_language_spec;

    #[test]
    fn splits_scoped_paths_with_turbofish_and_qualified_traits() {
        fn visit(node: &TsNode<'_>, out: &mut Vec<(Vec<String>, String)>) {
            if let Some(split) = split_scoped_path(node) {
                out.push(split);
                return;
            }
            for child in node.children() {
                visit(&child, out);
            }
        }

        let src = r"
fn demo<T: std::ops::Add<Output = T>>(a: T, b: T) -> Vec<String> {
    let _ = <T as std::ops::Add>::add(a, b);
    let _: Option<&dyn ::std::fmt::Debug> = None;
    Vec::<std::string::String>::new()
}
";
        let tree = SyntaxTree::parse("src/paths.rs", "src/paths.rs", rust_language_spec(), src);
        let mut results = Vec::new();
        visit(&tree.root(), &mut results);

        assert!(
            results.contains(&(vec!["Vec".to_string()], "new".to_string())),
            "expected turbofish Vec::<...>::new -> ([\"Vec\"], \"new\"), got {results:?}"
        );
        assert!(
            results.contains(&(Vec::new(), "add".to_string())),
            "expected qualified trait <T as Add>::add -> ([], \"add\"), got {results:?}"
        );
        assert!(
            results.contains(&(
                vec!["std".to_string(), "fmt".to_string()],
                "Debug".to_string()
            )),
            "expected ::std::fmt::Debug -> ([\"std\", \"fmt\"], \"Debug\"), got {results:?}"
        );
    }
}
