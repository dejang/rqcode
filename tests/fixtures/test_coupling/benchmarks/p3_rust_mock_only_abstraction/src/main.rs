mod order;

fn main() {
    let engine = order::OrderEngine::new(order::DefaultTaxCalc);
    println!("{}", engine.total(100));
}
