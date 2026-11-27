use super::TsNode;
use super::jsx;
use crate::ast::ModulePath;
use crate::ast::item::{TypeRef, Visibility};
use std::ops::Range;

pub(super) fn extract_static_string_key(node: &TsNode<'_>) -> Option<String> {
    match node.kind().as_ref() {
        "string" => {
            let unquoted = unquote_string(&node.text());
            (!unquoted.is_empty()).then_some(unquoted)
        }
        "template_string" => {
            if node.children().any(|c| c.kind() == "template_substitution") {
                return None;
            }
            let unquoted = unquote_string(&node.text());
            (!unquoted.is_empty()).then_some(unquoted)
        }
        _ => None,
    }
}

pub(super) fn collect_leaf_idents(
    node: &TsNode<'_>,
    excluded_range: Option<&Range<usize>>,
    out: &mut Vec<String>,
) {
    if excluded_range == Some(&node.range()) {
        return;
    }
    match node.kind().as_ref() {
        "jsx_closing_element" => return,
        "jsx_opening_element" | "jsx_self_closing_element" => {
            let skip_name_range = node.field("name").and_then(|name_node| {
                let is_intrinsic = name_node.kind() == "identifier"
                    && jsx::is_intrinsic_jsx_tag(name_node.text().as_ref());
                (is_intrinsic || name_node.kind() == "jsx_namespace_name")
                    .then(|| name_node.range())
            });
            for child in node.children() {
                if skip_name_range == Some(child.range()) {
                    continue;
                }
                collect_leaf_idents(&child, excluded_range, out);
            }
            return;
        }
        _ => {}
    }
    if node.is_leaf() {
        if node.kind() == "identifier" {
            out.push(node.text().into_owned());
        }
        return;
    }
    for child in node.children() {
        collect_leaf_idents(&child, excluded_range, out);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TestRunnerKind {
    Suite,
    Case,
}

impl TestRunnerKind {
    pub(super) fn from_call(call_node: &TsNode<'_>) -> Option<Self> {
        let mut target = call_node.field("function")?;
        if target.kind() == "call_expression" {
            let inner_fn = target.field("function")?;
            if inner_fn.kind() != "member_expression" {
                return None;
            }
            let prop = inner_fn.field("property")?.text();
            if !matches!(
                prop.as_ref(),
                "each" | "for" | "skipIf" | "runIf" | "extend"
            ) {
                return None;
            }
            target = inner_fn;
        }
        while target.kind() == "member_expression" {
            let prop = target.field("property")?.text();
            if !matches!(
                prop.as_ref(),
                "only"
                    | "skip"
                    | "todo"
                    | "concurrent"
                    | "sequential"
                    | "fails"
                    | "fail"
                    | "each"
                    | "for"
                    | "skipIf"
                    | "runIf"
                    | "extend"
                    | "fixme"
                    | "slow"
            ) {
                return None;
            }
            target = target.field("object")?;
        }
        if target.kind() != "identifier" {
            return None;
        }
        match target.text().as_ref() {
            "describe" | "context" | "suite" => Some(Self::Suite),
            "it" | "test" => Some(Self::Case),
            _ => None,
        }
    }
}

pub(super) fn extract_test_runner_args<'a>(
    call_node: &TsNode<'a>,
) -> Option<(Option<String>, TsNode<'a>)> {
    let args = call_node.field("arguments")?;
    let mut title: Option<String> = None;
    let mut callback = None;

    for arg in args.children() {
        if !arg.is_named() {
            continue;
        }
        if title.is_none() && matches!(arg.kind().as_ref(), "string" | "template_string") {
            let cleaned = unquote_string(&arg.text()).trim().to_string();
            if !cleaned.is_empty() {
                title = Some(cleaned);
            }
        } else if matches!(
            arg.kind().as_ref(),
            "arrow_function" | "function_expression"
        ) {
            callback = Some(arg);
        }
    }
    Some((title, callback?))
}

pub(super) fn join_dotted_path(raw: &str) -> Option<String> {
    let mut qualified = String::new();
    for segment in raw.split('.') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !qualified.is_empty() {
            qualified.push_str("::");
        }
        qualified.push_str(trimmed);
    }
    if qualified.is_empty() {
        None
    } else {
        Some(qualified)
    }
}

pub(super) fn unquote_string(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('\'') && trimmed.ends_with('\''))
            || (trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('`') && trimmed.ends_with('`')))
    {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_string()
}

pub(super) fn has_modifier_token(node: &TsNode<'_>, keyword: &str) -> bool {
    for child in node.children() {
        if !child.is_named() && child.kind() == keyword {
            return true;
        }
    }
    false
}

pub(super) fn is_parameter_property(node: &TsNode<'_>) -> bool {
    for child in node.children() {
        if matches!(
            child.kind().as_ref(),
            "accessibility_modifier" | "override_modifier"
        ) || (!child.is_named() && matches!(child.kind().as_ref(), "readonly" | "override"))
        {
            return true;
        }
    }
    false
}

