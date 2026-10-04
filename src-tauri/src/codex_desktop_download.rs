#[cfg(target_os = "macos")]
use std::path::Path;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::Duration;

use serde::Serialize;
#[cfg(target_os = "macos")]
use tokio::io::AsyncWriteExt;

use crate::commands::InstallResult;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MAC_URL: &str = "https://persistent.oaistatic.com/codex-app-prod/Codex.dmg";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const MAC_URL: &str = "https://persistent.oaistatic.com/codex-app-prod/Codex-latest-x64.dmg";
#[cfg(target_os = "macos")]
const MAX_SIZE: u64 = 1_200 * 1024 * 1024;

#[derive(Debug, Serialize, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: u8,
    pub detail: String,
}

#[cfg(target_os = "windows")]
pub async fn download(
    _http: &reqwest::Client,
    _app_dir: &std::path::Path,
    report: &impl Fn(DownloadProgress),
) -> Result<InstallResult, String> {
    install_windows(report).await
}

#[cfg(target_os = "macos")]
pub async fn download(
    _http: &reqwest::Client,
    app_dir: &Path,
    report: &impl Fn(DownloadProgress),
) -> Result<InstallResult, String> {
    // The relay client disables SNI for its own domains; the official CDN
    // requires standard TLS, so downloads must use an independent client.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建官方下载连接失败：{error}"))?;
    download_macos(&http, app_dir, report).await
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub async fn download(
    _http: &reqwest::Client,
    _app_dir: &std::path::Path,
    _report: &impl Fn(DownloadProgress),
) -> Result<InstallResult, String> {
    Err("当前平台暂不支持安装 Codex Desktop".into())
}

#[cfg(target_os = "macos")]
async fn download_macos(
    http: &reqwest::Client,
    app_dir: &Path,
    report: &impl Fn(DownloadProgress),
) -> Result<InstallResult, String> {
    report(DownloadProgress {
        downloaded: 0,
        total: None,
        percent: 5,
        detail: "正在连接 Codex Desktop 官方下载源".into(),
    });
    let response = http
        .get(MAC_URL)
        .timeout(Duration::from_secs(1800))
        .send()
        .await
        .map_err(|error| {
            format!(
                "连接 Codex Desktop 下载源失败：{error}。可在浏览器直接下载官方安装包：{MAC_URL}"
            )
        })?
        .error_for_status()
        .map_err(|error| format!("Codex Desktop 下载源返回错误：{error}"))?;
    let total_size = response.content_length();
    if total_size.is_some_and(|size| size > MAX_SIZE) {
        return Err("Codex Desktop 安装包超过 1.2 GB 限制".into());
    }
    let directory = app_dir.join("updates");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("创建下载目录失败：{error}"))?;
    let filename = if cfg!(target_arch = "aarch64") {
        "Codex-arm64.dmg"
    } else {
        "Codex-x64.dmg"
    };
    let target = directory.join(filename);
    let temporary = directory.join(format!("{filename}.download"));
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|error| format!("创建临时文件失败：{error}"))?;
    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    let mut last_percent = 0;
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|error| format!("下载 Codex Desktop 中断：{error}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_SIZE {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err("Codex Desktop 安装包超过 1.2 GB 限制".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入安装包失败：{error}"))?;
        let percent = total_size
            .map(|total| 8 + ((downloaded.saturating_mul(82) / total.max(1)) as u8).min(82))
            .unwrap_or(8);
        if percent != last_percent {
            last_percent = percent;
            report(DownloadProgress {
                downloaded,
                total: total_size,
                percent,
                detail: format!(
                    "正在下载 Codex Desktop（{} / {} MB）",
                    downloaded / 1024 / 1024,
                    total_size
                        .map(|size| (size / 1024 / 1024).to_string())
                        .unwrap_or_else(|| "?".into())
                ),
            });
        }
    }
    if downloaded == 0 || total_size.is_some_and(|size| downloaded != size) {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err("Codex Desktop 安装包不完整，请重试".into());
    }
    file.flush()
        .await
        .map_err(|error| format!("保存安装包失败：{error}"))?;
    drop(file);
    tokio::fs::rename(&temporary, &target)
        .await
        .map_err(|error| format!("准备安装包失败：{error}"))?;
    report(DownloadProgress {
        downloaded,
        total: total_size,
        percent: 92,
        detail: "下载完成，正在打开 macOS 安装器".into(),
    });
    tokio::process::Command::new("open")
        .arg(&target)
        .spawn()
        .map_err(|error| format!("打开 macOS 安装器失败：{error}"))?;
    report(DownloadProgress {
        downloaded,
        total: total_size,
        percent: 100,
        detail: "Codex Desktop 安装器已打开".into(),
    });
    Ok(InstallResult {
        ok: true,
        log: "Codex Desktop 安装包已下载，安装器已打开。".into(),
    })
}

