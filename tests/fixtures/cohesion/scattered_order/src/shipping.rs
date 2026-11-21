use crate::order::Order;

pub fn mark_shipped(order: &mut Order) {
    order.status = "shipped".to_string();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipping_works() {
        let mut o = Order {
            total: 100,
            status: "new".into(),
        };
        mark_shipped(&mut o);
    }
}