pub(super) fn collect_decorators(node: &TsNode<'_>, out: &mut Vec<String>) {
    for child in node.children() {
        if child.kind() != "decorator" {
            continue;
        }
        let Some(expr) = child.children().find(TsNode::is_named) else {
            continue;
        };
        let target = if expr.kind() == "call_expression" {
            match expr.field("function") {
                Some(func) => func,
                None => continue,
            }
        } else {
            expr
        };
        let name = match target.kind().as_ref() {
            "identifier" => target.text().into_owned(),
            "member_expression" => target
                .field("property")
                .map(|p| p.text().into_owned())
                .unwrap_or_default(),
            _ => String::new(),
        };
        if !name.is_empty() {
            out.push(name);
        }
    }
}

pub(super) fn has_explicit_visibility(node: &TsNode<'_>) -> bool {
    node.children()
        .any(|c| c.kind() == "accessibility_modifier")
}

pub(super) fn extract_visibility(node: &TsNode<'_>) -> Visibility {
    for child in node.children() {
        if child.kind() == "accessibility_modifier" {
            return match child.text().as_ref() {
                "private" => Visibility::Private,
                "protected" => Visibility::Internal,
                _ => Visibility::Public,
            };
        }
    }
    if let Some(name) = node.field("name")
        && (name.kind() == "private_property_identifier" || name.text().starts_with('#'))
    {
        return Visibility::Private;
    }
    Visibility::Public
}

pub(super) fn extract_param_name(node: &TsNode<'_>) -> Option<String> {
    let target = node.field("pattern").or_else(|| node.field("name"))?;
    match target.kind().as_ref() {
        "identifier" => Some(target.text().into_owned()),
        "rest_pattern" => target
            .children()
            .find(|c| c.kind() == "identifier")
            .map(|c| c.text().into_owned()),
        _ => None,
    }
}

pub(super) fn extract_type_ref(node: &TsNode<'_>) -> Option<TypeRef> {
    match node.kind().as_ref() {
        "type_annotation" | "array_type" | "readonly_type" | "parenthesized_type" => node
            .children()
            .find(TsNode::is_named)
            .and_then(|c| extract_type_ref(&c)),
        "this_type" => Some(TypeRef::SelfType),
        "type_identifier" => Some(TypeRef::Named(node.text().into_owned())),
        "nested_type_identifier" => join_dotted_path(&node.text()).map(TypeRef::Named),
        "generic_type" => {
            let outer = node.field("name")?;
            let outer_text = outer.text();
            let simple_outer = outer_text.rsplit('.').next().unwrap_or(outer_text.as_ref());
            if matches!(
                simple_outer,
                "Promise"
                    | "Array"
                    | "ReadonlyArray"
                    | "Readonly"
                    | "Partial"
                    | "Required"
                    | "NonNullable"
                    | "Set"
            ) && let Some(args) = node.field("type_arguments")
            {
                for arg in args.children() {
                    if arg.is_named() {
                        return extract_type_ref(&arg);
                    }
                }
            }
            extract_type_ref(&outer)
        }
        "union_type" => {
            for child in node.children() {
                if !child.is_named() {
                    continue;
                }
                if let Some(ty) = extract_type_ref(&child) {
                    return Some(ty);
                }
            }
            None
        }
        _ => None,
    }
}

pub(super) fn resolve_relative_module_path(
    base_dir: &[String],
    specifier: &str,
) -> Option<ModulePath> {
    let spec = specifier.trim().split('?').next().unwrap_or("");
    if spec != "." && spec != ".." && !spec.starts_with("./") && !spec.starts_with("../") {
        return None;
    }

    let mut base: Vec<String> = base_dir.to_vec();
    let parts: Vec<&str> = spec.split('/').filter(|s| !s.is_empty()).collect();
    let last_idx = parts.len().saturating_sub(1);
    for (idx, part) in parts.iter().enumerate() {
        match *part {
            "." => {}
            ".." => {
                base.pop()?;
            }
            seg => {
                let mut cleaned = seg;
                if idx == last_idx {
                    for ext in [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"] {
                        if seg.len() >= ext.len()
                            && seg.is_char_boundary(seg.len() - ext.len())
                            && seg[seg.len() - ext.len()..].eq_ignore_ascii_case(ext)
                        {
                            cleaned = &seg[..seg.len() - ext.len()];
                            break;
                        }
                    }
                }
                if !(cleaned.is_empty() || idx == last_idx && cleaned == "index") {
                    base.push(cleaned.to_string());
                }
            }
        }
    }

    if base.is_empty() {
        base.push(String::from("index"));
    }
    Some(ModulePath::new(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_leaf_idents_with_exclusion_skips_intrinsic_jsx_tags() {
        let src = "export default withFallback(<label>Loading</label>)(BannerView);";
        let tree = crate::ast::SyntaxTree::parse(
            "src/Banner.tsx",
            "src/Banner.tsx",
            crate::lang::typescript::tsx_language_spec(),
            src,
        );
        let root = tree.root();
        let export = root.children().next().expect("export");
        let call = export.field("value").expect("call");
        let args = call.field("arguments").expect("args");
        let target = args
            .children()
            .find(|c| c.kind() == "identifier")
            .expect("BannerView");
        let mut idents = Vec::new();
        collect_leaf_idents(&call, Some(&target.range()), &mut idents);
        assert!(idents.iter().any(|id| id == "withFallback"));
        assert!(!idents.iter().any(|id| id == "BannerView"));
        assert!(!idents.iter().any(|id| id == "label"));
    }
}
