// src/config.rs
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub paths: PathsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathsConfig {
    #[serde(default = "default_labs_dir")]
    pub labs_dir: PathBuf,
    #[serde(default = "default_reports_dir")]
    pub reports_dir: PathBuf,
    #[serde(default = "default_static_dir")]
    pub static_dir: PathBuf,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_migrations_dir")]
    pub migrations_dir: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { host: default_host(), port: default_port() }
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            labs_dir: default_labs_dir(),
            reports_dir: default_reports_dir(),
            static_dir: default_static_dir(),
            db_path: default_db_path(),
            migrations_dir: default_migrations_dir(),
        }
    }
}

fn default_host() -> String { "127.0.0.1".to_string() }
fn default_port() -> u16 { 7777 }
fn default_labs_dir() -> PathBuf { PathBuf::from("labs") }
fn default_reports_dir() -> PathBuf { PathBuf::from("reports") }
fn default_static_dir() -> PathBuf { PathBuf::from("static") }
fn default_db_path() -> String { "aegis.db".to_string() }
fn default_migrations_dir() -> PathBuf { PathBuf::from("migrations") }

impl Config {
    /// از فایل aegis.toml می‌خونه. اگه نبود، default استفاده می‌کنه.
    /// مسیر فایل از متغیر محیطی AEGIS_CONFIG قابل overrideـه.
    pub fn load() -> Self {
        let path = std::env::var("AEGIS_CONFIG").unwrap_or_else(|_| "aegis.toml".to_string());
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<Config>(&content) {
                Ok(cfg) => {
                    println!("✔ Loaded config from {}", path);
                    cfg
                }
                Err(e) => {
                    eprintln!("⚠️  Invalid config '{}': {}", path, e);
                    eprintln!("   Falling back to defaults.");
                    Self::default()
                }
            },
            Err(_) => {
                println!("ℹ️  No config file at '{}', using defaults.", path);
                Self::default()
            }
        }
    }

    pub fn server_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }

    pub fn server_url(&self) -> String {
        format!("http://{}", self.server_addr())
    }

    pub fn db_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.paths.db_path)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            paths: PathsConfig::default(),
        }
    }
}
