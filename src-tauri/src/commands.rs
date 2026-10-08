use futures_util::future::join_all;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use std::collections::HashMap;

use crate::api::{self, LoginOutcome};
use crate::proxy::{GroupKey, HostHealth, ProxyStatus};
use crate::state::{save_store, AppState, Session, SharedState, UserInfo, SITE_HOST};
use crate::{cli_manager, config_writer, diagnostics, updater};

type CmdResult<T> = Result<T, String>;

/// Everything the UI stored, read once so we don't take the lock repeatedly.
struct SettingsSnapshot {
    port: u16,
    hosts: Vec<String>,
    preferred_host: Option<String>,
    preferred_group_id: Option<i64>,
    claude_model: Option<String>,
    codex_model: Option<String>,
    computer_use: bool,
}

async fn snapshot(state: &AppState) -> SettingsSnapshot {
    let store = state.store.read().await;
    SettingsSnapshot {
        port: store.settings.proxy_port,
        hosts: store.hosts.clone(),
        preferred_host: store.settings.preferred_host.clone(),
        preferred_group_id: store.settings.preferred_group_id,
        claude_model: store.settings.claude_model.clone(),
        codex_model: store.settings.codex_model.clone(),
        computer_use: store.settings.computer_use,
    }
}

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

async fn bounded_blocking<T: Send + 'static>(
    deadline: std::time::Duration,
    operation: impl FnOnce() -> T + Send + 'static,
) -> CmdResult<T> {
    // timeout 无法终止系统调用；限制未完成的探测数量，防止反复刷新耗尽线程。
    static PROBES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let permit = PROBES.try_acquire().map_err(|_| "本地系统检测仍在进行".to_string())?;
    tokio::time::timeout(deadline, tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    }))
        .await
        .map_err(|_| "本地系统操作超时".to_string())?
        .map_err(e)
}

async fn require_token(state: &AppState) -> CmdResult<String> {
    state
        .session
        .read()
        .await
        .as_ref()
        .map(|s| s.access_token.clone())
        .ok_or_else(|| "未登录".to_string())
}

/// Probe all relay hosts concurrently so a slow or blocked first domain cannot
/// delay login and startup unnecessarily.
async fn pick_host(state: &AppState) -> String {
    let (hosts, preferred) = {
        let store = state.store.read().await;
        (store.hosts.clone(), store.settings.preferred_host.clone())
    };
    let cached = {
        let proxy = state.proxy.read().await;
        proxy
            .hosts
            .iter()
            .any(|host| host.latency_ms.is_some())
            .then(|| proxy.ordered_reachable_hosts())
    };
    // The sub relay is the canonical control-plane endpoint and is normally
    // the fastest route for plaza/model metadata. Do not block the first
    // request on probing every fallback domain.
    if let Some(preferred) = preferred.as_ref().filter(|host| hosts.contains(host)) {
        return preferred.clone();
    }
    if let Some(host) = preferred.as_ref().and_then(|preferred| {
        cached
            .as_ref()
            .and_then(|ordered| ordered.iter().find(|host| *host == preferred))
            .cloned()
    }) {
        return host;
    }
    if let Some(host) =
        cached.and_then(|ordered| ordered.into_iter().find(|host| hosts.contains(host)))
    {
        return host;
    }
    let probes = hosts.iter().cloned().map(|host| {
        let http = state.http.clone();
        async move {
            let started = std::time::Instant::now();
            let result = http
                .get(format!("{host}/health"))
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await;
            (
                host,
                result.is_ok_and(|response| response.status().is_success()),
                Some(started.elapsed().as_millis() as u64),
            )
        }
    });
    let results = join_all(probes).await;
    preferred
        .and_then(|host| {
            results
                .iter()
                .any(|(name, healthy, _)| *name == host && *healthy)
                .then_some(host)
        })
        .or_else(|| {
            results
                .into_iter()
                .filter(|(_, healthy, _)| *healthy)
                .min_by_key(|(_, _, latency)| latency.unwrap_or(u64::MAX))
                .map(|(host, _, _)| host)
        })
        .or_else(|| hosts.first().cloned())
        .unwrap_or_default()
}

#[derive(Serialize)]
pub struct LoginResult {
    pub requires_2fa: bool,
    pub temp_token: Option<String>,
    pub email_masked: Option<String>,
    pub user: Option<UserInfo>,
}

#[derive(Serialize)]
pub struct VerifyCodeResult {
    pub message: String,
    pub countdown: u64,
}

async fn store_session(
    state: &AppState,
    access_token: String,
    refresh_token: Option<String>,
    user: UserInfo,
) -> CmdResult<()> {
    {
        let mut store = state.store.write().await;
        if store.last_user_id != Some(user.id) {
            store.group_keys.clear();
            store.settings.preferred_group_id = None;
            store.settings.claude_model = None;
            store.settings.codex_model = None;
            let mut proxy = state.proxy.write().await;
            proxy.groups.clear();
            proxy.selection_revision = proxy.selection_revision.wrapping_add(1);
        }
        store.last_email = Some(user.email.clone());
        store.last_user_id = Some(user.id);
        store.refresh_token = refresh_token.clone();
        save_store(&state.app_dir, &store).map_err(e)?;
    }
    let mut session = state.session.write().await;
    *session = Some(Session {
        access_token,
        refresh_token,
        user,
    });
    Ok(())
}

#[tauri::command]
pub async fn login(
    state: State<'_, SharedState>,
    email: String,
    password: String,
) -> CmdResult<LoginResult> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let host = pick_host(&state).await;
    match api::login(&state.http, &host, &email, &password)
        .await
        .map_err(e)?
    {
        LoginOutcome::Success {
            access_token,
            refresh_token,
            user,
        } => {
            let u = user.clone();
            store_session(&state, access_token, refresh_token, user).await?;
            Ok(LoginResult {
                requires_2fa: false,
                temp_token: None,
                email_masked: None,
                user: Some(u),
            })
        }
        LoginOutcome::Needs2fa {
            temp_token,
            user_email_masked,
        } => Ok(LoginResult {
            requires_2fa: true,
            temp_token: Some(temp_token),
            email_masked: Some(user_email_masked),
            user: None,
        }),
    }
}

#[tauri::command]
pub async fn public_auth_settings(state: State<'_, SharedState>) -> CmdResult<serde_json::Value> {
    let state = state.inner().clone();
    let host = pick_host(&state).await;
    api::public_settings(&state.http, &host).await.map_err(e)
}

#[tauri::command]
pub async fn register(
    state: State<'_, SharedState>,
    email: String,
    password: String,
    verify_code: Option<String>,
    promo_code: Option<String>,
    invitation_code: Option<String>,
    aff_code: Option<String>,
) -> CmdResult<LoginResult> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let host = pick_host(&state).await;
    match api::register(
        &state.http,
        &host,
        &email,
        &password,
        verify_code.as_deref(),
        promo_code.as_deref(),
        invitation_code.as_deref(),
        aff_code.as_deref(),
    )
    .await
    .map_err(e)?
    {
        api::LoginOutcome::Success {
            access_token,
            refresh_token,
            user,
        } => {
            let result_user = user.clone();
            store_session(&state, access_token, refresh_token, user).await?;
            Ok(LoginResult {
                requires_2fa: false,
                temp_token: None,
                email_masked: None,
                user: Some(result_user),
            })
        }
        api::LoginOutcome::Needs2fa { .. } => Err("注册后登录需要 2FA，请使用登录流程验证".into()),
    }
}

#[tauri::command]
pub async fn send_verify_code(
    state: State<'_, SharedState>,
    email: String,
) -> CmdResult<VerifyCodeResult> {
    let state = state.inner().clone();
    let host = pick_host(&state).await;
    let result = api::send_verify_code(&state.http, &host, &email)
        .await
        .map_err(e)?;
    Ok(VerifyCodeResult {
        message: result["message"]
            .as_str()
            .unwrap_or("验证码已发送")
            .to_string(),
        countdown: result["countdown"].as_u64().unwrap_or(60),
    })
}

