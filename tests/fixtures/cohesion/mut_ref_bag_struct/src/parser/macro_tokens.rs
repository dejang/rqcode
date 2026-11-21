pub(super) struct MacroTokenScanner<'a, 'b> {
    pub(super) source: &'a str,
    pub(super) calls: &'b mut Vec<String>,
    pub(super) field_accesses: &'b mut Vec<String>,
}

impl MacroTokenScanner<'_, '_> {
    pub(super) fn scan(&mut self, token: &str) {
        if token.contains('.') {
            self.field_accesses.push(token.to_string());
        } else if !self.source.is_empty() {
            self.calls.push(token.to_string());
        }
    }
}
