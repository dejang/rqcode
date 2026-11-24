pub fn apply_tier_curve(subtotal_cents: u64, tier: u8) -> u64 {
    match tier {
        0 => subtotal_cents,
        1 => subtotal_cents * 95 / 100,
        _ => subtotal_cents * 90 / 100,
    }
}

#[cfg(test)]
mod tests {
    use super::apply_tier_curve;

    #[test]
    fn tier_one_applies_five_percent_discount() {
        assert_eq!(apply_tier_curve(1000, 1), 950);
    }
}
