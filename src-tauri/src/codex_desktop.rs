//! Codex Desktop 进程管理：关闭、启动（官方英文版或本地汉化副本）与完整性修复。
//!
//! Store 版 Codex 是 Electron「owl shell」应用，入口是 `ChatGPT.exe`，它内嵌了
//! `app.asar` 的 SHA-256 完整性校验（EnableEmbeddedAsarIntegrityValidation fuse）。
//! 上游汉化引擎只同步了 `Codex.exe` 的哈希、漏掉了 `ChatGPT.exe`，导致汉化后启动直接
//! 因完整性校验失败而中止——表现为「启动汉化版没有反应」。这里在启动前重算补丁后的
//! asar 头部哈希并写回 `ChatGPT.exe`，使汉化副本可独立启动。
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Electron 在可执行文件里嵌入的 app.asar 完整性标记（ASCII），其后紧跟 64 位十六进制
/// SHA-256。前缀 `resources\\` / `resources/` 在不同打包方式下写法不一，这里只匹配稳定
/// 的尾部，命中 app.asar 对应的那条记录。
const INTEGRITY_MARKER: &[u8] = b"app.asar\",\"alg\":\"SHA256\",\"value\":\"";

/// 计算 app.asar 头部字符串的 SHA-256（小写十六进制），与 Electron 的校验口径一致。
fn asar_header_hash(asar: &Path) -> Result<String> {
    let data = std::fs::read(asar).with_context(|| format!("无法读取 {}", asar.display()))?;
    if data.len() < 16 {
        bail!("app.asar 头部过短");
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let header_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let string_size = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    if magic != 4 || header_size == 0 || string_size == 0 || 16 + string_size > data.len() {
        bail!("app.asar 头部格式异常");
    }
    let mut hasher = Sha256::new();
    hasher.update(&data[16..16 + string_size]);
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        hex.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        hex.push(char::from_digit((byte & 0xf) as u32, 16).unwrap());
    }
    Ok(hex)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
/// 把 `exe` 内嵌的 app.asar 完整性哈希改写为 `want_hash`（64 位小写十六进制），返回改写
/// 的记录数。首次改写前会生成 `<exe>.integ-bak` 备份。
fn patch_exe_integrity(exe: &Path, want_hash: &str) -> Result<usize> {
    debug_assert_eq!(want_hash.len(), 64);
    let want = want_hash.as_bytes();
    let mut data = std::fs::read(exe).with_context(|| format!("无法读取 {}", exe.display()))?;
    let marker_len = INTEGRITY_MARKER.len();
    let mut changed = 0usize;
    let mut cursor = 0usize;
    while let Some(rel) = find_subslice(&data[cursor..], INTEGRITY_MARKER) {
        let value_at = cursor + rel + marker_len;
        cursor = value_at;
        if value_at + 64 > data.len()
            || !data[value_at..value_at + 64]
                .iter()
                .all(u8::is_ascii_hexdigit)
        {
            continue;
        }
        if &data[value_at..value_at + 64] != want {
            data[value_at..value_at + 64].copy_from_slice(want);
            changed += 1;
        }
    }
    if changed > 0 {
        let backup = exe.with_file_name(format!(
            "{}.integ-bak",
            exe.file_name().unwrap_or_default().to_string_lossy()
        ));
        if !backup.exists() {
            std::fs::copy(exe, &backup).with_context(|| format!("备份 {} 失败", exe.display()))?;
        }
        std::fs::write(exe, &data).with_context(|| format!("写入 {} 失败", exe.display()))?;
    }
    Ok(changed)
}

/// 修复汉化副本中所有内嵌完整性标记的可执行文件（关键是 `ChatGPT.exe`）。
fn repair_windows_exes(app_dir: &Path) -> Result<String> {
    let want = asar_header_hash(&app_dir.join("resources").join("app.asar"))?;
    let mut touched: Vec<&str> = Vec::new();
    for name in ["ChatGPT.exe", "Codex.exe", "codex.exe"] {
        let exe = app_dir.join(name);
        if exe.is_file() && patch_exe_integrity(&exe, &want)? > 0 {
            touched.push(name);
        }
    }
    Ok(if touched.is_empty() {
        "完整性校验已是最新".into()
    } else {
        format!("已校正 {} 的完整性哈希", touched.join("、"))
    })
}
fn codex_home() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        Some(path) => Some(PathBuf::from(path)),
        None => dirs::home_dir().map(|home| home.join(".codex")),
    }
}

