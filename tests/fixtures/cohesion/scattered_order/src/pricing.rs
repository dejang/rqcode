use crate::order::Order;

pub fn apply_discount(order: &mut Order, discount: u64) {
    order.total = order.total.saturating_sub(discount);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discount_works() {
        let mut o = Order {
            total: 100,
            status: "new".into(),
        };
        apply_discount(&mut o, 10);
    }
}
