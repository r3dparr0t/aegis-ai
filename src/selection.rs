// src/selection.rs
use std::sync::Arc;

use crate::domain::LlmProvider;
use crate::{input::prompt, provider::{
    OllamaProvider, OpenAiCompatibleProvider, TypesafeProvider,
    provider_config::{discover_available, AvailableProvider, ProviderKind},}
};
pub struct SelectedProvider {
    pub provider: Arc<dyn LlmProvider>,
    pub is_local: bool,   // true فقط برای Ollama
}


/// انتخاب provider: نمایش منو، گرفتن انتخاب کاربر، و ساخت Arc<dyn LlmProvider>.
/// همه‌ی جزئیات (لیست از providers.toml، مدل‌ها از API/Ollama، ورودی دستی) این‌جاست.
pub async fn select_provider() -> Option<SelectedProvider> {
    let available = discover_available();

    print_menu(&available);
    let choice_idx = prompt("Choice", "0").parse::<usize>().unwrap_or(0);

    if choice_idx == 0 {
        return Some(SelectedProvider {
            provider: build_ollama().await,
            is_local: true,
        });
    }
    if choice_idx >= 1 && choice_idx <= available.len() {
        return build_from_discovered(&available[choice_idx - 1])
            .await
            .map(|provider| SelectedProvider { provider, is_local: false });
    }
    build_manual().map(|provider| SelectedProvider { provider, is_local: false })
}
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn print_menu(available: &[AvailableProvider]) {
    println!("Which provider?");
    println!("  0) Ollama (local)");
    if available.is_empty() {
        println!("  ℹ️  No API keys found in .env, and/or providers.toml is missing.");
    } else {
        println!("  ── APIs found (.env × providers.toml) ──");
        for (i, p) in available.iter().enumerate() {
            println!("  {}) {}  [{}]  key={}", i + 1, p.entry.name, p.entry.env_var, p.masked);
        }
    }
    println!("  {}) Enter another API manually", available.len() + 1);
}

async fn build_ollama() -> Arc<dyn LlmProvider> {
    let url = prompt("Ollama base URL", "http://localhost:11434");
    let model = choose_ollama_model(&url, "qwen2.5:3b").await;
    Arc::new(OllamaProvider::new(url, model))
}

async fn build_from_discovered(p: &AvailableProvider) -> Option<Arc<dyn LlmProvider>> {
    let base_url = if p.entry.base_url.is_empty() {
        prompt("API base URL", "https://api.openai.com/v1")
    } else {
        prompt("API base URL", &p.entry.base_url)
    };

    let model_name = match p.entry.kind {
        ProviderKind::Typesafe => {
            println!("ℹ️  TypeSafe only accepts fixed model names (e.g. jev-latest).");
            prompt("Model name", &p.entry.default_model)
        }
        ProviderKind::OpenAiCompatible => {
            choose_openai_model(&base_url, &p.api_key, &p.entry.name, &p.entry.default_model).await
        }
    };

    let provider: Arc<dyn LlmProvider> = match p.entry.kind {
        ProviderKind::Typesafe => {
            Arc::new(TypesafeProvider::new(base_url, p.api_key.clone(), model_name))
        }
        ProviderKind::OpenAiCompatible => {
            Arc::new(OpenAiCompatibleProvider::new(base_url, p.api_key.clone(), model_name))
        }
    };
    Some(provider)
}

fn build_manual() -> Option<Arc<dyn LlmProvider>> {
    let base_url = prompt("API base URL", "https://api.openai.com/v1");
    let env_var = prompt("Environment variable holding the API key", "OPENAI_API_KEY");
    let model_name = prompt("Model name", "gpt-4o-mini");

    match OpenAiCompatibleProvider::from_env(base_url, &env_var, model_name) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            eprintln!("❌ {}", e);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Ollama model discovery
// ---------------------------------------------------------------------------

async fn fetch_ollama_models(base_url: &str) -> Vec<String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = match reqwest::get(&url).await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };
    json.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub async fn choose_ollama_model(base_url: &str, fallback: &str) -> String {
    let models = fetch_ollama_models(base_url).await;
    if models.is_empty() {
        println!("⚠️  Could not fetch model list from {} (is Ollama running?).", base_url);
        return prompt("Ollama model (type manually)", fallback);
    }
    println!("Available Ollama models:");
    for (i, m) in models.iter().enumerate() {
        println!("  {}) {}", i + 1, m);
    }
    let choice = prompt("Choose a model by number", "1");
    if let Some(m) = choice
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| models.get(i))
    {
        return m.clone();
    }
    if models.contains(&choice) {
        return choice;
    }
    println!("⚠️  Invalid choice, using default: {}", fallback);
    fallback.to_string()
}

// ---------------------------------------------------------------------------
// OpenAI-compatible model discovery
// ---------------------------------------------------------------------------

async fn fetch_openai_models(base_url: &str, api_key: &str) -> Vec<String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let response = match client.get(&url).bearer_auth(api_key).send().await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };
    json.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

async fn choose_openai_model(
    base_url: &str,
    api_key: &str,
    display_name: &str,
    fallback: &str,
) -> String {
    let models = fetch_openai_models(base_url, api_key).await;
    if models.is_empty() {
        println!(
            "⚠️  Could not fetch model list from {} (endpoint may not support GET /models).",
            base_url
        );
        return prompt("Model name", fallback);
    }
    let shown = models.len().min(50);
    println!("Available {} models:", display_name);
    for (i, m) in models.iter().take(shown).enumerate() {
        println!("  {}) {}", i + 1, m);
    }
    if models.len() > shown {
        println!("  ... ({} more not shown)", models.len() - shown);
    }
    let choice = prompt("Choose a model by number (or type its name)", "1");
    if let Some(m) = choice
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| models.get(i))
    {
        return m.clone();
    }
    if models.contains(&choice) {
        return choice;
    }
    println!("⚠️  Invalid choice, using default: {}", fallback);
    fallback.to_string()
}