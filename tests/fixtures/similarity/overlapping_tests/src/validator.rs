fn validate(input: &str) -> bool {
    !input.is_empty()
}

#[test]
fn test_valid_case_one() {
    let input = "alpha";
    let trimmed = input.trim();
    let ok = validate(trimmed);
    assert!(ok);
    assert_eq!(trimmed.len(), 5);
}

#[test]
fn test_valid_case_two() {
    let value = "omega";
    let cleaned = value.trim();
    let ok = validate(cleaned);
    assert!(ok);
    assert_eq!(cleaned.len(), 5);
}