#[tauri::command]
pub async fn forgot_password(state: State<'_, SharedState>, email: String) -> CmdResult<String> {
    let state = state.inner().clone();
    let host = pick_host(&state).await;
    let result = api::forgot_password(&state.http, &host, &email)
        .await
        .map_err(e)?;
    Ok(result["message"]
        .as_str()
        .unwrap_or("如果邮箱已注册，密码重置邮件将很快发送")
        .to_string())
}

#[tauri::command]
pub async fn reset_password(
    state: State<'_, SharedState>,
    email: String,
    token: String,
    new_password: String,
) -> CmdResult<String> {
    let state = state.inner().clone();
    let host = pick_host(&state).await;
    let result = api::reset_password(&state.http, &host, &email, &token, &new_password)
        .await
        .map_err(e)?;
    Ok(result["message"]
        .as_str()
        .unwrap_or("密码已重置，请重新登录")
        .to_string())
}

#[tauri::command]
pub async fn submit_2fa(
    state: State<'_, SharedState>,
    temp_token: String,
    totp_code: String,
) -> CmdResult<UserInfo> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let host = pick_host(&state).await;
    match api::login_2fa(&state.http, &host, &temp_token, &totp_code)
        .await
        .map_err(e)?
    {
        LoginOutcome::Success {
            access_token,
            refresh_token,
            user,
        } => {
            let u = user.clone();
            store_session(&state, access_token, refresh_token, user).await?;
            Ok(u)
        }
        LoginOutcome::Needs2fa { .. } => Err("2FA 校验未通过".into()),
    }
}

#[tauri::command]
pub async fn logout(state: State<'_, SharedState>) -> CmdResult<()> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    *state.session.write().await = None;
    // Retain the listener but remove keys so later requests cannot use the
    // previous account.
    {
        let mut proxy = state.proxy.write().await;
        proxy.groups.clear();
        proxy.selection_revision = proxy.selection_revision.wrapping_add(1);
    }
    let mut store = state.store.write().await;
    store.refresh_token = None;
    save_store(&state.app_dir, &store).map_err(e)?;
    Ok(())
}

fn site_path(page: &str) -> CmdResult<&'static str> {
    match page {
        "dashboard" => Ok("/dashboard"),
        "keys" => Ok("/keys"),
        "usage" => Ok("/usage"),
        "self-monitor" => Ok("/self-monitor"),
        "subscriptions" => Ok("/subscriptions"),
        "store" => Ok("/store"),
        "profile" => Ok("/profile"),
        "balance-notifications" => Ok("/balance-notifications"),
        "purchase" => Ok("/purchase"),
        "finance" => Ok("/finance"),
        "admin-dashboard" => Ok("/admin/dashboard"),
        "admin-users" => Ok("/admin/users"),
        "admin-groups" => Ok("/admin/groups"),
        "admin-channels" => Ok("/admin/channels"),
        "admin-accounts" => Ok("/admin/accounts"),
        "admin-subscriptions" => Ok("/admin/subscriptions"),
        "admin-orders" => Ok("/admin/orders"),
        "admin-redeem" => Ok("/admin/redeem"),
        "admin-settings" => Ok("/admin/settings"),
        "admin-usage" => Ok("/admin/usage"),
        "admin-feedback" => Ok("/admin/feedback"),
        _ => Err("不支持的站点页面".into()),
    }
}

fn site_bounds(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> CmdResult<(tauri::LogicalPosition<f64>, tauri::LogicalSize<f64>)> {
    if [x, y, width, height].iter().any(|value| !value.is_finite())
        || x < 0.0
        || y < 0.0
        || width < 100.0
        || height < 100.0
    {
        return Err("站点区域无效".into());
    }
    Ok((
        tauri::LogicalPosition::new(x, y),
        tauri::LogicalSize::new(width, height),
    ))
}

#[tauri::command]
pub async fn show_site(
    state: State<'_, SharedState>,
    app: AppHandle,
    page: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    navigate: bool,
) -> CmdResult<()> {
    use tauri::{webview::WebviewBuilder, Manager, WebviewUrl};
    let path = site_path(&page)?;
    let (position, size) = site_bounds(x, y, width, height)?;
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let session = state.session.read().await;
    let session = session.as_ref().ok_or("请先登录")?;
    if page.starts_with("admin-") && session.user.role != "admin" {
        return Err("需要管理员权限".into());
    }
    if page == "finance" && !matches!(session.user.role.as_str(), "admin" | "finance") {
        return Err("需要财务权限".into());
    }
    // Browser storage is isolated by origin. Keep every embedded page on one
    // trusted site even when API requests fail over between relay lines.
    let origin = reqwest::Url::parse(SITE_HOST).map_err(e)?;
    let url = origin.join(path).map_err(e)?;
    let window = app.get_window("main").ok_or("找不到主窗口")?;
    if let Some(view) = window
        .webviews()
        .into_iter()
        .find(|view| view.label() == "relay-site")
    {
        view.set_position(position).map_err(e)?;
        view.set_size(size).map_err(e)?;
        if view.url().map_err(e)?.origin() != url.origin() {
            view.close().map_err(e)?;
        } else {
            if navigate && view.url().map_err(e)?.path() != path {
                view.navigate(url).map_err(e)?;
            }
            return view.show().map_err(e);
        }
    }
    let token = serde_json::to_string(&session.access_token).map_err(e)?;
    let user = serde_json::to_string(&session.user).map_err(e)?;
    let refresh = serde_json::to_string(&session.refresh_token).map_err(e)?;
    let allowed_origin = serde_json::to_string(origin.as_str().trim_end_matches('/')).map_err(e)?;
    let embedded_css = serde_json::to_string(
        "aside.sidebar { display: none !important; } \
         aside.sidebar + div { margin-left: 0 !important; } \
         header.glass.sticky > div > div:first-child > button.btn-icon { display: none !important; }",
    )
    .map_err(e)?;
    // Native child webviews cover React overlays; render the transition inside the page too.
    let loading_script = r#"
    (() => {
      const mount = () => {
        if (!document.body || document.getElementById('jokerdeck-site-loading')) return;
        const overlay = document.createElement('div');
        overlay.id = 'jokerdeck-site-loading';
        overlay.innerHTML = '<div class="jd-spinner"></div><strong>正在打开中转站</strong><small>连接并加载页面...</small>';
        const style = document.createElement('style');
        style.textContent = '#jokerdeck-site-loading{position:fixed;inset:0;z-index:2147483647;background:#f5f7fb;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:16px;color:#44516b;font:14px system-ui;transition:opacity .25s}.jd-spinner{width:32px;height:32px;border:3px solid #dce3ef;border-top-color:#6375e9;border-radius:50%;animation:jd-spin .9s linear infinite}@keyframes jd-spin{to{transform:rotate(360deg)}}@media(prefers-reduced-motion:reduce){.jd-spinner{animation:none}}';
        document.head.appendChild(style);
        document.body.appendChild(overlay);
        let timeout;
        const finish = () => {
          const root = document.querySelector('#app, #root');
          if (root && root.children.length) {
            observer.disconnect(); clearTimeout(timeout);
            requestAnimationFrame(() => { overlay.style.opacity = '0'; setTimeout(() => overlay.remove(), 250); });
          }
        };
        const observer = new MutationObserver(finish);
        observer.observe(document.body, {childList:true,subtree:true});
        timeout = setTimeout(() => {
          observer.disconnect();
          if (document.querySelector('#app, #root')?.children.length) overlay.remove();
          else {
            overlay.querySelector('strong').textContent = '页面加载超时';
            overlay.querySelector('small').textContent = '请检查网络连接';
            overlay.querySelector('.jd-spinner').remove();
            const retry = document.createElement('button');
            retry.textContent = '重新加载'; retry.onclick = () => location.reload(); overlay.appendChild(retry);
          }
        }, 20000);
        finish();
      };
      if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, {once:true});
      else mount();
    })();
    "#;
    let script = format!(
        "if (location.origin === {allowed_origin}) {{ \
            localStorage.setItem('auth_token', {token}); \
            localStorage.setItem('auth_user', JSON.stringify({user})); \
            const refresh = {refresh}; \
            if (refresh) localStorage.setItem('refresh_token', refresh); \
            else localStorage.removeItem('refresh_token'); \
            const addEmbeddedStyle = () => {{ \
                if (document.getElementById('jokerdeck-embedded-style')) return; \
                const style = document.createElement('style'); \
                style.id = 'jokerdeck-embedded-style'; \
                style.textContent = {embedded_css}; \
                document.head.appendChild(style); \
            }}; \
            if (document.readyState === 'loading') \
                document.addEventListener('DOMContentLoaded', addEmbeddedStyle, {{ once: true }}); \
            else addEmbeddedStyle(); \
        }}"
    );
    let builder = WebviewBuilder::new("relay-site", WebviewUrl::External(url))
        .initialization_script(format!("{script}\n{loading_script}"))
        .background_color(tauri::window::Color(245, 247, 251, 255))
        .on_navigation(|url| url.scheme() == "https");
    window.add_child(builder, position, size).map_err(e)?;
    Ok(())
}

