pub(super) struct ExtractedBody {
    pub(super) tokens: Vec<String>,
}

pub(super) fn extract_body(source: &str) -> ExtractedBody {
    let mut walker = BodyWalker::new(source);
    walker.walk();
    walker.finish()
}

struct BodyWalker<'a> {
    source: &'a str,
    tokens: Vec<String>,
}

impl<'a> BodyWalker<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            tokens: Vec::new(),
        }
    }

    fn walk(&mut self) {
        for word in self.source.split_whitespace() {
            self.tokens.push(word.to_string());
        }
    }

    fn finish(self) -> ExtractedBody {
        ExtractedBody {
            tokens: self.tokens,
        }
    }
}
