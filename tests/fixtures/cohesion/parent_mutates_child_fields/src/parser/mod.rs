mod extract;

use self::extract::FileExtractor;

pub struct FileAst {
    pub imports: Vec<String>,
    pub types: Vec<String>,
    pub functions: Vec<String>,
    pub top_level_idents: Vec<String>,
}

pub struct ParserFrontend;

impl ParserFrontend {
    pub fn extract(&self, source: &str) -> FileAst {
        let mut extractor = FileExtractor::new(source);
        extractor.walk();
        extractor.top_level_idents.sort_unstable();
        extractor.top_level_idents.dedup();

        FileAst {
            imports: extractor.imports,
            types: extractor.types,
            functions: extractor.functions,
            top_level_idents: extractor.top_level_idents,
        }
    }
}
