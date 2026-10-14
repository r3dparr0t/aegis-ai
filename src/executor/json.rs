// src/executor/json.rs

/// از یک متن آزاد (که ممکنه reasoning، ``` fence، یا حتی </think> داشته باشه)
/// اولین JSON object معتبر رو بیرون می‌کشه.
///
/// نکته‌ی مهم: در حالت fallback (بدون fence)، از brace-matching استفاده
/// می‌کنیم نه `rfind('}')`. چون خروجی مدل می‌تونه حاوی JSON اصلی + متن
/// اضافی (lessons تزریق‌شده، توضیح مدل، مثال) باشه. اگه `rfind('}')`
/// بزنیم، آخرین `}` کل متن رو می‌گیریم که مال متن اضافیه و JSON اصلی رو
/// با garbage ترکیب می‌کنه => invalid_json.
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

    // ۳. brace-matching از اولین `{` — تا } متناظر.
    //    string-aware: `{` و `}` داخل "..." شمرده نمیشن، و `\"` escape
    //    رو درست مدیریت می‌کنیم.
    let start = body.find('{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;

    for (i, c) in body[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(body[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }

    None
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
    let re = regex::Regex::new(r#":\s*'([^']*)'"#).ok()?;
    let result = re.replace_all(text, |caps: &regex::Captures| {
        let inner = &caps[1];
        let escaped = inner
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!(": \"{}\"", escaped)
    });
    Some(result.to_string())
}