/// Windows：从 `~/.codex/zh-cn-patched-active.txt` 读取汉化副本根目录，返回其 `app` 子目录。
pub(crate) fn patched_app_dir() -> Option<PathBuf> {
    let text = std::fs::read_to_string(codex_home()?.join("zh-cn-patched-active.txt")).ok()?;
    let root = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let app = Path::new(root).join("app");
    app.join("resources")
        .join("app.asar")
        .is_file()
        .then_some(app)
}

/// macOS：定位已安装的 Codex.app。
pub(crate) fn macos_codex_app() -> Option<PathBuf> {
    let app = PathBuf::from("/Applications/Codex.app");
    app.join("Contents/Resources/app.asar")
        .is_file()
        .then_some(app)
}

/// 把 `~/.codex/config.toml` 的 `[desktop] localeOverride` 设为 `zh-CN`，保留其余内容。
pub(crate) fn set_locale_zh_cn() -> Result<()> {
    use toml_edit::{value, DocumentMut, Item, Table};
    let path = crate::config_writer::codex_config_path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse::<DocumentMut>()
            .context("Codex config.toml 格式无效")?
    };
    if !doc.get("desktop").map(Item::is_table).unwrap_or(false) {
        doc["desktop"] = Item::Table(Table::new());
    }
    doc["desktop"]["localeOverride"] = value("zh-CN");
    crate::config_writer::write_with_backup(&path, doc.to_string().as_bytes())
}

pub(crate) fn clear_locale_zh_cn() -> Result<()> {
    use toml_edit::{DocumentMut, Item};
    let path = crate::config_writer::codex_config_path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(());
    }
    let mut doc = text
        .parse::<DocumentMut>()
        .context("Codex config.toml 格式无效")?;
    if let Some(desktop) = doc.get_mut("desktop").filter(|item| item.is_table()) {
        desktop["localeOverride"] = Item::None;
    }
    crate::config_writer::write_with_backup(&path, doc.to_string().as_bytes())
}

pub(crate) fn locale_is_zh_cn() -> bool {
    let Ok(path) = crate::config_writer::codex_config_path() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.parse::<toml_edit::DocumentMut>()
        .ok()
        .and_then(|doc| {
            doc.get("desktop")?
                .get("localeOverride")?
                .as_str()
                .map(str::to_owned)
        })
        .is_some_and(|locale| locale.eq_ignore_ascii_case("zh-cn"))
}
/// 运行一段 PowerShell（Windows 启动/关闭用），不弹出控制台窗口。
async fn powershell(
    script: &str,
    envs: &[(&str, String)],
    timeout_s: u64,
) -> Result<(bool, String)> {
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ])
    .kill_on_drop(true);
    for (key, val) in envs {
        cmd.env(*key, val);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = tokio::time::timeout(Duration::from_secs(timeout_s), cmd.output())
        .await
        .context("Codex 操作超时")??;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), text.trim().to_string()))
}

