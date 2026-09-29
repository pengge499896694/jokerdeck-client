use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The release workflow injects the GitHub Release asset URL at build time.
/// Local builds keep the relay-hosted manifest as a backwards-compatible fallback.
pub const DEFAULT_MANIFEST: &str = match option_env!("JOKERDECK_UPDATE_MANIFEST_URL") {
    Some(url) => url,
    None => "https://jokerdeck.cc.cd/client/latest.json",
};

#[derive(Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub error: Option<String>,
}

pub async fn check(http: &reqwest::Client, manifest_url: &str) -> UpdateInfo {
    let mut info = UpdateInfo {
        current: CURRENT_VERSION.into(),
        latest: None,
        update_available: false,
        url: None,
        notes: None,
        error: None,
    };
    match http
        .get(manifest_url)
        .timeout(Duration::from_secs(12))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            match resp.text().await {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(body) => {
                        let v = body.get("data").unwrap_or(&body);
                        let latest = v
                            .get("version")
                            .or_else(|| v.get("latest_version"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        info.url = platform_url(v)
                            .or_else(|| {
                                v.get("url")
                                    .or_else(|| v.get("download_url"))
                                    .and_then(Value::as_str)
                            })
                            .map(str::to_string);
                        info.notes = v.get("notes").and_then(Value::as_str).map(str::to_string);
                        if latest.is_empty() {
                            info.error = Some("更新源未返回版本号".into());
                        } else {
                            info.update_available = is_newer(&latest, CURRENT_VERSION);
                            info.latest = Some(latest);
                        }
                    }
                    Err(_)
                        if text.trim_start().starts_with("<!doctype")
                            || text.trim_start().starts_with("<html") =>
                    {
                        info.error = Some("更新源暂未发布更新信息".into());
                    }
                    Err(err) => {
                        info.error = Some(format!("更新信息格式无效（HTTP {status}）：{err}"));
                    }
                },
                Err(err) => info.error = Some(format!("读取更新信息失败: {err}")),
            }
        }
        Err(e) => info.error = Some(format!("检查更新失败: {e}")),
    }
    info
}

fn platform_url(manifest: &Value) -> Option<&str> {
    let platforms = manifest.get("platforms")?.as_object()?;
    let keys = target_keys();
    keys.iter()
        .find_map(|key| platforms.get(*key).and_then(asset_url))
}

fn asset_url(value: &Value) -> Option<&str> {
    match value {
        Value::String(url) => Some(url.as_str()),
        Value::Object(map) => map
            .get("url")
            .or_else(|| map.get("download_url"))
            .and_then(Value::as_str),
        _ => None,
    }
}

fn target_keys() -> Vec<&'static str> {
    let mut keys = Vec::new();
    #[cfg(target_os = "windows")]
    {
        keys.extend(["windows-x86_64", "windows"]);
    }
    #[cfg(target_os = "macos")]
    {
        #[cfg(target_arch = "aarch64")]
        keys.extend(["darwin-aarch64", "macos-aarch64", "macos"]);
        #[cfg(target_arch = "x86_64")]
        keys.extend(["darwin-x86_64", "macos-x86_64", "macos"]);
    }
    #[cfg(target_os = "linux")]
    {
        keys.extend(["linux-x86_64", "linux"]);
    }
    keys
}

fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

fn parse(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .map(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}
