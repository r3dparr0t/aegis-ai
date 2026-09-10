// src/executor/http.rs
use std::time::Instant;
use async_trait::async_trait;
use serde_json::Value;
use crate::domain::{ExecutionOutcome, ExecutorError, TargetExecutor};

/// Executor که payload تولیدشده توسط مدل را واقعاً به یک HTTP API آسیب‌پذیر ارسال می‌کند.
///
/// انتظار می‌رود payload مدل به این شکل باشد:
/// ```json
/// { "endpoint": "/api/v1/fetch", "body": { "url": "http://internal-admin:8080/admin/secret-flag" } }
/// ```
/// یعنی `endpoint` باید یکی از موارد `allowed_endpoints` باشد و `body` عیناً به همان endpoint POST می‌شود.
pub struct HttpTargetExecutor {
    client: reqwest::Client,
    base_url: String,
    allowed_endpoints: Vec<String>,
}

impl HttpTargetExecutor {
    pub fn new(base_url: impl Into<String>, allowed_endpoints: Vec<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            allowed_endpoints: allowed_endpoints.into_iter().map(String::from).collect(),
        }
    }
}

#[async_trait]
impl TargetExecutor for HttpTargetExecutor {
    async fn execute(&self, payload: &Value) -> Result<ExecutionOutcome, ExecutorError> {
        let endpoint = payload
            .get("endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ExecutorError::InvalidPayload("missing 'endpoint' field".to_string()))?;

        if !self.allowed_endpoints.iter().any(|e| e == endpoint) {
            return Err(ExecutorError::UnknownEndpoint(endpoint.to_string()));
        }

        let body = payload
            .get("body")
            .ok_or_else(|| ExecutorError::InvalidPayload("missing 'body' field".to_string()))?;

        let url = format!("{}{}", self.base_url.trim_end_matches('/'), endpoint);
        let start_time = Instant::now();

        let http_response = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|e| ExecutorError::Network(e.to_string()))?;

        let status_code = http_response.status().as_u16();
        let response_body = http_response
            .text()
            .await
            .map_err(|e| ExecutorError::Network(e.to_string()))?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        Ok(ExecutionOutcome {
            status_code,
            body: response_body,
            latency_ms,
        })
    }
}
