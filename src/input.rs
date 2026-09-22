// src/input.rs
use std::io::{self, Write};

/// خواندن یک خط از ورودی کاربر، با مقدار پیش‌فرض در صورت خالی بودن.
pub fn prompt(message: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{} ", message);
    } else {
        print!("{} [{}]: ", message, default);
    }
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    let trimmed = input.trim().to_string();

    if trimmed.is_empty() { default.to_string() } else { trimmed }
}

/// مثل prompt، ولی حداقل طول رو اجباری می‌کنه.
pub fn prompt_min_len(message: &str, default: &str, min_len: usize) -> String {
    loop {
        let value = prompt(message, default);
        if value.chars().count() >= min_len {
            return value;
        }
        println!(
            "⚠️  Too short ({} char(s)). Please enter at least {} characters.",
            value.chars().count(),
            min_len
        );
    }
}