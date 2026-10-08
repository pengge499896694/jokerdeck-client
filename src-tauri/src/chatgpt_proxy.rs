//! Isolated outbound proxy for the desktop app. It never changes system proxy settings.
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Child;

const VERSION: &str = "v1.19.32";
const MAX_SUBSCRIPTION: usize = 4 * 1024 * 1024;
const MAX_ARCHIVE: usize = 80 * 1024 * 1024;

#[derive(Serialize)]
pub struct Status {
    pub configured: bool,
    pub running: bool,
    pub port: Option<u16>,
}

pub struct Runtime {
    pub port: u16,
    child: Child,
    config: PathBuf,
}

impl Runtime {
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.config);
    }
}

pub fn validate_subscription_url(raw: &str) -> Result<()> {
    let url = reqwest::Url::parse(raw.trim()).context("订阅链接格式无效")?;
    if url.scheme() != "https"
        || url.host().is_none()
        || url.username() != ""
        || url.password().is_some()
    {
        bail!("订阅链接必须是 HTTPS，且不得在地址中包含用户名或密码");
    }
    Ok(())
}

fn profile(raw: &str, port: u16) -> Result<String> {
    let parsed: Value = serde_yaml::from_str(raw).context("订阅不是有效的 Clash YAML")?;
    let nodes = parsed
        .get("proxies")
        .and_then(Value::as_sequence)
        .ok_or_else(|| anyhow!("订阅未包含 Clash 节点列表"))?;
    let mut names = Vec::new();
    let mut proxies = Vec::new();
    for node in nodes {
        let Some(map) = node.as_mapping() else {
            continue;
        };
        let Some(name) = map
            .get(Value::String("name".into()))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(kind) = map
            .get(Value::String("type".into()))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !matches!(
            kind,
            "ss" | "vmess" | "vless" | "trojan" | "hysteria2" | "tuic"
        ) || name.is_empty()
            || names.iter().any(|n| n == name)
        {
            continue;
        }
        names.push(name.to_owned());
        proxies.push(node.clone());
    }
    if names.is_empty() {
        bail!("订阅中没有可用的代理节点");
    }
    let mut group = Mapping::new();
    group.insert("name".into(), "chatgpt-auto".into());
    group.insert("type".into(), "url-test".into());
    group.insert("url".into(), "https://chatgpt.com/favicon.ico".into());
    group.insert("interval".into(), Value::Number(180.into()));
    group.insert("lazy".into(), Value::Bool(false));
    group.insert("proxies".into(), serde_yaml::to_value(names)?);
    let mut config = Mapping::new();
    config.insert("mixed-port".into(), Value::Number(port.into()));
    config.insert("bind-address".into(), "127.0.0.1".into());
    config.insert("allow-lan".into(), Value::Bool(false));
    config.insert("mode".into(), "rule".into());
    config.insert("log-level".into(), "warning".into());
    config.insert("proxies".into(), Value::Sequence(proxies));
    config.insert(
        "proxy-groups".into(),
        Value::Sequence(vec![Value::Mapping(group)]),
    );
    config.insert(
        "rules".into(),
        Value::Sequence(vec!["MATCH,chatgpt-auto".into()]),
    );
    Ok(serde_yaml::to_string(&config)?)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn asset() -> (&'static str, &'static str) {
    (
        "mihomo-windows-amd64-compatible-v1.19.32.zip",
        "974a4d7ad69aed27aa2e8f91d61113573c14dadb14562c63e58effabf59816f0",
    )
}
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn asset() -> (&'static str, &'static str) {
    (
        "mihomo-darwin-arm64-v1.19.32.gz",
        "3312a6780652c622890fd4357c6a853bbf865464fd047ac7b7f52dab8de18652",
    )
}
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn asset() -> (&'static str, &'static str) {
    (
        "mihomo-darwin-amd64-compatible-v1.19.32.gz",
        "18b382df77bded2ad0fb3db27db5636cb15b20729d5ba995a357eeb9b46bf507",
    )
}
#[cfg(not(any(
    all(windows, target_arch = "x86_64"),
    all(
        target_os = "macos",
        any(target_arch = "aarch64", target_arch = "x86_64")
    )
)))]
fn asset() -> (&'static str, &'static str) {
    ("", "")
}

