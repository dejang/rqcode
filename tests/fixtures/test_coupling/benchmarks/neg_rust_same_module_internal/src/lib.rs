mod discount;

pub fn checkout_total(subtotal_cents: u64, tier: u8) -> u64 {
    discount::apply_tier_curve(subtotal_cents, tier)
}
