pub struct Order {
    discount: u64,
}

impl Order {
    pub fn new(discount: u64) -> Self {
        Self { discount }
    }

    pub(crate) fn discount(&self) -> u64 {
        self.discount
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_discount() {
        let o = Order::new(10);
        assert_eq!(o.discount(), 10);
    }
}
