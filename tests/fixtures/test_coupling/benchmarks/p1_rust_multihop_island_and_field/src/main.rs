mod renderer;

struct Metrics;

impl Metrics {
    fn snapshot(&self) -> usize {
        1
    }
}

fn main() {
    let mut r = renderer::MessageRenderer::new();
    let m = Metrics;
    println!("{} {}", r.render("hi"), m.snapshot());
}
