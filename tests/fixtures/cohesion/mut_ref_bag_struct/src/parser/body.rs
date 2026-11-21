use super::macro_tokens::MacroTokenScanner;

pub(super) struct BodyWalker<'a, 'b> {
    pub(super) source: &'a str,
    pub(super) tokens: &'b mut Vec<String>,
    pub(super) calls: &'b mut Vec<String>,
    pub(super) field_accesses: &'b mut Vec<String>,
    pub(super) idents: &'b mut Vec<String>,
}

impl BodyWalker<'_, '_> {
    pub(super) fn walk(&mut self) {
        for word in self.source.split_whitespace() {
            self.tokens.push(word.to_string());
            self.idents.push(word.to_string());
            if word.ends_with('!') {
                let mut scanner = MacroTokenScanner {
                    source: self.source,
                    calls: self.calls,
                    field_accesses: self.field_accesses,
                };
                scanner.scan(word);
            }
        }
    }
}
