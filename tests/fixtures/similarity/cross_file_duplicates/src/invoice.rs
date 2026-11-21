pub fn compute_invoice_total(items: &[u64], discount: u64) -> u64 {
    let mut sum = 0;
    for item in items {
        if *item > 10 {
            sum += *item;
        }
    }
    if sum > discount {
        return sum - discount;
    }
    0
}
