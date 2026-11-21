mod boundaries;

use crate::ast::ModulePath;
use crate::ast::codebase::{Codebase, ExternalFuncUsage, IndexedType};
use crate::config::{CohesionConfig, Config};
use crate::diagnostic::{Diagnostic, Span};
use crate::rules::Rule;
use std::collections::BTreeSet;

/// Diagnostic IDs emitted by [`CohesionRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CohesionRuleId {
    ScatteredConcept,
    ExternalFieldMutation,
    SplitImpl,
    ScatteredTests,
    CyclicModuleDependency,
    LeakyModuleBoundary,
    StatelessImplMethod,
}

struct DiagnosticText {
    message: String,
    suggestion: String,
}

impl CohesionRuleId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScatteredConcept => "scattered-concept",
            Self::ExternalFieldMutation => "external-field-mutation",
            Self::SplitImpl => "split-impl",
            Self::ScatteredTests => "scattered-tests",
            Self::CyclicModuleDependency => "cyclic-module-dependency",
            Self::LeakyModuleBoundary => "leaky-module-boundary",
            Self::StatelessImplMethod => "stateless-impl-method",
        }
    }

    fn build_type_diagnostic(
        self,
        ty: &IndexedType,
        cfg: &CohesionConfig,
        text: DiagnosticText,
        related: impl IntoIterator<Item = (Span, String)>,
    ) -> Diagnostic {
        let mut diag = Diagnostic::new(
            self.as_str(),
            cfg.severity,
            ty.decl.span.clone(),
            text.message,
        )
        .with_suggestion(text.suggestion);
        for (span, label) in related {
            diag = diag.with_related(span, label);
        }
        diag
    }
}

/// Detects scattered domain logic and improper module boundaries:
/// - stateful types manipulated by free functions across other modules
/// - direct field mutations outside a type's defining module
/// - type methods split across multiple modules
/// - unit tests scattered across other modules instead of co-located with the type
/// - cyclic dependencies between non-ancestor modules
/// - leaky internal type boundaries (mutable reference bags and external field reach-in)
/// - stateless methods that neither take an instance receiver nor return an instance of the owner type
pub struct CohesionRule;

impl Rule for CohesionRule {
    fn id(&self) -> &'static str {
        "cohesion"
    }

    fn description(&self) -> &'static str {
        "Detects scattered type helpers, external field mutation, split methods, scattered tests, cyclic modules, leaky boundaries, and stateless methods"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.cohesion;
        if !cfg.enabled {
            return Vec::new();
        }
        let mut diagnostics = Vec::new();
        if cfg.boundaries.flag_cyclic_modules {
            diagnostics.extend(boundaries::check_cyclic_modules(codebase, cfg));
        }
        if cfg.boundaries.flag_leaky_boundaries {
            diagnostics.extend(boundaries::check_leaky_boundaries(codebase, cfg));
        }
        if cfg.boundaries.flag_stateless_impl_methods {
            diagnostics.extend(boundaries::check_stateless_impl_methods(codebase, cfg));
        }
        CohesionChecker::new(codebase, cfg).check_types(&mut diagnostics);
        diagnostics
    }
}

struct CohesionChecker<'a> {
    codebase: &'a Codebase,
    cfg: &'a CohesionConfig,
}

impl<'a> CohesionChecker<'a> {
    fn new(codebase: &'a Codebase, cfg: &'a CohesionConfig) -> Self {
        Self { codebase, cfg }
    }

    fn check_types(&self, diagnostics: &mut Vec<Diagnostic>) {
        let mut external_modules: BTreeSet<&ModulePath> = BTreeSet::new();
        let mut test_modules: BTreeSet<&ModulePath> = BTreeSet::new();

        for ty in self.codebase.types() {
            if !ty.decl.is_stateful_type()
                || ty.decl.in_test_module
                || self.codebase.file(ty.file_id).is_test_file()
            {
                continue;
            }

            let inherent_methods = self.codebase.inherent_methods_for(ty.id);
            let mut relevant_externals = self.codebase.external_free_functions_for(ty.id);
            relevant_externals.retain(|usage| {
                usage.mutates_fields()
                    || (usage.takes_mut_ref && usage.other_project_type_params == 0)
                    || (usage.accesses_fields() && usage.other_project_type_params == 0)
            });

            external_modules.clear();
            let mut has_external_mutation = false;
            for usage in &relevant_externals {
                external_modules.insert(&usage.func.module_path);
                if usage.takes_mut_ref || usage.mutates_fields() {
                    has_external_mutation = true;
                }
                if self.cfg.flag_external_field_mutation && usage.mutates_fields() {
                    diagnostics.push(self.build_external_mutation(ty, usage));
                }
            }

            let exceeds_module_scatter = relevant_externals.len()
                >= self.cfg.min_external_functions
                && external_modules.len() >= self.cfg.min_external_modules;
            let anemic_with_mutators = has_external_mutation
                && relevant_externals.len() >= self.cfg.min_external_functions
                && relevant_externals.len() > inherent_methods.len();

            if exceeds_module_scatter || anemic_with_mutators {
                diagnostics.push(self.build_scattered_concept(
                    ty,
                    inherent_methods.len(),
                    &relevant_externals,
                    external_modules.len(),
                ));
            }

            diagnostics.extend(self.check_split_impl_and_tests(
                ty,
                !relevant_externals.is_empty(),
                &mut test_modules,
            ));
        }
    }

