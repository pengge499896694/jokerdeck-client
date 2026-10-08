//! Browser compatibility adapted from BigPizzaV3/CodexPlusPlus 1.6.0 (AGPL-3.0-only).
//! Native execution and operation approvals remain in Codex's original runtime.
pub mod native_browser;
pub mod native_browser_connection;

fn codex_home() -> std::path::PathBuf {
    crate::config_writer::codex_config_path()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
}

fn state_root() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("jokerdeck-chatgpt")
}

fn append_diagnostic_log(event: &str, details: serde_json::Value) -> anyhow::Result<()> {
    tracing::info!(event, %details, "Codex++ native browser compatibility");
    Ok(())
}

static MONITOR: tokio::sync::Mutex<Option<native_browser::BrowserMonitor>> =
    tokio::sync::Mutex::const_new(None);

/// Serialize restarts so only one owner can patch or restore the native service.
pub async fn configure(enabled: bool) {
    let mut monitor = MONITOR.lock().await;
    if let Some(previous) = monitor.take() {
        previous.stop().await;
    }
    *monitor = native_browser::start_monitor(enabled).await;
}

pub async fn stop() {
    let mut monitor = MONITOR.lock().await;
    if let Some(previous) = monitor.take() {
        previous.stop().await;
    }
}
