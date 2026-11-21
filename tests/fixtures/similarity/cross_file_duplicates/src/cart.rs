pub fn compute_cart_subtotal(prices: &[u64], coupon: u64) -> u64 {
    let mut acc = 0;
    for price in prices {
        if *price > 5 {
            acc += *price;
        }
    }
    if acc > coupon {
        return acc - coupon;
    }
    0
}
