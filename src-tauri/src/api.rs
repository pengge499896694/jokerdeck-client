use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::state::UserInfo;

/// A model/rate group the user is allowed to use.
#[derive(Serialize, Clone)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub multiplier: Option<f64>,
}

#[derive(Serialize, Clone)]
pub struct ApiKey {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub group_id: Option<i64>,
    pub status: String,
}

pub enum LoginOutcome {
    Success {
        access_token: String,
        refresh_token: Option<String>,
        user: UserInfo,
    },
    Needs2fa {
        temp_token: String,
        user_email_masked: String,
    },
}

/// Unwrap the panel envelope `{code,message,data}`; code 0 == success.
fn unwrap_envelope(body: Value) -> Result<Value> {
    let code = body.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code == 0 {
        Ok(body.get("data").cloned().unwrap_or(Value::Null))
    } else {
        let msg = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("请求失败");
        Err(anyhow!("{}", msg))
    }
}

async fn post_json(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    payload: Value,
) -> Result<Value> {
    let mut req = http
        .post(url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(20));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let body: Value = resp.json().await?;
    unwrap_envelope(body)
}

async fn get_json(http: &reqwest::Client, url: &str, token: &str) -> Result<Value> {
    let resp = http
        .get(url)
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;
    let body: Value = resp.json().await?;
    unwrap_envelope(body)
}

async fn public_get_json(http: &reqwest::Client, url: &str) -> Result<Value> {
    let body: Value = http
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?
        .json()
        .await?;
    unwrap_envelope(body)
}