    fn check_split_impl_and_tests(
        &self,
        ty: &IndexedType,
        has_external_helpers: bool,
        test_modules: &mut BTreeSet<&'a ModulePath>,
    ) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let split_methods = self.codebase.split_impl_methods_for(ty.id);
        if !split_methods.is_empty() {
            out.push(CohesionRuleId::SplitImpl.build_type_diagnostic(
                ty,
                self.cfg,
                DiagnosticText {
                    message: format!(
                        "`{}` has {} method(s) defined outside its module `{}`",
                        ty.decl.name,
                        split_methods.len(),
                        ty.module_path
                    ),
                    suggestion: format!(
                        "Consolidate methods for `{}` into `{}` so all behavior lives alongside the type definition.",
                        ty.decl.name,
                        ty.decl.span.file().display()
                    ),
                },
                split_methods.into_iter().map(|m| {
                    (
                        m.decl.span.clone(),
                        format!(
                            "method `{}` defined in `{}`",
                            m.display_name(),
                            m.module_path
                        ),
                    )
                }),
            ));
        }

        if !self.cfg.flag_test_scatter {
            return out;
        }

        let profile = self.codebase.tests_for(ty.id);
        if !profile.in_defining_module.is_empty() || profile.scattered_unit_tests.is_empty() {
            return out;
        }

        test_modules.clear();
        for t in &profile.scattered_unit_tests {
            test_modules.insert(&t.module_path);
        }

        if test_modules.len() >= 2 || has_external_helpers {
            out.push(CohesionRuleId::ScatteredTests.build_type_diagnostic(
                ty,
                self.cfg,
                DiagnosticText {
                    message: format!(
                        "`{}` has 0 unit tests in `{}` but {} unit test(s) scattered across {} other module(s)",
                        ty.decl.name,
                        ty.module_path,
                        profile.scattered_unit_tests.len(),
                        test_modules.len()
                    ),
                    suggestion: format!(
                        "Co-locate unit tests validating `{}`'s behavior alongside `{}` with non-overlapping test cases.",
                        ty.decl.name,
                        ty.decl.span.file().display()
                    ),
                },
                profile.scattered_unit_tests.into_iter().map(|t| {
                    (
                        t.decl.span.clone(),
                        format!(
                            "scattered unit test `{}` in `{}`",
                            t.decl.name, t.module_path
                        ),
                    )
                }),
            ));
        }
        out
    }

    fn build_external_mutation(
        &self,
        ty: &IndexedType,
        usage: &ExternalFuncUsage<'_>,
    ) -> Diagnostic {
        Diagnostic::new(
            CohesionRuleId::ExternalFieldMutation.as_str(),
            self.cfg.severity,
            usage.func.decl.span.clone(),
            format!(
                "free function `{}` in `{}` directly mutates field(s) [{}] of `{}` (defined in `{}`)",
                usage.func.decl.name,
                usage.func.module_path,
                usage.direct_field_writes.join(", "),
                ty.decl.name,
                ty.module_path
            ),
        )
        .with_related(
            ty.decl.span.clone(),
            format!("`{}` defined here", ty.decl.name),
        )
        .with_suggestion(format!(
            "Move `{}` onto `{}` in `{}` and make the mutated field(s) private.",
            usage.func.decl.name,
            ty.decl.name,
            ty.decl.span.file().display()
        ))
    }

    fn build_scattered_concept(
        &self,
        ty: &IndexedType,
        inherent_method_count: usize,
        externals: &[ExternalFuncUsage<'_>],
        external_module_count: usize,
    ) -> Diagnostic {
        let text = DiagnosticText {
            message: format!(
                "`{}` (in `{}`) has {} method(s) and {} exposed field(s), but {} external free function(s) across {} module(s) operate on it",
                ty.decl.name,
                ty.module_path,
                inherent_method_count,
                ty.decl.exposed_field_count(),
                externals.len(),
                external_module_count
            ),
            suggestion: format!(
                "Encapsulate this behavior as methods on `{}` inside `{}`, make fields private, and co-locate unit tests there.",
                ty.decl.name,
                ty.decl.span.file().display()
            ),
        };
        let related = externals.iter().map(|usage| {
            let detail = if !usage.direct_field_writes.is_empty() {
                format!("mutates [{}]", usage.direct_field_writes.join(", "))
            } else if usage.takes_mut_ref {
                String::from("takes mutable reference")
            } else if !usage.direct_field_reads.is_empty() {
                format!("reads [{}]", usage.direct_field_reads.join(", "))
            } else {
                String::new()
            };
            (
                usage.func.decl.span.clone(),
                format!(
                    "free function `{}` in `{}` ({detail})",
                    usage.func.decl.name, usage.func.module_path
                ),
            )
        });

        CohesionRuleId::ScatteredConcept.build_type_diagnostic(ty, self.cfg, text, related)
    }
}
