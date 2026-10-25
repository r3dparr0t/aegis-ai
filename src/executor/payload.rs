// src/executor/payload.rs
//! تفسیر payload خام LLM: فیلدهای اختیاری انکودینگ رو قبل از ارسال اعمال می‌کنه.
//! هر انکودینگ یه تابع مستقل. اضافه‌کردن مورد جدید = یه تابع + یه خط تو apply_transforms.

use serde_json::Value;
use crate::executor::ip::{rewrite_url_host, IpFormat};
use base64::Engine;

pub struct TransformApplied {
    pub field: &'static str,
    pub value: String,
}

/// همه‌ی فیلدهای انکودینگ رو از payload می‌خونه و به ترتیب ثابت اعمال می‌کنه.
/// فیلدها بعد از اعمال حذف می‌شن (executor نباید ببینشون).
pub fn apply_transforms(payload: &mut Value) -> Result<Vec<TransformApplied>, String> {
    let mut applied = Vec::new();
    if let Some(a) = apply_ip_encoding(payload)? { applied.push(a); }
    if let Some(a) = apply_base64(payload)? { applied.push(a); }
    Ok(applied)
}
// "ip_encoding": "<decimal|hex|octal>" is placeholder
fn apply_ip_encoding(payload: &mut Value) -> Result<Option<TransformApplied>, String> {
    let Some(fmt_val) = payload.get("ip_encoding") else { return Ok(None) };
    let fmt_str = fmt_val.as_str()
        .ok_or("string must be ip_encoding")?
        .to_string();
    let fmt = IpFormat::parse(&fmt_str)
        .ok_or_else(|| format!("incorrect ip formatting for ip_encoding: '{}'", fmt_str))?;

    remove_field(payload, "ip_encoding");

    if let Some(body) = payload.get("body").cloned() {
        set_body(payload, rewrite_urls_in_value(&body, fmt));
    }

    Ok(Some(TransformApplied { field: "ip_encoding", value: fmt_str }))
}
// base64-decode is placeholder
fn apply_base64(payload: &mut Value) -> Result<Option<TransformApplied>, String> {
    let Some(field_val) = payload.get("base64_field") else { return Ok(None) };
    let field_name: String = field_val.as_str()
        .ok_or("string must be base64_field")?
        .to_string();

    remove_field(payload, "base64_field");

    let mut body = payload.get("body").cloned()
        .ok_or("base64_field needs body")?;
    let obj = body.as_object_mut()
        .ok_or("body must be an object")?;
    let v = obj.get_mut(&field_name)
        .ok_or_else(|| format!("field '{}' in body not found", field_name))?;
    let s = v.as_str()
        .ok_or_else(|| format!("body.{} is not a string", field_name))?;
    *v = Value::String(base64::engine::general_purpose::STANDARD.encode(s));

    set_body(payload, body);

    Ok(Some(TransformApplied { field: "base64_field", value: field_name }))
}

fn remove_field(payload: &mut Value, key: &str) {
    if let Some(obj) = payload.as_object_mut() { obj.remove(key); }
}

fn set_body(payload: &mut Value, body: Value) {
    if let Some(obj) = payload.as_object_mut() { obj.insert("body".to_string(), body); }
}

fn rewrite_urls_in_value(value: &Value, fmt: IpFormat) -> Value {
    match value {
        Value::String(s) if s.starts_with("http://") || s.starts_with("https://") => {
            rewrite_url_host(s, fmt).map(Value::String).unwrap_or_else(|| value.clone())
        }
        Value::Object(map) => Value::Object(
            map.iter().map(|(k, v)| (k.clone(), rewrite_urls_in_value(v, fmt))).collect(),
        ),
        Value::Array(arr) => Value::Array(arr.iter().map(|v| rewrite_urls_in_value(v, fmt)).collect()),
        other => other.clone(),
    }
}