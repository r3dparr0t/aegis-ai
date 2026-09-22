// src/provider/provider_config.rs
use serde::Deserialize;
use std::{collections::HashSet, fs};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[serde(rename = "openai_compatible")]   // ← این خط
    OpenAiCompatible,
    Typesafe,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderEntry {
    pub env_var: String,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub kind: ProviderKind,
}

#[derive(Debug, Deserialize)]
struct ProvidersFile {
    #[serde(default)]
    provider: Vec<ProviderEntry>,
}

pub fn load_providers_file() -> Vec<ProviderEntry> {
    let content = match fs::read_to_string("providers.toml") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("⚠️  Could not read providers.toml: {}", e);
            return Vec::new();
        }
    };
    match toml::from_str::<ProvidersFile>(&content) {
        Ok(f) => f.provider,
        Err(e) => {
            eprintln!("⚠️  providers.toml parse error: {}", e);
            Vec::new()
        }
    }
}

pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{}…{} ({} chars)", prefix, suffix, chars.len())
}

pub struct AvailableProvider {
    pub entry: ProviderEntry,
    pub api_key: String,
    pub masked: String,
}

pub fn discover_available() -> Vec<AvailableProvider> {
    let known = load_providers_file();
    let mut result: Vec<AvailableProvider> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for entry in &known {
        if let Ok(key) = std::env::var(&entry.env_var) {
            if !key.trim().is_empty() && seen.insert(entry.env_var.clone()) {
                result.push(AvailableProvider {
                    entry: entry.clone(),
                    masked: mask_key(&key),
                    api_key: key,
                });
            }
        }
    }

    if let Ok(content) = fs::read_to_string(".env") {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');

            if value.is_empty() || seen.contains(key) {
                continue;
            }
            let looks_like_key = key.ends_with("_API_KEY")
                || key.ends_with("_KEY")
                || key.ends_with("_TOKEN");
            if !looks_like_key {
                continue;
            }

            seen.insert(key.to_string());
            result.push(AvailableProvider {
                entry: ProviderEntry {
                    env_var: key.to_string(),
                    name: key.to_string(),
                    base_url: String::new(),
                    default_model: String::new(),
                    kind: ProviderKind::OpenAiCompatible,
                },
                masked: mask_key(value),
                api_key: value.to_string(),
            });
        }
    }

    result
}