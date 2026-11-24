use std::fmt;

mod sealed {
    pub trait Sealed {}
}

pub trait Currency: sealed::Sealed {
    fn code(&self) -> &'static str;
}

pub struct Usd;

impl sealed::Sealed for Usd {}

impl Currency for Usd {
    fn code(&self) -> &'static str {
        "USD"
    }
}

#[derive(Debug, PartialEq)]
pub struct Money {
    pub cents: i64,
    pub currency: &'static str,
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}.{:02}",
            self.currency,
            self.cents / 100,
            self.cents % 100
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        input: Money,
        expected: &'static str,
    }

    #[test]
    fn formats_public_money_dto_via_display() {
        let c = Case {
            input: Money {
                cents: 1250,
                currency: Usd.code(),
            },
            expected: "USD 12.50",
        };
        assert_eq!(format!("{}", c.input), c.expected);
    }
}
