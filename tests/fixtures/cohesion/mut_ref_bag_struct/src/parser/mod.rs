mod body;
mod extract;
mod macro_tokens;

use self::extract::extract_file;

pub struct ParsedFunction {
    pub tokens: Vec<String>,
    pub calls: Vec<String>,
    pub field_accesses: Vec<String>,
    pub idents: Vec<String>,
}

pub fn parse(source: &str) -> ParsedFunction {
    extract_file(source)
}