#[tauri::command]
pub async fn hide_site(app: AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    if let Some(window) = app.get_window("main") {
        if let Some(view) = window
            .webviews()
            .into_iter()
            .find(|view| view.label() == "relay-site")
        {
            view.hide().map_err(e)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn close_site(app: AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    if let Some(window) = app.get_window("main") {
        if let Some(view) = window
            .webviews()
            .into_iter()
            .find(|view| view.label() == "relay-site")
        {
            view.clear_all_browsing_data().map_err(e)?;
            view.close().map_err(e)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_account(state: State<'_, SharedState>) -> CmdResult<UserInfo> {
    let state = state.inner().clone();
    let token = require_token(&state).await?;
    let host = pick_host(&state).await;
    let user = api::get_me(&state.http, &host, &token).await.map_err(e)?;
    if let Some(s) = state.session.write().await.as_mut() {
        s.user = user.clone();
    }
    Ok(user)
}

#[derive(Serialize)]
pub struct ApplyResult {
    pub proxy_port: u16,
    pub base_url: String,
    pub groups: usize,
    pub files: Vec<String>,
    /// Group the proxy is now using (so the UI can highlight it).
    pub active_group_id: Option<i64>,
    pub warnings: Vec<String>,
}

/// Provision only the selected group's key and apply group-pinned configs.
#[tauri::command]
pub async fn apply_config(
    state: State<'_, SharedState>,
    app: AppHandle,
    configure_claude: bool,
    configure_codex: bool,
    codex_model: Option<String>,
    claude_model: Option<String>,
    group_id: Option<i64>,
) -> CmdResult<ApplyResult> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let progress = |percent: u8, detail: &str| {
        let _ = app.emit(
            "setup-progress",
            serde_json::json!({ "percent": percent, "detail": detail }),
        );
    };
    progress(5, "正在连接 sub2api 线路");
    if !configure_claude && !configure_codex {
        return Err("请至少选择一个工具".into());
    }
    #[cfg(target_os = "android")]
    return Err("Android 无法修改 Windows 配置，请在 Windows 客户端应用配置".into());
    let token = require_token(&state).await?;
    let host = pick_host(&state).await;
    let cfg = snapshot(&state).await;

    // `groups/available` already scopes itself to what this account may bind a
    // key to, so do NOT narrow it further: filtering by `allowed_groups` used to
    // silently drop public groups the user can legitimately use, leaving the
    // picker offering groups the proxy had no key for.
    let (groups, plaza) = tokio::try_join!(
        api::list_available_groups(&state.http, &host, &token),
        api::fetch_plaza(&state.http, &host, &token),
    )
    .map_err(e)?;
    progress(20, "已读取可用分组");
    if groups.is_empty() {
        return Err("没有可用分组，请联系管理员开通".into());
    }
    let selected_id = group_id
        .or(cfg.preferred_group_id)
        .ok_or_else(|| "请先选择分组，再一键应用配置".to_string())?;
    if !groups.iter().any(|g| g.id == selected_id) {
        return Err("所选分组未开通或已停用，请重新选择".into());
    }
    progress(35, "已读取分组模型");
    let selected = plaza
        .iter()
        .find(|g| g.id == selected_id)
        .ok_or_else(|| "所选分组没有模型信息".to_string())?;
    let claude_names: Vec<&str> = selected
        .models
        .iter()
        .filter(|m| {
            let platform = if m.platform.is_empty() {
                &selected.platform
            } else {
                &m.platform
            };
            selected.platform.eq_ignore_ascii_case("anthropic")
                || platform.eq_ignore_ascii_case("anthropic")
                || platform.eq_ignore_ascii_case("claude")
        })
        .map(|m| m.name.as_str())
        .collect();
    let claude_choice = if configure_claude {
        Some(select_model(
            claude_model.as_deref(),
            cfg.claude_model.as_deref(),
            &claude_names,
        )?)
    } else {
        None
    };
    let existing = api::list_keys(&state.http, &host, &token)
        .await
        .map_err(e)?;
    progress(48, "正在检查分组 Key");
    let stored = state.store.read().await.group_keys.clone();
    let mut new_keys: HashMap<i64, String> = HashMap::new();
    let mut group_keys: Vec<GroupKey> = Vec::new();
    // Only provision the selected group. A click must not create keys in every
    // group or silently select the cheapest one.
    for g in groups.iter().filter(|g| g.id == selected_id) {
        let name = format!("jokerdeck-client-g{}", g.id);
        let key = existing
            .iter()
            .find(|k| k.group_id == Some(g.id) && k.name == name && k.status == "active")
            .and_then(|k| {
                if k.key.is_empty() {
                    stored.get(&g.id)
                } else {
                    Some(&k.key)
                }
            })
            .cloned();
        let key = match key {
            Some(key) => key,
            None => {
                api::create_key(&state.http, &host, &token, &name, Some(g.id))
                    .await
                    .map_err(e)?
                    .key
            }
        };
        progress(62, "分组 Key 已就绪");
        new_keys.insert(g.id, key.clone());
        group_keys.push(GroupKey {
            group_id: g.id,
            name: g.name.clone(),
            key,
            multiplier: g.multiplier,
        });
    }
    let catalog = if configure_codex {
        Some(
            api::fetch_codex_catalog(&state.http, &host, &group_keys[0].key)
                .await
                .map_err(e)?,
        )
    } else {
        None
    };
    progress(
        72,
        if configure_codex {
            "正在同步 Codex 模型目录"
        } else {
            "正在准备配置文件"
        },
    );
    let codex_names: Vec<&str> = catalog
        .as_ref()
        .and_then(|c| c["models"].as_array())
        .map(|models| {
            models
                .iter()
                .filter(|m| {
                    m["visibility"]
                        .as_str()
                        .is_none_or(|visibility| visibility == "list")
                })
                .filter_map(|m| m["slug"].as_str())
                .collect()
        })
        .unwrap_or_default();
    let codex_choice = if configure_codex {
        Some(select_model(
            codex_model.as_deref(),
            cfg.codex_model.as_deref(),
            &codex_names,
        )?)
    } else {
        None
    };
    config_writer::validate_existing(configure_claude, configure_codex).map_err(e)?;
    let mut paths = Vec::new();
    if configure_claude {
        paths.push(config_writer::claude_settings_path().map_err(e)?);
    }
    if configure_codex {
        paths.push(config_writer::codex_config_path().map_err(e)?);
        paths.push(config_writer::codex_catalog_path().map_err(e)?);
    }
    progress(86, "正在写入 Claude Code / Codex 配置");
    let transaction = config_writer::ConfigTransaction::begin(paths).map_err(e)?;
    let need_start = {
        let ps = state.proxy.read().await;
        !ps.running
    };
    if need_start {
        let ctx = crate::proxy::ProxyCtx {
            shared: state.proxy.clone(),
            http: state.http.clone(),
            app: app.clone(),
        };
        let runtime = crate::proxy::start(ctx, cfg.port).await.map_err(e)?;
        *state.proxy_runtime.lock().await = Some(runtime);
    }
    let base = format!("http://127.0.0.1:{}/groups/{selected_id}", cfg.port);
    let mut files: Vec<String> = Vec::new();
    if configure_claude {
        files.push(
            config_writer::write_claude(
                &base,
                claude_choice.as_deref().ok_or("未选择 Claude Code 模型")?,
                &claude_names
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>(),
            )
            .map_err(e)?
            .display()
            .to_string(),
        );
    }
    if configure_codex {
        files.extend(
            config_writer::write_codex(
                &format!("{base}/v1"),
                codex_choice.as_deref(),
                catalog.as_ref(),
            )
            .map_err(e)?
            .into_iter()
            .map(|p| p.display().to_string()),
        );
    }
    // Rehydrate previously applied groups only after verifying their saved keys
    // against this account's live key list.
    for g in groups.iter().filter(|g| g.id != selected_id) {
        if let Some(key) = stored.get(&g.id).filter(|key| {
            existing
                .iter()
                .any(|k| k.group_id == Some(g.id) && &k.key == *key && k.status == "active")
        }) {
            group_keys.push(GroupKey {
                group_id: g.id,
                name: g.name.clone(),
                key: key.clone(),
                multiplier: g.multiplier,
            });
        }
    }
    {
        let mut store = state.store.write().await;
        let mut next = store.clone();
        next.group_keys.extend(new_keys);
        next.settings.preferred_group_id = Some(selected_id);
        next.settings.auto_fallback = false;
        if configure_claude {
            next.settings.claude_model = claude_choice;
        }
        if configure_codex {
            next.settings.codex_model = codex_choice;
        }
        save_store(&state.app_dir, &next).map_err(e)?;
        *store = next;
    }
    {
        let mut ps = state.proxy.write().await;
        ps.hosts = cfg
            .hosts
            .iter()
            .map(|h| HostHealth {
                host: h.clone(),
                healthy: ps
                    .hosts
                    .iter()
                    .find(|entry| &entry.host == h)
                    .map(|entry| entry.healthy)
                    .unwrap_or(true),
                latency_ms: ps
                    .hosts
                    .iter()
                    .find(|entry| &entry.host == h)
                    .and_then(|entry| entry.latency_ms),
            })
            .collect();
        // Retain previously applied tool groups: Claude and Codex can use
        // different groups through their individually pinned URL paths.
        ps.groups = group_keys.clone();
        ps.active_group_idx = ps
            .groups
            .iter()
            .position(|g| g.group_id == selected_id)
            .unwrap_or_else(|| ps.hosts.iter().position(|entry| entry.healthy).unwrap_or(0));
        ps.selection_revision = ps.selection_revision.wrapping_add(1);
        ps.preferred_host = cfg.preferred_host.filter(|host| cfg.hosts.contains(host));
        ps.active_host_idx = ps
            .preferred_host
            .as_ref()
            .and_then(|host| ps.hosts.iter().position(|entry| &entry.host == host))
            .unwrap_or(0);
        ps.auto_fallback = false;
        ps.extra_headers = if cfg.computer_use {
            vec![
                ("anthropic-beta".into(), "computer-use-2025-01-24".into()),
            ]
        } else {
            Vec::new()
        };
    }
    transaction.commit();
    progress(100, "一键配置完成");
    let mut warnings = if configure_claude {
        config_writer::claude_config_warnings()
    } else {
        Vec::new()
    };
    if let Err(error) = crate::startup::set_enabled(true) {
        warnings.push(format!(
            "登录启动项设置失败，电脑重启后请打开客户端恢复代理配置：{error}"
        ));
    }
    if configure_codex && state.store.read().await.settings.provider_auto_sync {
        if let Err(error) = crate::desktop_features::provider_action(&state, "sync", None).await {
            warnings.push(format!("配置已应用，但历史 Provider 同步失败：{error}"));
        }
    }
    Ok(ApplyResult {
        proxy_port: cfg.port,
        base_url: base,
        groups: group_keys.len(),
        files,
        active_group_id: {
            let ps = state.proxy.read().await;
            ps.groups.get(ps.active_group_idx).map(|g| g.group_id)
        },
        warnings,
    })
}

fn select_model(
    explicit: Option<&str>,
    saved: Option<&str>,
    available: &[&str],
) -> CmdResult<String> {
    if let Some(model) = explicit.filter(|m| !m.is_empty()) {
        return if available.contains(&model) {
            Ok(model.to_string())
        } else {
            Err(format!("当前分组不支持模型 {model}，请重新选择"))
        };
    }
    saved
        .filter(|m| available.contains(m))
        .or_else(|| available.first().copied())
        .map(str::to_string)
        .ok_or_else(|| "当前分组没有适用于所选工具的模型".into())
}

#[tauri::command]
pub async fn list_groups(state: State<'_, SharedState>) -> CmdResult<Vec<api::Group>> {
    let state = state.inner().clone();
    let token = require_token(&state).await?;
    let host = pick_host(&state).await;
    api::list_available_groups(&state.http, &host, &token)
        .await
        .map_err(e)
}

/// Groups the user can use, each with its models — powers the group/model pickers.
#[tauri::command]
pub async fn fetch_plaza(state: State<'_, SharedState>) -> CmdResult<Vec<api::PlazaGroup>> {
    let state = state.inner().clone();
    let token = require_token(&state).await?;
    let host = pick_host(&state).await;
    api::fetch_plaza(&state.http, &host, &token)
        .await
        .map_err(e)
}

#[derive(Serialize)]
pub struct GroupModels {
    pub group_id: i64,
    pub claude_models: Vec<String>,
    pub codex_models: Vec<String>,
    pub codex_error: Option<String>,
}

async fn group_models(state: &AppState, group_id: i64) -> CmdResult<GroupModels> {
    let token = require_token(state).await?;
    let host = pick_host(state).await;
    let (groups, plaza) = tokio::try_join!(
        api::list_available_groups(&state.http, &host, &token),
        api::fetch_plaza(&state.http, &host, &token),
    )
    .map_err(e)?;
    if !groups.iter().any(|g| g.id == group_id) {
        return Err("该分组不可用，请重新选择".into());
    }
    let group = plaza
        .iter()
        .find(|g| g.id == group_id)
        .ok_or("分组没有模型信息")?;
    let claude_models: Vec<String> = group
        .models
        .iter()
        .filter(|m| {
            let platform = if m.platform.is_empty() {
                &group.platform
            } else {
                &m.platform
            };
            group.platform.eq_ignore_ascii_case("anthropic")
                || platform.eq_ignore_ascii_case("anthropic")
                || platform.eq_ignore_ascii_case("claude")
        })
        .map(|m| m.name.clone())
        .collect();
    let existing = api::list_keys(&state.http, &host, &token)
        .await
        .map_err(e)?;
    let name = format!("jokerdeck-client-g{group_id}");
    let stored = state.store.read().await.group_keys.get(&group_id).cloned();
    let key = match existing
        .iter()
        .find(|k| k.group_id == Some(group_id) && k.name == name && k.status == "active")
    {
        Some(k) if !k.key.is_empty() => k.key.clone(),
        Some(_) if stored.is_some() => stored.unwrap(),
        _ => {
            api::create_key(&state.http, &host, &token, &name, Some(group_id))
                .await
                .map_err(e)?
                .key
        }
    };
    {
        let mut store = state.store.write().await;
        if store.group_keys.get(&group_id) != Some(&key) {
            let mut next = store.clone();
            next.group_keys.insert(group_id, key.clone());
            save_store(&state.app_dir, &next).map_err(e)?;
            *store = next;
        }
    }
    let (codex_models, codex_error) = match api::fetch_codex_catalog(&state.http, &host, &key).await
    {
        Ok(catalog) => {
            let names = catalog["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|model| {
                    model["visibility"]
                        .as_str()
                        .is_none_or(|visibility| visibility == "list")
                })
                .filter_map(|model| model["slug"].as_str().map(str::to_string))
                .collect();
            (names, None)
        }
        Err(err) => (Vec::new(), Some(format!("Codex 模型目录拉取失败：{err}"))),
    };
    Ok(GroupModels {
        group_id,
        claude_models,
        codex_models,
        codex_error,
    })
}

#[tauri::command]
pub async fn fetch_group_models(
    state: State<'_, SharedState>,
    group_id: i64,
) -> CmdResult<GroupModels> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    group_models(&state, group_id).await
}

#[derive(Serialize)]
pub struct ModelProbe {
    pub ok: bool,
    pub detail: String,
}

#[tauri::command]
pub async fn test_group_model(
    state: State<'_, SharedState>,
    group_id: i64,
    tool: String,
    model: String,
) -> CmdResult<ModelProbe> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    probe_group_model(&state, group_id, &tool, &model).await
}

async fn probe_group_model(
    state: &AppState,
    group_id: i64,
    tool: &str,
    model: &str,
) -> CmdResult<ModelProbe> {
    let models = group_models(&state, group_id).await?;
    let available = match tool {
        "claude" => &models.claude_models,
        "codex" => &models.codex_models,
        _ => return Err("不支持的工具".into()),
    };
    if !available.iter().any(|name| name == model) {
        return Err("所选模型不在当前分组的可用列表中".into());
    }
    let host = pick_host(&state).await;
    let key = state
        .store
        .read()
        .await
        .group_keys
        .get(&group_id)
        .cloned()
        .ok_or("分组 Key 缺失")?;
    let request = match tool {
        "claude" => state.http.post(format!("{host}/v1/messages"))
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({"model":model,"max_tokens":16,"messages":[{"role":"user","content":"ping"}]})),
        _ => state.http.post(format!("{host}/v1/responses"))
            .bearer_auth(&key)
            .json(&serde_json::json!({"model":model,"input":"ping","max_output_tokens":128,"stream":true})),
    };
    let response = request
        .timeout(std::time::Duration::from_secs(35))
        .send()
        .await
        .map_err(e)?;
    let (ok, detail) = crate::model_probe::read(response).await?;
    Ok(ModelProbe { ok, detail })
}

#[derive(Serialize)]
pub struct ToolConfigView {
    pub path: String,
    pub content: String,
}

#[tauri::command]
pub fn read_tool_config(which: String) -> CmdResult<ToolConfigView> {
    let view = config_writer::read_config(&which).map_err(e)?;
    Ok(ToolConfigView {
        path: view.path.display().to_string(),
        content: view.content,
    })
}

#[tauri::command]
pub async fn proxy_status(state: State<'_, SharedState>) -> CmdResult<ProxyStatus> {
    let state = state.inner().clone();
    proxy_status_from_state(&state).await
}

#[tauri::command]
pub async fn stop_proxy(state: State<'_, SharedState>) -> CmdResult<ProxyStatus> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    // Never leave CLI clients pointing at a listener that has been stopped.
    Ok(restore_config_inner(&state).await?.status)
}

async fn stop_proxy_inner(state: &AppState) -> CmdResult<ProxyStatus> {
    if let Some(runtime) = state.proxy_runtime.lock().await.take() {
        runtime.stop().await;
    }
    {
        let mut proxy = state.proxy.write().await;
        proxy.running = false;
        proxy.groups.clear();
        proxy.selection_revision = proxy.selection_revision.wrapping_add(1);
    }
    proxy_status_from_state(&state).await
}

#[derive(Serialize)]
pub struct RestoreConfigResult {
    pub files: Vec<String>,
    pub warnings: Vec<String>,
    pub status: ProxyStatus,
}

#[tauri::command]
pub async fn restore_config(state: State<'_, SharedState>) -> CmdResult<RestoreConfigResult> {
    if cfg!(target_os = "android") {
        return Err("Android 无法恢复 Windows 工具配置".into());
    }
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    restore_config_inner(&state).await
}

async fn restore_config_inner(state: &AppState) -> CmdResult<RestoreConfigResult> {
    let restored = config_writer::restore_configs().map_err(e)?;
    if !restored.files.is_empty() {
        let mut store = state.store.write().await;
        let mut next = store.clone();
        if restored
            .files
            .contains(&config_writer::claude_settings_path().map_err(e)?)
        {
            next.settings.claude_model = None;
        }
        if restored
            .files
            .contains(&config_writer::codex_config_path().map_err(e)?)
        {
            next.settings.codex_model = None;
        }
        next.settings.preferred_group_id = None;
        save_store(&state.app_dir, &next).map_err(e)?;
        *store = next;
    }
    let status = stop_proxy_inner(&state).await?;
    if let Err(error) = crate::startup::set_enabled(false) {
        tracing::warn!("无法撤销代理恢复登录启动项：{error}");
    }
    Ok(RestoreConfigResult {
        files: restored
            .files
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        warnings: restored.warnings,
        status,
    })
}

#[tauri::command]
pub async fn quit_app(
    state: State<'_, SharedState>,
    app: AppHandle,
    restore: bool,
) -> CmdResult<()> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    if restore {
        let result = restore_config_inner(&state).await?;
        if !result.warnings.is_empty() {
            return Err(format!("配置未完全恢复：{}", result.warnings.join("；")));
        }
    } else if state.proxy.read().await.running {
        return Err("请先关闭代理并恢复配置".into());
    }
    crate::codexplusplus::stop().await;
    app.exit(0);
    Ok(())
}

