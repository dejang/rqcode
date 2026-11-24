use super::{CallerMap, FieldReaderMap, TestCouplingIndex, TestCouplingRuleId};
use crate::ast::codebase::{FuncId, IndexedFunc, TypeId};
use crate::ast::item::Visibility;
use crate::config::TestCouplingConfig;
use crate::diagnostic::Diagnostic;
use std::collections::{HashMap, HashSet};

pub(super) struct TestUsageSummary<'a> {
    pub(super) direct_test_callers: CallerMap<'a>,
    pub(super) test_field_readers: FieldReaderMap<'a>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

pub(super) struct TestUsageCollector<'a> {
    index: &'a TestCouplingIndex<'a>,
    cfg: &'a TestCouplingConfig,
    direct_test_callers: CallerMap<'a>,
    test_field_readers: FieldReaderMap<'a>,
    diagnostics: Vec<Diagnostic>,
    seen_call_targets: HashSet<FuncId>,
    seen_private_fields: HashSet<(TypeId, &'a str)>,
    seen_read_fields: HashSet<(TypeId, &'a str)>,
}

impl<'a> TestUsageCollector<'a> {
    pub(super) fn new(index: &'a TestCouplingIndex<'a>, cfg: &'a TestCouplingConfig) -> Self {
        Self {
            index,
            cfg,
            direct_test_callers: HashMap::new(),
            test_field_readers: HashMap::new(),
            diagnostics: Vec::new(),
            seen_call_targets: HashSet::new(),
            seen_private_fields: HashSet::new(),
            seen_read_fields: HashSet::new(),
        }
    }

    pub(super) fn collect(mut self) -> TestUsageSummary<'a> {
        for func in self.index.codebase().functions() {
            if !self.index.is_test_func(func) {
                continue;
            }
            self.process_test_calls(func);
            self.record_test_field_reads(func);
            if self.cfg.core.flag_private_test_coupling {
                self.collect_private_field_diags(func);
            }
        }
        TestUsageSummary {
            direct_test_callers: self.direct_test_callers,
            test_field_readers: self.test_field_readers,
            diagnostics: self.diagnostics,
        }
    }

    fn process_test_calls(&mut self, func: &'a IndexedFunc) {
        self.seen_call_targets.clear();
        for (call_idx, call) in func.decl.calls.iter().enumerate() {
            let targets = self.index.resolve_prod_call(func, call_idx);
            let single_target = targets.len() == 1;
            for target_id in targets {
                if !self.seen_call_targets.insert(target_id) {
                    continue;
                }
                self.direct_test_callers
                    .entry(target_id)
                    .or_default()
                    .push((func, call.span.clone(), single_target));

                let target = self.index.codebase().func(target_id);
                if self.cfg.core.flag_private_test_coupling
                    && single_target
                    && target.decl.visibility == Visibility::Private
                {
                    let kind = if target.decl.is_inherent_method() {
                        "private method"
                    } else {
                        "private function"
                    };
                    let target_name = target.display_name();
                    self.diagnostics.push(
                        Diagnostic::new(
                            TestCouplingRuleId::TestCallsPrivateApi.as_str(),
                            self.cfg.severity,
                            call.span.clone(),
                            format!(
                                "test `{}` directly calls {kind} `{target_name}`",
                                func.display_name()
                            ),
                        )
                        .with_related(
                            target.decl.span.clone(),
                            format!("{kind} `{target_name}` declared here"),
                        )
                        .with_suggestion(format!(
                            "Exercise `{target_name}` indirectly through the module's public behavior so internal refactors do not break tests."
                        )),
                    );
                }
            }
        }
    }

    fn record_test_field_reads(&mut self, func: &'a IndexedFunc) {
        self.seen_read_fields.clear();
        for (access_idx, access) in func.decl.field_accesses.iter().enumerate() {
            if !access.is_read {
                continue;
            }
            let Some(type_id) = func
                .resolved_field_accesses
                .get(access_idx)
                .copied()
                .flatten()
            else {
                continue;
            };
            if self
                .seen_read_fields
                .insert((type_id, access.field_name.as_str()))
            {
                self.test_field_readers
                    .entry((type_id, access.field_name.as_str()))
                    .or_default()
                    .push((func, access.span.clone()));
            }
        }
    }

    fn collect_private_field_diags(&mut self, test_func: &'a IndexedFunc) {
        self.seen_private_fields.clear();
        let init_action = format!(
            "initializes via {}",
            self.index.codebase().field_init_noun(test_func.file_id)
        );
        let field_events = test_func
            .decl
            .field_accesses
            .iter()
            .enumerate()
            .filter_map(|(idx, access)| {
                let type_id = test_func
                    .resolved_field_accesses
                    .get(idx)
                    .copied()
                    .flatten()?;
                let action = if access.is_write { "mutates" } else { "reads" };
                Some((type_id, access.field_name.as_str(), &access.span, action))
            })
            .chain(test_func.resolved_field_inits.iter().map(|init| {
                (
                    init.type_id,
                    init.field_name.as_str(),
                    &init.span,
                    init_action.as_str(),
                )
            }));

        for (type_id, field_name, span, action) in field_events {
            let Some(field_span) = self.index.private_field_span(type_id, field_name) else {
                continue;
            };
            if !self.seen_private_fields.insert((type_id, field_name)) {
                continue;
            }
            let ty = self.index.codebase().ty(type_id);
            self.diagnostics.push(
                Diagnostic::new(
                    TestCouplingRuleId::TestAccessesPrivateField.as_str(),
                    self.cfg.severity,
                    span.clone(),
                    format!(
                        "test `{}` directly {action} private field `{field_name}` of `{}`",
                        test_func.display_name(),
                        ty.decl.name
                    ),
                )
                .with_related(
                    field_span.clone(),
                    format!("private field `{field_name}` declared here"),
                )
                .with_suggestion(format!(
                    "Use `{}`'s public API/constructor instead of accessing private field `{field_name}` directly.",
                    ty.decl.name
                )),
            );
        }
    }
}
