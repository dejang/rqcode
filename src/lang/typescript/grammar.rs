use crate::ast::{CommentStyle, LanguageGrammar, LanguageId, LanguageSpec};
use ast_grep_core::language::Language;
use ast_grep_core::tree_sitter::{LanguageExt, TSLanguage};
use ast_grep_language::{Tsx as AstGrepTsx, TypeScript as AstGrepTypeScript};
use std::borrow::Cow;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum TsDialect {
    TypeScript,
    Tsx,
}

struct TsDialectGrammar {
    dialect: TsDialect,
}

impl LanguageGrammar for TsDialectGrammar {
    fn id(&self) -> LanguageId {
        match self.dialect {
            TsDialect::TypeScript => LanguageId::new("typescript"),
            TsDialect::Tsx => LanguageId::new("tsx"),
        }
    }

    fn semantic_id(&self) -> LanguageId {
        LanguageId::new("typescript")
    }

    fn aliases(&self) -> &'static [&'static str] {
        match self.dialect {
            TsDialect::TypeScript => &["typescript", "ts"],
            TsDialect::Tsx => &["tsx"],
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        match self.dialect {
            TsDialect::TypeScript => &["ts", "mts", "cts"],
            TsDialect::Tsx => &["tsx"],
        }
    }

    fn comment_style(&self) -> CommentStyle {
        CommentStyle::new("//", None, Some(("/*", "*/")), &[])
    }

    fn member_separator(&self) -> &'static str {
        "."
    }

    fn ts_language(&self) -> TSLanguage {
        match self.dialect {
            TsDialect::TypeScript => AstGrepTypeScript.get_ts_language(),
            TsDialect::Tsx => AstGrepTsx.get_ts_language(),
        }
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        match self.dialect {
            TsDialect::TypeScript => AstGrepTypeScript.kind_to_id(kind),
            TsDialect::Tsx => AstGrepTsx.kind_to_id(kind),
        }
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        match self.dialect {
            TsDialect::TypeScript => AstGrepTypeScript.field_to_id(field),
            TsDialect::Tsx => AstGrepTsx.field_to_id(field),
        }
    }

    fn expando_char(&self) -> char {
        match self.dialect {
            TsDialect::TypeScript => AstGrepTypeScript.expando_char(),
            TsDialect::Tsx => AstGrepTsx.expando_char(),
        }
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        match self.dialect {
            TsDialect::TypeScript => AstGrepTypeScript.pre_process_pattern(query),
            TsDialect::Tsx => AstGrepTsx.pre_process_pattern(query),
        }
    }
}

/// Construct the [`LanguageSpec`] for TypeScript (`.ts`, `.mts`, `.cts`).
#[must_use]
pub fn typescript_language_spec() -> LanguageSpec {
    LanguageSpec::new(Arc::new(TsDialectGrammar {
        dialect: TsDialect::TypeScript,
    }))
}

/// Construct the [`LanguageSpec`] for TSX (`.tsx`).
#[must_use]
pub fn tsx_language_spec() -> LanguageSpec {
    LanguageSpec::new(Arc::new(TsDialectGrammar {
        dialect: TsDialect::Tsx,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SyntaxTree;

    #[test]
    fn dialect_specs_separate_grammar_ids_and_unify_semantic_id() {
        let ts = typescript_language_spec();
        let tsx = tsx_language_spec();

        assert_ne!(ts, tsx);
        assert_eq!(ts.id().as_str(), "typescript");
        assert_eq!(tsx.id().as_str(), "tsx");
        assert_eq!(ts.semantic_id(), tsx.semantic_id());
        assert_eq!(tsx.semantic_id().as_str(), "typescript");
        assert!(ts.matches_name("TypeScript"));
        assert!(ts.matches_name("ts"));
        assert!(!ts.matches_name("tsx"));
        assert!(tsx.matches_name("Tsx"));
        assert!(tsx.matches_name("tsx"));
        assert!(!tsx.matches_name("typescript"));
        assert_eq!(ts.extensions(), &["ts", "mts", "cts"]);
        assert_eq!(tsx.extensions(), &["tsx"]);
        assert_ne!(
            ts.kind_to_id("call_expression"),
            tsx.kind_to_id("call_expression")
        );

        let tree = SyntaxTree::parse(
            "src/Card.tsx",
            "src/Card.tsx",
            tsx,
            "export const Card = () => <div>ok</div>;\n",
        );
        assert_eq!(tree.language_id().as_str(), "tsx");
        assert_eq!(tree.semantic_language_id().as_str(), "typescript");
        assert!(!tree.root().is_error());
    }
}
