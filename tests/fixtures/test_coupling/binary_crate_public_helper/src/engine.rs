pub fn run() {}

pub fn dump_internal_cache() -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_cache() {
        assert_eq!(dump_internal_cache(), 0);
    }
}
