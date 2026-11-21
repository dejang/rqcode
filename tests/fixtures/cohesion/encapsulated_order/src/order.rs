pub struct Order {
    total: u64,
    status: String,
}

impl Order {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            status: "new".into(),
        }
    }

    pub fn apply_discount(&mut self, discount: u64) {
        self.total = self.total.saturating_sub(discount);
    }

    pub fn mark_shipped(&mut self) {
        self.status = "shipped".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_lifecycle() {
        let mut o = Order::new(100);
        o.apply_discount(10);
        o.mark_shipped();
    }
}
