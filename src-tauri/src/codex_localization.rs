use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::{path::Path, time::Duration};

const VERSION: &str = "v0.1.2";
const ASSET: &str = "codex-zh-CN-v0.1.2.zip";
const RELEASE_API: &str = "https://api.github.com/repos/xqnode/codex-zh-CN/releases/tags/v0.1.2";
const MAX_ARCHIVE: usize = 12 * 1024 * 1024;
const LOCALIZATION_MANIFEST_PATH: &str = "/client/codex-localization/latest.json";
const LOCALIZATION_DOWNLOAD_PATH: &str = "/client/codex-localization/download";

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: usize,
}

pub async fn run(app_dir: &Path, http: &reqwest::Client, action: &str) -> Result<String> {
    if !matches!(action, "install" | "uninstall" | "launch") {
        bail!("未知的 Codex 汉化操作");
    }
    let root = app_dir.join(format!("codex-zh-CN-{VERSION}"));
    let launcher = find_file(&root, "launch-codex-zh-cn.ps1");
    if action == "launch" {
        let script = launcher.ok_or_else(|| anyhow!("请先安装 Codex 汉化包"))?;
        return execute(&script, &[]).await;
    }
    if action == "uninstall" {
        let script = find_file(&root, "scripts/install_windows.ps1")
            .ok_or_else(|| anyhow!("未找到本客户端安装的汉化包，无法恢复英文"))?;
        return execute(&script, &["-Action", "uninstall", "-NoPause"]).await;
    }

    // Always verify a fresh release before executing its scripts; never trust a stale cache.
    download_verified(http, &root).await?;
    let script = find_file(&root, "scripts/install_windows.ps1")
        .ok_or_else(|| anyhow!("汉化包缺少安装脚本"))?;
    execute(&script, &["-Action", action, "-NoPause"]).await
}

fn find_file(root: &Path, suffix: &str) -> Option<std::path::PathBuf> {
    let direct = root.join(suffix);
    if direct.is_file() {
        return Some(direct);
    }
    let nested = root.join(format!("codex-zh-CN-{VERSION}")).join(suffix);
    nested.is_file().then_some(nested)
}

async fn download_verified(http: &reqwest::Client, root: &Path) -> Result<()> {
    let (release, preferred_asset_url) = fetch_release(http).await?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == ASSET)
        .ok_or_else(|| anyhow!("发行版缺少预期的汉化包"))?;
    let digest = asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| anyhow!("发行包没有可校验的 SHA-256，已停止安装"))?;
    if asset.size == 0 || asset.size > MAX_ARCHIVE {
        bail!("汉化包大小异常");
    }
    let bytes = download_asset(http, asset, preferred_asset_url.as_deref()).await?;
    std::fs::create_dir_all(root)?;
    let archive = root.join(ASSET);
    std::fs::write(&archive, bytes)?;
    let result = verify_and_unpack(&archive, root, digest).await;
    let _ = std::fs::remove_file(&archive);
    result
}

async fn fetch_release(http: &reqwest::Client) -> Result<(Release, Option<String>)> {
    let mut last_error = None;
    for source in release_sources() {
        match http
            .get(&source)
            .header("User-Agent", "jokerdeck-desktop")
            .timeout(Duration::from_secs(15))
            .send()
            .await
        {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.json::<Release>().await {
                    Ok(release) => {
                        let asset_url = release
                            .assets
                            .iter()
                            .find(|asset| asset.name == ASSET)
                            .map(|asset| asset.browser_download_url.clone());
                        if asset_url.is_some() {
                            return Ok((release, asset_url));
                        }
                        last_error = Some(format!("{source}: 发行版缺少预期的汉化包"));
                    }
                    Err(error) => last_error = Some(format!("{source}: {error}")),
                },
                Err(error) => last_error = Some(format!("{source}: {error}")),
            },
            Err(error) => last_error = Some(format!("{source}: {error}")),
        }
    }
    Err(anyhow!(
        "无法获取汉化包版本信息{}",
        last_error
            .map(|error| format!("：{error}"))
            .unwrap_or_default()
    ))
}

fn release_sources() -> Vec<String> {
    let mut sources = crate::state::DEFAULT_HOSTS
        .iter()
        .map(|host| format!("{host}{LOCALIZATION_MANIFEST_PATH}"))
        .collect::<Vec<_>>();
    sources.push(RELEASE_API.to_string());
    sources
}

async fn download_asset(
    http: &reqwest::Client,
    asset: &Asset,
    preferred_url: Option<&str>,
) -> Result<bytes::Bytes> {
    let mut urls = relay_asset_urls();
    if let Some(url) = preferred_url.filter(|url| is_github_release_url(url)) {
        urls.push(url.to_string());
    }

    let mut last_error = None;
    for raw_url in urls {
        let url = match reqwest::Url::parse(&raw_url) {
            Ok(url) => url,
            Err(error) => {
                last_error = Some(format!("{raw_url}: {error}"));
                continue;
            }
        };
        match http
            .get(url)
            .header("User-Agent", "jokerdeck-desktop")
            .timeout(Duration::from_secs(120))
            .send()
            .await
        {
            Ok(response) => match response.error_for_status() {
                Ok(response) => {
                    if response
                        .content_length()
                        .is_some_and(|size| size > MAX_ARCHIVE as u64)
                    {
                        last_error = Some(format!("{raw_url}: 文件超过大小限制"));
                        continue;
                    }
                    match response.bytes().await {
                        Ok(bytes) if bytes.len() == asset.size && bytes.len() <= MAX_ARCHIVE => {
                            return Ok(bytes);
                        }
                        Ok(bytes) => {
                            last_error = Some(format!(
                                "{raw_url}: 下载大小异常（期望 {}，实际 {}）",
                                asset.size,
                                bytes.len()
                            ));
                        }
                        Err(error) => last_error = Some(format!("{raw_url}: {error}")),
                    }
                }
                Err(error) => last_error = Some(format!("{raw_url}: {error}")),
            },
            Err(error) => last_error = Some(format!("{raw_url}: {error}")),
        }
    }
    Err(anyhow!(
        "汉化包下载失败{}",
        last_error
            .map(|error| format!("：{error}"))
            .unwrap_or_default()
    ))
}

