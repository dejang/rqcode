use super::extract::normalize_ident;

pub(super) fn analyze_body(source: &str) -> Vec<String> {
    source.split_whitespace().map(normalize_ident).collect()
}
