use crate::ast::LanguageId;
use crate::ast::codebase::{Codebase, FuncId, IndexedFunc};
use crate::ast::similarity::SimilarityCorpus;
use crate::config::Config;
use crate::diagnostic::Diagnostic;
use crate::rules::Rule;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

/// Diagnostic IDs emitted by [`SimilarityRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimilarityRuleId {
    DuplicateFunction,
    OverlappingTests,
}

impl SimilarityRuleId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateFunction => "duplicate-function",
            Self::OverlappingTests => "overlapping-tests",
        }
    }
}

/// Detects duplicate production functions and overlapping/redundant tests using the 4-stage
/// structural AST similarity pipeline (eligibility folding -> consistent Merkle subtree
/// prefix-filtered Jaccard -> weighted Tree Edit Distance verification).
///
/// Functions and tests are partitioned by opaque [`LanguageId`] so cross-language polyglot
/// repositories never produce unactionable cross-language duplication errors.
pub struct SimilarityRule;

impl Rule for SimilarityRule {
    fn id(&self) -> &'static str {
        "similarity"
    }

    fn description(&self) -> &'static str {
        "Detects duplicate functions and overlapping tests via structural AST similarity"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.similarity;
        if !cfg.enabled {
            return Vec::new();
        }

        let params = cfg.to_params();
        let mut prod_corpora: BTreeMap<LanguageId, SimilarityCorpus> = BTreeMap::new();
        let mut test_corpora: BTreeMap<LanguageId, SimilarityCorpus> = BTreeMap::new();
        for func in codebase.functions() {
            let file = codebase.file(func.file_id);
            let target = if func.decl.role.is_test() {
                Some(&mut test_corpora)
            } else if !func.decl.role.is_test_code() && !file.is_test_file() {
                Some(&mut prod_corpora)
            } else {
                None
            };
            if let Some(corpora) = target {
                corpora
                    .entry(file.language)
                    .or_insert_with(|| SimilarityCorpus::with_params(params))
                    .add_unit(func.id.0, &func.decl.body_tree);
            }
        }

        let mut diagnostics = Vec::new();
        for_each_match(
            &prod_corpora,
            cfg.function_threshold,
            codebase,
            |left, right, pct, qualifier| {
                diagnostics.push(
                    Diagnostic::new(
                        SimilarityRuleId::DuplicateFunction.as_str(),
                        cfg.severity,
                        left.decl.span.clone(),
                        format!(
                            "function `{}` has {pct}% structural AST similarity{qualifier} with `{}`",
                            left.display_name(),
                            right.display_name()
                        ),
                    )
                    .with_related(
                        right.decl.span.clone(),
                        format!("overlapping implementation `{}`", right.display_name()),
                    )
                    .with_suggestion(
                        "Extract the shared logic into a single reusable function or method and parameterize the differing inputs.",
                    ),
                );
            },
        );

        let mut right_callees: HashSet<&str> = HashSet::new();
        let mut detail = String::new();
        for_each_match(
            &test_corpora,
            cfg.test_threshold,
            codebase,
            |left, right, pct, qualifier| {
                right_callees.clear();
                right_callees.extend(right.decl.distinct_callees());
                detail.clear();
                for callee in left.decl.distinct_callees() {
                    if right_callees.contains(callee) {
                        let sep = if detail.is_empty() {
                            " (both exercise: "
                        } else {
                            ", "
                        };
                        let _ = write!(detail, "{sep}{callee}");
                    }
                }
                if !detail.is_empty() {
                    detail.push(')');
                }
                diagnostics.push(
                    Diagnostic::new(
                        SimilarityRuleId::OverlappingTests.as_str(),
                        cfg.severity,
                        left.decl.span.clone(),
                        format!(
                            "test `{}` overlaps {pct}%{qualifier} with test `{}`{detail}",
                            left.display_name(),
                            right.display_name()
                        ),
                    )
                    .with_related(
                        right.decl.span.clone(),
                        format!("overlapping test `{}`", right.display_name()),
                    )
                    .with_suggestion(
                        "Remove the redundant test or consolidate both into one parameterized/table-driven test covering distinct behaviors.",
                    ),
                );
            },
        );

        diagnostics
    }
}

fn for_each_match<'a, F>(
    corpora: &BTreeMap<LanguageId, SimilarityCorpus>,
    threshold: f64,
    codebase: &'a Codebase,
    mut emit: F,
) where
    F: FnMut(&'a IndexedFunc, &'a IndexedFunc, String, &'static str),
{
    for corpus in corpora.values() {
        for m in corpus.similar_pairs(threshold) {
            let left = codebase.func(FuncId(m.left_id()));
            let right = codebase.func(FuncId(m.right_id()));
            let pct = format!("{:.0}", (m.score() * 100.0).clamp(0.0, 100.0));
            let qualifier = if m.tsed().is_some() {
                ""
            } else {
                " (Merkle subtree estimate)"
            };
            emit(left, right, pct, qualifier);
        }
    }
}
