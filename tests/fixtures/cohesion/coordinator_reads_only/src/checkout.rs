use crate::cart::Cart;
use crate::customer::Customer;

pub fn quote(customer: &Customer, cart: &Cart) -> u64 {
    if customer.tier > 1 {
        cart.subtotal
    } else {
        cart.subtotal + 5
    }
}
