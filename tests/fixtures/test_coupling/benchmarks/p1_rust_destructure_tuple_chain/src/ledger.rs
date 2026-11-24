pub struct SecretToken(String);

pub struct Ledger {
    entries: Vec<i64>,
    cached_checksum: u64,
}

impl Ledger {
    pub fn open(raw: &[i64]) -> Result<Self, &'static str> {
        if raw.is_empty() {
            return Err("empty");
        }
        let mut ledger = Self {
            entries: raw.to_vec(),
            cached_checksum: 0,
        };
        ledger.cached_checksum = ledger.recompute_checksum();
        Ok(ledger)
    }

    pub fn balance(&self) -> i64 {
        self.entries.iter().sum()
    }

    fn recompute_checksum(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |acc, &x| acc.wrapping_add(x as u64))
    }
}

impl SecretToken {
    pub fn issue(raw: &str) -> Self {
        Self(raw.to_string())
    }

    pub fn matches(&self, other: &str) -> bool {
        self.0 == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructures_private_field_through_result_unwrap() {
        let Ledger {
            cached_checksum, ..
        } = Ledger::open(&[10, 20]).unwrap();
        assert_eq!(cached_checksum, 30);
    }

    #[test]
    fn calls_private_method_via_chained_unwrap() {
        assert_eq!(Ledger::open(&[1, 2]).unwrap().recompute_checksum(), 3);
    }

    #[test]
    fn reads_private_tuple_struct_field() {
        let tok = SecretToken::issue("xyz");
        assert_eq!(tok.0, "xyz");
    }

    #[test]
    fn destructures_private_tuple_struct_pattern() {
        let SecretToken(raw) = SecretToken::issue("abc");
        assert_eq!(raw, "abc");
    }
}
