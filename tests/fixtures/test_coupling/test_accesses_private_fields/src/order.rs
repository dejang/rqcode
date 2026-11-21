pub struct Order {
    secret: u64,
}

impl Order {
    pub fn new(secret: u64) -> Self {
        Self { secret }
    }

    pub fn total(&self) -> u64 {
        self.secret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peeks_field() {
        let o = Order::new(50);
        assert_eq!(o.secret, 50);
    }

    #[test]
    fn bypasses_ctor() {
        let o = Order { secret: 50 };
        assert_eq!(o.total(), 50);
    }
}
