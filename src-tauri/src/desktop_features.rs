use crate::state::{save_store, SharedState};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

const PACKAGE: &str = "@dailin521/codex-provider-sync@0.5.0";
#[derive(Clone, Serialize)]
struct Bridge {
    url: String,
    key: String,
}
static BRIDGE: OnceLock<Bridge> = OnceLock::new();
#[derive(Clone)]
struct Server {
    state: SharedState,
    key: String,
    cache: Arc<tokio::sync::Mutex<Option<(i64, Instant, Value, Vec<Value>, bool)>>>,
}

pub(crate) fn start(state: SharedState) -> Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|err| anyhow::anyhow!("随机密钥生成失败：{err}"))?;
    let key = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let bridge = Bridge {
        url: format!(
            "http://127.0.0.1:{}/snapshot",
            listener.local_addr()?.port()
        ),
        key: key.clone(),
    };
    BRIDGE
        .set(bridge)
        .map_err(|_| anyhow::anyhow!("统计服务已启动"))?;
    let app = Router::new()
        .route("/snapshot", get(snapshot))
        .with_state(Server {
            state,
            key,
            cache: Arc::new(tokio::sync::Mutex::new(None)),
        });
    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => {
                if let Err(error) = axum::serve(listener, app).await {
                    tracing::error!("统计服务停止：{error}");
                }
            }
            Err(error) => tracing::error!("统计服务启动失败：{error}"),
        }
    });
    Ok(())
}

pub(crate) fn bridge_origin() -> Option<String> {
    BRIDGE
        .get()
        .map(|bridge| bridge.url.trim_end_matches("/snapshot").to_owned())
}

#[cfg(test)]
pub(crate) fn enable_test_bridge() {
    let _ = BRIDGE.set(Bridge {
        url: "http://127.0.0.1:39877/snapshot".into(),
        key: "fixture-only".into(),
    });
}

pub(crate) fn runtime_script() -> String {
    BRIDGE
        .get()
        .map(|bridge| {
            format!(
                "\n// jokerdeck-desktop-features-v1\n({})({});",
                include_str!("desktop_features.js"),
                serde_json::to_string(bridge).unwrap()
            )
        })
        .unwrap_or_default()
}

