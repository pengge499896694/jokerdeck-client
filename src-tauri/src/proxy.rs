use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, HeaderName, StatusCode,
    },
    response::Response,
    Router,
};
use serde::Serialize;
use tauri::Emitter;
use tokio::{
    sync::{oneshot, RwLock},
    task::JoinHandle,
};

/// One candidate relay domain and its last observed health.
#[derive(Clone, Serialize)]
pub struct HostHealth {
    pub host: String,
    pub healthy: bool,
    pub latency_ms: Option<u64>,
}

/// An API key bound to a specific rate group. Sorted low -> high multiplier.
#[derive(Clone, Serialize)]
pub struct GroupKey {
    pub group_id: i64,
    pub name: String,
    #[serde(skip)]
    pub key: String,
    pub multiplier: Option<f64>,
}

#[derive(Default)]
pub struct ProxyState {
    pub hosts: Vec<HostHealth>,
    pub active_host_idx: usize,
    pub preferred_host: Option<String>,
    pub groups: Vec<GroupKey>,
    pub active_group_idx: usize,
    pub auto_fallback: bool,
    pub running: bool,
    pub port: u16,
    pub selection_revision: u64,
    /// Extra headers injected on every upstream request (e.g. anthropic-beta
    /// for opt-in features). Keyed name -> value.
    pub extra_headers: Vec<(String, String)>,
}

pub type ProxyShared = Arc<RwLock<ProxyState>>;

pub fn shared_with(hosts: Vec<String>, auto_fallback: bool) -> ProxyShared {
    let st = ProxyState {
        hosts: hosts
            .into_iter()
            .map(|h| HostHealth {
                host: h,
                healthy: true,
                latency_ms: None,
            })
            .collect(),
        auto_fallback,
        ..Default::default()
    };
    Arc::new(RwLock::new(st))
}

/// Snapshot exposed to the UI.
#[derive(Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub base_url: String,
    pub active_host: Option<String>,
    pub preferred_host: Option<String>,
    pub hosts: Vec<HostHealth>,
    /// All provisioned groups, so the UI can offer a switcher.
    pub groups: Vec<GroupKey>,
    pub active_group: Option<GroupKey>,
    pub auto_fallback: bool,
}

impl ProxyState {
    pub fn ordered_reachable_hosts(&self) -> Vec<String> {
        self.ordered_hosts()
            .into_iter()
            .filter(|(index, _)| self.hosts[*index].healthy)
            .map(|(_, host)| host)
            .collect()
    }
    fn ordered_hosts(&self) -> Vec<(usize, String)> {
        // A manually preferred line is tried first. Failed requests still
        // fall back to the remaining lines without changing that preference.
        let mut idxs: Vec<usize> = (0..self.hosts.len()).collect();
        idxs.sort_by_key(|&i| {
            let h = &self.hosts[i];
            let active_bonus = if i == self.active_host_idx { 0 } else { 1 };
            let health_rank = if h.healthy { 0 } else { 2 };
            let manual_rank = if self.preferred_host.as_deref() == Some(&h.host) {
                0
            } else {
                1
            };
            (
                manual_rank,
                health_rank,
                active_bonus,
                h.latency_ms.unwrap_or(u64::MAX),
            )
        });
        idxs.into_iter()
            .map(|i| (i, self.hosts[i].host.clone()))
            .collect()
    }
}

// hop-by-hop and auth headers we never forward upstream
const STRIP: &[&str] = &[
    "host",
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "content-length",
    "connection",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "keep-alive",
    "te",
    "trailer",
];

fn should_strip(name: &HeaderName) -> bool {
    let n = name.as_str().to_ascii_lowercase();
    STRIP.contains(&n.as_str())
}

#[derive(Clone)]
pub struct ProxyCtx {
    pub shared: ProxyShared,
    pub http: reqwest::Client,
    pub app: tauri::AppHandle,
}

pub struct ProxyRuntime {
    shutdown: oneshot::Sender<()>,
    server: JoinHandle<()>,
    probe: JoinHandle<()>,
}

impl ProxyRuntime {
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(());
        if tokio::time::timeout(Duration::from_secs(5), &mut self.server)
            .await
            .is_err()
        {
            self.server.abort();
            let _ = self.server.await;
        }
        self.probe.abort();
        let _ = self.probe.await;
    }
}

