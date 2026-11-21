use crate::ast::codebase::Codebase;
use crate::diagnostic::{Diagnostic, Severity};
use serde::Serialize;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReportStats {
    pub files_analyzed: usize,
    pub types_analyzed: usize,
    pub functions_analyzed: usize,
    pub suppressed_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
}

/// Aggregated, deterministically ordered result of running all rules on a codebase.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    stats: ReportStats,
    diagnostics: Vec<Diagnostic>,
}

impl Report {
    #[must_use]
    pub fn from_raw(codebase: &Codebase, raw_diagnostics: Vec<Diagnostic>) -> Self {
        let mut diagnostics = Vec::with_capacity(raw_diagnostics.len());
        let mut suppressed_count = 0;

        for diag in raw_diagnostics {
            if codebase.is_suppressed(diag.rule_id(), diag.span()) {
                suppressed_count += 1;
            } else {
                diagnostics.push(diag);
            }
        }

        diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        let error_count = diagnostics
            .iter()
            .filter(|d| d.severity() == Severity::Error)
            .count();
        let warning_count = diagnostics
            .iter()
            .filter(|d| d.severity() == Severity::Warning)
            .count();

        let stats = ReportStats {
            files_analyzed: codebase.files().len(),
            types_analyzed: codebase.types().len(),
            functions_analyzed: codebase.functions().len(),
            suppressed_count,
            error_count,
            warning_count,
        };

        Self { stats, diagnostics }
    }

    #[must_use]
    pub fn stats(&self) -> ReportStats {
        self.stats
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.stats.error_count > 0
    }

    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for diag in &self.diagnostics {
            out.push_str(&diag.format_text());
        }

        if !self.diagnostics.is_empty() {
            out.push('\n');
        }

        let _ = write!(
            out,
            "rqcode: checked {} file(s), {} type(s), {} function(s) — {} error(s), {} warning(s)",
            self.stats.files_analyzed,
            self.stats.types_analyzed,
            self.stats.functions_analyzed,
            self.stats.error_count,
            self.stats.warning_count,
        );
        if self.stats.suppressed_count > 0 {
            let _ = write!(out, " ({} suppressed)", self.stats.suppressed_count);
        }
        out.push('\n');
        out
    }

    /// Serialize this report to pretty-printed JSON.
    ///
    /// # Errors
    /// Returns [`serde_json::Error`] if serialization fails.
    pub fn render_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SyntaxTree;
    use crate::diagnostic::Span;
    use crate::lang::LanguageFrontend;
    use crate::lang::rust::{RustFrontend, rust_language_spec};
    use std::path::Path;

    #[test]
    fn filters_suppressed_diagnostics_and_sorts_deterministically() {
        let src = "// rqcode:allow(no-todo-macro)\nfn a() { todo!(); }\n";
        let tree = SyntaxTree::parse("src/a.rs", "src/a.rs", rust_language_spec(), src);
        let ast = RustFrontend.extract(&tree, Path::new("."));
        let codebase = Codebase::build(vec![tree], vec![ast]);

        let raw = vec![
            Diagnostic::new(
                "duplicate-function",
                Severity::Error,
                Span::new("src/a.rs", 2, 1, 2, 10),
                "second",
            ),
            Diagnostic::new(
                "no-todo-macro",
                Severity::Error,
                Span::new("src/a.rs", 2, 10, 2, 16),
                "suppressed",
            ),
        ];

        let report = Report::from_raw(&codebase, raw);
        assert_eq!(report.stats().suppressed_count, 1);
        assert_eq!(report.diagnostics().len(), 1);
        assert_eq!(report.diagnostics()[0].rule_id(), "duplicate-function");
        assert!(report.has_errors());
        assert!(
            report
                .render_text()
                .contains("1 error(s), 0 warning(s) (1 suppressed)")
        );
    }
}
