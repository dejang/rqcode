use ast_grep_core::language::Language;
use ast_grep_core::tree_sitter::{LanguageExt, TSLanguage};
use rqcode::Analyzer;
use rqcode::ast::{
    CommentStyle, FileAst, FileRole, LanguageGrammar, LanguageId, LanguageSpec, ModulePath,
    SyntaxTree,
};
use rqcode::config::Config;
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use rqcode::lang::{LanguageFrontend, LanguageRegistry};
use rqcode::rules::RuleRegistry;
use rqcode::rules::ast_grep::RuleOrigin;
use rqcode::source::{SourceFile, SourceTree};
use std::borrow::Cow;
use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

struct CustomScriptGrammar {
    base: LanguageSpec,
}

impl LanguageGrammar for CustomScriptGrammar {
    fn id(&self) -> LanguageId {
        LanguageId::new("customscript")
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["customscript", "csx"]
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["csx"]
    }

    fn comment_style(&self) -> CommentStyle {
        self.base.comment_style()
    }

    fn member_separator(&self) -> &'static str {
        self.base.member_separator()
    }

    fn ts_language(&self) -> TSLanguage {
        self.base.get_ts_language()
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        self.base.kind_to_id(kind)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.base.field_to_id(field)
    }

    fn expando_char(&self) -> char {
        self.base.expando_char()
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        self.base.pre_process_pattern(query)
    }
}

struct CustomScriptFrontend;

impl LanguageFrontend for CustomScriptFrontend {
    fn grammar(&self) -> LanguageSpec {
        LanguageSpec::new(Arc::new(CustomScriptGrammar {
            base: typescript_language_spec(),
        }))
    }

    fn module_path(&self, root: &Path, relative_path: &Path) -> ModulePath {
        TypeScriptFrontend.module_path(root, &relative_path.with_extension("ts"))
    }

    fn file_role(&self, relative_path: &Path) -> FileRole {
        TypeScriptFrontend.file_role(&relative_path.with_extension("ts"))
    }

    fn extract_module(
        &self,
        tree: &SyntaxTree,
        module_path: ModulePath,
        role: FileRole,
    ) -> FileAst {
        TypeScriptFrontend.extract_module(tree, module_path, role)
    }
}

