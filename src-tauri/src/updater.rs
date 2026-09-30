use std::{path::Path, time::Duration};

use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELAY_MANIFEST: &str = "https://sub2api.186-244-245-198.sslip.io/client/latest.json";
pub const RELAY_MANIFEST_FALLBACK: &str = "https://jokerdeck.cc.cd/client/latest.json";
pub const GITHUB_MANIFEST: &str =
    "https://github.com/pengge499896694/jokerdeck-client/releases/latest/download/latest.json";
/// The release workflow may inject a preferred manifest URL at build time.
/// Runtime fallbacks remain available when that source is blocked.
pub const DEFAULT_MANIFEST: &str = match option_env!("JOKERDECK_UPDATE_MANIFEST_URL") {
    Some(url) => url,
    None => RELAY_MANIFEST,
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

#[derive(Serialize)]
pub struct InstallUpdateResult {
    pub ok: bool,
    pub message: String,
}

pub async fn check(http: &reqwest::Client, manifest_url: &str) -> UpdateInfo {
    let mut last_error = None;
    for source in manifest_sources(manifest_url) {
        match check_source(http, source).await {
            Ok(info) => return info,
            Err(error) => last_error = Some(format!("{source}: {error}")),
        }
    }
    UpdateInfo {
        current: CURRENT_VERSION.into(),
        latest: None,
        update_available: false,
        url: None,
        notes: None,
        error: last_error.map(|error| format!("所有更新源均不可用：{error}")),
    }
}

pub async fn download_and_install(
    http: &reqwest::Client,
    app_dir: &Path,
    raw_url: &str,
) -> Result<InstallUpdateResult, String> {
    let url = reqwest::Url::parse(raw_url).map_err(|_| "更新地址无效".to_string())?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("sub2api.186-244-245-198.sslip.io") | Some("jokerdeck.cc.cd") | Some("github.com")
        )
    {
        return Err("更新地址不是受信任的中转或官方发布地址".into());
    }
    let filename = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty() && name.len() <= 128)
        .ok_or_else(|| "更新地址缺少安装包文件名".to_string())?
        .to_string();
    let extension = Path::new(&filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    #[cfg(windows)]
    if extension != "exe" {
        return Err("Windows 更新包格式错误".into());
    }
    #[cfg(target_os = "macos")]
    if extension != "dmg" {
        return Err("macOS 更新包格式错误".into());
    }
    let max_size: u64 = 300 * 1024 * 1024;
    let response = http
        .get(url)
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .map_err(|error| format!("下载更新失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("更新服务器返回错误：{error}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > max_size)
    {
        return Err("更新包超过 300 MB 限制".into());
    }
    let directory = app_dir.join("updates");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("创建更新目录失败：{error}"))?;
    let target = directory.join(&filename);
    let temporary = target.with_extension(format!("{extension}.download"));
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|error| format!("创建临时更新文件失败：{error}"))?;
    let mut stream = response.bytes_stream();
    let mut total = 0u64;
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|error| format!("下载更新中断：{error}"))?;
        total = total.saturating_add(chunk.len() as u64);
        if total > max_size {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err("更新包超过 300 MB 限制".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入更新包失败：{error}"))?;
    }
    file.flush()
        .await
        .map_err(|error| format!("保存更新包失败：{error}"))?;
    drop(file);
    tokio::fs::rename(&temporary, &target)
        .await
        .map_err(|error| format!("准备安装更新失败：{error}"))?;
    launch_installer(&target).map_err(|error| {
        let _ = std::fs::remove_file(&target);
        error
    })?;
    Ok(InstallUpdateResult {
        ok: true,
        message: format!("更新包已下载，正在启动安装器：{filename}"),
    })
}

fn launch_installer(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new(path)
            .spawn()
            .map_err(|error| format!("启动 Windows 安装器失败：{error}"))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("打开 macOS 安装包失败：{error}"))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("当前平台暂不支持自动安装更新".into())
}

fn manifest_sources(primary: &str) -> Vec<&str> {
    let mut sources = Vec::new();
    for source in [
        primary,
        RELAY_MANIFEST,
        RELAY_MANIFEST_FALLBACK,
        GITHUB_MANIFEST,
    ] {
        if !source.trim().is_empty() && !sources.contains(&source) {
            sources.push(source);
        }
    }
    sources
}

async fn check_source(http: &reqwest::Client, source: &str) -> Result<UpdateInfo, String> {
    let response = http
        .get(source)
        .timeout(Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| format!("连接失败：{error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取失败：{error}"))?;
    let body = serde_json::from_str::<Value>(&text).map_err(|error| {
        if text.trim_start().starts_with("<!doctype") || text.trim_start().starts_with("<html") {
            "服务端返回了网页而不是更新清单".to_string()
        } else {
            format!("更新信息格式无效（HTTP {status}）：{error}")
        }
    })?;
    let body = body.get("data").unwrap_or(&body);
    let latest = body
        .get("version")
        .or_else(|| body.get("latest_version"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if latest.is_empty() {
        return Err("更新清单未返回版本号".into());
    }
    let url = platform_url(body)
        .or_else(|| {
            body.get("url")
                .or_else(|| body.get("download_url"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .map(|url| mirror_asset_url(source, &url));
    Ok(UpdateInfo {
        current: CURRENT_VERSION.into(),
        latest: Some(latest.clone()),
        update_available: is_newer(&latest, CURRENT_VERSION),
        url,
        notes: body
            .get("notes")
            .and_then(Value::as_str)
            .map(str::to_string),
        error: None,
    })
}

fn platform_url(manifest: &Value) -> Option<&str> {
    let platforms = manifest.get("platforms")?.as_object()?;
    let keys = target_keys();
    keys.iter()
        .find_map(|key| platforms.get(*key).and_then(asset_url))
}

fn mirror_asset_url(source: &str, asset: &str) -> String {
    let Ok(source_url) = reqwest::Url::parse(source) else {
        return asset.to_string();
    };
    let Ok(asset_url) = reqwest::Url::parse(asset) else {
        return asset.to_string();
    };
    if source_url.path() == "/client/latest.json"
        && asset_url.host_str() == Some("github.com")
        && asset_url.path().contains("/releases/download/")
    {
        if let Some(filename) = asset_url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
        {
            return format!(
                "{}://{}/client/download/{}",
                source_url.scheme(),
                source_url.host_str().unwrap_or_default(),
                filename
            );
        }
    }
    asset.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_sources_are_unique_and_keep_relay_before_github() {
        let sources = manifest_sources(RELAY_MANIFEST);
        assert_eq!(sources.first(), Some(&RELAY_MANIFEST));
        assert_eq!(sources.last(), Some(&GITHUB_MANIFEST));
        assert_eq!(sources.len(), 3);
    }

    #[test]
    fn relay_manifest_rewrites_github_release_asset_to_relay_download() {
        let source = "https://sub2api.example/client/latest.json";
        let asset = "https://github.com/pengge499896694/jokerdeck-client/releases/download/v0.1.4/jokerdeck_v0.1.4_windows-x86_64.exe";
        assert_eq!(
            mirror_asset_url(source, asset),
            "https://sub2api.example/client/download/jokerdeck_v0.1.4_windows-x86_64.exe"
        );
    }

    #[test]
    fn direct_github_manifest_keeps_github_asset_url() {
        let source = GITHUB_MANIFEST;
        let asset =
            "https://github.com/pengge499896694/jokerdeck-client/releases/download/v0.1.4/app.exe";
        assert_eq!(mirror_asset_url(source, asset), asset);
    }
}
