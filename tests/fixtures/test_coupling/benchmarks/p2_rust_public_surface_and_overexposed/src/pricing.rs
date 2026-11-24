use crate::discount::apply_tier_curve;

pub struct QuoteEngine {
    base_fee: u64,
}

impl QuoteEngine {
    pub fn new(base_fee: u64) -> Self {
        Self { base_fee }
    }

    pub fn quote(&self, raw_cents: u64, tier: u8) -> u64 {
        let normalized = self.normalize_cents(raw_cents);
        apply_tier_curve(normalized, tier) + self.base_fee
    }

    pub(crate) fn normalize_cents(&self, raw: u64) -> u64 {
        raw - (raw % 5)
    }
}
