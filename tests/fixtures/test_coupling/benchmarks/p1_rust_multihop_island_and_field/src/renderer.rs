pub trait Part {
    fn render(&self, s: &str) -> String;
}

struct Header;

impl Part for Header {
    fn render(&self, s: &str) -> String {
        format!("<h1>{s}</h1>")
    }
}

pub struct MessageRenderer {
    sub_renderers: Vec<Box<dyn Part>>,
    pub last_rendered: Option<String>,
}

impl MessageRenderer {
    pub fn new() -> Self {
        Self {
            sub_renderers: vec![Box::new(Header)],
            last_rendered: None,
        }
    }

    pub fn render(&mut self, s: &str) -> String {
        let out: String = self.sub_renderers.iter().map(|p| p.render(s)).collect();
        self.last_rendered = Some(out.clone());
        out
    }

    #[allow(dead_code)]
    pub fn snapshot(&self) -> usize {
        self.count_internal_nodes()
    }

    #[allow(dead_code)]
    fn count_internal_nodes(&self) -> usize {
        self.sub_renderers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_test_only_field_and_subgraph() {
        let mut r = MessageRenderer::new();
        let _ = r.render("hi");
        assert_eq!(r.last_rendered.as_deref(), Some("<h1>hi</h1>"));
        assert_eq!(r.snapshot(), 1);
    }
}