/// Run before login as well as after login, giving the UI an immediate line
/// status and ensuring the first login request uses the fastest reachable host.
#[tauri::command]
pub async fn probe_hosts(state: State<'_, SharedState>) -> CmdResult<Vec<HostHealth>> {
    let state = state.inner().clone();
    let hosts = { state.store.read().await.hosts.clone() };
    let probes = hosts.into_iter().map(|host| {
        let http = state.http.clone();
        async move {
            let started = std::time::Instant::now();
            let healthy = http
                .get(format!("{host}/health"))
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            HostHealth {
                host,
                healthy,
                latency_ms: Some(started.elapsed().as_millis() as u64),
            }
        }
    });
    let mut result = join_all(probes).await;
    result.sort_by_key(|host| (!host.healthy, host.latency_ms.unwrap_or(u64::MAX)));
    if !result.is_empty()
        && state
            .store
            .read()
            .await
            .hosts
            .iter()
            .all(|host| result.iter().any(|item| &item.host == host))
    {
        let mut proxy = state.proxy.write().await;
        proxy.hosts = result.clone();
        proxy.active_host_idx = proxy
            .preferred_host
            .as_ref()
            .and_then(|host| proxy.hosts.iter().position(|item| &item.host == host))
            .unwrap_or(0);
        proxy.selection_revision = proxy.selection_revision.wrapping_add(1);
    }
    Ok(result)
}

