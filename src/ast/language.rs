use crate::ast::item::Suppression;
use ast_grep_core::language::Language;
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use serde::{Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

/// Canonical identifier for a source language (e.g., `"rust"`, `"typescript"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LanguageId(&'static str);

impl LanguageId {
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for LanguageId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

/// Comment syntax specification used to parse suppression directives for a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentStyle {
    pub line_prefix: &'static str,
    pub file_level_prefix: Option<&'static str>,
    pub block_delimiters: Option<(&'static str, &'static str)>,
    pub ignore_line_prefixes: &'static [&'static str],
}

impl CommentStyle {
    #[must_use]
    pub const fn new(
        line_prefix: &'static str,
        file_level_prefix: Option<&'static str>,
        block_delimiters: Option<(&'static str, &'static str)>,
        ignore_line_prefixes: &'static [&'static str],
    ) -> Self {
        Self {
            line_prefix,
            file_level_prefix,
            block_delimiters,
            ignore_line_prefixes,
        }
    }

    /// Extract inline `rqcode:allow(...)` and `rqcode:allow-file(...)` directives from `source`.
    #[must_use]
    pub fn extract_suppressions(&self, source: &str) -> Vec<Suppression> {
        let mut suppressions = Vec::new();
        let mut in_block_comment = false;

        for (idx, line) in source.lines().enumerate() {
            let line_num = idx + 1;
            if !in_block_comment
                && let Some(fl_prefix) = self.file_level_prefix
                && let Some(rest) = line.trim().strip_prefix(fl_prefix)
            {
                push_comment_directives(rest, true, line_num, &mut suppressions);
                continue;
            }

            let mut slice = line;
            while !slice.is_empty() {
                if in_block_comment {
                    let (next_slice, still_in_block) =
                        self.consume_block_comment(slice, line_num, &mut suppressions);
                    slice = next_slice;
                    in_block_comment = still_in_block;
                    continue;
                }

                let line_pos = self.find_line_comment(slice);
                let block_pos = self
                    .block_delimiters
                    .and_then(|(open, _)| slice.find(open).map(|pos| (pos, open.len())));

                match (line_pos, block_pos) {
                    (Some((lp, _)), Some((bp, blen))) if bp < lp => {
                        slice = &slice[bp + blen..];
                        in_block_comment = true;
                    }
                    (Some((lp, llen)), _) => {
                        push_comment_directives(
                            &slice[lp + llen..],
                            false,
                            line_num,
                            &mut suppressions,
                        );
                        break;
                    }
                    (None, Some((bp, blen))) => {
                        slice = &slice[bp + blen..];
                        in_block_comment = true;
                    }
                    (None, None) => break,
                }
            }
        }

        suppressions
    }

    fn consume_block_comment<'a>(
        &self,
        slice: &'a str,
        line_num: usize,
        suppressions: &mut Vec<Suppression>,
    ) -> (&'a str, bool) {
        let Some((_, close)) = self.block_delimiters else {
            return ("", false);
        };
        if let Some(close_idx) = slice.find(close) {
            push_comment_directives(&slice[..close_idx], false, line_num, suppressions);
            return (&slice[close_idx + close.len()..], false);
        }
        push_comment_directives(slice, false, line_num, suppressions);
        ("", true)
    }

    fn find_line_comment(&self, slice: &str) -> Option<(usize, usize)> {
        if self.line_prefix.is_empty() {
            return None;
        }
        let mut offset = 0;
        while let Some(rel) = slice[offset..].find(self.line_prefix) {
            let pos = offset + rel;
            let candidate = &slice[pos..];
            let mut ignored = false;
            for &prefix in self.ignore_line_prefixes {
                if !prefix.is_empty() && candidate.starts_with(prefix) {
                    ignored = true;
                    break;
                }
            }
            if !ignored {
                return Some((pos, self.line_prefix.len()));
            }
            offset = pos + self.line_prefix.len();
        }
        None
    }
}

fn push_comment_directives(
    body: &str,
    default_file_level: bool,
    line_num: usize,
    out: &mut Vec<Suppression>,
) {
    for (marker, is_file_level) in [
        ("rqcode:allow-file(", true),
        ("rqcode:allow(", default_file_level),
    ] {
        let mut rest = body;
        while let Some((_, after)) = rest.split_once(marker) {
            let Some((rule_list, tail)) = after.split_once(')') else {
                break;
            };
            for raw_rule in rule_list.split(',') {
                let rule_id = raw_rule.trim();
                if rule_id.is_empty() {
                    continue;
                }
                if is_file_level {
                    out.push(Suppression::file_level(rule_id));
                } else {
                    out.push(Suppression::line_level(rule_id, line_num));
                }
            }
            rest = tail;
        }
    }
}

