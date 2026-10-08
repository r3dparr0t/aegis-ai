// src/executor/raw.rs
use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::domain::{ExecutionOutcome, ExecutorError, TargetExecutor};

/// Executor برای CVEهایی که خودِ اکسپلویتشون به encoding خامِ دست‌نخورده‌ی
/// path بستگی داره (مثل CVE-2021-41773: ".%2e").
///
/// چرا نه reqwest: کتابخونه‌ی `url` طبق استاندارد WHATWG URL، سگمنت‌های
/// ".%2e" / "%2e." / "%2e%2e" رو دقیقاً مثل "." و ".." می‌بینه و موقع
/// **parse کردن** URL — یعنی قبل از اینکه درخواست اصلاً فرستاده بشه —
/// collapse‌شون می‌کنه. یعنی خودِ exploit سمت کلاینت خنثی می‌شه، نه سرور.
/// (curl هم دقیقاً برای همین `--path-as-is` داره.)
///
/// راه‌حل: یه کانکشن TCP خام بزن و request-line رو بایت‌به‌بایت، دست‌نخورده
/// بفرست — هیچ URL parserی وسط نباشه.
pub struct RawHttpExecutor {
    host: String,
    port: u16,
    scheme: String,
    default_method: String,
    default_headers: Vec<(String, String)>,
}

impl RawHttpExecutor {
    /// اجرای یه درخواست تکی. تمام منطق قبلیِ execute این‌جاست.
    async fn execute_one(&self, payload: &Value) -> Result<ExecutionOutcome, ExecutorError> {
        if self.scheme != "http" {
            return Err(ExecutorError::Network(format!(
                "RawHttpExecutor only supports http:// (No '{}://') — TLS Not wired on soket.",
                self.scheme
            )));
        }

        let path = payload
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ExecutorError::InvalidPayload("missing 'path' field".into()))?;
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        let method = payload
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_method)
            .to_uppercase();

        let body: String = payload
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut headers: Vec<(String, String)> = self.default_headers.clone();
        if let Some(obj) = payload.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    headers.push((k.clone(), s.to_string()));
                }
            }
        }

        let mut request = format!("{} {} HTTP/1.1\r\n", method, path);
        let host_header = if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        request.push_str(&format!("Host: {}\r\n", host_header));
        request.push_str("Connection: close\r\n");
        let has_content_length = headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
        for (k, v) in &headers {
            request.push_str(&format!("{}: {}\r\n", k, v));
        }
        if !body.is_empty() && !has_content_length {
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        request.push_str("\r\n");
        request.push_str(&body);

        let start = Instant::now();
        let addr = format!("{}:{}", self.host, self.port);
        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| ExecutorError::Network(format!("connect to {}: {}", addr, e)))?;

        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| ExecutorError::Network(format!("write: {}", e)))?;

        let mut raw_response = Vec::new();
        stream
            .read_to_end(&mut raw_response)
            .await
            .map_err(|e| ExecutorError::Network(format!("read: {}", e)))?;

        let latency_ms = start.elapsed().as_millis() as u64;
        let (status_code, headers, resp_body) = parse_raw_http_response(&raw_response);

        Ok(ExecutionOutcome {
            status_code,
            headers,
            body: resp_body,
            latency_ms,
        })
    }

    pub fn new(
        base_url: impl Into<String>,
        default_method: impl Into<String>,
        default_headers: HashMap<String, String>,
    ) -> Self {
        let base_url = base_url.into();
        // base_url خودش هیچ dot-segmentی نداره (فقط scheme://host:port)،
        // پس اینجا parse کردنش با url::Url کاملاً امنه — فقط برای درآوردن
        // host/port/scheme، نه برای ساختن request واقعی.
        let parsed = url::Url::parse(&base_url)
            .unwrap_or_else(|e| panic!("invalid base_url '{}' for RawHttpExecutor: {}", base_url, e));
        let scheme = parsed.scheme().to_string();
        let host = parsed.host_str().unwrap_or("localhost").to_string();
        let port = parsed.port_or_known_default().unwrap_or(80);

        Self {
            host,
            port,
            scheme,
            default_method: default_method.into(),
            default_headers: default_headers.into_iter().collect(),
        }
    }
}

#[async_trait]
impl TargetExecutor for RawHttpExecutor {
    async fn execute(&self, payload: &Value) -> Result<ExecutionOutcome, ExecutorError> {
        // اگه payload یه آرایه‌ی `requests` داشت، همه رو به ترتیب اجرا کن و
        // فقط *آخرین* response رو برگردون. این برای CVEهای چندمرحله‌ای
        // (Spring4Shell: یه POST برای نوشتن shell، بعد یه GET برای اجراش).
        if let Some(requests) = payload.get("requests").and_then(|v| v.as_array()) {
            if requests.is_empty() {
                return Err(ExecutorError::InvalidPayload(
                    "'requests' array is empty".into(),
                ));
            }
            let total = requests.len();
            let mut last: Option<ExecutionOutcome> = None;
            for (i, req) in requests.iter().enumerate() {
                match self.execute_one(req).await {
                    Ok(o) => last = Some(o),
                    Err(e) => {
                        return Err(ExecutorError::StepFailed {
                            step: i + 1,
                            total,
                            inner: Box::new(e),
                        });
                    }
                }
            }
            return Ok(last.unwrap());
        }
        self.execute_one(payload).await
    }
}

/// پارسِ ساده‌ی یه پاسخ HTTP/1.x خام: status code از خط اول، بدنه از بعد
/// اولین "\r\n\r\n". چون `Connection: close` می‌فرستیم، سرور کانکشن رو
/// می‌بنده و `read_to_end` کل پاسخ رو می‌گیره — صرف‌نظر از اینکه
/// Content-Length داشته یا نه. (توجه: اگه سرور chunked جواب بده، بدنه‌ی
/// خام شامل خطوط اندازه‌ی chunk هم می‌شه — برای این لب که پاسخ CGI کوچیکه
/// معمولاً مشکلی نیست، ولی اگه یه لب دیگه با پاسخ chunked بزرگ داشتی و
/// evaluator regex دیگه match نکرد، احتمالاً باید یه decoder chunked هم
/// اضافه کنیم.)
fn parse_raw_http_response(raw: &[u8]) -> (u16, Vec<(String, String)>, String) {
    let text = String::from_utf8_lossy(raw);
    let status_code = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    let split = match text.find("\r\n\r\n") {
        Some(i) => i,
        None => return (status_code, Vec::new(), String::new()),
    };
    let head = &text[..split];
    let raw_body = &text[split + 4..];

    let mut headers = Vec::new();
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let is_chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    let body = if is_chunked {
        decode_chunked(raw_body).unwrap_or_else(|| raw_body.to_string())
    } else {
        raw_body.to_string()
    };

    (status_code, headers, body)
}

fn decode_chunked(input: &str) -> Option<String> {
    let mut out = String::new();
    let mut pos = 0;
    let bytes = input.as_bytes();
    while pos < bytes.len() {
        // پیدا کردن \r\n بعدی
        let line_end = input[pos..].find("\r\n")?;
        let size_line = &input[pos..pos + line_end];
        let size_str = size_line.split(';').next()?.trim();
        let size = usize::from_str_radix(size_str, 16).ok()?;
        pos += line_end + 2;
        if size == 0 {
            break;
        }
        if pos + size > bytes.len() {
            return None;
        }
        out.push_str(&input[pos..pos + size]);
        pos += size;
        if pos + 2 <= bytes.len() && &input[pos..pos + 2] == "\r\n" {
            pos += 2;
        }
    }
    Some(out)
}