#[tauri::command]
pub async fn set_preferred_host(
    state: State<'_, SharedState>,
    host: Option<String>,
) -> CmdResult<ProxyStatus> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let mut store = state.store.write().await;
    if let Some(ref selected) = host {
        if !store.hosts.contains(selected) {
            return Err("该线路不在已配置的线路列表中".into());
        }
    }
    let mut next = store.clone();
    next.settings.preferred_host = host.clone();
    save_store(&state.app_dir, &next).map_err(e)?;
    *store = next;
    drop(store);
    {
        let mut proxy = state.proxy.write().await;
        proxy.preferred_host = host.clone();
        if let Some(ref selected) = host {
            if let Some(index) = proxy.hosts.iter().position(|entry| &entry.host == selected) {
                proxy.active_host_idx = index;
            }
        } else if let Some((index, _)) = proxy
            .hosts
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.healthy)
            .min_by_key(|(_, entry)| entry.latency_ms.unwrap_or(u64::MAX))
        {
            proxy.active_host_idx = index;
        }
        proxy.selection_revision = proxy.selection_revision.wrapping_add(1);
    }
    proxy_status_from_state(&state).await
}

async fn proxy_status_from_state(state: &AppState) -> CmdResult<ProxyStatus> {
    let st = state.proxy.read().await;
    let active_host = st.hosts.get(st.active_host_idx).map(|h| h.host.clone());
    let active_group = st.groups.get(st.active_group_idx).cloned();
    Ok(ProxyStatus {
        running: st.running,
        port: st.port,
        base_url: if st.port > 0 {
            format!("http://127.0.0.1:{}", st.port)
        } else {
            String::new()
        },
        active_host,
        preferred_host: st.preferred_host.clone(),
        hosts: st.hosts.clone(),
        groups: st.groups.clone(),
        active_group,
        auto_fallback: st.auto_fallback,
    })
}

