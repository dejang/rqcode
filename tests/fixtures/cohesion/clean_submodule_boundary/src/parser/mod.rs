mod body;
mod extract;

use self::extract::extract_file;

pub struct FileAst {
    pub imports: Vec<String>,
    pub tokens: Vec<String>,
    pub top_level_idents: Vec<String>,
}

pub struct ParserFrontend;

impl ParserFrontend {
    pub fn extract(&self, source: &str) -> FileAst {
        extract_file(source)
    }
}
