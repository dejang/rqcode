use super::SwiftNode;
use super::syntax;
use crate::ast::SyntaxTree;
use crate::ast::item::{
    CallReceiver, ConditionalGuard, EncapsulationEscape, FunctionCouplingFacts, ImportOrigin,
    InterfaceImpl, InterfaceImplRole, SubclassPolicy, TypeRef,
};

pub(super) fn classify_import_origin(module_name: &str) -> ImportOrigin {
    if matches!(
        module_name,
        "Network"
            | "CoreData"
            | "SQLite"
            | "SQLite3"
            | "GRDB"
            | "Realm"
            | "RealmSwift"
            | "Alamofire"
            | "StoreKit"
            | "CloudKit"
            | "AsyncHTTPClient"
            | "Vapor"
            | "PostgresNIO"
            | "Security"
            | "KeychainAccess"
            | "FirebaseCore"
            | "FirebaseFirestore"
            | "GRPC"
            | "NIO"
            | "MongoKitten"
    ) {
        ImportOrigin::ExternalBoundary
    } else if matches!(
        module_name,
        "Swift"
            | "Foundation"
            | "XCTest"
            | "Testing"
            | "Combine"
            | "Dispatch"
            | "Darwin"
            | "Glibc"
            | "UIKit"
            | "SwiftUI"
            | "AppKit"
            | "CoreGraphics"
            | "OSLog"
    ) {
        ImportOrigin::StandardValue
    } else {
        ImportOrigin::Project
    }
}

pub(super) fn extract_subclass_policy(node: &SwiftNode<'_>) -> SubclassPolicy {
    for child in node.children() {
        if child.kind() == "visibility_modifier" && child.text().trim() == "open" {
            return SubclassPolicy::Open;
        }
        if child.kind() == "modifiers"
            && child
                .children()
                .any(|m| m.kind() == "visibility_modifier" && m.text().trim() == "open")
        {
            return SubclassPolicy::Open;
        }
    }
    SubclassPolicy::Closed
}

pub(super) fn extract_inheritance(
    node: &SwiftNode<'_>,
    owner_name: &str,
    in_test_context: bool,
    emit_all_impls: bool,
    tree: &SyntaxTree,
) -> (Vec<TypeRef>, Vec<InterfaceImpl>) {
    let mut base_types = Vec::new();
    let mut impls = Vec::new();
    for child in node.children() {
        if child.kind() != "inheritance_specifier" {
            continue;
        }
        let target = child.field("inherits_from").or_else(|| {
            child.children().find(|c| {
                c.is_named() && !matches!(c.kind().as_ref(), "comment" | "multiline_comment")
            })
        });
        let Some(ty) = target.and_then(|t| syntax::extract_type_ref(&t, &[])) else {
            continue;
        };
        let role = if let TypeRef::Named(ref name) = ty
            && matches!(
                name.rsplit("::").next(),
                Some("CustomDebugStringConvertible" | "CustomStringConvertible")
            ) {
            InterfaceImplRole::CustomDebugRepr
        } else {
            InterfaceImplRole::Normal
        };
        if role == InterfaceImplRole::CustomDebugRepr || emit_all_impls {
            impls.push(InterfaceImpl {
                interface_name: ty.clone(),
                target_type: TypeRef::Named(owner_name.to_string()),
                module_path: None,
                in_test_module: in_test_context,
                role,
                span: tree.node_span(node),
            });
        }
        base_types.push(ty);
    }
    (base_types, impls)
}

#[derive(Clone, Copy)]
enum DebugBranchState {
    DebugOnly,
    NegatedDebug,
    Other,
}

/// Tracks Swift `#if DEBUG` / `#elseif` / `#else` / `#endif` compilation directives in $O(1)$.
#[derive(Default)]
pub(super) struct DebugGuardStack {
    stack: Vec<DebugBranchState>,
    active_debug: usize,
}

impl DebugGuardStack {
    pub(super) fn update(&mut self, node: &SwiftNode<'_>) -> bool {
        let text = node.text();
        let trimmed = text.trim();
        if trimmed.starts_with("#if") {
            let cond = trimmed.trim_start_matches("#if").trim();
            let state = classify_debug_condition(cond);
            self.stack.push(state);
            if matches!(state, DebugBranchState::DebugOnly) {
                self.active_debug += 1;
            }
            return true;
        }
        if trimmed.starts_with("#elseif") {
            let cond = trimmed.trim_start_matches("#elseif").trim();
            let state = classify_debug_condition(cond);
            if let Some(top) = self.stack.last_mut() {
                if matches!(*top, DebugBranchState::DebugOnly) {
                    self.active_debug = self.active_debug.saturating_sub(1);
                }
                *top = state;
                if matches!(state, DebugBranchState::DebugOnly) {
                    self.active_debug += 1;
                }
            }
            return true;
        }
        if trimmed.starts_with("#else") {
            if let Some(top) = self.stack.last_mut() {
                let prev = *top;
                if matches!(prev, DebugBranchState::DebugOnly) {
                    self.active_debug = self.active_debug.saturating_sub(1);
                }
                if matches!(prev, DebugBranchState::NegatedDebug) {
                    *top = DebugBranchState::DebugOnly;
                    self.active_debug += 1;
                } else {
                    *top = DebugBranchState::Other;
                }
            }
            return true;
        }
        if trimmed.starts_with("#endif") {
            if let Some(prev) = self.stack.pop()
                && matches!(prev, DebugBranchState::DebugOnly)
            {
                self.active_debug = self.active_debug.saturating_sub(1);
            }
            return true;
        }
        false
    }

