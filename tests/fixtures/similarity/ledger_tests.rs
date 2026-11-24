#[test]
fn test_invoice_total_with_cap() {
    let items = [10, -5, 40, 25, 30];
    let mut acc = 0;
    for &item in &items {
        if item > 0 {
            acc += item;
        }
        if acc > 80 {
            acc = 80;
        }
    }
    let final_total = if acc > 15 { acc - 15 } else { 0 };
    assert_eq!(final_total, 65);
}

#[test]
fn test_cart_subtotal_with_ceiling() {
    let prices = [12, -3, 35, 20, 45];
    let mut sum = 0;
    for &price in &prices {
        if price > 0 {
            sum += price;
        }
        if sum > 90 {
            sum = 90;
        }
    }
    let payable = if sum > 10 { sum - 10 } else { 0 };
    assert_eq!(payable, 80);
}

#[test]
fn test_distinct_string_normalization() {
    let raw = "  alpha, beta, gamma  ";
    let parts: Vec<&str> = raw.trim().split(',').map(str::trim).collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "alpha");
    assert_eq!(parts[2], "gamma");
}
