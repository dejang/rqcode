pub struct Order {
    total: u64,
}

impl Order {
    pub fn new(total: u64) -> Self {
        Self { total }
    }

    pub fn final_total(&self) -> u64 {
        self.compute_tax() + self.total
    }

    fn compute_tax(&self) -> u64 {
        self.total / 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_tax() {
        let o = Order::new(100);
        assert_eq!(o.compute_tax(), 10);
    }
}
