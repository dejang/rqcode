mod boundaries;
mod feature_envy;
mod graph;
pub mod lexical;
pub mod type_cohesion;

use crate::ast::ModulePath;
use crate::ast::codebase::{Codebase, ExternalFuncUsage, FuncId, IndexedFunc, IndexedType, TypeId};
use crate::config::{CohesionConfig, Config};
use crate::diagnostic::{Diagnostic, Span};
use crate::rules::Rule;
use lexical::{LexicalCorpus, LexicalParams};
use std::collections::{BTreeSet, HashMap, HashSet};

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
    NoBarrelModule,
    LowCohesionType,
    FeatureEnvy,
    ArchitecturalCrossing,
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
            Self::NoBarrelModule => "no-barrel-module",
            Self::LowCohesionType => "low-cohesion-type",
            Self::FeatureEnvy => "feature-envy",
            Self::ArchitecturalCrossing => "architectural-crossing",
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
/// - cyclic dependencies and high fan-in/fan-out crossings between non-ancestor modules
/// - leaky internal type boundaries (mutable reference bags and external field reach-in)
/// - stateless methods that neither take an instance receiver nor return an instance of the owner type
/// - barrel re-export modules, low-cohesion types, and feature-envious methods
pub struct CohesionRule;

impl Rule for CohesionRule {
    fn id(&self) -> &'static str {
        "cohesion"
    }

    fn description(&self) -> &'static str {
        "Detects scattered type helpers, external field mutation, split methods, scattered tests, cyclic modules, architectural crossings, leaky boundaries, stateless methods, barrel modules, low-cohesion types, and feature envy"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.cohesion;
        if !cfg.enabled {
            return Vec::new();
        }
        let mut diagnostics = Vec::new();
        if cfg.boundaries.flag_cyclic_modules || cfg.crossing.enabled {
            let module_graph = graph::ModuleGraph::build(codebase);
            if cfg.boundaries.flag_cyclic_modules {
                diagnostics.extend(module_graph.check_cyclic_modules(cfg));
            }
            if cfg.crossing.enabled {
                diagnostics.extend(module_graph.check_architectural_crossings(cfg));
            }
        }
        if cfg.boundaries.flag_leaky_boundaries {
            diagnostics.extend(boundaries::check_leaky_boundaries(codebase, cfg));
        }
        if cfg.boundaries.flag_stateless_impl_methods {
            diagnostics.extend(boundaries::check_stateless_impl_methods(codebase, cfg));
        }
        if cfg.boundaries.barrels.flag_barrel_modules {
            diagnostics.extend(boundaries::check_barrel_modules(codebase, cfg));
        }
        let checker = CohesionChecker::new(codebase, cfg);
        let external_mutations = checker.check_external_field_mutations(&mut diagnostics);
        checker.check_types(&mut diagnostics);
        if cfg.type_cohesion.enabled || cfg.feature_envy.enabled {
            let corpus = LexicalCorpus::build(codebase, LexicalParams::default());
            if cfg.type_cohesion.enabled {
                diagnostics.extend(type_cohesion::check_low_cohesion_types(
                    codebase, cfg, &corpus,
                ));
            }
            if cfg.feature_envy.enabled {
                diagnostics.extend(feature_envy::check_feature_envy(
                    codebase,
                    cfg,
                    &corpus,
                    &external_mutations,
                ));
            }
        }
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

    fn check_external_field_mutations(
        &self,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> HashSet<(FuncId, TypeId)> {
        let mut emitted = HashSet::new();
        if !self.cfg.flag_external_field_mutation {
            return emitted;
        }
        let declared_fields: HashMap<TypeId, HashSet<&str>> = self
            .codebase
            .types()
            .iter()
            .filter(|ty| {
                ty.decl.is_stateful_type()
                    && !ty.decl.in_test_module
                    && !self.codebase.file(ty.file_id).is_test_file()
            })
            .map(|ty| {
                let names = ty.decl.fields.iter().map(|f| f.name.as_str()).collect();
                (ty.id, names)
            })
            .collect();

        let mut type_order: Vec<TypeId> = Vec::new();
        let mut mutated_fields: HashMap<TypeId, Vec<&str>> = HashMap::new();
        let mut seen_fields: HashSet<(TypeId, &str)> = HashSet::new();

        for func in self.codebase.functions() {
            if func.decl.role.is_test_code() || self.codebase.file(func.file_id).is_test_file() {
                continue;
            }
            type_order.clear();
            mutated_fields.clear();
            seen_fields.clear();

            for (access, resolved_ty) in func
                .decl
                .field_accesses
                .iter()
                .zip(&func.resolved_field_accesses)
            {
                let Some(type_id) = *resolved_ty else {
                    continue;
                };
                if !access.is_write
                    || access.receiver_is_self
                    || func.resolved_owner == Some(type_id)
                {
                    continue;
                }
                let Some(valid_fields) = declared_fields.get(&type_id) else {
                    continue;
                };
                let ty = self.codebase.ty(type_id);
                let field_name = access.field_name.as_str();
                if func.module_path == ty.module_path || !valid_fields.contains(field_name) {
                    continue;
                }
                if seen_fields.insert((type_id, field_name)) {
                    let entry = mutated_fields.entry(type_id).or_insert_with(|| {
                        type_order.push(type_id);
                        Vec::new()
                    });
                    entry.push(field_name);
                }
            }

            for &type_id in &type_order {
                if let Some(fields) = mutated_fields.get(&type_id) {
                    let ty = self.codebase.ty(type_id);
                    emitted.insert((func.id, type_id));
                    diagnostics.push(self.build_external_mutation(func, ty, fields));
                }
            }
        }
        emitted
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

            let inherent_behavioral_methods = self
                .codebase
                .inherent_methods_for(ty.id)
                .into_iter()
                .filter(|m| {
                    !m.decl.role.is_test_code()
                        && !self.codebase.file(m.file_id).is_test_file()
                        && !m.decl.is_lifecycle
                        && m.decl.has_self_receiver()
                        && !m.decl.returns_self_type()
                        && m.resolved_return_type != Some(ty.id)
                })
                .count();

            let mut relevant_externals = self.codebase.external_free_functions_for(ty.id);
            relevant_externals.retain(|usage| {
                usage.mutates_fields()
                    || (usage.takes_mut_ref && usage.other_project_types == 0)
                    || (usage.accesses_fields() && usage.other_project_types == 0)
            });

            external_modules.clear();
            let mut has_external_mutation = false;
            for usage in &relevant_externals {
                external_modules.insert(&usage.func.module_path);
                if usage.takes_mut_ref || usage.mutates_fields() {
                    has_external_mutation = true;
                }
            }

            let is_pure_read_only_dto = (ty.decl.has_implicit_field_readers
                || ty.decl.is_immutable_record)
                && ty.decl.exposed_field_count() == ty.decl.fields.len()
                && inherent_behavioral_methods == 0
                && !has_external_mutation;

            let exceeds_module_scatter = relevant_externals.len()
                >= self.cfg.min_external_functions
                && external_modules.len() >= self.cfg.min_external_modules;
            let anemic_with_mutators = has_external_mutation
                && relevant_externals.len() >= self.cfg.min_external_functions
                && relevant_externals.len() > inherent_behavioral_methods;

            if !is_pure_read_only_dto && (exceeds_module_scatter || anemic_with_mutators) {
                diagnostics.push(self.build_scattered_concept(
                    ty,
                    inherent_behavioral_methods,
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
        func: &IndexedFunc,
        ty: &IndexedType,
        mutated_fields: &[&str],
    ) -> Diagnostic {
        let kind_label = if func.decl.is_free_function() {
            "free function"
        } else {
            "method"
        };
        let display_name = func.display_name();
        Diagnostic::new(
            CohesionRuleId::ExternalFieldMutation.as_str(),
            self.cfg.severity,
            func.decl.span.clone(),
            format!(
                "{kind_label} `{display_name}` in `{}` directly mutates field(s) [{}] of `{}` (defined in `{}`)",
                func.module_path,
                mutated_fields.join(", "),
                ty.decl.name,
                ty.module_path
            ),
        )
        .with_related(
            ty.decl.span.clone(),
            format!("`{}` defined here", ty.decl.name),
        )
        .with_suggestion(format!(
            "Move `{display_name}` onto `{}` in `{}` and make the mutated field(s) private.",
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