#[tauri::command]
pub async fn set_auto_fallback(state: State<'_, SharedState>, enabled: bool) -> CmdResult<()> {
    let state = state.inner().clone();
    state.proxy.write().await.auto_fallback = enabled;
    let mut store = state.store.write().await;
    store.settings.auto_fallback = enabled;
    save_store(&state.app_dir, &store).map_err(e)
}

/// Toggle opt-in beta features by injecting headers upstream. Whether a feature
/// actually works still depends on the relay/upstream supporting it.
#[tauri::command]
pub async fn set_extensions(state: State<'_, SharedState>, computer_use: bool) -> CmdResult<()> {
    let state = state.inner().clone();
    let mut headers: Vec<(String, String)> = Vec::new();
    if computer_use {
        headers.push(("anthropic-beta".into(), "computer-use-2025-01-24".into()));
    }
    state.proxy.write().await.extra_headers = headers;
    // Persist so the toggle survives a restart and re-apply.
    let mut store = state.store.write().await;
    store.settings.computer_use = computer_use;
    save_store(&state.app_dir, &store).map_err(e)
}

#[tauri::command]
pub async fn detect_clis() -> CmdResult<cli_manager::CliReport> {
    Ok(cli_manager::detect_all().await)
}

#[tauri::command]
pub async fn native_browser_status(state: State<'_, SharedState>) -> CmdResult<serde_json::Value> {
    let enabled = state.store.read().await.settings.native_browser_compatibility;
    let connection = crate::codexplusplus::native_browser_connection::check_connection().await;
    let runtime = crate::codexplusplus::native_browser::read_status();
    Ok(serde_json::json!({"enabled": enabled, "runtime": runtime, "connection": connection}))
}

#[tauri::command]
pub async fn computer_tools_status() -> CmdResult<crate::computer_tools::Status> {
    tokio::task::spawn_blocking(crate::computer_tools::status).await.map_err(e)?.map_err(e)
}

#[tauri::command]
pub async fn configure_computer_tools(state: State<'_, SharedState>, enabled: bool) -> CmdResult<crate::computer_tools::Status> {
    let _guard = state.configuration_lock.lock().await;
    if enabled && !cfg!(any(windows, target_os = "macos")) { return Err("仅支持 Windows 和 macOS 桌面系统".into()); }
    let mut store = state.store.write().await;
    let mut next = store.clone();
    next.settings.native_computer_tools = enabled;
    save_store(&state.app_dir, &next).map_err(e)?;
    *store = next;
    crate::computer_tools::set_enabled(enabled);
    crate::computer_tools::status().map_err(e)
}

#[tauri::command]
pub async fn client_provider_policy(state: State<'_, SharedState>) -> CmdResult<serde_json::Value> {
    let host = pick_host(&state).await;
    let policy = api::client_provider_policy(&state.http, &host).await.map_err(e)?;
    let token = require_token(&state).await?;
    let user = api::get_me(&state.http, &host, &token).await.map_err(e)?;
    let provider = tokio::task::spawn_blocking(crate::provider_manager::status).await.map_err(e)?.map_err(e)?;
    Ok(serde_json::json!({"allow_provider_switch": policy["allow_provider_switch"] == true,
        "eligible": policy["allow_provider_switch"] == true && user.total_recharged > 0.0,
        "is_admin": user.role == "admin", "provider": provider}))
}

#[tauri::command]
pub async fn set_client_provider_policy(state: State<'_, SharedState>, allowed: bool) -> CmdResult<()> {
    let host = pick_host(&state).await;
    let token = require_token(&state).await?;
    // The server's admin middleware is authoritative, regardless of local UI state.
    api::set_client_provider_policy(&state.http, &host, &token, allowed).await.map_err(e)?;
    Ok(())
}

#[tauri::command]
pub async fn switch_external_provider(state: State<'_, SharedState>, base_url: String, api_key: String, model: String) -> CmdResult<crate::provider_manager::Status> {
    let _guard = state.configuration_lock.lock().await;
    let host = pick_host(&state).await;
    let token = require_token(&state).await?;
    let policy = api::client_provider_policy(&state.http, &host).await.map_err(e)?;
    let user = api::get_me(&state.http, &host, &token).await.map_err(e)?;
    if policy["allow_provider_switch"] != true { return Err("管理员未开放服务商切换".into()); }
    if user.total_recharged <= 0.0 || !user.total_recharged.is_finite() { return Err("仅充值过的用户可以切换外部服务商".into()); }
    if crate::codex_desktop::running().await.map_err(e)? {
        return Err("请先关闭 Codex Desktop，再切换服务商以保留一致的会话状态".into());
    }
    tokio::task::spawn_blocking(move || crate::provider_manager::switch(&base_url, &api_key, &model)).await.map_err(e)?.map_err(e)
}

#[tauri::command]
pub async fn configure_native_browser(state: State<'_, SharedState>, enabled: bool) -> CmdResult<()> {
    if enabled && !cfg!(windows) {
        return Err("Codex++ 原生 Browser 兼容目前仅支持 Windows".into());
    }
    let _guard = state.configuration_lock.lock().await;
    {
        let mut store = state.store.write().await;
        let mut next = store.clone();
        next.settings.native_browser_compatibility = enabled;
        save_store(&state.app_dir, &next).map_err(e)?;
        *store = next;
    }
    crate::codexplusplus::configure(enabled).await;
    Ok(())
}

#[derive(Serialize)]
pub struct InstallResult {
    pub ok: bool,
    pub log: String,
}

#[tauri::command]
pub async fn install_cli(which: String) -> CmdResult<InstallResult> {
    if cfg!(target_os = "android") {
        return Err("请在 Windows 客户端安装桌面工具".into());
    }
    let (ok, log) = cli_manager::install(&which).await;
    Ok(InstallResult { ok, log })
}

