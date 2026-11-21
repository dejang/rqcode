pub struct Order {
    total: u64,
}

impl Order {
    pub fn new(total: u64) -> Self {
        Self { total }
    }

    pub fn parse_all(raw: &[u64]) -> Vec<Self> {
        raw.iter().copied().map(Self::from_raw).collect()
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    fn from_raw(v: u64) -> Self {
        Self { total: v }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFixture {
        expected: u64,
    }

    fn make_order(amount: u64) -> Order {
        Order::new(amount)
    }

    #[test]
    fn behavioral_public_api_test_passes_cleanly() {
        let fix = TestFixture { expected: 42 };
        let order = make_order(fix.expected);
        assert_eq!(order.total(), fix.expected);
        assert_eq!(Order::parse_all(&[1, 2]).len(), 2);
    }
}