/// Bind the local proxy on 127.0.0.1:<port> and start background health probing.
pub async fn start(ctx: ProxyCtx, port: u16) -> anyhow::Result<ProxyRuntime> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let (shutdown, shutdown_rx) = oneshot::channel();
    let probe = spawn_health_probe(ctx.clone());
    let app = Router::new().fallback(handle).with_state(ctx.clone());
    let shared = ctx.shared.clone();
    let server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!("local proxy exited: {e}");
            shared.write().await.running = false;
        }
    });
    {
        let mut st = ctx.shared.write().await;
        st.running = true;
        st.port = port;
    }
    Ok(ProxyRuntime {
        shutdown,
        server,
        probe,
    })
}

fn spawn_health_probe(ctx: ProxyCtx) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let hosts: Vec<String> = {
                let st = ctx.shared.read().await;
                st.hosts.iter().map(|h| h.host.clone()).collect()
            };
            for host in hosts {
                let url = format!("{host}/health");
                let started = Instant::now();
                let result = ctx
                    .http
                    .get(&url)
                    .timeout(Duration::from_secs(6))
                    .send()
                    .await;
                let (healthy, latency) = match result {
                    // any HTTP answer (even 401/404) means the host is reachable
                    Ok(_) => (true, Some(started.elapsed().as_millis() as u64)),
                    Err(_) => (false, None),
                };
                let mut st = ctx.shared.write().await;
                if let Some(h) = st.hosts.iter_mut().find(|h| h.host == host) {
                    h.healthy = healthy;
                    h.latency_ms = latency;
                }
            }
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    })
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[tokio::test]
    async fn stopping_proxy_releases_listener_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown, receiver) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new())
                .with_graceful_shutdown(async {
                    let _ = receiver.await;
                })
                .await
                .unwrap();
        });
        let probe = tokio::spawn(std::future::pending::<()>());
        ProxyRuntime {
            shutdown,
            server,
            probe,
        }
        .stop()
        .await;
        assert!(tokio::net::TcpListener::bind(addr).await.is_ok());
    }
}

struct Attempt {
    host_idx: usize,
    host: String,
    key: String,
    group_idx: usize,
}

fn route_path(path: &str) -> Result<(Option<i64>, String), String> {
    if let Some(rest) = path.strip_prefix("/groups/") {
        let (id, suffix) = rest.split_once('/').ok_or("分组路由缺少 API 路径")?;
        let id = id
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or("无效的分组路由")?;
        Ok((Some(id), format!("/{suffix}")))
    } else {
        Ok((None, path.to_string()))
    }
}

fn is_retriable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

async fn handle(State(ctx): State<ProxyCtx>, req: Request) -> Response {
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let (pinned_group_id, path_and_query) = match route_path(&path_and_query) {
        Ok(route) => route,
        Err(err) => return json_error(StatusCode::BAD_REQUEST, &err),
    };
    let in_headers = req.headers().clone();
    let body_bytes = match axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "读取请求体失败"),
    };

    // Build the attempt plan under a read lock, then release before network I/O.
    let (attempts, current_group_idx, revision, extra_headers) = {
        let st = ctx.shared.read().await;
        if st.groups.is_empty() {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "尚未配置分组密钥,请先登录并应用配置",
            );
        }
        let ordered = st.ordered_hosts();
        if ordered.is_empty() {
            return json_error(StatusCode::SERVICE_UNAVAILABLE, "没有可用的中转域名");
        }
        let cur = match pinned_group_id {
            Some(id) => match st.groups.iter().position(|g| g.group_id == id) {
                Some(index) => index,
                None => {
                    return json_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "配置的分组尚未加载，请在客户端重新应用配置",
                    )
                }
            },
            None => st.active_group_idx.min(st.groups.len() - 1),
        };
        let mut attempts: Vec<Attempt> = Vec::new();
        for (hi, host) in &ordered {
            attempts.push(Attempt {
                host_idx: *hi,
                host: host.clone(),
                key: st.groups[cur].key.clone(),
                group_idx: cur,
            });
        }
        if st.auto_fallback && pinned_group_id.is_none() {
            for gi in (cur + 1)..st.groups.len() {
                for (hi, host) in &ordered {
                    attempts.push(Attempt {
                        host_idx: *hi,
                        host: host.clone(),
                        key: st.groups[gi].key.clone(),
                        group_idx: gi,
                    });
                }
            }
        }
        (
            attempts,
            cur,
            st.selection_revision,
            st.extra_headers.clone(),
        )
    };

    // Headers forwarded on every attempt (auth is injected per attempt below).
    let mut fwd = HeaderMap::new();
    for (name, value) in in_headers.iter() {
        if !should_strip(name) {
            fwd.insert(name.clone(), value.clone());
        }
    }
    for (name, val) in &extra_headers {
        if let (Ok(n), Ok(v)) = (name.parse::<HeaderName>(), val.parse()) {
            fwd.insert(n, v);
        }
    }

    let mut last_err = String::from("no upstream reachable");
    for attempt in attempts {
        let url = format!("{}{}", attempt.host, path_and_query);
        let mut headers = fwd.clone();
        if let Ok(v) = format!("Bearer {}", attempt.key).parse() {
            headers.insert(AUTHORIZATION, v);
        }
        if let Ok(v) = attempt.key.parse() {
            headers.insert(HeaderName::from_static("x-api-key"), v);
        }
        let sent = ctx
            .http
            .request(method.clone(), &url)
            .headers(headers)
            .body(body_bytes.clone())
            .send()
            .await;
        match sent {
            Ok(resp) => {
                let status = resp.status();
                if is_retriable(status) {
                    last_err = format!("{} -> {}", attempt.host, status.as_u16());
                    continue;
                }
                if status.is_success() {
                    commit_selection(
                        &ctx,
                        attempt.host_idx,
                        attempt.group_idx,
                        current_group_idx,
                        revision,
                        pinned_group_id.is_some(),
                    )
                    .await;
                }
                return build_streaming_response(resp);
            }
            Err(e) => {
                last_err = format!("{} 连接失败: {}", attempt.host, e);
                mark_unhealthy(&ctx, attempt.host_idx).await;
                continue;
            }
        }
    }
    json_error(
        StatusCode::BAD_GATEWAY,
        &format!("全部中转不可用: {last_err}"),
    )
}

