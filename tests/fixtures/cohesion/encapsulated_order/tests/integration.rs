use crate::order::Order;

#[test]
fn e2e_order() {
    let mut o = Order::new(50);
    o.mark_shipped();
}
