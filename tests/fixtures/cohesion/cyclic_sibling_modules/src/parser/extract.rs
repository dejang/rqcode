use super::ParsedFile;
use super::body::analyze_body;

pub(super) fn parse_source(source: &str) -> ParsedFile {
    let functions = analyze_body(source);
    ParsedFile { functions }
}

pub(super) fn normalize_ident(raw: &str) -> String {
    raw.trim().to_string()
}
