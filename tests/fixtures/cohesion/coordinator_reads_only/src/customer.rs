pub struct Customer {
    pub tier: u8,
}

#[test]
fn customer_tier() {
    let _c = Customer { tier: 1 };
}
