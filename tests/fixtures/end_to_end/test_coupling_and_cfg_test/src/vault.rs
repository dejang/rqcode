#[cfg_attr(test, derive(PartialEq, Debug))]
pub struct Vault {
    balance: u64,
}

impl Vault {
    pub fn new(balance: u64) -> Self {
        Self { balance }
    }

    pub fn balance(&self) -> u64 {
        self.balance + self.fee()
    }

    pub(crate) fn raw_balance(&self) -> u64 {
        self.balance
    }

    fn fee(&self) -> u64 {
        self.balance / 100
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub fn test_hook(v: u64) -> u64 {
    v + 1
}

#[cfg(not(test))]
pub fn prod_only_guard(v: u64) -> bool {
    v > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Vault {
        fn peek_internal(&self) -> u64 {
            self.balance
        }
    }

    #[test]
    fn coupled_to_internals() {
        let v = Vault::new(50);
        assert_eq!(v.raw_balance(), 50);
        assert_eq!(v.fee(), 0);
        assert_eq!(v.balance, 50);
        assert_eq!(v.peek_internal(), 50);
    }
}