#[tauri::command]
pub async fn download_codex_desktop(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> CmdResult<InstallResult> {
    if cfg!(target_os = "android") {
        return Err("当前平台不支持安装 Codex Desktop".into());
    }
    let report = |progress| {
        let _ = app.emit("codex-desktop-download-progress", progress);
    };
    crate::codex_desktop_download::download(&state.http, &state.app_dir, &report)
        .await
        .map_err(e)
}

#[tauri::command]
pub async fn restart_codex(state: State<'_, SharedState>) -> CmdResult<InstallResult> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    let app_dir = state.app_dir.clone();
    let (_, localized_active) = bounded_blocking(std::time::Duration::from_secs(5), move || {
        crate::codex_localization::status(&app_dir)
    }).await?;
    let (ok, log) = crate::codex_desktop::restart(localized_active).await;
    Ok(InstallResult { ok, log })
}

#[tauri::command]
pub async fn codex_desktop_running() -> CmdResult<bool> {
    crate::codex_desktop::running().await.map_err(e)
}

#[tauri::command]
pub async fn stop_codex_desktop(state: State<'_, SharedState>) -> CmdResult<()> {
    let state = state.inner().clone();
    let _guard = state.configuration_lock.lock().await;
    crate::codex_desktop::stop().await.map_err(e)
}

#[tauri::command]
pub async fn codex_localization(
    app: AppHandle,
    state: State<'_, SharedState>,
    action: String,
) -> CmdResult<String> {
    let preferred_host = {
        let store = state.store.read().await;
        store
            .settings
            .preferred_host
            .clone()
            .unwrap_or_else(|| crate::state::SITE_HOST.to_string())
    };
    let _guard = state.configuration_lock.lock().await;
    let completed = std::sync::atomic::AtomicU8::new(0);
    let report = |mut value: crate::codex_localization::LocalizationProgress| {
        // Installer substeps can report 100 before the actual desktop launch.
        let percent = value.percent.min(99);
        value.percent = completed
            .fetch_max(percent, std::sync::atomic::Ordering::Relaxed)
            .max(percent);
        let _ = app.emit("codex-localization-progress", value);
    };
    let result = crate::codex_localization::run(
        &state.app_dir,
        &state.http,
        &action,
        &preferred_host,
        &report,
    )
    .await
    .map_err(e)?;
    let _ = app.emit(
        "codex-localization-progress",
        crate::codex_localization::LocalizationProgress {
            percent: 100,
            detail: "桌面应用启动完成".into(),
        },
    );
    Ok(result)
}

#[tauri::command]
pub async fn list_codex_sessions() -> CmdResult<Vec<crate::codex_sessions::Session>> {
    tokio::task::spawn_blocking(crate::codex_sessions::list)
        .await
        .map_err(e)?
        .map_err(e)
}

#[tauri::command]
pub async fn read_codex_session(id: String) -> CmdResult<crate::codex_sessions::SessionDetail> {
    tokio::task::spawn_blocking(move || crate::codex_sessions::detail(&id))
        .await
        .map_err(e)?
        .map_err(e)
}

#[tauri::command]
pub async fn delete_codex_session(id: String) -> CmdResult<()> {
    crate::codex_sessions::delete(&id).await.map_err(e)
}

#[tauri::command]
pub async fn codex_enhancement_status() -> CmdResult<crate::codex_enhancement::Status> {
    tokio::task::spawn_blocking(crate::codex_enhancement::status)
        .await
        .map_err(e)?
        .map_err(e)
}

#[tauri::command]
pub async fn enable_codex_marketplace(state: State<'_, SharedState>) -> CmdResult<String> {
    let _guard = state.configuration_lock.lock().await;
    tokio::task::spawn_blocking(crate::codex_enhancement::enable_marketplace)
        .await
        .map_err(e)?
        .map_err(e)
}

#[tauri::command]
pub async fn register_codex_plugin_cache(state: State<'_, SharedState>) -> CmdResult<String> {
    let _guard = state.configuration_lock.lock().await;
    tokio::task::spawn_blocking(crate::codex_enhancement::register_cache)
        .await
        .map_err(e)?
        .map_err(e)
}

#[tauri::command]
pub async fn run_diagnostics(state: State<'_, SharedState>) -> CmdResult<diagnostics::DiagReport> {
    let state = state.inner().clone();
    Ok(diagnostics::run(&state).await)
}

#[tauri::command]
pub async fn check_update(state: State<'_, SharedState>) -> CmdResult<updater::UpdateInfo> {
    let state = state.inner().clone();
    let url = state
        .store
        .read()
        .await
        .settings
        .update_manifest_url
        .clone()
        .unwrap_or_else(|| updater::DEFAULT_MANIFEST.to_string());
    Ok(updater::check(&state.http, &url).await)
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, SharedState>,
    url: String,
) -> CmdResult<updater::InstallUpdateResult> {
    let state = state.inner().clone();
    updater::download_and_install(&state.http, &state.app_dir, &url, &app)
        .await
        .map_err(Into::into)
}

#[derive(Serialize)]
pub struct Bootstrap {
    pub logged_in: bool,
    pub last_email: Option<String>,
    pub user: Option<UserInfo>,
    pub version: String,
    /// Current persisted choices, so the UI can prefill its pickers.
    pub preferred_group_id: Option<i64>,
    pub claude_model: Option<String>,
    pub codex_model: Option<String>,
    pub computer_use: bool,
    pub auto_fallback: bool,
    pub saved_password: Option<String>,
    pub remember_password: bool,
    pub site_url: String,
    pub hosts: Vec<String>,
    pub preferred_host: Option<String>,
    pub desktop_supported: bool,
    pub codex_localization_available: bool,
    pub codex_localization_active: bool,
}

#[tauri::command]
pub async fn get_bootstrap(state: State<'_, SharedState>) -> CmdResult<Bootstrap> {
    let state = state.inner().clone();
    let (logged_in, user) = {
        let session = state.session.read().await;
        (session.is_some(), session.as_ref().map(|s| s.user.clone()))
    };
    // 状态刷新不持锁访问 Keychain 或启动系统进程，避免阻塞代理与其他命令。
    let store = state.store.read().await.clone();
    let remember_password = store.saved_password.is_some();
    let saved_password = if logged_in {
        None
    } else if store.saved_password.is_some() {
        store.saved_password.clone()
    } else {
        let credentials = store.clone();
        match bounded_blocking(std::time::Duration::from_secs(3), move || {
            saved_login_password(&credentials)
        }).await {
            Ok(password) => password,
            Err(error) => {
                tracing::warn!(%error, "保存的密码读取超时或失败，可手动登录");
                None
            }
        }
    };
    let app_dir = state.app_dir.clone();
    let (codex_localization_available, codex_localization_active) =
        bounded_blocking(std::time::Duration::from_secs(5), move || {
            crate::codex_localization::status(&app_dir)
        }).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "Codex 本地化状态检测失败");
            (false, false)
        });
    Ok(Bootstrap {
        logged_in,
        last_email: store.last_email.clone(),
        user,
        version: updater::CURRENT_VERSION.to_string(),
        preferred_group_id: store.settings.preferred_group_id,
        claude_model: store.settings.claude_model.clone(),
        codex_model: store.settings.codex_model.clone(),
        computer_use: store.settings.computer_use,
        auto_fallback: store.settings.auto_fallback,
        remember_password: remember_password || saved_password.is_some(),
        saved_password,
        site_url: store.hosts.first().cloned().unwrap_or_default(),
        hosts: store.hosts.clone(),
        preferred_host: store.settings.preferred_host.clone(),
        desktop_supported: !cfg!(target_os = "android"),
        codex_localization_available,
        codex_localization_active,
    })
}

#[tauri::command]
pub async fn save_login(
    state: State<'_, SharedState>,
    email: String,
    password: String,
    remember: bool,
) -> CmdResult<()> {
    if remember && !cfg!(any(windows, target_os = "macos")) {
        return Err("当前平台暂不支持安全保存密码".into());
    }
    let state = state.inner();
    let _guard = state.configuration_lock.lock().await;
    let email = email.trim().to_owned();
    #[cfg(target_os = "macos")]
    {
        let previous = state.store.read().await.last_email.clone();
        let credential_email = email.clone();
        let credential_password = password.clone();
        // Keychain 写入可能等待系统授权，但不能占用异步线程或 Store 锁。
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            if let Some(previous) = previous.as_deref() {
                if previous != credential_email || !remember {
                    crate::secret_store::remove_password(previous)?;
                }
            }
            if remember {
                crate::secret_store::save_password(&credential_email, &credential_password)?;
            }
            Ok(())
        }).await.map_err(e)?.map_err(e)?;
    }
    let mut store = state.store.write().await;
    store.last_email = Some(email);
    store.saved_password = if remember { Some(password) } else { None };
    save_store(&state.app_dir, &store).map_err(e)
}

