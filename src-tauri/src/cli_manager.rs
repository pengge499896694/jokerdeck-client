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
    pub chatgpt_desktop: CliStatus,
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
    #[cfg(target_os = "macos")]
    cmd.env(
        "PATH",
        format!(
            "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{}",
            dirs::home_dir()
                .map(|home| home.display().to_string())
                .unwrap_or_default(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into())
        ),
    );
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

#[cfg(any(target_os = "macos", test))]
fn latest_macos_node_lts(index: &serde_json::Value) -> Option<&str> {
    index.as_array()?.iter().find_map(|release| {
        let version = release.get("version")?.as_str()?;
        let has_pkg = release
            .get("files")?
            .as_array()?
            .iter()
            .any(|file| file.as_str() == Some("osx-x64-pkg"));
        (release.get("lts")?.as_str().is_some()
            && has_pkg
            && version.starts_with('v')
            && version[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
            && version[1..].split('.').count() == 3
            && version[1..].split('.').all(|part| !part.is_empty()))
        .then_some(version)
    })
}

#[cfg(target_os = "macos")]
async fn install_macos_node_pkg() -> (bool, String) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => return (false, format!("创建下载客户端失败：{error}")),
    };
    let index = async {
        client
            .get("https://nodejs.org/dist/index.json")
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await
    }
    .await;
    let index = match index {
        Ok(index) => index,
        Err(error) => return (false, format!("获取 Node.js 官方版本列表失败：{error}")),
    };
    let Some(version) = latest_macos_node_lts(&index) else {
        return (
            false,
            "Node.js 官方版本列表中没有可用的 macOS LTS 安装包".into(),
        );
    };
    let script = format!(
        r#"set -eu
tmp="$(/usr/bin/mktemp -d /tmp/jokerdeck-node.XXXXXXXX)" || exit 1
trap '/bin/rm -f "$tmp/SHASUMS256.txt" "$tmp/node.pkg"; /bin/rmdir "$tmp"' EXIT
base="https://nodejs.org/dist/{version}"
printf '正在下载 Node.js {version} 官方 macOS 安装包...\n'
/usr/bin/curl -fLsS --retry 3 --retry-delay 2 --connect-timeout 15 --max-time 60 "$base/SHASUMS256.txt" -o "$tmp/SHASUMS256.txt" || exit 1
expected="$(/usr/bin/awk '$2 == "node-{version}.pkg" {{print $1}}' "$tmp/SHASUMS256.txt")"
if [ -z "$expected" ]; then
  printf '官方校验文件中没有匹配的安装包。\n'
  exit 1
fi
/usr/bin/curl -fLsS --retry 3 --retry-delay 2 --connect-timeout 30 --max-time 900 "$base/node-{version}.pkg" -o "$tmp/node.pkg" || exit 1
printf '%s  %s\n' "$expected" "$tmp/node.pkg" | /usr/bin/shasum -a 256 -c - || exit 1
printf '校验通过，正在请求 macOS 管理员授权...\n'
NODE_INSTALLER_PKG="$tmp/node.pkg" /usr/bin/osascript -e 'do shell script "/usr/sbin/installer -pkg " & quoted form of (system attribute "NODE_INSTALLER_PKG") & " -target /" with administrator privileges'
printf 'Node.js {version} 安装完成。\n'"#
    );
    let (ok, output) = sh_with_timeout(&script, 1800).await;
    if !ok {
        return (false, format!("Node.js 官方安装包安装失败：\n{output}"));
    }
    let node = detect("node", "-v").await;
    let npm = detect("npm", "-v").await;
    if !node.installed || !npm.installed {
        return (
            false,
            format!(
                "安装器已完成，但客户端未检测到 Node.js 或 npm，请重新打开客户端后检测。\n{output}"
            ),
        );
    }
    (
        true,
        format!(
            "{output}\n检测到 Node.js {}、npm {}",
            node.version.unwrap_or_default(),
            npm.version.unwrap_or_default()
        ),
    )
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
        chatgpt_desktop: detect_chatgpt_desktop(),
    }
}

fn detect_chatgpt_desktop() -> CliStatus {
    if !cfg!(target_os = "macos") {
        return CliStatus::default();
    }
    let mut roots = vec![std::path::PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    for root in roots {
        for name in ["ChatGPT.app", "ChatGPT Classic.app"] {
            let app = root.join(name);
            if app.join("Contents/Info.plist").is_file() {
                return CliStatus {
                    installed: true,
                    version: None,
                    path: Some(app.display().to_string()),
                };
            }
        }
    }
    CliStatus::default()
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
        if let Some(path) = crate::codex_desktop::macos_codex_app() {
            return CliStatus {
                installed: true,
                version: None,
                path: Some(path.display().to_string()),
            };
        }
    }
    CliStatus::default()
}

/// Install a CLI. On macOS, avoid the system-owned npm global prefix.
pub async fn install(which_cli: &str) -> (bool, String) {
    match which_cli {
        "claude" if cfg!(target_os = "macos") => {
            sh_with_timeout(
                r#"set -eu
prefix="$HOME/.local"
npm --prefix "$prefix" install -g @anthropic-ai/claude-code \
  --registry=https://registry.npmmirror.com \
  --allow-scripts=@anthropic-ai/claude-code
package_dir="$prefix/lib/node_modules/@anthropic-ai/claude-code"
if [ -f "$package_dir/install.cjs" ]; then
  (cd "$package_dir" && node install.cjs)
fi
bin="$prefix/bin/claude"
if [ ! -x "$bin" ]; then
  printf 'Claude Code 安装包已下载，但 native binary 未生成。\n'
  printf '请检查网络或重新运行安装。\n'
  exit 1
fi
"$bin" --version
printf '\nClaude Code 已安装到 %s\n如终端找不到 claude，请将 $HOME/.local/bin 加入 PATH。\n' "$bin""#,
                1200,
            )
            .await
        }
        "claude" => sh("npm i -g @anthropic-ai/claude-code").await,
        // Codex CLI (the "ChatGPT/Codex" coding CLI that can be pointed at a relay).
        "codex" if cfg!(target_os = "macos") => {
            sh(
                r#"npm --prefix "$HOME/.local" install -g @openai/codex && printf '\n如终端找不到 codex，请将 $HOME/.local/bin 加入 PATH。\n'"#,
            )
            .await
        }
        "codex" => sh("npm i -g @openai/codex").await,
        "node" => {
            if cfg!(windows) {
                sh("winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements").await
            } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
                #[cfg(target_os = "macos")]
                return install_macos_node_pkg().await;
                #[allow(unreachable_code)]
                (false, "当前平台暂不支持自动安装 Node.js".into())
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

#[cfg(test)]
mod tests {
    use super::latest_macos_node_lts;

    #[test]
    fn picks_latest_lts_with_macos_pkg() {
        let index = serde_json::json!([
            {"version": "v26.0.0", "lts": false, "files": ["osx-x64-pkg"]},
            {"version": "v24.21.0", "lts": "Krypton", "files": ["osx-x64-pkg"]},
            {"version": "v22.20.0", "lts": "Jod", "files": ["osx-x64-pkg"]}
        ]);
        assert_eq!(latest_macos_node_lts(&index), Some("v24.21.0"));
    }

    #[test]
    fn rejects_missing_package_and_unsafe_version() {
        let index = serde_json::json!([
            {"version": "v24.21.0;echo", "lts": "Krypton", "files": ["osx-x64-pkg"]},
            {"version": "v24.21.0", "lts": "Krypton", "files": ["osx-arm64-tar"]}
        ]);
        assert_eq!(latest_macos_node_lts(&index), None);
    }
}
