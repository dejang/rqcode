use crate::pricing::QuoteEngine;

pub fn total_for_cart(raw_cents: u64, tier: u8) -> u64 {
    QuoteEngine::new(10).quote(raw_cents, tier)
}

#[cfg(test)]
mod tests {
    use crate::discount::apply_tier_curve;
    use crate::pricing::QuoteEngine;

    #[test]
    fn tests_internal_curve_directly() {
        assert_eq!(apply_tier_curve(500, 2), 400);
    }

    #[test]
    fn tests_widened_helper_directly() {
        let engine = QuoteEngine::new(10);
        assert_eq!(engine.normalize_cents(103), 100);
    }
}
