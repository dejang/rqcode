#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Red,
    Blue,
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn debug_shows_variant_name() {
        assert_eq!(format!("{:?}", Color::Red), "Red");
    }
}