    pub(super) const fn current(&self, inherited: ConditionalGuard) -> ConditionalGuard {
        if matches!(inherited, ConditionalGuard::DebugOnly) || self.active_debug > 0 {
            ConditionalGuard::DebugOnly
        } else {
            ConditionalGuard::None
        }
    }
}

fn classify_debug_condition(cond: &str) -> DebugBranchState {
    if cond.contains("||") {
        return DebugBranchState::Other;
    }
    let trimmed = cond.trim().trim_matches(|c| c == '(' || c == ')').trim();
    if let Some(rest) = trimmed.strip_prefix('!') {
        let inner = rest.trim().trim_matches(|c| c == '(' || c == ')').trim();
        if inner == "DEBUG" {
            return DebugBranchState::NegatedDebug;
        }
    }
    for term in cond.split("&&") {
        let t = term.trim().trim_matches(|c| c == '(' || c == ')').trim();
        if t == "DEBUG" {
            return DebugBranchState::DebugOnly;
        }
    }
    DebugBranchState::Other
}

pub(super) fn inspect_body_call(
    tree: &SyntaxTree,
    call_node: &SwiftNode<'_>,
    facts: &mut FunctionCouplingFacts,
) {
    let callee_text = call_node
        .children()
        .find(|c| {
            c.is_named() && !matches!(c.kind().as_ref(), "call_suffix" | "constructor_suffix")
        })
        .map(|c| c.text().trim().to_string());
    let Some(callee) = callee_text.as_deref() else {
        return;
    };
    if matches!(
        callee.split('.').next().unwrap_or(""),
        "URLSession" | "FileManager" | "UserDefaults" | "Process"
    ) {
        facts.mark_ambient_io();
    }
    if callee != "Mirror" && callee != "Swift.Mirror" {
        return;
    }
    let Some(suffix) = call_node
        .children()
        .find(|c| matches!(c.kind().as_ref(), "call_suffix" | "constructor_suffix"))
    else {
        return;
    };
    let Some(args) = suffix.children().find(|c| c.kind() == "value_arguments") else {
        return;
    };
    let mut target_receiver = None;
    let mut has_reflecting = false;
    for arg in args.children() {
        if arg.kind() != "value_argument" {
            continue;
        }
        let label_matches = arg
            .field("name")
            .is_some_and(|n| n.text().trim() == "reflecting")
            || arg
                .text()
                .trim()
                .split_once(':')
                .is_some_and(|(lhs, _)| lhs.trim() == "reflecting");
        if label_matches {
            has_reflecting = true;
            if let Some(mut val) = arg
                .field("value")
                .or_else(|| arg.children().filter(SwiftNode::is_named).last())
            {
                while matches!(
                    val.kind().as_ref(),
                    "bang" | "postfix_expression" | "parenthesized_expression"
                ) {
                    let Some(inner) = val.children().find(SwiftNode::is_named) else {
                        break;
                    };
                    val = inner;
                }
                target_receiver = match val.kind().as_ref() {
                    "self_expression" => Some(CallReceiver::SelfInstance),
                    "simple_identifier" => Some(CallReceiver::Named(val.text().into_owned())),
                    "navigation_expression" => {
                        let txt = val.text();
                        txt.trim()
                            .strip_prefix("self.")
                            .map(|rest| CallReceiver::Named(rest.trim().to_string()))
                    }
                    "call_expression" | "constructor_expression" => val
                        .children()
                        .find(|c| {
                            c.is_named()
                                && !matches!(
                                    c.kind().as_ref(),
                                    "call_suffix" | "constructor_suffix"
                                )
                        })
                        .map(|c| CallReceiver::Named(c.text().trim().to_string())),
                    _ => None,
                };
            }
        }
    }
    if has_reflecting {
        facts.encapsulation_escapes.push(EncapsulationEscape {
            mechanism: String::from("Mirror(reflecting:)"),
            target_receiver,
            span: tree.node_span(call_node),
        });
    }
}