#[cfg(target_os = "windows")]
async fn install_windows(report: &impl Fn(DownloadProgress)) -> Result<InstallResult, String> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    report(DownloadProgress {
        downloaded: 0,
        total: None,
        percent: 8,
        detail: "正在准备 Microsoft Store 官方安装源".into(),
    });
    let mut command = tokio::process::Command::new("winget");
    command.args([
        "install",
        "--id",
        "9PLM9XGG6VKS",
        "--exact",
        "--source",
        "msstore",
        "--accept-source-agreements",
        "--accept-package-agreements",
        "--silent",
        "--disable-interactivity",
    ]);
    command.creation_flags(0x08000000);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 winget：{error}"))?;
    let mut stdout = child.stdout.take().ok_or("无法读取 winget 安装进度")?;
    let mut stderr = child.stderr.take().ok_or("无法读取 winget 错误详情")?;
    let stderr_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let mut buffer = [0u8; 4096];
    let mut details = String::new();
    let mut last_percent = 8u8;
    let download = async {
        loop {
            let size = stdout
                .read(&mut buffer)
                .await
                .map_err(|error| format!("读取 winget 进度失败：{error}"))?;
            if size == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buffer[..size]);
            details.push_str(&chunk);
            if details.len() > 16_384 {
                details.drain(..details.len() - 8_192);
            }
            for segment in chunk.split(['\r', '\n']) {
                if let Some(percent) = segment.split_whitespace().find_map(|part| {
                    part.strip_suffix('%')
                        .and_then(|value| value.parse::<u8>().ok())
                }) {
                    let mapped = 8 + (u16::from(percent.min(100)) * 75 / 100) as u8;
                    if mapped > last_percent {
                        last_percent = mapped;
                        report(DownloadProgress {
                            downloaded: 0,
                            total: None,
                            percent: mapped,
                            detail: format!("Microsoft Store 正在下载安装：{percent}%"),
                        });
                    }
                }
            }
        }
        child
            .wait()
            .await
            .map_err(|error| format!("等待 winget 结束失败：{error}"))
    };
    let status = tokio::time::timeout(Duration::from_secs(1800), download)
        .await
        .map_err(|_| "Codex Desktop 安装超时，请检查 Microsoft Store 状态".to_string())??;
    report(DownloadProgress {
        downloaded: 0,
        total: None,
        percent: 85,
        detail: "Microsoft Store 安装任务已完成，正在重新检测".into(),
    });
    if !status.success() {
        let stderr = stderr_reader
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let error = String::from_utf8_lossy(&stderr);
        return Err(format!(
            "Codex Desktop 安装失败（退出码 {:?}）：{} {}",
            status.code(),
            details.trim(),
            error.trim()
        ));
    }
    report(DownloadProgress {
        downloaded: 0,
        total: None,
        percent: 100,
        detail: "Codex Desktop 安装完成".into(),
    });
    Ok(InstallResult {
        ok: true,
        log: "Codex Desktop 已通过 Microsoft Store 安装。".into(),
    })
}