/// Tree-sitter / `ast-grep` grammar and lexical metadata provided by a language frontend.
pub trait LanguageGrammar: Send + Sync {
    fn id(&self) -> LanguageId;
    fn aliases(&self) -> &'static [&'static str];
    fn extensions(&self) -> &'static [&'static str];
    fn comment_style(&self) -> CommentStyle;
    fn member_separator(&self) -> &'static str;
    /// Language-appropriate noun for direct field-initialization syntax (defaults to `"field initializer"`).
    fn field_init_noun(&self) -> &'static str {
        "field initializer"
    }
    fn ts_language(&self) -> TSLanguage;
    fn kind_to_id(&self, kind: &str) -> u16;
    fn field_to_id(&self, field: &str) -> Option<u16>;
    fn expando_char(&self) -> char;
    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str>;
}

/// Cloneable handle to an injected [`LanguageGrammar`] usable with `ast-grep`.
#[derive(Clone)]
pub struct LanguageSpec(Arc<dyn LanguageGrammar>);

impl LanguageSpec {
    #[must_use]
    pub fn new(grammar: Arc<dyn LanguageGrammar>) -> Self {
        Self(grammar)
    }

    #[must_use]
    pub fn id(&self) -> LanguageId {
        self.0.id()
    }

    #[must_use]
    pub fn aliases(&self) -> &'static [&'static str] {
        self.0.aliases()
    }

    #[must_use]
    pub fn extensions(&self) -> &'static [&'static str] {
        self.0.extensions()
    }

    #[must_use]
    pub fn comment_style(&self) -> CommentStyle {
        self.0.comment_style()
    }

    #[must_use]
    pub fn member_separator(&self) -> &'static str {
        self.0.member_separator()
    }

    #[must_use]
    pub fn field_init_noun(&self) -> &'static str {
        self.0.field_init_noun()
    }

    #[must_use]
    pub fn matches_name(&self, raw: &str) -> bool {
        let needle = raw.trim();
        if needle.is_empty() {
            return false;
        }
        if self.id().as_str().eq_ignore_ascii_case(needle) {
            return true;
        }
        for &alias in self.aliases() {
            if alias.eq_ignore_ascii_case(needle) {
                return true;
            }
        }
        for &ext in self.extensions() {
            if ext.eq_ignore_ascii_case(needle) {
                return true;
            }
        }
        false
    }
}

impl PartialEq for LanguageSpec {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for LanguageSpec {}

impl Hash for LanguageSpec {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl fmt::Debug for LanguageSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LanguageSpec").field(&self.id()).finish()
    }
}

impl fmt::Display for LanguageSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.id(), f)
    }
}

impl Language for LanguageSpec {
    fn from_path<P: AsRef<Path>>(_path: P) -> Option<Self> {
        None
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        self.0.kind_to_id(kind)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.0.field_to_id(field)
    }

    fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        builder.build(|src| StrDoc::try_new(src, self.clone()))
    }

    fn expando_char(&self) -> char {
        self.0.expando_char()
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        self.0.pre_process_pattern(query)
    }
}

impl LanguageExt for LanguageSpec {
    fn get_ts_language(&self) -> TSLanguage {
        self.0.ts_language()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::rust::rust_language_spec;

    #[test]
    fn extracts_block_comment_hash_comment_and_allow_file_suppressions() {
        let c_style = CommentStyle::new("//", None, Some(("/*", "*/")), &[]);
        let c_src =
            "/* rqcode:allow-file(max-file-lines) */\n/* rqcode:allow(no-todo) */\nfn f() {}\n";
        let c_supps = c_style.extract_suppressions(c_src);
        assert_eq!(c_supps.len(), 2);
        assert!(c_supps.iter().any(|s| s.matches("max-file-lines", 99)));
        assert!(c_supps.iter().any(|s| s.matches("no-todo", 3)));

        let hash_style = CommentStyle::new("#", None, None, &["#!", "#["]);
        let py_src = "#!/usr/bin/env python3\n# rqcode:allow-file(scattered-tests)\n#[ignored]\n# rqcode:allow(dup)\ndef f(): pass\n";
        let py_supps = hash_style.extract_suppressions(py_src);
        assert_eq!(py_supps.len(), 2);
        assert!(py_supps.iter().any(|s| s.matches("scattered-tests", 50)));
        assert!(py_supps.iter().any(|s| s.matches("dup", 5)));
    }

    #[test]
    fn language_spec_matches_names_case_insensitively() {
        let rust = rust_language_spec();
        assert!(rust.matches_name("Rust"));
        assert!(rust.matches_name("rust"));
        assert!(rust.matches_name("rs"));
        assert!(!rust.matches_name("cobol"));
        assert_eq!(
            serde_json::to_string(&rust.id()).expect("serialize id"),
            "\"rust\""
        );
    }
}
