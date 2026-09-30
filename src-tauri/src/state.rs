use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::proxy::{ProxyRuntime, ProxyShared};

/// Baked-in fallback domains. The proxy probes these and picks the healthiest.
/// Ideally overridden at runtime by the relay's public domain-list endpoint.
/// All resolve to the same relay and are covered by its SAN cert; SNI is
/// disabled on the HTTP client so GFW SNI-based RST can't block them.
pub const DEFAULT_HOSTS: &[&str] = &[
    "https://jokerdeck.cc.cd",
    "https://jokerdeck.de5.net",
    "https://jokere.duckdns.org",
    "https://api.jokere.asia",
    "https://sub2api.186-244-245-198.sslip.io",
];
pub const SITE_HOST: &str = "https://sub2api.186-244-245-198.sslip.io";

pub const DEFAULT_PROXY_PORT: u16 = 8788;

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub proxy_port: u16,
    pub auto_fallback: bool,
    #[serde(default)]
    pub preferred_host: Option<String>,
    /// Group the user picked to use by default. `None` = cheapest available.
    pub preferred_group_id: Option<i64>,
    /// Model written into Claude Code's config (`ANTHROPIC_MODEL`).
    pub claude_model: Option<String>,
    /// Model written into Codex's config.
    pub codex_model: Option<String>,
    /// Inject the anthropic-beta header for opt-in features (computer-use).
    #[serde(default)]
    pub computer_use: bool,
    pub update_manifest_url: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            proxy_port: DEFAULT_PROXY_PORT,
            auto_fallback: false,
            preferred_host: None,
            preferred_group_id: None,
            claude_model: None,
            codex_model: None,
            computer_use: false,
            update_manifest_url: None,
        }
    }
}

/// Windows persists this entire store using user-scoped DPAPI.
/// macOS passwords live only in Keychain, never in this JSON store.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Store {
    #[serde(default)]
    pub hosts: Vec<String>,
    pub last_email: Option<String>,
    #[serde(default)]
    pub last_user_id: Option<i64>,
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub saved_password: Option<String>,
    #[serde(default)]
    pub group_keys: HashMap<i64, String>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct UserInfo {
    pub id: i64,
    pub email: String,
    pub balance: f64,
    #[serde(default)]
    pub frozen_balance: f64,
    #[serde(default)]
    pub total_recharged: f64,
    #[serde(default)]
    pub allowed_groups: Vec<i64>,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub status: String,
}

pub struct Session {
    pub access_token: String,
    // Kept for the token-refresh flow (v1 relies on re-login; refresh is a TODO).
    #[allow(dead_code)]
    pub refresh_token: Option<String>,
    pub user: UserInfo,
}

pub struct AppState {
    pub http: reqwest::Client,
    pub app_dir: PathBuf,
    pub session: RwLock<Option<Session>>,
    pub store: RwLock<Store>,
    pub proxy: ProxyShared,
    pub proxy_runtime: tokio::sync::Mutex<Option<ProxyRuntime>>,
    pub configuration_lock: tokio::sync::Mutex<()>,
}

pub type SharedState = Arc<AppState>;

pub fn store_path(app_dir: &std::path::Path) -> PathBuf {
    app_dir.join(if cfg!(windows) {
        "store.dpapi"
    } else {
        "store.json"
    })
}

pub fn load_store(app_dir: &std::path::Path) -> anyhow::Result<Store> {
    let path = store_path(app_dir);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&crate::secret_store::protect(
            &bytes, true,
        )?)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let legacy = app_dir.join("store.json");
            if cfg!(windows) && legacy.exists() {
                let store = serde_json::from_slice(&std::fs::read(&legacy)?)?;
                save_store(app_dir, &store)?;
                std::fs::remove_file(legacy)?;
                Ok(store)
            } else {
                Ok(Store::default())
            }
        }
        Err(err) => Err(err.into()),
    }
}

pub fn save_store(app_dir: &std::path::Path, store: &Store) -> anyhow::Result<()> {
    std::fs::create_dir_all(app_dir)?;
    #[cfg(target_os = "macos")]
    let stored = Store {
        saved_password: None,
        ..store.clone()
    };
    #[cfg(not(target_os = "macos"))]
    let stored = store;
    let bytes = crate::secret_store::protect(&serde_json::to_vec(&stored)?, false)?;
    let temporary = app_dir.join("store.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, store_path(app_dir))?;
    Ok(())
}