fn parse_user(v: &Value) -> UserInfo {
    UserInfo {
        id: v.get("id").and_then(Value::as_i64).unwrap_or(0),
        email: v
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        balance: v.get("balance").and_then(Value::as_f64).unwrap_or(0.0),
        frozen_balance: v
            .get("frozen_balance")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        total_recharged: v
            .get("total_recharged")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        allowed_groups: v
            .get("allowed_groups")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default(),
        role: v
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

pub async fn login(
    http: &reqwest::Client,
    host: &str,
    email: &str,
    password: &str,
) -> Result<LoginOutcome> {
    let url = format!("{host}/api/v1/auth/login");
    let data = post_json(
        http,
        &url,
        None,
        json!({ "email": email, "password": password }),
    )
    .await?;

    if data.get("requires_2fa").and_then(Value::as_bool) == Some(true) {
        return Ok(LoginOutcome::Needs2fa {
            temp_token: data
                .get("temp_token")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            user_email_masked: data
                .get("user_email_masked")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    parse_auth_response(&data)
}

pub async fn login_2fa(
    http: &reqwest::Client,
    host: &str,
    temp_token: &str,
    totp_code: &str,
) -> Result<LoginOutcome> {
    let url = format!("{host}/api/v1/auth/login/2fa");
    let data = post_json(
        http,
        &url,
        None,
        json!({ "temp_token": temp_token, "totp_code": totp_code }),
    )
    .await?;
    parse_auth_response(&data)
}

pub async fn public_settings(http: &reqwest::Client, host: &str) -> Result<Value> {
    public_get_json(http, &format!("{host}/api/v1/settings/public")).await
}

pub async fn register(
    http: &reqwest::Client,
    host: &str,
    email: &str,
    password: &str,
    verify_code: Option<&str>,
    promo_code: Option<&str>,
    invitation_code: Option<&str>,
    aff_code: Option<&str>,
) -> Result<LoginOutcome> {
    let mut payload = json!({ "email": email, "password": password });
    for (key, value) in [
        ("verify_code", verify_code),
        ("promo_code", promo_code),
        ("invitation_code", invitation_code),
        ("aff_code", aff_code),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            payload[key] = json!(value);
        }
    }
    let data = post_json(http, &format!("{host}/api/v1/auth/register"), None, payload).await?;
    parse_auth_response(&data)
}

pub async fn send_verify_code(http: &reqwest::Client, host: &str, email: &str) -> Result<Value> {
    post_json(
        http,
        &format!("{host}/api/v1/auth/send-verify-code"),
        None,
        json!({ "email": email }),
    )
    .await
}

pub async fn forgot_password(http: &reqwest::Client, host: &str, email: &str) -> Result<Value> {
    post_json(
        http,
        &format!("{host}/api/v1/auth/forgot-password"),
        None,
        json!({ "email": email }),
    )
    .await
}

pub async fn reset_password(
    http: &reqwest::Client,
    host: &str,
    email: &str,
    token: &str,
    new_password: &str,
) -> Result<Value> {
    post_json(
        http,
        &format!("{host}/api/v1/auth/reset-password"),
        None,
        json!({ "email": email, "token": token, "new_password": new_password }),
    )
    .await
}

fn parse_auth_response(data: &Value) -> Result<LoginOutcome> {
    let token = data
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("登录响应缺少 access_token"))?;
    let user = parse_user(data.get("user").unwrap_or(&Value::Null));
    if user.id <= 0 || user.email.is_empty() {
        return Err(anyhow!("登录响应缺少账户信息"));
    }
    Ok(LoginOutcome::Success {
        access_token: token.to_string(),
        refresh_token: data
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        user,
    })
}

pub async fn get_me(http: &reqwest::Client, host: &str, token: &str) -> Result<UserInfo> {
    let url = format!("{host}/api/v1/auth/me");
    let data = get_json(http, &url, token).await?;
    Ok(parse_user(&data))
}

/// Fetch allowed groups, best-effort merging rate multipliers when available.
pub async fn list_available_groups(
    http: &reqwest::Client,
    host: &str,
    token: &str,
) -> Result<Vec<Group>> {
    let url = format!("{host}/api/v1/groups/available");
    let data = get_json(http, &url, token).await?;
    let arr = match &data {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .get("groups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => vec![],
    };
    let mut groups: Vec<Group> = arr
        .iter()
        .filter_map(|g| {
            let id = g.get("id").and_then(Value::as_i64)?;
            let name = g
                .get("name")
                .or_else(|| g.get("display_name"))
                .and_then(Value::as_str)
                .unwrap_or("group")
                .to_string();
            let multiplier = g
                .get("rate_multiplier")
                .or_else(|| g.get("group_rate_multiplier"))
                .or_else(|| g.get("multiplier"))
                .and_then(Value::as_f64);
            Some(Group {
                id,
                name,
                multiplier,
            })
        })
        .collect();
    groups.sort_by(|a, b| {
        a.multiplier
            .unwrap_or(f64::MAX)
            .partial_cmp(&b.multiplier.unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(groups)
}

pub async fn list_keys(http: &reqwest::Client, host: &str, token: &str) -> Result<Vec<ApiKey>> {
    let mut keys = Vec::new();
    for page in 1..=100 {
        let data = get_json(
            http,
            &format!("{host}/api/v1/keys?page={page}&page_size=100&status=active"),
            token,
        )
        .await?;
        let arr = data
            .as_array()
            .or_else(|| data.get("items").and_then(Value::as_array))
            .ok_or_else(|| anyhow!("API Key 列表响应无效"))?;
        keys.extend(arr.iter().map(parse_key));
        let pages = data.get("pages").and_then(Value::as_u64).unwrap_or(1);
        if page >= pages {
            return Ok(keys);
        }
    }
    Err(anyhow!("API Key 分页数量过多，已停止配置"))
}

pub async fn create_key(
    http: &reqwest::Client,
    host: &str,
    token: &str,
    name: &str,
    group_id: Option<i64>,
) -> Result<ApiKey> {
    let url = format!("{host}/api/v1/keys");
    let mut payload = json!({ "name": name });
    if let Some(gid) = group_id {
        payload["group_id"] = json!(gid);
    }
    let data = post_json(http, &url, Some(token), payload).await?;
    let key = parse_key(&data);
    if key.key.is_empty() || key.group_id != group_id {
        return Err(anyhow!("创建的 API Key 未绑定所选分组，已停止配置"));
    }
    Ok(key)
}

pub async fn fetch_codex_catalog(http: &reqwest::Client, host: &str, key: &str) -> Result<Value> {
    // The relay resolves its canonical client version for this direct route.
    // A fixed historical client_version can hide newer group models.
    let response = http
        .get(format!(
            "{}/backend-api/codex/models",
            host.trim_end_matches('/')
        ))
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Codex 模型目录同步失败: HTTP {}",
            response.status().as_u16()
        ));
    }
    let catalog: Value = response.json().await?;
    if catalog
        .get("models")
        .and_then(Value::as_array)
        .filter(|m| !m.is_empty())
        .is_none()
    {
        return Err(anyhow!(
            "中转没有返回 Codex model manifest，请检查分组是否支持 Responses"
        ));
    }
    Ok(catalog)
}

fn parse_key(v: &Value) -> ApiKey {
    ApiKey {
        id: v.get("id").and_then(Value::as_i64).unwrap_or(0),
        key: v
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        group_id: v.get("group_id").and_then(Value::as_i64),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

// ---- 模型广场：一次拿到「分组 + 该分组下的模型」 ----

#[derive(Serialize, Clone)]
pub struct PlazaModel {
    pub name: String,
    /// `anthropic` / `openai` / ... — decides whether it suits Claude Code or Codex.
    pub platform: String,
}

#[derive(Serialize, Clone)]
pub struct PlazaGroup {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub description: String,
    /// The multiplier the user actually pays (user-specific rate wins).
    pub multiplier: Option<f64>,
    pub models: Vec<PlazaModel>,
}

/// `GET /api/v1/model-plaza` — groups the user can see, each with its models.
/// This is the same source the website's model plaza uses, so the client's
/// group/model pickers stay in sync with the relay.
pub async fn fetch_plaza(
    http: &reqwest::Client,
    host: &str,
    token: &str,
) -> Result<Vec<PlazaGroup>> {
    let url = format!("{host}/api/v1/model-plaza");
    let data = get_json(http, &url, token).await?;
    let arr = data
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut groups: Vec<PlazaGroup> = arr
        .iter()
        .filter_map(|g| {
            let id = g.get("id").and_then(Value::as_i64)?;
            let models: Vec<PlazaModel> = g
                .get("models")
                .and_then(Value::as_array)
                .map(|ms| {
                    ms.iter()
                        .filter_map(|m| {
                            let name = m.get("name").and_then(Value::as_str)?.to_string();
                            let platform = m
                                .get("platform")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            Some(PlazaModel { name, platform })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(PlazaGroup {
                id,
                name: g
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("group")
                    .to_string(),
                platform: g
                    .get("platform")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                description: g
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                // user_rate_multiplier is this user's effective rate; fall back to the group's.
                multiplier: g
                    .get("user_rate_multiplier")
                    .and_then(Value::as_f64)
                    .or_else(|| g.get("rate_multiplier").and_then(Value::as_f64)),
                models,
            })
        })
        .collect();
    groups.sort_by(|a, b| {
        a.multiplier
            .unwrap_or(f64::MAX)
            .partial_cmp(&b.multiplier.unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(groups)
}
