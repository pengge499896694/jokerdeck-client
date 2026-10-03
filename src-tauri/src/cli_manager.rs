use serde::Serialize;

#[derive(Serialize, Default)]
pub struct CliStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct CliReport {
    pub node: CliStatus,
    pub npm: CliStatus,
    pub claude: CliStatus,
    pub codex: CliStatus,
    pub codex_desktop: CliStatus,
}

/// Windows: don't flash a console window when we shell out to node/npm/winget.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Run a shell command cross-platform, returning (success, combined output).
/// Spawned without a visible console window on Windows.
async fn sh(command: &str) -> (bool, String) {
    sh_with_timeout(command, 600).await
}

async fn sh_with_timeout(command: &str, seconds: u64) -> (bool, String) {
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd", vec!["/C", command])
    } else {
        ("sh", vec!["-c", command])
    };
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    match tokio::time::timeout(std::time::Duration::from_secs(seconds), cmd.output()).await {
        Ok(Ok(out)) => {
            let mut s = String::from_utf8_lossy(&out.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            (out.status.success(), s.trim().to_string())
        }
        Ok(Err(e)) => (false, format!("执行失败: {e}")),
        Err(_) => (false, "执行超时，请检查网络后重试".into()),
    }
}

async fn which(bin: &str) -> Option<String> {
    let cmd = if cfg!(windows) {
        format!("where {bin}")
    } else {
        format!("command -v {bin}")
    };
    let (ok, out) = sh(&cmd).await;
    if ok {
        out.lines().next().map(|s| s.trim().to_string())
    } else {
        None
    }
}

async fn detect(bin: &str, version_arg: &str) -> CliStatus {
    let (ok, out) = sh(&format!("{bin} {version_arg}")).await;
    if ok {
        CliStatus {
            installed: true,
            version: out.lines().next().map(|s| s.trim().to_string()),
            path: which(bin).await,
        }
    } else {
        CliStatus::default()
    }
}

fn macos_brew_candidates() -> &'static [&'static str] {
    &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
}

async fn macos_brew() -> Option<String> {
    if let Ok(path) = std::env::var("HOMEBREW_PREFIX") {
        let candidate = std::path::Path::new(&path).join("bin/brew");
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    for candidate in macos_brew_candidates() {
        if std::path::Path::new(candidate).is_file() {
            return Some((*candidate).to_owned());
        }
    }
    let (ok, output) = sh("/usr/bin/which brew").await;
    ok.then(|| output.lines().next().unwrap_or_default().trim().to_owned())
        .filter(|path| !path.is_empty())
}

async fn install_macos_brew() -> (bool, String) {
    // Both endpoints serve Homebrew/install; keep the installer on disk until
    // the download succeeds so a reset connection cannot execute partial data.
    sh_with_timeout(
        r#"tmp="$(mktemp)" || exit 1
trap 'rm -f "$tmp"' EXIT
downloaded=0
for url in \
  https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh \
  https://cdn.jsdelivr.net/gh/Homebrew/install@HEAD/install.sh \
  https://gcore.jsdelivr.net/gh/Homebrew/install@HEAD/install.sh; do
  if curl -fLsS --retry 2 --retry-delay 2 --connect-timeout 15 --max-time 90 "$url" -o "$tmp" && \
     /usr/bin/grep -q 'Homebrew' "$tmp"; then
    downloaded=1
    break
  fi
  printf '下载源不可用：%s\n' "$url"
done
if [ "$downloaded" -ne 1 ]; then
  printf 'Homebrew 官方安装脚本下载失败，请检查网络后重试。\n'
  exit 1
fi
NONINTERACTIVE=1 /bin/bash "$tmp""#,
        1800,
    )
    .await
}

pub async fn detect_all() -> CliReport {
    let (node, npm, claude, codex, codex_desktop) = tokio::join!(
        detect("node", "-v"),
        detect("npm", "-v"),
        detect("claude", "--version"),
        detect("codex", "--version"),
        detect_codex_desktop(),
    );
    CliReport {
        node,
        npm,
        claude,
        codex,
        codex_desktop,
    }
}

async fn detect_codex_desktop() -> CliStatus {
    #[cfg(windows)]
    {
        let (ok, out) = sh("powershell -NoProfile -NonInteractive -Command \"Get-AppxPackage *Codex* | Where-Object { $_.Name -notmatch 'Codex\\+\\+' } | Sort-Object Version -Descending | Select-Object -First 1 | ForEach-Object { $_.InstallLocation }\"").await;
        if ok && !out.is_empty() {
            return CliStatus {
                installed: true,
                version: None,
                path: Some(out),
            };
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            for relative in [
                "Programs/Codex/Codex.exe",
                "Codex/Codex.exe",
                "Programs/Codex/ChatGPT.exe",
                "Codex/ChatGPT.exe",
            ] {
                let path = std::path::PathBuf::from(&local).join(relative);
                if path.is_file() {
                    return CliStatus {
                        installed: true,
                        version: None,
                        path: Some(path.display().to_string()),
                    };
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for path in ["/Applications/Codex.app", "/System/Applications/Codex.app"] {
            if std::path::Path::new(path).exists() {
                return CliStatus {
                    installed: true,
                    version: None,
                    path: Some(path.into()),
                };
            }
        }
    }
    CliStatus::default()
}

/// Install a CLI. `claude`/`codex` go through npm; Node uses the native
/// package manager for the current desktop platform.
pub async fn install(which_cli: &str) -> (bool, String) {
    match which_cli {
        "claude" => sh("npm i -g @anthropic-ai/claude-code").await,
        // Codex CLI (the "ChatGPT/Codex" coding CLI that can be pointed at a relay).
        "codex" => sh("npm i -g @openai/codex").await,
        "node" => {
            if cfg!(windows) {
                sh("winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements").await
            } else if cfg!(target_os = "macos") {
                let mut details = String::from("正在检测 Homebrew…");
                let brew = if let Some(path) = macos_brew().await {
                    details.push_str(&format!("\n已找到 Homebrew：{path}"));
                    path
                } else {
                    details.push_str("\n未找到 Homebrew，正在自动安装…");
                    let (ok, output) = install_macos_brew().await;
                    if !output.is_empty() {
                        details.push('\n');
                        details.push_str(&output);
                    }
                    if !ok {
                        return (false, format!("Homebrew 安装失败：\n{details}"));
                    }
                    let Some(path) = macos_brew().await else {
                        return (
                            false,
                            format!("Homebrew 安装命令已完成，但仍未找到 brew：\n{details}"),
                        );
                    };
                    details.push_str(&format!("\nHomebrew 安装完成：{path}"));
                    path
                };
                details.push_str("\n正在通过 Homebrew 安装 Node.js…");
                let (ok, output) = sh_with_timeout(
                    &format!("\"{}\" install node", brew.replace('"', "\\\"")),
                    1800,
                )
                .await;
                if !output.is_empty() {
                    details.push('\n');
                    details.push_str(&output);
                }
                return (ok, details);
            } else {
                (false, "当前平台暂不支持自动安装 Node.js".into())
            }
        }
        other => (false, format!("未知的安装目标: {other}")),
    }
}