/// 运行一个外部命令并合并输出（macOS 的 open / codesign / PlistBuddy / xattr 用）。
async fn run_tool(program: &str, args: &[&str]) -> Result<(bool, String)> {
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new(program)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .with_context(|| format!("{program} 执行超时"))?
    .with_context(|| format!("无法执行 {program}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), text.trim().to_string()))
}

/// 只关闭 Codex Desktop 自身的进程：按安装路径过滤，避免误杀同名的 ChatGPT 桌面端或
/// VSCode 的 codex CLI。
const WINDOWS_STOP_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
Get-Process -Name ChatGPT, Codex, codex-helper, CodexHelper -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and (
        $_.Path -notmatch '(?i)\\Codex\+\+\\|codex-plus-plus' -and
        (
            $_.Path -like '*OpenAI.Codex*' -or
            $_.Path -like '*zh-cn-patched*' -or
            $_.Path -like '*\Programs\Codex\*' -or
            $_.Path -like '*\Codex\*'
        )
    ) } |
    Stop-Process -Force
Start-Sleep -Milliseconds 400"#;

async fn stop() {
    if cfg!(windows) {
        let _ = powershell(WINDOWS_STOP_SCRIPT, &[], 20).await;
    } else if cfg!(target_os = "macos") {
        let _ = run_tool("pkill", &["-f", "Codex.app/Contents/MacOS"]).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
async fn launch_patched_windows(app_dir: &Path) -> Result<(bool, String)> {
    let exe = ["ChatGPT.exe", "Codex.exe", "codex.exe"]
        .into_iter()
        .map(|name| app_dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("汉化副本缺少可执行文件，请重新安装汉化包"))?;
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$process = Start-Process -FilePath $env:CODEX_LAUNCH_EXE -WorkingDirectory $env:CODEX_LAUNCH_DIR -PassThru
for ($attempt = 0; $attempt -lt 20; $attempt++) {
    Start-Sleep -Milliseconds 250
    $running = Get-Process -Name ChatGPT, Codex -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($env:CODEX_LAUNCH_DIR + '\', [StringComparison]::OrdinalIgnoreCase) }
    if ($running) {
        Start-Sleep -Seconds 2
        $alive = Get-Process -Id $running[0].Id -ErrorAction SilentlyContinue
        if ($alive) { exit 0 }
    }
}
throw '汉化版 Codex 未能启动；请重新安装汉化包或检查 Codex 更新是否覆盖补丁'
"#;
    let envs = [
        ("CODEX_LAUNCH_EXE", exe.to_string_lossy().into_owned()),
        ("CODEX_LAUNCH_DIR", app_dir.to_string_lossy().into_owned()),
    ];
    powershell(SCRIPT, &envs, 30).await
}

/// 通过 AUMID 启动官方（英文）Store 版，动态解析 PackageFamilyName。
const WINDOWS_LAUNCH_STORE_SCRIPT: &str = "\
$ErrorActionPreference = 'Stop'
$pkg = Get-AppxPackage -Name 'OpenAI.Codex' | Sort-Object Version -Descending | Select-Object -First 1
if (-not $pkg) { throw '未找到 Codex Desktop（Microsoft Store 版），请先安装' }
Start-Process ('shell:AppsFolder\\' + $pkg.PackageFamilyName + '!App')";

async fn launch_store_windows() -> Result<(bool, String)> {
    powershell(WINDOWS_LAUNCH_STORE_SCRIPT, &[], 30).await
}

async fn launch_installed_windows() -> Result<(bool, String)> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$paths = @(
    "$env:LOCALAPPDATA\Programs\Codex\Codex.exe",
    "$env:LOCALAPPDATA\Codex\Codex.exe",
    "$env:LOCALAPPDATA\Programs\Codex\ChatGPT.exe",
    "$env:LOCALAPPDATA\Codex\ChatGPT.exe"
)
$path = $paths |
    Where-Object { Test-Path -LiteralPath $_ } |
    Where-Object { $_ -notmatch '(?i)\\Codex\+\+\\|codex-plus-plus' } |
    Select-Object -First 1
if (-not $path) { throw '未找到 Codex Desktop 启动入口' }
Start-Process -FilePath $path -WorkingDirectory (Split-Path -Parent $path)"#;
    powershell(SCRIPT, &[], 30).await
}

async fn launch_macos(app_bundle: &Path) -> Result<(bool, String)> {
    run_tool("open", &["-n", &app_bundle.to_string_lossy()]).await
}

/// 启动「汉化版」Codex：关闭现有进程 → 校正完整性 → 写入 zh-CN → 启动补丁副本。
/// 仅用于显式点击「启动汉化版」按钮。
pub async fn launch_localized() -> Result<String> {
    if cfg!(windows) {
        let app = patched_app_dir().ok_or_else(|| anyhow!("请先安装 Codex 汉化包"))?;
        stop().await;
        let repair = repair_windows_exes(&app)?;
        set_locale_zh_cn()?;
        let (ok, log) = launch_patched_windows(&app).await?;
        if !ok {
            bail!("启动汉化版 Codex 失败：{log}");
        }
        Ok(format!("已启动汉化版 Codex；{repair}。"))
    } else if cfg!(target_os = "macos") {
        let app = macos_codex_app().ok_or_else(|| anyhow!("未找到 /Applications/Codex.app"))?;
        stop().await;
        set_locale_zh_cn()?;
        let (ok, log) = launch_macos(&app).await?;
        if !ok {
            bail!("启动 Codex 失败：{log}");
        }
        Ok("已启动 Codex（简体中文）。".into())
    } else {
        bail!("当前平台暂不支持 Codex 桌面端汉化")
    }
}

/// 启动官方英文版 Codex，并清除可能残留的中文语言覆盖。
pub async fn launch_english() -> Result<String> {
    clear_locale_zh_cn()?;
    stop().await;
    let launched = if cfg!(windows) {
        match launch_store_windows().await {
            Ok((true, log)) => Ok((true, log)),
            _ => launch_installed_windows().await,
        }
    } else if cfg!(target_os = "macos") {
        match macos_codex_app() {
            Some(app) => launch_macos(&app).await,
            None => run_tool("open", &["-a", "Codex"]).await,
        }
    } else {
        bail!("当前平台暂不支持一键重启 Codex Desktop");
    };
    match launched {
        Ok((true, _)) => Ok("已启动英文版 Codex Desktop。".into()),
        Ok((false, log)) => bail!("启动英文版 Codex 失败：{log}"),
        Err(error) => Err(anyhow!("启动英文版 Codex 失败：{error}")),
    }
}

/// 一键重启跟随当前语言状态：已汉化则保持全中文，否则启动英文版。
pub async fn restart(localized_active: bool) -> (bool, String) {
    if localized_active {
        return match launch_localized().await {
            Ok(message) => (true, message),
            Err(error) => (false, error.to_string()),
        };
    }
    match launch_english().await {
        Ok(message) => (true, message),
        Err(error) => (false, error.to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_integrity_hash_in_place_and_backs_up() {
        let dir = std::env::temp_dir().join(format!("jd-codex-desktop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ChatGPT.exe");
        let old = "0".repeat(64);
        let mut bytes = b"PADDINGX".to_vec();
        bytes.extend_from_slice(INTEGRITY_MARKER);
        bytes.extend_from_slice(old.as_bytes());
        bytes.extend_from_slice(b"\"}]PADDING");
        std::fs::write(&exe, &bytes).unwrap();

        let want = "a".repeat(64);
        assert_eq!(patch_exe_integrity(&exe, &want).unwrap(), 1);
        let after = std::fs::read(&exe).unwrap();
        assert_eq!(after.len(), bytes.len(), "改写必须等长，不破坏 PE 布局");
        assert!(String::from_utf8_lossy(&after).contains(&want));
        assert!(exe.with_file_name("ChatGPT.exe.integ-bak").exists());
        // 幂等：再次运行无需改写。
        assert_eq!(patch_exe_integrity(&exe, &want).unwrap(), 0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn asar_header_hash_matches_sha256_of_header_string() {
        let header = br#"{"files":{}}"#;
        let str_size = header.len() as u32;
        let payload = 4 + str_size;
        let payload_aligned = payload + ((4 - (payload % 4)) % 4);
        let header_size = 4 + payload_aligned;
        let mut data = 4u32.to_le_bytes().to_vec();
        data.extend_from_slice(&header_size.to_le_bytes());
        data.extend_from_slice(&payload_aligned.to_le_bytes());
        data.extend_from_slice(&str_size.to_le_bytes());
        data.extend_from_slice(header);
        while data.len() < 8 + header_size as usize {
            data.push(0);
        }
        let dir = std::env::temp_dir().join(format!("jd-asar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let asar = dir.join("app.asar");
        std::fs::write(&asar, &data).unwrap();

        let expected: String = Sha256::digest(header)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(asar_header_hash(&asar).unwrap(), expected);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
