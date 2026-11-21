use crate::ast::codebase::{Codebase, FuncId};
use crate::ast::vector::TfIdfCorpus;
use crate::config::Config;
use crate::diagnostic::Diagnostic;
use crate::rules::Rule;
use std::collections::HashSet;
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

/// Detects duplicate production functions and overlapping/redundant tests using
/// TF-IDF weighted cosine similarity over normalized AST n-grams.
pub struct SimilarityRule;

impl Rule for SimilarityRule {
    fn id(&self) -> &'static str {
        "similarity"
    }

    fn description(&self) -> &'static str {
        "Detects duplicate functions and overlapping tests via AST cosine similarity"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.similarity;
        if !cfg.enabled {
            return Vec::new();
        }

        let mut prod_corpus = TfIdfCorpus::new(cfg.ngram_size);
        let mut test_corpus = TfIdfCorpus::new(cfg.ngram_size);

        for func in codebase.functions() {
            if func.decl.body_tokens.len() < cfg.min_tokens {
                continue;
            }
            if func.decl.is_test() {
                test_corpus.add_document(func.id.0, &func.decl.body_tokens);
            } else if !func.decl.is_test_code() && !codebase.file(func.file_id).is_test_file() {
                prod_corpus.add_document(func.id.0, &func.decl.body_tokens);
            }
        }

        let mut diagnostics = Vec::new();

        for m in prod_corpus.similar_pairs(cfg.function_threshold) {
            let left = codebase.func(FuncId(m.left_id()));
            let right = codebase.func(FuncId(m.right_id()));
            let pct = format_percent(m.score());

            diagnostics.push(
                Diagnostic::new(
                    SimilarityRuleId::DuplicateFunction.as_str(),
                    cfg.severity,
                    left.decl.span.clone(),
                    format!(
                        "function `{}` has {pct}% structural cosine similarity with `{}`",
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
        }

        let mut right_callees: HashSet<&str> = HashSet::new();
        let mut detail = String::new();

        for m in test_corpus.similar_pairs(cfg.test_threshold) {
            let left = codebase.func(FuncId(m.left_id()));
            let right = codebase.func(FuncId(m.right_id()));
            let pct = format_percent(m.score());

            right_callees.clear();
            for callee in right.decl.distinct_callees() {
                right_callees.insert(callee);
            }

            detail.clear();
            let mut first = true;
            for callee in left.decl.distinct_callees() {
                if right_callees.contains(callee) {
                    if first {
                        detail.push_str(" (both exercise: ");
                        first = false;
                    } else {
                        detail.push_str(", ");
                    }
                    let _ = write!(detail, "{callee}");
                }
            }
            if !first {
                detail.push(')');
            }

            diagnostics.push(
                Diagnostic::new(
                    SimilarityRuleId::OverlappingTests.as_str(),
                    cfg.severity,
                    left.decl.span.clone(),
                    format!(
                        "test `{}` overlaps {pct}% with test `{}`{detail}",
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
        }

        diagnostics
    }
}

fn format_percent(score: f64) -> String {
    format!("{:.0}", (score * 100.0).clamp(0.0, 100.0))
}
