pub fn compute_invoice_total(prices: &[i64], coupon: i64, cap: i64) -> i64 {
    let mut acc = 0;
    for &price in prices {
        if price > 0 {
            acc += price;
        }
        if acc > cap {
            acc = cap;
        }
    }
    if acc > coupon { acc - coupon } else { 0 }
}
pub fn compute_cart_subtotal(items: &[i64], discount: i64, ceiling: i64) -> i64 {
    let mut sum = 0;
    let extra = 0;
    for &item in items {
        if item > 0 {
            sum += item;
        }
        if sum > ceiling {
            sum = ceiling;
        }
    }
    if sum > discount {
        sum - discount
    } else {
        extra
    }
}
pub fn handle_user_pipeline(input: &str) -> String {
    let u = fetch_user(input);
    let c = validate_email(u);
    format_greeting(trim_name(c))
}
pub fn handle_order_pipeline(raw: &str) -> String {
    let inv = load_invoice(raw);
    let t = compute_tax(inv);
    render_receipt(round_money(t))
}
pub fn lookup_http_status(code: u16) -> &'static str {
    match code {
        200 => "ok",
        201 => "c",
        204 => "n",
        400 => "b",
        401 => "u",
        403 => "f",
        404 => "m",
        500 => "e",
        _ => "x",
    }
}
pub fn lookup_day_name(day: u8) -> &'static str {
    match day {
        1 => "m",
        2 => "t",
        3 => "w",
        4 => "h",
        5 => "f",
        6 => "s",
        7 => "u",
        8 => "a",
        _ => "z",
    }
}
