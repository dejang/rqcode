use super::ParsedFunction;
use super::body::BodyWalker;

pub(super) fn extract_file(source: &str) -> ParsedFunction {
    let mut tokens = Vec::new();
    let mut calls = Vec::new();
    let mut field_accesses = Vec::new();
    let mut idents = Vec::new();

    let mut walker = BodyWalker {
        source,
        tokens: &mut tokens,
        calls: &mut calls,
        field_accesses: &mut field_accesses,
        idents: &mut idents,
    };
    walker.walk();

    idents.sort_unstable();
    idents.dedup();

    ParsedFunction {
        tokens,
        calls,
        field_accesses,
        idents,
    }
}
