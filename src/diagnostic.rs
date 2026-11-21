use serde::{Deserialize, Serialize};
use std::fmt::{self, Write as _};
use std::path::PathBuf;

/// Severity level of an analysis diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

/// 1-based source location span for human- and agent-readable reporting.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Span {
    file: PathBuf,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
}

impl Span {
    pub fn new(
        file: impl Into<PathBuf>,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> Self {
        Self {
            file: file.into(),
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }

    #[must_use]
    pub fn file(&self) -> &PathBuf {
        &self.file
    }

    #[must_use]
    pub fn start_line(&self) -> usize {
        self.start_line
    }

    #[must_use]
    pub fn start_col(&self) -> usize {
        self.start_col
    }

    #[must_use]
    pub fn end_line(&self) -> usize {
        self.end_line
    }

    #[must_use]
    pub fn end_col(&self) -> usize {
        self.end_col
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.file.display(),
            self.start_line,
            self.start_col
        )
    }
}

/// Secondary location attached to a diagnostic (e.g., duplicate pair or scattered helper).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedLocation {
    span: Span,
    label: String,
}

impl RelatedLocation {
    pub fn new(span: Span, label: impl Into<String>) -> Self {
        Self {
            span,
            label: label.into(),
        }
    }

    #[must_use]
    pub fn span(&self) -> &Span {
        &self.span
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A single actionable finding emitted by an analysis rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    rule_id: String,
    severity: Severity,
    span: Span,
    message: String,
    suggestion: Option<String>,
    related: Vec<RelatedLocation>,
}

impl Diagnostic {
    pub fn new(
        rule_id: impl Into<String>,
        severity: Severity,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            severity,
            span,
            message: message.into(),
            suggestion: None,
            related: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    #[must_use]
    pub fn with_related(mut self, span: Span, label: impl Into<String>) -> Self {
        self.related.push(RelatedLocation::new(span, label));
        self
    }

    #[must_use]
    pub fn rule_id(&self) -> &str {
        &self.rule_id
    }

    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }

    #[must_use]
    pub fn span(&self) -> &Span {
        &self.span
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn suggestion(&self) -> Option<&str> {
        self.suggestion.as_deref()
    }

    #[must_use]
    pub fn related(&self) -> &[RelatedLocation] {
        &self.related
    }

    #[must_use]
    pub fn sort_key(&self) -> (&PathBuf, usize, usize, &str, &str) {
        (
            self.span.file(),
            self.span.start_line(),
            self.span.start_col(),
            self.rule_id(),
            self.message(),
        )
    }

    #[must_use]
    pub fn format_text(&self) -> String {
        let mut out = format!(
            "[{}] {} ({}): {}\n",
            self.severity, self.rule_id, self.span, self.message
        );
        for rel in &self.related {
            let _ = writeln!(out, "  -> {}: {}", rel.span(), rel.label());
        }
        if let Some(fix) = &self.suggestion {
            let _ = writeln!(out, "  fix: {fix}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_diagnostic_with_related_locations_and_fix() {
        let diag = Diagnostic::new(
            "scattered-concept",
            Severity::Error,
            Span::new("src/order.rs", 10, 1, 15, 2),
            "Order has scattered helpers",
        )
        .with_related(
            Span::new("src/pricing.rs", 22, 1, 30, 2),
            "mutates field `total`",
        )
        .with_suggestion("Move helpers into `impl Order` in `src/order.rs`");

        let rendered = diag.format_text();
        assert!(rendered.contains("[error] scattered-concept (src/order.rs:10:1)"));
        assert!(rendered.contains("-> src/pricing.rs:22:1: mutates field `total`"));
        assert!(rendered.contains("fix: Move helpers into `impl Order` in `src/order.rs`"));
    }

    #[test]
    fn serializes_diagnostic_to_json() {
        let diag = Diagnostic::new(
            "duplicate-function",
            Severity::Warning,
            Span::new("src/a.rs", 1, 1, 5, 2),
            "similar to b",
        );

        let value: serde_json::Value =
            serde_json::to_value(&diag).expect("diagnostic should serialize");
        assert_eq!(value["rule_id"], "duplicate-function");
        assert_eq!(value["severity"], "warning");
        assert_eq!(value["span"]["start_line"], 1);
    }
}