#[test]
fn ast_and_rules_never_depend_on_concrete_languages() -> Result<(), Box<dyn Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ast_dir = manifest_dir.join("src/ast");
    let rules_dir = manifest_dir.join("src/rules");

    let mut pending_dirs = vec![ast_dir.clone(), rules_dir.clone()];
    let mut rs_files = Vec::new();
    while let Some(dir) = pending_dirs.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending_dirs.push(path);
            } else if path.extension().and_then(OsStr::to_str) == Some("rs") {
                rs_files.push(path);
            }
        }
    }
    rs_files.sort();

    assert!(
        rs_files.iter().any(|p| p.starts_with(&ast_dir)),
        "expected Rust source files under {}",
        ast_dir.display()
    );
    assert!(
        rs_files.iter().any(|p| p.starts_with(&rules_dir)),
        "expected Rust source files under {}",
        rules_dir.display()
    );

    let forbidden = [
        "ast_grep_language",
        "crate::lang::rust",
        "crate::lang::swift",
        "crate::lang::typescript",
        "crate::lang::{",
        "super::lang::",
        "\"main.rs\"",
        "\"lib.rs\"",
        "Visibility::Crate",
        "struct literal",
        "has_derive",
    ];

    let mut violations = Vec::new();
    for path in &rs_files {
        let content = std::fs::read_to_string(path)?;
        let relative = path.strip_prefix(manifest_dir).unwrap_or(path);
        let mut pending_cfg_test = false;
        let mut cfg_test_depth: usize = 0;

        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if cfg_test_depth > 0 {
                for ch in trimmed.chars() {
                    if ch == '{' {
                        cfg_test_depth += 1;
                    } else if ch == '}' {
                        cfg_test_depth = cfg_test_depth.saturating_sub(1);
                    }
                }
                continue;
            }
            if trimmed.starts_with("#[cfg(test)]") {
                pending_cfg_test = true;
                continue;
            }
            if pending_cfg_test {
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                    continue;
                }
                let opens = trimmed.chars().filter(|&c| c == '{').count();
                let closes = trimmed.chars().filter(|&c| c == '}').count();
                if opens > 0 {
                    cfg_test_depth = opens.saturating_sub(closes);
                    pending_cfg_test = false;
                    continue;
                }
                if trimmed.ends_with(';') {
                    pending_cfg_test = false;
                    continue;
                }
                continue;
            }
            for needle in forbidden {
                if line.contains(needle) {
                    violations.push(format!(
                        "{}:{}: forbidden reference `{needle}` in `{}`",
                        relative.display(),
                        idx + 1,
                        trimmed
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "language-agnostic core (src/ast and src/rules) leaked concrete language details:\n{}",
        violations.join("\n")
    );
    Ok(())
}

#[test]
fn custom_language_frontend_is_injectable_without_touching_core() -> Result<(), Box<dyn Error>> {
    let mut languages = LanguageRegistry::empty();
    languages.register(Box::new(CustomScriptFrontend));

    assert_eq!(
        languages
            .frontend_for_path(Path::new("src/order.csx"))
            .map(LanguageFrontend::id),
        Some(LanguageId::new("customscript"))
    );
    assert!(
        languages
            .frontend_for_path(Path::new("src/lib.rs"))
            .is_none()
    );

    let mut config = Config::default();
    config.ast_grep.builtin_rules = false;

    let mut rules = RuleRegistry::from_config(&config, Path::new("."), &languages)?;
    let yaml_rule = "\
id: no-console-log
language: customscript
severity: warning
message: avoid console.log in production code
rule:
  pattern: console.log($$$ARGS)
";
    for rule in RuleOrigin::Inline.load_rules(yaml_rule, &languages)? {
        rules.register(Box::new(rule));
    }

    let order_src = "\
export class Order {
    constructor(private secret: number, public total: number) {}
    static create(total: number): Order { return new Order(1, total); }
    static statelessHelper(x: number): number { return x + 1; }
    private internalStep(): void { this.secret += 1; }
    public applyDiscount(rate: number): number {
        console.log(rate);
        this.internalStep();
        return this.total * (1 - rate);
    }
}
";
    let test_src = "\
import { Order } from './order';
describe('Order suite', () => {
    it('applies discount', () => {
        const o = Order.create(100);
        o.applyDiscount(0.1);
        (o as any).internalStep();
    });
});
";

    let spec = CustomScriptFrontend.grammar();
    let sources = SourceTree::from_files(
        ".",
        vec![
            SourceFile::new("src/order.csx", "src/order.csx", spec.clone(), order_src),
            SourceFile::new("src/order.test.csx", "src/order.test.csx", spec, test_src),
        ],
    );

    let report = Analyzer::new(languages, rules).check_sources(&sources, &config);
    assert_eq!(report.stats().files_analyzed, 2);
    assert_eq!(report.stats().types_analyzed, 1);

    let found: Vec<(&str, &str)> = report
        .diagnostics()
        .iter()
        .map(|d| (d.rule_id(), d.message()))
        .collect();
    assert!(
        found.iter().any(
            |(id, msg)| *id == "stateless-impl-method" && msg.contains("Order.statelessHelper")
        ),
        "expected stateless-impl-method diagnostic for custom language, got: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(id, msg)| *id == "test-calls-private-api" && msg.contains("Order.internalStep")),
        "expected test-calls-private-api diagnostic for custom language, got: {found:?}"
    );
    assert!(
        found.iter().any(|(id, _)| *id == "no-console-log"),
        "expected custom ast-grep YAML rule to match custom language, got: {found:?}"
    );
    assert!(
        !found
            .iter()
            .any(|(_, msg)| msg.contains("Order.constructor") || msg.contains("Order.create")),
        "constructor and factory returning Order must not be flagged: {found:?}"
    );
    Ok(())
}
