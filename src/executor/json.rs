// src/executor/json.rs

/// از یک متن آزاد (که ممکنه reasoning، ``` fence، یا حتی </think> داشته باشه)
/// اولین JSON object معتبر رو بیرون می‌کشه.
pub fn extract_json_object(text: &str) -> Option<String> {
    // ۰. اگه </think> داره، فقط بعدش رو نگاه کن
    let body = if let Some(idx) = text.rfind("</think>") {
        &text[idx + "</think>".len()..]
    } else {
        text
    };

    // ۱. اگه ```json ... ``` داره، محتواش رو بگیر
    if let Some(start) = body.find("```json") {
        let after = &body[start + 7..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('{') {
                return Some(inner.to_string());
            }
        }
    }

    // ۲. ``` ... ``` بدون زبان
    if let Some(start) = body.find("```") {
        let after = &body[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('{') {
                return Some(inner.to_string());
            }
        }
    }

    // ۳. اولین `{` تا آخرین `}`
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    if end > start {
        Some(body[start..=end].to_string())
    } else {
        None
    }
}
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

pub fn repair_json(text: &str) -> Option<String> {
    // اگه strict کار کرد، دست نزن
    if serde_json::from_str::<serde_json::Value>(text).is_ok() {
        return Some(text.to_string());
    }

    // فقط ساده‌ترین حالت: top-level field با value تک‌کوتیشنی
    // "body": '...'   →   "body": "..."
    // الگوی امن: فقط top-level keys که value رو با ' شروع کرده و با ' تموم می‌کنه
    let re = regex::Regex::new(r#":\s*'([^']*)'"#).ok()?;
    let result = re.replace_all(text, |caps: &regex::Captures| {
        let inner = &caps[1];
        // escape double quotes and backslashes داخل value
        let escaped = inner
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!(": \"{}\"", escaped)
    });
    Some(result.to_string())
}
