mod body;
mod extract;

use self::extract::parse_source;

pub struct ParsedFile {
    pub functions: Vec<String>,
}

pub fn parse_file(source: &str) -> ParsedFile {
    parse_source(source)
}
