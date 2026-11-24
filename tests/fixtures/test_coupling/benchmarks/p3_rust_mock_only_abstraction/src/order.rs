pub trait TaxCalc {
    fn compute(&self, subtotal: u64) -> u64;
}

pub struct DefaultTaxCalc;

impl TaxCalc for DefaultTaxCalc {
    fn compute(&self, subtotal: u64) -> u64 {
        subtotal / 10
    }
}

pub struct OrderEngine<T: TaxCalc> {
    tax: T,
}

impl<T: TaxCalc> OrderEngine<T> {
    pub fn new(tax: T) -> Self {
        Self { tax }
    }

    pub fn total(&self, subtotal: u64) -> u64 {
        subtotal + self.tax.compute(subtotal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FlatRateCalc;

    impl TaxCalc for FlatRateCalc {
        fn compute(&self, _subtotal: u64) -> u64 {
            15
        }
    }

    #[test]
    fn computes_total_with_injected_calc() {
        let engine = OrderEngine::new(FlatRateCalc);
        assert_eq!(engine.total(100), 115);
    }
}
