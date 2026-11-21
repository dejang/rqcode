pub fn sum_positive(nums: &[i64]) -> i64 {
    let mut total = 0;
    for n in nums {
        if *n > 0 {
            total += *n;
        }
    }
    total
}

pub fn describe_code(code: u16) -> &'static str {
    match code {
        200 => "ok",
        404 => "not found",
        500 => "error",
        _ => "other",
    }
}

pub fn tiny_a(x: i32) -> i32 {
    x + 1
}

pub fn tiny_b(y: i32) -> i32 {
    y + 2
}
