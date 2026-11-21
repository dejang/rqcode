pub struct Cart {
    pub subtotal: u64,
}

#[test]
fn cart_subtotal() {
    let _c = Cart { subtotal: 50 };
}
