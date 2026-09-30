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
    match tokio::time::timeout(std::time::Duration::from_secs(600), cmd.output()).await {
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
        let (ok, out) = sh("powershell -NoProfile -NonInteractive -Command \"Get-AppxPackage *Codex* | Select-Object -First 1 | ForEach-Object { $_.InstallLocation }\"").await;
        if ok && !out.is_empty() {
            return CliStatus {
                installed: true,
                version: None,
                path: Some(out),
            };
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            for relative in ["Programs/Codex/Codex.exe", "Codex/Codex.exe"] {
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
                let (brew_ok, brew) = sh("command -v brew").await;
                if brew_ok && !brew.is_empty() {
                    let brew = brew.lines().next().unwrap_or("brew").trim();
                    sh(&format!("\"{brew}\" install node")).await
                } else {
                    sh("NONINTERACTIVE=1 /bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\" && eval \"$(/opt/homebrew/bin/brew shellenv 2>/dev/null || /usr/local/bin/brew shellenv)\" && brew install node").await
                }
            } else {
                (false, "当前平台暂不支持自动安装 Node.js".into())
            }
        }
        other => (false, format!("未知的安装目标: {other}")),
    }
}

pub async fn restart_codex() -> (bool, String) {
    if cfg!(windows) {
        return sh(r#"powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$ErrorActionPreference='SilentlyContinue'; Stop-Process -Name Codex,CodexHelper -Force; Start-Sleep -Milliseconds 500; $app=Get-StartApps | Where-Object { $_.Name -match 'Codex' } | Select-Object -First 1; if ($app) { Start-Process ('shell:AppsFolder\' + $app.AppID) } else { $paths=@($env:LOCALAPPDATA + '\Programs\Codex\Codex.exe',$env:LOCALAPPDATA + '\Codex\Codex.exe'); $path=$paths | Where-Object { Test-Path $_ } | Select-Object -First 1; if (-not $path) { throw '未找到 Codex Desktop 启动入口' }; Start-Process $path }"#).await;
    }
    if cfg!(target_os = "macos") {
        return sh("pkill -f '/Codex' >/dev/null 2>&1 || true; sleep 0.5; open -a Codex").await;
    }
    (false, "当前平台暂不支持一键重启 Codex Desktop".into())
}
