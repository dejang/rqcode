use crate::order::Order;

pub fn ship_order(order: &mut Order) {
    order.shipped = true;
}

pub fn compute_cart(prices: &[u64], limit: u64) -> u64 {
    let mut total = 0;
    for p in prices {
        if *p > 3 {
            total += *p;
        }
    }
    if total > limit {
        return total - limit;
    }
    0
}

pub fn pending_feature() {
    todo!("wire carrier API");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shipping_flag() {
        let mut order = Order {
            subtotal: 50,
            discount: 0,
            shipped: false,
        };
        ship_order(&mut order);
        assert!(order.shipped);
    }
}