fn allowed(headers: &HeaderMap, key: &str) -> bool {
    // Electron file/custom-scheme pages use an opaque origin. Ordinary websites must never read local usage.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    let origin_ok = origin.is_none() || matches!(origin, Some("null" | "app://." | "app://-"));
    origin_ok
        && headers.get("authorization").and_then(|v| v.to_str().ok())
            == Some(format!("Bearer {key}").as_str())
}
fn reply(status: StatusCode, data: Value) -> Response {
    let mut response = (status, Json(data)).into_response();
    response
        .headers_mut()
        .insert("access-control-allow-origin", "*".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}
#[derive(Deserialize)]
struct SnapshotQuery {
    thread: Option<String>,
    key: Option<String>,
}
async fn snapshot(
    State(server): State<Server>,
    Query(query): Query<SnapshotQuery>,
    mut headers: HeaderMap,
) -> Response {
    // A URL capability avoids CORS preflights in Electron. Never include panel tokens or API keys.
    if let Some(key) = query.key {
        if let Ok(value) = format!("Bearer {key}").parse() {
            headers.insert("authorization", value);
        }
    }
    if !allowed(&headers, &server.key) {
        return reply(StatusCode::FORBIDDEN, json!({"error":"forbidden"}));
    }
    let settings = server.state.store.read().await.settings.clone();
    let theme = settings.codex_theme;
    if !settings.usage_overlay {
        return reply(StatusCode::OK, json!({"theme":theme,"overlay":false}));
    }
    let thread = query.thread.filter(|id| valid_uuid(id));
    let tokens = if let Some(id) = thread.clone() {
        tokio::task::spawn_blocking(move || session_tokens(&id))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
    } else {
        None
    };
    let external = tokio::task::spawn_blocking(crate::provider_manager::status).await
        .ok().and_then(Result::ok).is_some_and(|status| status.external);
    if external {
        return reply(StatusCode::OK, json!({"theme":theme,"overlay":true,"session_tokens":tokens,
            "billing_scope":"外部服务商账单不可用；本会话 Token 来自本地记录"}));
    }
    let (user_id, token) = {
        let session = server.state.session.read().await;
        let Some(session) = session.as_ref() else {
            return reply(
                StatusCode::OK,
                json!({"theme":theme,"overlay":true,"session_tokens":tokens,"error":"请在 client 登录以读取账单"}),
            );
        };
        (session.user.id, session.access_token.clone())
    };
    let host = server
        .state
        .store
        .read()
        .await
        .settings
        .preferred_host
        .clone()
        .unwrap_or_else(|| crate::state::SITE_HOST.to_owned());
    let mut cache = server.cache.lock().await;
    if cache.as_ref().is_none_or(|(user, time, _, _, _)| {
        *user != user_id || time.elapsed() > Duration::from_secs(20)
    }) {
        let (stats, records) = tokio::join!(
            panel(
                &server.state,
                &host,
                &token,
                "/api/v1/usage/dashboard/stats"
            ),
            usage_records(
                &server.state,
                &host,
                &token,
            )
        );
        match (stats, records) {
            (Ok(stats), Ok(records)) => {
                let rows = records["items"].as_array().cloned().unwrap_or_default();
                let complete = records["total"]
                    .as_u64()
                    .is_some_and(|count| count <= rows.len() as u64);
                *cache = Some((user_id, Instant::now(), stats, rows, complete));
            }
            (Err(error), _) | (_, Err(error)) => {
                return reply(
                    StatusCode::OK,
                    json!({"theme":theme,"overlay":true,"session_tokens":tokens,"error":error.to_string()}),
                )
            }
        }
    }
    let (_, _, stats, rows, complete) = cache.as_ref().unwrap();
    let matching: Vec<_> = rows
        .iter()
        .filter(|row| {
            thread
                .as_deref()
                .is_some_and(|id| row["session_id"].as_str() == Some(id))
        })
        .collect();
    let cost: f64 = matching
        .iter()
        .filter_map(|row| row["actual_cost"].as_f64())
        .sum();
    reply(
        StatusCode::OK,
        json!({"theme":theme,"overlay":true,"total_tokens":stats["total_tokens"],"total_cost":stats["total_actual_cost"],
        "today_tokens":stats["today_tokens"],"today_cost":stats["today_actual_cost"],"session_tokens":tokens,
        "session_cost":if matching.is_empty(){None}else{Some(cost)},"session_cost_complete":complete,"matched_requests":matching.len(),"billing_scope":"已分页加载的结算账单"}),
    )
}
/// Load enough usage pages to make per-session totals reliable. The previous
/// single 100-row request silently reported incomplete or zero session costs.
async fn usage_records(state: &SharedState, host: &str, token: &str) -> Result<Value> {
    let mut items = Vec::new();
    let mut total = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    for page in 1..=20 {
        let path = format!("/api/v1/usage?page={page}&page_size=100");
        // Large histories must not keep the overlay waiting beyond its fetch deadline.
        let value = match tokio::time::timeout_at(deadline, panel(state, host, token, &path)).await {
            Ok(Ok(value)) => value,
            Ok(Err(error)) if items.is_empty() => return Err(error),
            Err(_) if items.is_empty() => bail!("账单查询超时"),
            _ => break,
        };
        if total.is_none() { total = value["total"].as_u64(); }
        let rows = value["items"].as_array().cloned().unwrap_or_default();
        let count = rows.len();
        items.extend(rows);
        if count == 0 || total.is_some_and(|n| items.len() as u64 >= n) || count < 100 { break; }
    }
    Ok(json!({"items": items, "total": total}))
}
async fn panel(state: &SharedState, host: &str, token: &str, path: &str) -> Result<Value> {
    let response = state
        .http
        .get(format!("{host}{path}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(12))
        .send()
        .await?
        .error_for_status()?;
    let body: Value = response.json().await?;
    if body["code"] != 0 {
        bail!(
            "账单读取失败：{}",
            body["message"].as_str().unwrap_or("服务暂不可用")
        );
    }
    Ok(body["data"].clone())
}
fn valid_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn session_tokens(id: &str) -> Result<Option<Value>> {
    use std::collections::HashMap;
    static PATHS: OnceLock<std::sync::Mutex<HashMap<String, std::path::PathBuf>>> = OnceLock::new();
    let root = crate::config_writer::codex_config_path()?
        .parent()
        .context("配置目录无效")?
        .join("sessions");
    let key = format!("{}:{id}", root.display());
    let paths = PATHS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let cached = paths
        .lock()
        .map_err(|_| anyhow::anyhow!("会话缓存不可用"))?
        .get(&key)
        .cloned();
    if let Some(path) = cached.filter(|path| path.is_file()) {
        return usage_tail(&path);
    }
    let mut pending = vec![(root, 0)];
    let mut count = 0;
    while let Some((dir, depth)) = pending.pop() {
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            count += 1;
            if count > 100_000 {
                bail!("会话扫描超过上限");
            }
            if kind.is_dir() && depth < 5 {
                pending.push((entry.path(), depth + 1));
            } else if kind.is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(&format!("{id}.jsonl"))
            {
                let path = entry.path();
                let mut cache = paths
                    .lock()
                    .map_err(|_| anyhow::anyhow!("会话缓存不可用"))?;
                if cache.len() >= 128 {
                    cache.clear();
                }
                cache.insert(key, path.clone());
                drop(cache);
                return usage_tail(&path);
            }
        }
    }
    Ok(None)
}
fn usage_tail(path: &Path) -> Result<Option<Value>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(512 * 1024)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    // An arbitrary tail boundary can split both a JSON record and a UTF-8 character.
    for line in String::from_utf8_lossy(&bytes).lines().rev() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value["type"] == "event_msg" && value["payload"]["type"] == "token_count" {
                let usage = &value["payload"]["info"]["total_token_usage"];
                if usage.is_object() {
                    return Ok(Some(usage.clone()));
                }
            }
        }
    }
    Ok(None)
}

