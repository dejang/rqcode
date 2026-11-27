use super::TsNode;
use super::conventions::TypeScriptConventions;
use super::syntax;
use crate::ast::item::{
    CallReceiver, FunctionCouplingFacts, ImportOrigin, InterfaceImpl, InterfaceImplRole,
    ModuleMock, SpySite, TestHookSite, TypeRef,
};
use crate::ast::{ModulePath, SyntaxTree};

pub(super) fn classify_import_origin(
    specifier: &str,
    resolved: Option<&ModulePath>,
    conventions: &TypeScriptConventions,
) -> ImportOrigin {
    if conventions.is_asset_specifier(specifier) {
        return ImportOrigin::StandardValue;
    }
    if resolved.is_some()
        || specifier.starts_with("@/")
        || specifier.starts_with("~/")
        || specifier.starts_with('#')
    {
        return ImportOrigin::Project;
    }
    if conventions.is_external_boundary_package(specifier) {
        return ImportOrigin::ExternalBoundary;
    }
    ImportOrigin::StandardValue
}

pub(super) fn extract_class_heritage(
    node: &TsNode<'_>,
    class_name: &str,
    in_test_context: bool,
    tree: &SyntaxTree,
) -> (Vec<TypeRef>, Vec<InterfaceImpl>) {
    let mut base_types = Vec::new();
    let mut interface_impls = Vec::new();
    let Some(heritage) = node.children().find(|c| c.kind() == "class_heritage") else {
        return (base_types, interface_impls);
    };
    for clause in heritage.children() {
        match clause.kind().as_ref() {
            "extends_clause" => {
                if let Some(val) = clause.field("value")
                    && let Some(ty) = extract_heritage_type(&val)
                {
                    base_types.push(ty);
                } else {
                    for child in clause.children() {
                        if child.is_named()
                            && !matches!(child.kind().as_ref(), "type_arguments" | "comment")
                            && let Some(ty) = extract_heritage_type(&child)
                        {
                            base_types.push(ty);
                            break;
                        }
                    }
                }
            }
            "implements_clause" => {
                for child in clause.children() {
                    if !child.is_named() || child.kind() == "comment" {
                        continue;
                    }
                    if let Some(iface) = extract_heritage_type(&child) {
                        interface_impls.push(InterfaceImpl {
                            interface_name: iface,
                            target_type: TypeRef::Named(class_name.to_string()),
                            module_path: None,
                            in_test_module: in_test_context,
                            role: InterfaceImplRole::Normal,
                            span: tree.node_span(node),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    (base_types, interface_impls)
}

fn extract_heritage_type(node: &TsNode<'_>) -> Option<TypeRef> {
    if matches!(
        node.kind().as_ref(),
        "call_expression" | "as_expression" | "parenthesized_expression"
    ) {
        return None;
    }
    if let Some(ty) = syntax::extract_type_ref(node) {
        return Some(ty);
    }
    let text = node.text();
    let base = text.split('<').next().unwrap_or("").trim();
    if base.is_empty()
        || !base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$')
    {
        return None;
    }
    Some(TypeRef::Named(base.replace('.', "::")))
}

pub(super) struct TsCouplingInspector<'a> {
    tree: &'a SyntaxTree,
    base_dir: &'a [String],
    conventions: &'a TypeScriptConventions,
    facts: &'a mut FunctionCouplingFacts,
}

impl<'a> TsCouplingInspector<'a> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        base_dir: &'a [String],
        conventions: &'a TypeScriptConventions,
        facts: &'a mut FunctionCouplingFacts,
    ) -> Self {
        Self {
            tree,
            base_dir,
            conventions,
            facts,
        }
    }

    pub(super) fn inspect(&mut self, node: &TsNode<'_>) {
        match node.kind().as_ref() {
            "binary_expression" => {
                if let Some(hook) = detect_node_env_test_hook(self.tree, node) {
                    self.facts.test_hooks.push(hook);
                }
            }
            "identifier" => {
                if matches!(
                    node.text().as_ref(),
                    "fetch"
                        | "WebSocket"
                        | "XMLHttpRequest"
                        | "localStorage"
                        | "sessionStorage"
                        | "indexedDB"
                ) {
                    self.facts.mark_ambient_io();
                }
            }
            "call_expression" => self.inspect_call(node),
            _ => {}
        }
    }

    fn inspect_call(&mut self, node: &TsNode<'_>) {
        if let Some(mock) =
            detect_module_mock(self.tree, self.base_dir, self.conventions, node, true)
        {
            self.facts.module_mocks.push(mock);
        }
        let Some(func) = node.field("function") else {
            return;
        };
        if func.kind() != "member_expression" {
            return;
        }
        let Some(prop) = func.field("property") else {
            return;
        };
        let prop_text = prop.text();
        let prop_name = prop_text.trim();

        if let Some(obj) = func.field("object") {
            let obj_text = obj.text();
            let obj_name = obj_text.trim();
            if matches!((obj_name, prop_name), ("vi" | "jest", "spyOn")) {
                self.record_spy(node);
            } else if obj_name == "assert" {
                self.facts.record_behavioral_assertion();
            }
        }
        self.record_expect(&func, prop_name);
    }

    fn record_spy(&mut self, node: &TsNode<'_>) {
        let Some(args) = node.field("arguments") else {
            return;
        };
        let mut named_args = args
            .children()
            .filter(|c| c.is_named() && c.kind() != "comment");
        let (Some(mut target_node), Some(method_node)) = (named_args.next(), named_args.next())
        else {
            return;
        };
        if !matches!(method_node.kind().as_ref(), "string" | "template_string") {
            return;
        }
        while matches!(
            target_node.kind().as_ref(),
            "as_expression"
                | "satisfies_expression"
                | "non_null_expression"
                | "parenthesized_expression"
        ) {
            let Some(inner) = target_node
                .children()
                .find(|c| c.is_named() && c.kind() != "comment")
            else {
                break;
            };
            target_node = inner;
        }
        let target_receiver = match target_node.kind().as_ref() {
            "this" => Some(CallReceiver::SelfInstance),
            "identifier" => Some(CallReceiver::Named(target_node.text().into_owned())),
            "member_expression" => {
                match (target_node.field("object"), target_node.field("property")) {
                    (Some(o), Some(p))
                        if p.text().trim() == "prototype" && o.kind() == "identifier" =>
                    {
                        Some(CallReceiver::Named(o.text().into_owned()))
                    }
                    (Some(o), Some(p)) if o.kind() == "this" => {
                        Some(CallReceiver::Named(p.text().trim().to_string()))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        self.facts.spy_sites.push(SpySite {
            target_receiver,
            method_name: syntax::unquote_string(&method_node.text()),
            span: self.tree.node_span(node),
        });
    }

    fn record_expect(&mut self, func: &TsNode<'_>, prop_name: &str) {
        let mut current = func.field("object");
        let mut hops = 0usize;
        let mut rooted_in_expect = false;
        while let Some(cur) = current {
            hops += 1;
            if hops > 5 {
                break;
            }
            match cur.kind().as_ref() {
                "call_expression" => {
                    rooted_in_expect = cur.field("function").is_some_and(|f| {
                        (f.kind() == "identifier" && f.text().trim() == "expect")
                            || (f.kind() == "member_expression"
                                && f.field("object")
                                    .is_some_and(|o| o.text().trim() == "expect")
                                && f.field("property")
                                    .is_some_and(|p| matches!(p.text().trim(), "soft" | "poll")))
                    });
                    break;
                }
                "member_expression" => {
                    let is_modifier = cur.field("property").is_some_and(|p| {
                        matches!(
                            p.text().trim(),
                            "not" | "resolves" | "rejects" | "to" | "be"
                        )
                    });
                    if !is_modifier {
                        break;
                    }
                    current = cur.field("object");
                }
                _ => break,
            }
        }
        if !rooted_in_expect {
            return;
        }
        if matches!(
            prop_name,
            "toHaveBeenCalled"
                | "toHaveBeenCalledOnce"
                | "toHaveBeenCalledTimes"
                | "toHaveBeenCalledWith"
                | "toHaveBeenCalledExactlyOnceWith"
                | "toHaveBeenLastCalledWith"
                | "toHaveBeenNthCalledWith"
                | "toHaveBeenCalledBefore"
                | "toHaveBeenCalledAfter"
                | "toHaveReturned"
                | "toHaveReturnedTimes"
                | "toHaveReturnedWith"
                | "toHaveLastReturnedWith"
                | "toHaveNthReturnedWith"
                | "toHaveResolved"
                | "toHaveResolvedTimes"
                | "toHaveResolvedWith"
                | "toHaveLastResolvedWith"
                | "toHaveNthResolvedWith"
                | "toReturn"
                | "toReturnTimes"
                | "toReturnWith"
                | "lastReturnedWith"
                | "nthReturnedWith"
                | "toBeCalled"
                | "toBeCalledTimes"
                | "toBeCalledWith"
                | "lastCalledWith"
                | "nthCalledWith"
        ) {
            self.facts.record_interaction_assertion();
        } else {
            self.facts.record_behavioral_assertion();
        }
    }
}

pub(super) fn detect_node_env_test_hook(
    tree: &SyntaxTree,
    node: &TsNode<'_>,
) -> Option<TestHookSite> {
    if tree
        .relative_path()
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains(".config."))
    {
        return None;
    }
    let op = node.field("operator")?.text();
    if !matches!(op.as_ref(), "===" | "==" | "!==" | "!=") {
        return None;
    }
    let left = node.field("left")?;
    let right = node.field("right")?;
    let is_match = (is_test_env_access(&left) && is_test_string_literal(&right))
        || (is_test_env_access(&right) && is_test_string_literal(&left));
    if !is_match {
        return None;
    }
    Some(TestHookSite {
        label: node.text().trim().to_string(),
        span: tree.node_span(node),
    })
}

fn is_test_env_access(node: &TsNode<'_>) -> bool {
    let text = node.text();
    let trimmed = text.trim();
    matches!(
        trimmed,
        "process.env.NODE_ENV"
            | "process.env?.NODE_ENV"
            | "process.env[\"NODE_ENV\"]"
            | "process.env['NODE_ENV']"
            | "import.meta.env.MODE"
            | "import.meta.env?.MODE"
    )
}

fn is_test_string_literal(node: &TsNode<'_>) -> bool {
    matches!(node.kind().as_ref(), "string" | "template_string")
        && syntax::unquote_string(&node.text()) == "test"
}

pub(super) fn detect_module_mock(
    tree: &SyntaxTree,
    base_dir: &[String],
    conventions: &TypeScriptConventions,
    call_node: &TsNode<'_>,
    in_test_context: bool,
) -> Option<ModuleMock> {
    let func = call_node.field("function")?;
    if func.kind() != "member_expression" {
        return None;
    }
    let obj = func.field("object")?.text();
    let prop = func.field("property")?.text();
    let is_mock_call = matches!(
        (obj.trim(), prop.trim()),
        ("vi" | "jest", "mock" | "doMock") | ("jest", "unstable_mockModule") | ("mock", "module")
    );
    if !is_mock_call {
        return None;
    }
    let args = call_node.field("arguments")?;
    let first_arg = args
        .children()
        .find(|c| c.is_named() && c.kind() != "comment")?;
    let spec_node = if matches!(first_arg.kind().as_ref(), "string" | "template_string") {
        first_arg
    } else if first_arg.kind() == "call_expression"
        && first_arg
            .field("function")
            .is_some_and(|f| f.text().trim() == "import")
        && let Some(imp_args) = first_arg.field("arguments")
        && let Some(inner) = imp_args
            .children()
            .find(|c| c.is_named() && matches!(c.kind().as_ref(), "string" | "template_string"))
    {
        inner
    } else {
        return None;
    };
    let raw_text = spec_node.text();
    if spec_node.kind() == "template_string" && raw_text.contains("${") {
        return None;
    }
    let raw_specifier = syntax::unquote_string(&raw_text);
    let resolved_module = if conventions.is_asset_specifier(&raw_specifier) {
        None
    } else {
        syntax::resolve_relative_module_path(base_dir, &raw_specifier)
    };
    let origin = classify_import_origin(&raw_specifier, resolved_module.as_ref(), conventions);
    Some(ModuleMock {
        resolved_module,
        raw_specifier,
        origin,
        in_test_context,
        span: tree.node_span(call_node),
    })
}
