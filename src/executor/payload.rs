// src/executor/payload.rs
//! تفسیر payload خام LLM به شکل قابل‌ارسال به هدف.
//!
//! LLM می‌تونه فیلد اختیاری `ip_encoding` بده تا از برنامه بخواد
//! هر IPv4 dotted-quad تو body رو به فرمت خواسته‌شده تبدیل کنه.

use serde_json::Value;
use crate::executor::ip::{rewrite_url_host, IpFormat};

/// اگه payload فیلد `ip_encoding` داشت:
/// - هر URL dotted-quad تو `body` رو با فرمت خواسته‌شده بازنویسی می‌کنه
/// - فیلد `ip_encoding` رو حذف می‌کنه (executor نباید ببینش)
/// فرمت اعمال‌شده رو برمی‌گردونه (برای log در engine).
pub fn apply_ip_encoding(payload: &mut Value) -> Option<IpFormat> {
    let fmt_str = payload.get("ip_encoding")?.as_str()?.to_string();
    let fmt = IpFormat::parse(&fmt_str)?;

    if let Some(obj) = payload.as_object_mut() {
        obj.remove("ip_encoding");
    }

    if let Some(body) = payload.get("body").cloned() {
        let new_body = rewrite_urls_in_value(&body, fmt);
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("body".to_string(), new_body);
        }
    }

    Some(fmt)
}

fn rewrite_urls_in_value(value: &Value, fmt: IpFormat) -> Value {
    match value {
        Value::String(s) => {
            if s.starts_with("http://") || s.starts_with("https://") {
                match rewrite_url_host(s, fmt) {
                    Some(new) => Value::String(new),
                    None => Value::String(s.clone()),
                }
            } else {
                Value::String(s.clone())
            }
        }
        Value::Object(map) => Value::Object(
            map.iter().map(|(k, v)| (k.clone(), rewrite_urls_in_value(v, fmt))).collect(),
        ),
        Value::Array(arr) => Value::Array(
            arr.iter().map(|v| rewrite_urls_in_value(v, fmt)).collect(),
        ),
        other => other.clone(),
    }
}