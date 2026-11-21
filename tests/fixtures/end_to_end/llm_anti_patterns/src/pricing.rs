use crate::order::Order;

pub fn apply_discount(order: &mut Order, amount: u64) {
    if amount > 0 {
        order.discount = amount;
        order.subtotal = order.subtotal.saturating_sub(amount);
    }
}

pub fn compute_invoice(values: &[u64], cap: u64) -> u64 {
    let mut sum = 0;
    for v in values {
        if *v > 5 {
            sum += *v;
        }
    }
    if sum > cap {
        return sum - cap;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discount_small() {
        let mut order = Order {
            subtotal: 100,
            discount: 0,
            shipped: false,
        };
        apply_discount(&mut order, 10);
        assert_eq!(order.subtotal, 90);
        assert_eq!(order.discount, 10);
    }

    #[test]
    fn test_discount_large() {
        let mut cart = Order {
            subtotal: 200,
            discount: 0,
            shipped: false,
        };
        apply_discount(&mut cart, 50);
        assert_eq!(cart.subtotal, 150);
        assert_eq!(cart.discount, 50);
    }
}