#[derive(Serialize)]
pub struct FeatureStatus {
    pub theme: String,
    pub overlay: bool,
    pub auto_sync: bool,
    pub sync_installed: bool,
    pub provider: String,
    pub node_version: Option<String>,
}
pub async fn status(state: &SharedState) -> Result<FeatureStatus> {
    let settings = state.store.read().await.settings.clone();
    let node_version = compatible_node().await.ok().map(|(_, version)| version);
    let config = std::fs::read_to_string(crate::config_writer::codex_config_path()?)
        .unwrap_or_default()
        .parse::<toml_edit::DocumentMut>()?;
    Ok(FeatureStatus {
        theme: settings.codex_theme,
        overlay: settings.usage_overlay,
        auto_sync: settings.provider_auto_sync,
        sync_installed: service_path(&state.app_dir).is_file(),
        provider: config
            .get("model_provider")
            .and_then(|v| v.as_str())
            .unwrap_or("openai")
            .to_owned(),
        node_version,
    })
}
pub async fn configure(
    state: &SharedState,
    theme: String,
    overlay: bool,
    auto_sync: bool,
) -> Result<FeatureStatus> {
    if !matches!(theme.as_str(), "native" | "rose" | "midnight" | "jade") {
        bail!("不支持的主题");
    }
    let mut store = state.store.write().await;
    let mut next = store.clone();
    next.settings.codex_theme = theme;
    next.settings.usage_overlay = overlay;
    next.settings.provider_auto_sync = auto_sync;
    save_store(&state.app_dir, &next)?;
    *store = next;
    drop(store);
    status(state).await
}
fn service_path(root: &Path) -> std::path::PathBuf {
    root.join("provider-sync/node_modules/@dailin521/codex-provider-sync/src/service.js")
}
pub async fn provider_action(
    state: &SharedState,
    action: &str,
    backup: Option<String>,
) -> Result<Value> {
    if !matches!(action, "install" | "status" | "sync" | "restore") {
        bail!("不支持的 Provider 操作");
    }
    let (node, _) = compatible_node().await?;
    let service = service_path(&state.app_dir);
    if action == "install" {
        let prefix = state.app_dir.join("provider-sync");
        std::fs::create_dir_all(&prefix)?;
        #[cfg(windows)]
        let mut command = process("cmd.exe");
        #[cfg(windows)]
        command.args(["/d", "/c", "npm.cmd"]);
        #[cfg(not(windows))]
        let mut command = process("npm");
        command
            .args([
                "install",
                "--ignore-scripts",
                "--omit=optional",
                "--no-audit",
                "--no-fund",
                "--prefix",
            ])
            .arg(&prefix)
            .arg(PACKAGE);
        let output = tokio::time::timeout(Duration::from_secs(180), command.output())
            .await
            .context("同步组件安装超时")??;
        if !output.status.success() {
            bail!(
                "同步组件安装失败：{}",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(800)
                    .collect::<String>()
            );
        }
    }
    if !service.is_file() {
        bail!("请先安装 Provider 同步组件");
    }
    let wrapper = state.app_dir.join("provider-sync/runner.mjs");
    std::fs::write(&wrapper, include_str!("provider_sync.mjs"))?;
    let home = crate::config_writer::codex_config_path()?
        .parent()
        .context("配置目录无效")?
        .to_owned();
    let mut command = process(node.as_os_str());
    command
        .arg(wrapper)
        .arg(service)
        .arg(if action == "install" {
            "status"
        } else {
            action
        })
        .arg(home);
    if let Some(backup) = backup {
        command.arg(backup);
    }
    let output = tokio::time::timeout(Duration::from_secs(180), command.output())
        .await
        .context("同步操作超时，备份保留；请查看同步状态再重试")??;
    if !output.status.success() {
        bail!(
            "Provider 操作失败：{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1200)
                .collect::<String>()
        );
    }
    serde_json::from_slice(&output.stdout).context("同步组件返回了无效结果")
}
async fn compatible_node() -> Result<(std::path::PathBuf, String)> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    let mut candidates = Vec::new();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("ProgramFiles") {
        candidates.push(std::path::PathBuf::from(root).join("nodejs").join(name));
    }
    #[cfg(target_os = "macos")]
    candidates.extend([
        std::path::PathBuf::from("/opt/homebrew/bin/node"),
        std::path::PathBuf::from("/usr/local/bin/node"),
    ]);
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(name)));
    }
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates
        .into_iter()
        .filter(|path| path.is_file())
        .take(32)
    {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            process(candidate.as_os_str())
                .args([
                    "--input-type=module",
                    "-e",
                    "import {backup} from 'node:sqlite'; if (typeof backup !== 'function') process.exit(1); console.log(process.version)",
                ])
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = result {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                return Ok((candidate, version));
            }
        }
    }
    bail!("未找到支持 SQLite backup 的 Node.js 24+。请在工具与修复升级 Node.js 后重试")
}

fn process(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.kill_on_drop(true);
    #[cfg(target_os = "macos")]
    cmd.env(
        "PATH",
        format!(
            "/opt/homebrew/bin:/usr/local/bin:{}",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
        ),
    );
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_web_origins_and_wrong_capabilities() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(allowed(&h, "secret"));
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(!allowed(&h, "secret"));
        h.insert("origin", "null".parse().unwrap());
        assert!(!allowed(&h, "wrong"));
    }
    #[test]
    fn usage_tail_uses_latest_snapshot_and_tolerates_split_utf8() {
        let path =
            std::env::temp_dir().join(format!("jokerdeck-usage-tail-{}.jsonl", std::process::id()));
        let mut text = "中".repeat(200_000);
        text.push('\n');
        text.push_str("{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":100}}}}\n");
        text.push_str("{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":250}}}}\n{\"partial\":");
        std::fs::write(&path, text).unwrap();
        assert_eq!(usage_tail(&path).unwrap().unwrap()["total_tokens"], 250);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn restricts_thread_identifiers() {
        assert!(valid_uuid("01a0f476-f4c7-7523-b71e-32dd083b2b5b"));
        assert!(!valid_uuid("../../secret"));
    }
}