fn relay_asset_urls() -> Vec<String> {
    crate::state::DEFAULT_HOSTS
        .iter()
        .map(|host| format!("{host}{LOCALIZATION_DOWNLOAD_PATH}/{ASSET}"))
        .collect()
}

fn is_github_release_url(raw_url: &str) -> bool {
    reqwest::Url::parse(raw_url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.path().contains("/releases/download/")
    })
}

async fn verify_and_unpack(archive: &Path, root: &Path, digest: &str) -> Result<()> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$hash = (Get-FileHash -LiteralPath $env:CODEX_ZH_ARCHIVE -Algorithm SHA256).Hash
if ($hash -ne $env:CODEX_ZH_DIGEST) { throw '汉化包 SHA-256 校验失败' }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($env:CODEX_ZH_ARCHIVE)
try {
    if ($zip.Entries.Count -gt 150) { throw '汉化包文件数量异常' }
    $base = [System.IO.Path]::GetFullPath($env:CODEX_ZH_ROOT).TrimEnd('\') + '\'
    $total = [long]0
    foreach ($entry in $zip.Entries) {
        $total += $entry.Length
        if ($total -gt 33554432) { throw '汉化包解压大小异常' }
        $target = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($base, $entry.FullName))
        if (-not $target.StartsWith($base, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw '汉化包包含非法路径'
        }
        if ((($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
            throw '汉化包包含符号链接'
        }
    }
    foreach ($entry in $zip.Entries) {
        $target = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($base, $entry.FullName))
        if ($entry.FullName.EndsWith('/')) {
            [System.IO.Directory]::CreateDirectory($target) | Out-Null
        } else {
            [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($target)) | Out-Null
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $true)
        }
    }
} finally { $zip.Dispose() }
"#;
    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("CODEX_ZH_ARCHIVE", archive)
        .env("CODEX_ZH_ROOT", root)
        .env("CODEX_ZH_DIGEST", digest)
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(Duration::from_secs(60), command.output()).await??;
    if !output.status.success() {
        bail!(
            "汉化包校验或解压失败：{}",
            decode_powershell_output(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn execute(script: &Path, args: &[&str]) -> Result<String> {
    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .args(args)
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(Duration::from_secs(180), command.output())
        .await
        .context("汉化操作超时")??;
    if !output.status.success() {
        bail!(
            "汉化操作失败：{}",
            decode_powershell_output(&output.stderr).trim()
        );
    }
    Ok(match args.first().copied() {
        Some("-Action") if args.get(1) == Some(&"install") =>
            "安装已启动。若弹出 UAC，请授权；Codex 可能被关闭并重新启动。Store 版日常请使用下方“启动汉化版”。".into(),
        Some("-Action") =>
            "恢复操作已启动。若弹出 UAC，请授权；完成后重新打开 Codex。".into(),
        _ => "已启动 Codex 汉化版。".into(),
    })
}

fn decode_powershell_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::{GetACP, MultiByteToWideChar};
        let codepage = unsafe { GetACP() };
        let length = unsafe {
            MultiByteToWideChar(
                codepage,
                0,
                bytes.as_ptr(),
                bytes.len() as i32,
                std::ptr::null_mut(),
                0,
            )
        };
        if length > 0 {
            let mut wide = vec![0u16; length as usize];
            let written = unsafe {
                MultiByteToWideChar(
                    codepage,
                    0,
                    bytes.as_ptr(),
                    bytes.len() as i32,
                    wide.as_mut_ptr(),
                    length,
                )
            };
            if written > 0 {
                return String::from_utf16_lossy(&wide[..written as usize]);
            }
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_action_without_network_or_filesystem_changes() {
        let http = reqwest::Client::new();
        let result =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(run(Path::new("."), &http, "delete"));
        assert!(result.is_err());
    }

    #[test]
    fn only_accepts_github_release_urls_as_last_resort() {
        assert!(is_github_release_url(
            "https://github.com/xqnode/codex-zh-CN/releases/download/v0.1.2/codex-zh-CN-v0.1.2.zip"
        ));
        assert!(!is_github_release_url(
            "https://example.com/codex-zh-CN-v0.1.2.zip"
        ));
        assert!(!is_github_release_url(
            "http://github.com/xqnode/codex-zh-CN/releases/download/v0.1.2/codex-zh-CN-v0.1.2.zip"
        ));
    }

    #[test]
    fn relay_download_urls_are_generated_for_all_builtin_hosts() {
        let urls = relay_asset_urls();
        assert_eq!(urls.len(), crate::state::DEFAULT_HOSTS.len());
        assert!(urls.iter().all(|url| url.ends_with(ASSET)));
        assert!(urls.iter().all(|url| !url.contains("github.com")));
    }
}