fn saved_login_password(store: &crate::state::Store) -> Option<String> {
    #[cfg(target_os = "macos")]
    return store
        .last_email
        .as_deref()
        .and_then(crate::secret_store::load_password);
    #[cfg(not(target_os = "macos"))]
    return store.saved_password.clone();
}

#[tauri::command]
pub async fn open_site(
    state: State<'_, SharedState>,
    app: AppHandle,
    page: String,
) -> CmdResult<()> {
    let path = match page.as_str() {
        "register" => "/register",
        "forgot-password" => "/forgot-password",
        "dashboard" => "/dashboard",
        "keys" => "/keys",
        "usage" => "/usage",
        "subscriptions" => "/subscriptions",
        "profile" => "/profile",
        "recharge" => "/recharge",
        "login" => "/login",
        _ => return Err("不支持的站点页面".into()),
    };
    let _ = state;
    open_url(app, format!("{SITE_HOST}{path}")).await
}

#[tauri::command]
pub async fn forget_password(state: State<'_, SharedState>) -> CmdResult<()> {
    let state = state.inner();
    let _guard = state.configuration_lock.lock().await;
    #[cfg(target_os = "macos")]
    {
        let email = state.store.read().await.last_email.clone();
        if let Some(email) = email {
            tokio::task::spawn_blocking(move || crate::secret_store::remove_password(&email))
                .await.map_err(e)?.map_err(e)?;
        }
    }
    let mut store = state.store.write().await;
    store.saved_password = None;
    save_store(&state.app_dir, &store).map_err(e)
}

#[tauri::command]
pub async fn set_site_url(state: State<'_, SharedState>, url: String) -> CmdResult<()> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|_| "请输入完整的 http:// 或 https:// 站点地址")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err("请输入站点根地址，不要包含 /v1、账号或查询参数".into());
    }
    let host = parsed.as_str().trim_end_matches('/').to_string();
    let state = state.inner();
    let _guard = state.configuration_lock.lock().await;
    #[cfg(target_os = "macos")]
    {
        let previous = {
            let store = state.store.read().await;
            if store.hosts.first() != Some(&host) { store.last_email.clone() } else { None }
        };
        if let Some(email) = previous {
            tokio::task::spawn_blocking(move || crate::secret_store::remove_password(&email))
                .await.map_err(e)?.map_err(e)?;
        }
    }
    let mut store = state.store.write().await;
    if store.hosts.first() != Some(&host) {
        store.hosts = vec![host];
        store.group_keys.clear();
        store.refresh_token = None;
        store.saved_password = None;
        store.last_email = None;
        store.last_user_id = None;
        store.settings.preferred_group_id = None;
        store.settings.claude_model = None;
        store.settings.codex_model = None;
        store.settings.preferred_host = None;
        save_store(&state.app_dir, &store).map_err(e)?;
        *state.session.write().await = None;
        let mut proxy = state.proxy.write().await;
        proxy.groups.clear();
        proxy.preferred_host = None;
        proxy.selection_revision = proxy.selection_revision.wrapping_add(1);
    }
    Ok(())
}

#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> CmdResult<()> {
    let parsed = reqwest::Url::parse(&url).map_err(e)?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err("只能打开 http 或 https 地址".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(e)
}

#[tauri::command]
pub fn open_devtools(app: AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    let window = app.get_webview_window("main").ok_or("找不到主窗口")?;
    window.open_devtools();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::HeaderMap,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[tokio::test(flavor = "current_thread")]
    async fn stalled_system_probe_times_out_without_blocking_async_runtime() {
        let (release, waiting) = std::sync::mpsc::channel::<()>();
        let started = std::time::Instant::now();
        let result = bounded_blocking(std::time::Duration::from_millis(30), move || {
            // 模拟等待 Keychain 授权的系统调用，最长 2 秒防止失败时挂住测试。
            let _ = waiting.recv_timeout(std::time::Duration::from_secs(2));
        }).await;
        let elapsed = started.elapsed();
        let _ = release.send(());
        assert!(result.is_err());
        assert!(elapsed < std::time::Duration::from_secs(1), "系统探测阻塞了异步线程：{elapsed:?}");
        assert!(bounded_blocking(std::time::Duration::from_secs(1), || 42).await.is_ok());
    }

    #[test]
    fn model_choice_rejects_wrong_group_and_discards_stale_saved_model() {
        assert!(super::select_model(Some("old"), None, &["new"]).is_err());
        assert_eq!(
            super::select_model(None, Some("old"), &["new"]).unwrap(),
            "new"
        );
        assert_eq!(
            super::select_model(Some("chosen"), Some("default"), &["default", "chosen"]).unwrap(),
            "chosen"
        );
        assert!(super::select_model(None, None, &[]).is_err());
    }

    #[tokio::test]
    async fn group_manifest_and_probe_use_only_the_bound_key_and_selected_model() {
        let calls = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/api/v1/groups/available", get(|| async {
                Json(json!({"code":0,"data":[{"id":47,"name":"selected"}]}))
            }))
            .route("/api/v1/model-plaza", get(|| async {
                Json(json!({"code":0,"data":{"groups":[{"id":47,"name":"selected","platform":"composite","models":[
                    {"name":"claude-relay","platform":"anthropic"},{"name":"gpt-relay","platform":"openai"}
                ]}]}}))
            }))
            .route("/api/v1/keys", get(|| async {
                Json(json!({"code":0,"data":[{"id":1,"group_id":47,"name":"jokerdeck-client-g47","key":"bound-key","status":"active"}]}))
            }))
            .route("/backend-api/codex/models", get(|headers: HeaderMap| async move {
                assert_eq!(headers["authorization"], "Bearer bound-key");
                Json(json!({"models":[{"slug":"gpt-relay","visibility":"list"},{"slug":"hidden","visibility":"hide"}]}))
            }))
            .route("/v1/responses", post(|State(calls): State<Arc<AtomicUsize>>, headers: HeaderMap, Json(payload): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer bound-key");
                assert_eq!(payload["model"], "gpt-relay");
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"output":[]}))
            }))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let state = AppState {
            http: reqwest::Client::new(),
            app_dir: PathBuf::from("."),
            session: tokio::sync::RwLock::new(Some(Session {
                access_token: "session".into(),
                refresh_token: None,
                user: UserInfo::default(),
            })),
            store: tokio::sync::RwLock::new({
                let mut store = crate::state::Store::default();
                store.hosts = vec![host.clone()];
                store.group_keys.insert(47, "bound-key".into());
                store
            }),
            proxy: crate::proxy::shared_with(vec![host], false),
            proxy_runtime: tokio::sync::Mutex::new(None),
            configuration_lock: tokio::sync::Mutex::new(()),
        };
        let models = group_models(&state, 47).await.unwrap();
        assert_eq!(models.claude_models, ["claude-relay"]);
        assert_eq!(models.codex_models, ["gpt-relay"]);
        assert!(probe_group_model(&state, 47, "codex", "hidden")
            .await
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            probe_group_model(&state, 47, "codex", "gpt-relay")
                .await
                .unwrap()
                .ok
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
}

#[tauri::command]
pub async fn desktop_feature_status(
    state: State<'_, SharedState>,
) -> CmdResult<crate::desktop_features::FeatureStatus> {
    crate::desktop_features::status(state.inner())
        .await
        .map_err(e)
}
#[tauri::command]
pub async fn configure_desktop_features(
    state: State<'_, SharedState>,
    theme: String,
    overlay: bool,
    auto_sync: bool,
) -> CmdResult<crate::desktop_features::FeatureStatus> {
    let _guard = state.configuration_lock.lock().await;
    crate::desktop_features::configure(state.inner(), theme, overlay, auto_sync)
        .await
        .map_err(e)
}
#[tauri::command]
pub async fn provider_sync_action(
    state: State<'_, SharedState>,
    action: String,
    backup: Option<String>,
) -> CmdResult<serde_json::Value> {
    let _guard = state.configuration_lock.lock().await;
    crate::desktop_features::provider_action(state.inner(), &action, backup)
        .await
        .map_err(e)
}
