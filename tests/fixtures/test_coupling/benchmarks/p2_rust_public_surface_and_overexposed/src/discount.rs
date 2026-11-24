pub fn apply_tier_curve(cents: u64, tier: u8) -> u64 {
    cents.saturating_sub(u64::from(tier) * 50)
}
