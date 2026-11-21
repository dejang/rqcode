use super::FileAst;
use super::body::extract_body;

pub(super) fn extract_file(source: &str) -> FileAst {
    let mut extractor = FileExtractor::new(source);
    extractor.walk();
    extractor.finish()
}

struct FileExtractor<'a> {
    source: &'a str,
    imports: Vec<String>,
    tokens: Vec<String>,
    top_level_idents: Vec<String>,
}

impl<'a> FileExtractor<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            imports: Vec::new(),
            tokens: Vec::new(),
            top_level_idents: Vec::new(),
        }
    }

    fn walk(&mut self) {
        for line in self.source.lines() {
            if line.starts_with("use ") {
                self.imports.push(line.to_string());
            } else if line.starts_with("fn ") {
                let body = extract_body(line);
                self.tokens.extend(body.tokens);
            } else if !line.is_empty() {
                self.top_level_idents.push(line.to_string());
            }
        }
    }

    fn finish(mut self) -> FileAst {
        self.top_level_idents.sort_unstable();
        self.top_level_idents.dedup();
        FileAst {
            imports: self.imports,
            tokens: self.tokens,
            top_level_idents: self.top_level_idents,
        }
    }
}
