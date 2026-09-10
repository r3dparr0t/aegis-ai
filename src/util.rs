// src/util.rs

/// حذف Fenceهای Markdown (مثل ```json ... ``` یا ``` ... ```) از خروجی خام مدل
pub fn strip_code_fences(output: &str) -> String {
    let trimmed = output.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
    } else {
        trimmed.to_string()
    }
}