async fn executable(http: &reqwest::Client, dir: &Path, bundled: Option<&Path>) -> Result<PathBuf> {
    if let Some(bundled) = bundled.filter(|path| path.is_file()) {
        return Ok(bundled.to_path_buf());
    }
    let (name, expected) = asset();
    if name.is_empty() {
        bail!("当前系统不支持 ChatGPT 专用代理核心");
    }
    let path = dir.join(if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    });
    if path.is_file() {
        return Ok(path);
    }
    let url = format!("https://github.com/MetaCubeX/mihomo/releases/download/{VERSION}/{name}");
    let response = http
        .get(url)
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .context("无法下载代理核心，请检查 GitHub 连通性")?
        .error_for_status()
        .context("代理核心下载失败")?;
    if response
        .content_length()
        .is_some_and(|size| size as usize > MAX_ARCHIVE)
    {
        bail!("代理核心压缩包超过安全大小");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_ARCHIVE || format!("{:x}", Sha256::digest(&bytes)) != expected {
        bail!("代理核心 SHA-256 校验失败");
    }
    let data = if name.ends_with(".gz") {
        let decoder = flate2::read::GzDecoder::new(bytes.as_ref());
        let mut buffer = Vec::new();
        decoder
            .take(MAX_ARCHIVE as u64 + 1)
            .read_to_end(&mut buffer)?;
        buffer
    } else {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        let entry = (0..archive.len())
            .find_map(|index| {
                let file = archive.by_index(index).ok()?;
                file.name().ends_with(".exe").then_some(index)
            })
            .ok_or_else(|| anyhow!("代理核心压缩包缺少可执行文件"))?;
        let mut buffer = Vec::new();
        archive
            .by_index(entry)?
            .take(MAX_ARCHIVE as u64 + 1)
            .read_to_end(&mut buffer)?;
        buffer
    };
    if data.is_empty() || data.len() > MAX_ARCHIVE {
        bail!("代理核心文件大小异常");
    }
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join("mihomo.download");
    std::fs::write(&temporary, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::rename(temporary, &path)?;
    Ok(path)
}

pub async fn start(app_dir: &Path, subscription: &str, bundled: Option<&Path>) -> Result<Runtime> {
    validate_subscription_url(subscription)?;
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    let response = http
        .get(subscription)
        .header("User-Agent", "clash")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| anyhow!("订阅下载失败，请检查网络连接"))?
        .error_for_status()
        .map_err(|response| {
            anyhow!(
                "订阅服务返回 HTTP {}",
                response.status().unwrap_or_default()
            )
        })?;
    if response
        .content_length()
        .is_some_and(|size| size as usize > MAX_SUBSCRIPTION)
    {
        bail!("订阅文件超过安全大小");
    }
    let content = response
        .bytes()
        .await
        .map_err(|_| anyhow!("订阅下载中断，请稍后重试"))?;
    if content.len() > MAX_SUBSCRIPTION {
        bail!("订阅文件超过安全大小");
    }
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.port()
    };
    let yaml = profile(std::str::from_utf8(&content)?, port)?;
    let dir = app_dir.join("chatgpt-outbound");
    let binary = executable(&http, &dir, bundled).await?;
    // Clean up profiles left by a previous crash without touching other files.
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("session-")
            && name.to_string_lossy().ends_with(".yaml")
            && entry.file_type()?.is_file()
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let config = dir.join(format!("session-{}.yaml", uuid::Uuid::new_v4()));
    std::fs::write(&config, yaml)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600))?;
    }
    let child = tokio::process::Command::new(binary)
        .arg("-f")
        .arg(&config)
        .arg("-d")
        .arg(&dir)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("无法启动 ChatGPT 专用代理")?;
    let mut runtime = Runtime {
        port,
        child,
        config,
    };
    let probe = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut last_error = String::from("尚未建立连接");
    for _ in 0..10 {
        if !runtime.is_running() {
            bail!("ChatGPT 专用代理意外退出");
        }
        match probe.get("https://chatgpt.com/favicon.ico").send().await {
            Ok(_) => return Ok(runtime),
            Err(error) => {
                last_error = if error.is_connect() {
                    "代理或节点连接失败".into()
                } else if error.is_timeout() {
                    "节点连接官方服务超时".into()
                } else {
                    format!("网络错误：{error}")
                };
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!("ChatGPT 专用代理未能连接官方服务：{last_error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_supported_nodes_are_forwarded() {
        let config = profile("proxies:\n  - {name: usable, type: ss, server: example.com, port: 443}\n  - {name: local, type: direct}\n", 44001).unwrap();
        assert!(config.contains("usable"));
        assert!(!config.contains("name: local"));
        assert!(config.contains("127.0.0.1"));
    }
    #[test]
    fn rejects_insecure_subscriptions() {
        assert!(validate_subscription_url("http://example.com/secret").is_err());
        assert!(validate_subscription_url("https://example.com/sub").is_ok());
    }
    #[tokio::test]
    #[ignore = "requires a real subscription and external network access"]
    async fn live_subscription_starts_proxy() {
        let url = std::env::var("JOKERDECK_TEST_SUBSCRIPTION").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = start(dir.path(), &url, None).await.unwrap();
        assert!(runtime.is_running());
    }
}
