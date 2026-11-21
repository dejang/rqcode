pub(super) struct FileExtractor<'a> {
    source: &'a str,
    pub(super) imports: Vec<String>,
    pub(super) types: Vec<String>,
    pub(super) functions: Vec<String>,
    pub(super) top_level_idents: Vec<String>,
}

impl<'a> FileExtractor<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            source,
            imports: Vec::new(),
            types: Vec::new(),
            functions: Vec::new(),
            top_level_idents: Vec::new(),
        }
    }

    pub(super) fn walk(&mut self) {
        for line in self.source.lines() {
            if line.starts_with("use ") {
                self.imports.push(line.to_string());
            } else if line.starts_with("struct ") {
                self.types.push(line.to_string());
            } else if line.starts_with("fn ") {
                self.functions.push(line.to_string());
            } else if !line.is_empty() {
                self.top_level_idents.push(line.to_string());
            }
        }
    }
}