fn build_streaming_response(resp: reqwest::Response) -> Response {
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = Body::from_stream(resp.bytes_stream());
    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if should_strip(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
        .body(body)
        .unwrap_or_else(|_| json_error(StatusCode::BAD_GATEWAY, "构造响应失败"))
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    let body =
        serde_json::json!({ "error": { "message": msg, "type": "proxy_error" } }).to_string();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("static error response is valid")
}

async fn mark_unhealthy(ctx: &ProxyCtx, host_idx: usize) {
    let mut st = ctx.shared.write().await;
    if let Some(h) = st.hosts.get_mut(host_idx) {
        h.healthy = false;
    }
}

async fn commit_selection(
    ctx: &ProxyCtx,
    host_idx: usize,
    group_idx: usize,
    prev_group_idx: usize,
    revision: u64,
    pinned: bool,
) {
    let mut host_changed = None;
    let mut group_changed = None;
    {
        let mut st = ctx.shared.write().await;
        // An older in-flight retry must never undo a newer manual selection.
        if st.selection_revision != revision {
            return;
        }
        if st.active_host_idx != host_idx {
            st.active_host_idx = host_idx;
            host_changed = st.hosts.get(host_idx).map(|h| h.host.clone());
        }
        if !pinned && group_idx != prev_group_idx {
            st.active_group_idx = group_idx;
            group_changed = st.groups.get(group_idx).cloned();
        }
    }
    if let Some(host) = host_changed {
        let _ = ctx.app.emit("host-switched", host);
    }
    if let Some(g) = group_changed {
        let _ = ctx.app.emit(
            "group-switched",
            serde_json::json!({ "name": g.name, "multiplier": g.multiplier }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_group_routes_preserve_paths_and_query() {
        assert_eq!(
            route_path("/groups/42/v1/models?client_version=1").unwrap(),
            (Some(42), "/v1/models?client_version=1".into())
        );
        assert_eq!(
            route_path("/groups/7/v1/messages").unwrap(),
            (Some(7), "/v1/messages".into())
        );
        assert_eq!(
            route_path("/v1/responses").unwrap(),
            (None, "/v1/responses".into())
        );
        for path in [
            "/groups/no/v1/models",
            "/groups/0/v1/models",
            "/groups/-1/v1/models",
            "/groups/1",
        ] {
            assert!(route_path(path).is_err());
        }
    }
    #[test]
    fn errors_do_not_trigger_inappropriate_retries() {
        assert!(is_retriable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retriable(StatusCode::UNAUTHORIZED));
        assert!(!is_retriable(StatusCode::BAD_REQUEST));
    }
    #[test]
    fn manual_host_takes_priority_even_when_marked_unhealthy() {
        let mut state = ProxyState::default();
        state.hosts = vec![
            HostHealth {
                host: "https://first.example".into(),
                healthy: true,
                latency_ms: Some(10),
            },
            HostHealth {
                host: "https://manual.example".into(),
                healthy: false,
                latency_ms: None,
            },
        ];
        state.preferred_host = Some("https://manual.example".into());
        assert_eq!(state.ordered_hosts()[0].1, "https://manual.example");
        state.preferred_host = None;
        assert_eq!(state.ordered_hosts()[0].1, "https://first.example");
    }
}
