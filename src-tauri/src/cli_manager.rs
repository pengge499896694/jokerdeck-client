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
    CliStatus::default()
}

/// Install a CLI. `claude`/`codex` go through npm; `node` uses winget on Windows.
pub async fn install(which_cli: &str) -> (bool, String) {
    match which_cli {
        "claude" => sh("npm i -g @anthropic-ai/claude-code").await,
        // Codex CLI (the "ChatGPT/Codex" coding CLI that can be pointed at a relay).
        "codex" => sh("npm i -g @openai/codex").await,
        "node" => {
            if cfg!(windows) {
                sh("winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements").await
            } else {
                (false, "请从 https://nodejs.org 安装 Node.js 后重试".into())
            }
        }
        other => (false, format!("未知的安装目标: {other}")),
    }
}
