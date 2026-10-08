use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

const VERSION: &str = "v0.1.2";
const ASSET: &str = "codex-zh-CN-v0.1.2.zip";
const MAX_ARCHIVE: usize = 12 * 1024 * 1024;
const LOCALIZATION_MANIFEST_PATH: &str = "/client/codex-localization/latest.json";
const LOCALIZATION_DOWNLOAD_PATH: &str = "/client/codex-localization/download";
const SAFE_PATCH_STAMP: &str = "safe-localization-v2";

#[derive(Clone, Serialize)]
pub struct LocalizationProgress {
    pub percent: u8,
    pub detail: String,
}

fn progress(
    report: &(dyn Fn(LocalizationProgress) + Send + Sync),
    percent: u8,
    detail: impl Into<String>,
) {
    report(LocalizationProgress {
        percent,
        detail: detail.into(),
    });
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    digest: Option<String>,
    size: usize,
}

pub async fn run(
    app_dir: &Path,
    http: &reqwest::Client,
    action: &str,
    preferred_host: &str,
    report: &(dyn Fn(LocalizationProgress) + Send + Sync),
) -> Result<String> {
    if !matches!(action, "install" | "uninstall" | "launch") {
        bail!("未知的 Codex 汉化操作");
    }
    if cfg!(target_os = "macos") {
        progress(report, 10, "检查 Codex Desktop");
        if crate::codex_desktop::macos_codex_app().is_none() {
            bail!("未找到 Codex.app，请先安装 Codex Desktop");
        }
        let result = match action {
            "uninstall" => crate::codex_desktop::launch_english().await,
            "install" => crate::codex_desktop::launch_localized().await,
            "launch" => crate::codex_desktop::launch_localized().await,
            _ => unreachable!(),
        };
        if result.is_ok() {
            progress(report, 100, "操作完成");
        }
        return result;
    }

    if !cfg!(windows) {
        bail!("当前平台暂不支持 Codex 桌面端汉化");
    }

    let root = app_dir.join(format!("codex-zh-CN-{VERSION}"));
    if action == "uninstall" {
        let script = find_file(&root, "scripts/patch-codex-zh-cn.mjs")
            .ok_or_else(|| anyhow!("未找到本客户端安装的汉化包，无法恢复英文"))?;
        progress(report, 5, "正在恢复英文界面");
        let result = execute(&script, "uninstall", report).await;
        if result.is_ok() {
            crate::codex_desktop::clear_locale_zh_cn()?;
            let _ = std::fs::remove_file(root.join(SAFE_PATCH_STAMP));
            progress(report, 100, "英文界面已恢复");
        }
        return result;
    }

    progress(report, 3, "检查汉化包");
    let package_missing = !package_available(app_dir);
    if action == "install" || package_missing {
        progress(report, 6, "下载并校验汉化包");
        download_verified(http, &root, preferred_host).await?;
        progress(report, 13, "汉化包校验完成");
    }
    patch_bundle_compatibility(&root)?;
    patch_native_menu_locale(&root)?;
    if action == "launch" {
        let (_, active) = status(app_dir);
        if !active || !installed_menu_matches(&root) || !root.join(SAFE_PATCH_STAMP).is_file() {
            let _ = std::fs::remove_file(root.join(SAFE_PATCH_STAMP));
            let script = find_file(&root, "scripts/patch-codex-zh-cn.mjs")
                .ok_or_else(|| anyhow!("汉化包缺少安装脚本"))?;
            execute(&script, "install", report).await?;
            if !installed_menu_matches(&root) {
                bail!("汉化安装尚未完成，请检查安装详情后重试");
            }
            std::fs::write(root.join(SAFE_PATCH_STAMP), b"")?;
        }
        progress(report, 93, "启动汉化版 Codex");
        let result = crate::codex_desktop::launch_localized().await;
        if result.is_ok() {
            progress(report, 100, "汉化版 Codex 已启动");
        }
        return result;
    }
    let _ = std::fs::remove_file(root.join(SAFE_PATCH_STAMP));
    let script = find_file(&root, "scripts/patch-codex-zh-cn.mjs")
        .ok_or_else(|| anyhow!("汉化包缺少安装脚本"))?;
    let result = execute(&script, "install", report).await?;
    if !installed_menu_matches(&root) {
        bail!("汉化未通过完整性检查，请查看安装详情");
    }
    std::fs::write(root.join(SAFE_PATCH_STAMP), b"")?;
    progress(report, 100, "汉化安装完成");
    Ok(result)
}

pub fn status(app_dir: &Path) -> (bool, bool) {
    if cfg!(target_os = "macos") {
        let available = crate::codex_desktop::macos_codex_app().is_some();
        return (
            available,
            available && crate::codex_desktop::locale_is_zh_cn(),
        );
    }
    let root = app_dir.join(format!("codex-zh-CN-{VERSION}"));
    let package_available = package_available_at(&root);
    let active_file = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(std::path::PathBuf::from)
                .map(|path| path.join(".codex"))
        })
        .map(|path| path.join("zh-cn-patched-active.txt"))
        .is_some_and(|path| path.is_file());
    (package_available, package_available && active_file)
}

fn package_available_at(root: &Path) -> bool {
    find_file(root, "launch-codex-zh-cn.ps1").is_some()
        && find_file(root, "scripts/install_windows.ps1").is_some()
}

fn package_available(app_dir: &Path) -> bool {
    package_available_at(&app_dir.join(format!("codex-zh-CN-{VERSION}")))
}

fn patch_bundle_compatibility(root: &Path) -> Result<()> {
    const OLD: &str = "const insertAt = text.lastIndexOf(\"};export\");";
    const NEW: &str = "const modernEnd = text.lastIndexOf(\"}}))();export\");\n  const insertAt = modernEnd >= 0 ? modernEnd : text.lastIndexOf(\"};export\");";
    const INJECT: &str = "const zhPatched = patchZhBundle(zhContent, menuTitleMap);";
    const SKIP: &str = "const zhPatched = { buffer: zhContent, count: 0 };";
    let script = find_file(root, "scripts/patch-codex-zh-cn.mjs")
        .ok_or_else(|| anyhow!("汉化包缺少补丁脚本"))?;
    let original = std::fs::read_to_string(&script)?;
    // The upstream insertion point is not stable across Codex releases. Keep the
    // bundled zh-CN translations intact and patch menu JSON through the ASAR API.
    let source = original.replacen(NEW, OLD, 1);
    let source = if source.matches(INJECT).count() == 1 {
        source.replacen(INJECT, SKIP, 1)
    } else if source.contains(SKIP) {
        source
    } else {
        bail!("汉化包补丁脚本格式已变化，无法安全应用兼容修正");
    };
    let source = patch_webview_text_replacements(&source)?;
    let source = patch_i18n_gate_script(&source)?;
    let source = source.replace(
        "{ encoding: \"utf8\"",
        "{ windowsHide: true, encoding: \"utf8\"",
    );
    if source != original {
        std::fs::write(&script, source)
            .with_context(|| format!("无法更新汉化包补丁脚本：{}", script.display()))?;
    }
    patch_hardcoded_menu_locale(root)
}

fn patch_i18n_gate_script(source: &str) -> Result<String> {
    const ANCHOR: &str =
        "function patchWebviewBundles(asarPath, replacements = WEBVIEW_TEXT_PATCHES) {";
    const CALL_ANCHOR: &str = "  const webviewPatchCount = patchWebviewBundles(asarPath);";
    const CALL: &str = "  patchWebviewLocaleGate(asarPath);\n";
    const PATCH: &str = r#"function patchWebviewLocaleGate(asarPath) {
  const data = fs.readFileSync(asarPath);
  const parsed = readAsarHeader(data, asarPath);
  const bundles = walkAsarFiles(parsed.header)
    .map(([filePath]) => filePath)
    .filter((filePath) => /(^|\/)webview\/assets\/app-initial-[^/]+\.js$/.test(filePath));
  if (bundles.length !== 1) {
    throw new Error("无法唯一定位 Codex WebView 初始化脚本，未启用界面汉化");
  }
  const filePath = bundles[0];
  const entry = getAsarFileEntry(parsed.header, filePath);
  const offset = 8 + parsed.headerSize + Number(entry.offset);
  const original = data.subarray(offset, offset + Number(entry.size)).toString("utf8");
  const anchor = "c=s?.get(`enable_i18n`,!1),t[0]=s,t[1]=c";
  const enabled = "c=!0,t[0]=s,t[1]=c";
  if (original.includes(enabled)) return;
  if (original.split(anchor).length !== 2) {
    logOk("当前 Codex 使用新版语言入口；保留官方 zh-CN 设置，不修改未知 WebView 结构");
    return;
  }
  replaceAsarFileContent(asarPath, filePath, Buffer.from(original.replace(anchor, enabled), "utf8"));
  logOk("已启用 Codex WebView 中文语言包");
}

"#;
    if source.contains("function patchWebviewLocaleGate(asarPath) {") {
        if source.contains(&format!("{CALL}{CALL_ANCHOR}")) {
            return Ok(source.to_owned());
        }
        bail!("汉化包 WebView 语言开关补丁不完整");
    }
    if source.matches(ANCHOR).count() != 1 || source.matches(CALL_ANCHOR).count() != 1 {
        bail!("汉化包补丁脚本格式已变化，无法启用 WebView 中文语言包");
    }
    let updated = source.replacen(ANCHOR, &format!("{PATCH}{ANCHOR}"), 1);
    Ok(updated.replacen(CALL_ANCHOR, &format!("{CALL}{CALL_ANCHOR}"), 1))
}

fn patch_webview_text_replacements(source: &str) -> Result<String> {
    const OLD: &str = r#"const WEBVIEW_TEXT_PATCHES = [["title:`Featured`", "title:`精选`"]];"#;
    const NEW: &str = r#"const WEBVIEW_TEXT_PATCHES = [
  ["title:`Featured`", "title:`精选`"],
  ["New Chat", "新建聊天"],
  ["New chat", "新建聊天"],
  ["New Window", "新建窗口"],
  ["New Temporary Chat", "新建临时聊天"],
  ["Temporary Chat", "临时聊天"],
  ["Pull requests", "拉取请求"],
  ["Pull Requests", "拉取请求"],
  ["Schedule", "计划任务"],
  ["Plugins", "插件"],
  ["Projects", "项目"],
  ["Open Folder...", "打开文件夹..."],
  ["Open Folder…", "打开文件夹…"],
  ["Close", "关闭"],
  ["Log Out", "退出登录"],
  ["Settings...", "设置..."],
  ["Settings…", "设置…"],
  ["Toggle Sidebar", "切换侧边栏"],
  ["Toggle Bottom Panel", "切换底部面板"],
  ["Toggle Pinned Summary", "切换置顶摘要"],
  ["Open Terminal", "打开终端"],
  ["Switch between Chat and tabs", "在聊天与标签页之间切换"],
  ["Find", "查找"],
  ["Previous Chat", "上一个聊天"],
  ["Next Chat", "下一个聊天"],
  ["Back", "后退"],
  ["Forward", "前进"],
  ["About ChatGPT", "关于 ChatGPT"],
  ["Keyboard Shortcuts", "键盘快捷键"],
  ["Documentation", "文档"],
  ["What's New", "更新内容"],
  ["Troubleshooting", "故障排除"],
  ["System Status", "系统状态"],
  ["Send Feedback", "发送反馈"],
  ["Task Manager", "任务管理器"],
  ["Start Performance Trace", "开始性能跟踪"],
  ["Check for Updates...", "检查更新..."],
  ["Check for Updates…", "检查更新…"],
  ["Zoom In", "放大"],
  ["Zoom Out", "缩小"],
  ["Actual Size", "实际大小"],
  ["Toggle Full Screen", "切换全屏"],
  ["Back to app", "返回应用"],
  ["Search settings...", "搜索设置..."],
  ["Search settings…", "搜索设置…"],
  ["Personal", "个人"],
  ["General", "常规"],
  ["Appearance", "外观"],
  ["Configuration", "配置"],
  ["Personalization", "个性化"],
  ["Pets", "宠物"],
  ["Keyboard shortcuts", "键盘快捷键"],
  ["Account", "账户"],
  ["Integrations", "集成"],
  ["Computer use", "计算机操作"],
  ["Browser", "浏览器"],
  ["Coding", "编码"],
  ["Hooks", "Hooks"],
  ["Git", "Git"],
  ["Environments", "环境"],
  ["Worktrees", "工作树"],
  ["Archived", "已归档"],
  ["Archived chats", "已归档聊天"],
  ["Permissions", "权限"],
  ["Default permissions", "默认权限"],
  ["By default, ChatGPT can read and edit files in its workspace. It can ask for additional access when needed", "默认情况下，ChatGPT 可以读取和编辑工作区中的文件，需要时可以请求额外访问权限"],
  ["Full access", "完全访问权限"],
  ["When ChatGPT runs with full access, it can edit any file on your computer and run commands with network, without your approval. This significantly increases the risk of data loss, leaks, or unexpected behavior.", "使用完全访问权限运行时，ChatGPT 可以编辑电脑上的任何文件，并在未经你批准的情况下运行联网命令。这会显著增加数据丢失、数据泄露或意外行为的风险。"],
  ["Learn more about elevated risks.", "详细了解提升权限的风险。"],
  ["Projectless task folder", "无项目任务文件夹"],
  ["The location where tasks started outside of projects store their data by default.", "在项目外启动的任务默认存储数据的位置。"],
  ["Change", "更改"],
  ["Default file open destination", "默认文件打开位置"],
  ["Where files and folders open by default", "文件和文件夹默认打开的位置"],
  ["No targets found", "未找到目标"],
  ["Integrated terminal shell", "集成终端 Shell"],
  ["Choose which shell opens in the integrated terminal.", "选择集成终端中打开的 Shell。"],
  ["Language", "语言"],
  ["Language for the app UI", "应用界面语言"],
  ["Bottom panel", "底部面板"],
  ["File", "文件"],
  ["Edit", "编辑"],
  ["View", "查看"],
  ["Help", "帮助"],
];"#;
    if source.contains(NEW) {
        return Ok(source.replacen(NEW, OLD, 1));
    }
    if source.matches(OLD).count() != 1 {
        bail!("汉化包补丁脚本缺少可识别的 WebView 文案补丁入口");
    }
    Ok(source.to_owned())
}

fn patch_hardcoded_menu_locale(root: &Path) -> Result<()> {
    let path = find_file(root, "resources/menu-hardcoded-zh-CN.json")
        .ok_or_else(|| anyhow!("汉化包缺少主进程菜单语言文件"))?;
    let source = std::fs::read(&path)?;
    let mut locale: Vec<[String; 2]> =
        serde_json::from_slice(&source).context("主进程菜单语言文件格式无效")?;
    for pair in hardcoded_menu_translations() {
        if !locale.iter().any(|existing| existing == &pair) {
            locale.push(pair);
        }
    }
    let updated = serde_json::to_vec_pretty(&locale)?;
    if updated != source {
        std::fs::write(&path, updated)
            .with_context(|| format!("无法更新主进程菜单语言文件：{}", path.display()))?;
    }
    Ok(())
}

fn hardcoded_menu_translations() -> Vec<[String; 2]> {
    [
        ["New Chat", "新建聊天"],
        ["New chat", "新建聊天"],
        ["New Window", "新建窗口"],
        ["New Temporary Chat", "新建临时聊天"],
        ["Temporary Chat", "临时聊天"],
        ["Pull requests", "拉取请求"],
        ["Pull Requests", "拉取请求"],
        ["Schedule", "计划任务"],
        ["Plugins", "插件"],
        ["Projects", "项目"],
        ["Open Folder...", "打开文件夹..."],
        ["Open Folder…", "打开文件夹…"],
        ["Close", "关闭"],
        ["Log Out", "退出登录"],
        ["Settings...", "设置..."],
        ["Settings…", "设置…"],
        ["Toggle Sidebar", "切换侧边栏"],
        ["Toggle Bottom Panel", "切换底部面板"],
        ["Toggle Pinned Summary", "切换置顶摘要"],
        ["Open Terminal", "打开终端"],
        ["Switch between Chat and tabs", "在聊天与标签页之间切换"],
        ["Find", "查找"],
        ["Previous Chat", "上一个聊天"],
        ["Next Chat", "下一个聊天"],
        ["Back", "后退"],
        ["Forward", "前进"],
        ["About ChatGPT", "关于 ChatGPT"],
        ["Keyboard Shortcuts", "键盘快捷键"],
        ["Documentation", "文档"],
        ["What's New", "更新内容"],
        ["Troubleshooting", "故障排除"],
        ["System Status", "系统状态"],
        ["Send Feedback", "发送反馈"],
        ["Task Manager", "任务管理器"],
        ["Start Performance Trace", "开始性能跟踪"],
        ["Check for Updates...", "检查更新..."],
        ["Check for Updates…", "检查更新…"],
        ["Zoom In", "放大"],
        ["Zoom Out", "缩小"],
        ["Actual Size", "实际大小"],
        ["Toggle Full Screen", "切换全屏"],
    ]
    .map(|[source, target]| [source.into(), target.into()])
    .into()
}

fn patch_native_menu_locale(root: &Path) -> Result<()> {
    const MENU: &[(&str, &str)] = &[
        ("windowsMenuBar.file", "文件"),
        ("windowsMenuBar.edit", "编辑"),
        ("windowsMenuBar.view", "查看"),
        ("windowsMenuBar.help", "帮助"),
        ("electron.appMenu.window", "窗口"),
        ("electron.appMenu.edit.undo", "撤销"),
        ("electron.appMenu.edit.redo", "重做"),
        ("electron.appMenu.edit.cut", "剪切"),
        ("electron.appMenu.edit.copy", "复制"),
        ("electron.appMenu.edit.paste", "粘贴"),
        ("electron.appMenu.edit.pasteAndMatchStyle", "粘贴并匹配样式"),
        ("electron.appMenu.edit.delete", "删除"),
        ("electron.appMenu.edit.selectAll", "全选"),
        ("electron.appMenu.edit.substitutions", "替换"),
        ("electron.appMenu.edit.showSubstitutions", "显示替换"),
        ("electron.appMenu.edit.smartQuotes", "智能引号"),
        ("electron.appMenu.edit.smartDashes", "智能破折号"),
        ("electron.appMenu.edit.textReplacement", "文本替换"),
        ("electron.appMenu.edit.speech", "语音"),
        ("electron.appMenu.edit.startSpeaking", "开始朗读"),
        ("electron.appMenu.edit.stopSpeaking", "停止朗读"),
        ("electron.appMenu.window.minimize", "最小化"),
        ("electron.appMenu.window.zoom", "缩放"),
        ("electron.appMenu.window.bringAllToFront", "全部置于最前"),
        ("electron.appMenu.view.reloadWindow", "重新加载窗口"),
        ("electron.appMenu.view.actualSize", "实际大小"),
        ("electron.appMenu.view.toggleFullScreen", "切换全屏"),
        ("electron.appMenu.app.checkForUpdates", "检查更新…"),
        ("electron.appMenu.app.quit", "退出 {appName}"),
        ("electron.appMenu.app.services", "服务"),
        ("electron.appMenu.app.hide", "隐藏 {appName}"),
        ("electron.appMenu.app.hideOthers", "隐藏其他"),
        ("electron.appMenu.app.showAll", "全部显示"),
        ("loadingPage.documentationLink", "文档"),
        ("sidebarHelp.whatsNew", "更新内容"),
        ("electron.appMenu.help.troubleshooting", "故障排除"),
        ("electron.appMenu.help.systemStatus", "系统状态"),
        ("electron.appMenu.help.taskManager", "任务管理器"),
        ("artifactFeedback.button.label", "发送反馈"),
        ("codex.command.newThread", "新建聊天"),
        ("codex.command.temporaryChat", "新建临时聊天"),
        ("codex.command.openFolder", "打开文件夹"),
        ("codex.command.closeWindow", "关闭"),
        ("codex.command.closeTabOrWindow", "关闭"),
        ("codex.command.logOut", "退出登录"),
        ("codex.command.settings", "设置"),
        ("codex.command.toggleSidebar", "切换侧边栏"),
        ("codex.command.toggleBottomPanel", "切换底部面板"),
        ("codex.command.togglePinnedSummary", "切换置顶摘要"),
        ("codex.command.toggleTerminal", "打开终端"),
        (
            "codex.command.showWorkspaceTabView",
            "在聊天与标签页之间切换",
        ),
        ("codex.command.toggleSidePanel", "在聊天与标签页之间切换"),
        ("codex.command.findInThread", "查找"),
        ("codex.command.previousThread", "上一个聊天"),
        ("codex.command.nextThread", "下一个聊天"),
        ("codex.command.navigateBack", "后退"),
        ("codex.command.navigateForward", "前进"),
        ("codex.command.keyboardShortcuts", "键盘快捷键"),
        ("codex.command.showKeyboardShortcuts", "键盘快捷键"),
        ("codex.command.stepWorkspaceLayout", "切换工作区布局"),
        ("codex.command.toggleDebugModal", "切换调试面板"),
        ("debug.sidebarRamUsageIndicator.toggle", "显示/隐藏内存占用"),
        ("settings.nav.browser-use", "浏览器"),
        ("browserSidebar.zoomBanner.zoomIn", "放大"),
        ("browserSidebar.zoomBanner.zoomOut", "缩小"),
        ("plugins.detail.information.developer", "开发者"),
    ];
    let path = find_file(root, "resources/native-menu-zh-CN.json")
        .ok_or_else(|| anyhow!("汉化包缺少原生菜单语言文件"))?;
    let source = std::fs::read(&path)?;
    let mut locale: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&source).context("原生菜单语言文件格式无效")?;
    for &(id, label) in MENU {
        locale.insert(id.into(), label.into());
    }
    let updated = serde_json::to_vec_pretty(&locale)?;
    if updated != source {
        std::fs::write(&path, updated)
            .with_context(|| format!("无法更新原生菜单语言文件：{}", path.display()))?;
    }
    Ok(())
}

fn installed_menu_matches(root: &Path) -> bool {
    let Some(expected) = find_file(root, "resources/native-menu-zh-CN.json") else {
        return false;
    };
    let Some(active) = crate::codex_desktop::patched_app_dir() else {
        return false;
    };
    menu_matches_in_asar(&active.join("resources/app.asar"), &expected).unwrap_or(false)
}

fn menu_matches_in_asar(asar_path: &Path, expected_path: &Path) -> Result<bool> {
    let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(expected_path)?)?;
    let mut asar = std::fs::File::open(asar_path)?;
    let mut prefix = [0u8; 16];
    asar.read_exact(&mut prefix)?;
    let header_size = u32::from_le_bytes(prefix[4..8].try_into()?) as u64;
    let string_size = u32::from_le_bytes(prefix[12..16].try_into()?) as usize;
    if u32::from_le_bytes(prefix[0..4].try_into()?) != 4
        || string_size == 0
        || string_size as u64 > header_size.saturating_sub(8)
        || string_size > 32 * 1024 * 1024
    {
        bail!("app.asar 头部无效");
    }
    let mut header = vec![0; string_size];
    asar.read_exact(&mut header)?;
    let index: serde_json::Value = serde_json::from_slice(&header)?;
    let entry = ["native-menu-locales", "zh-CN.json"]
        .iter()
        .try_fold(&index, |node, part| node.get("files")?.get(*part))
        .ok_or_else(|| anyhow!("汉化副本缺少菜单资源"))?;
    let offset: u64 = entry["offset"]
        .as_str()
        .ok_or_else(|| anyhow!("菜单偏移无效"))?
        .parse()?;
    let size = entry["size"]
        .as_u64()
        .ok_or_else(|| anyhow!("菜单大小无效"))?;
    if size == 0 || size > 1024 * 1024 {
        bail!("菜单大小异常");
    }
    asar.seek(SeekFrom::Start(8 + header_size + offset))?;
    let mut content = vec![0; size as usize];
    asar.read_exact(&mut content)?;
    let installed: serde_json::Value = serde_json::from_slice(&content)?;
    Ok(installed == expected)
}

fn find_file(root: &Path, suffix: &str) -> Option<std::path::PathBuf> {
    // Releases may be packaged with a top-level `launchers` directory or
    // with the repository root preserved as an extra nested directory.
    let candidates = [
        root.join(suffix),
        root.join("launchers")
            .join(suffix.strip_prefix("launchers/").unwrap_or(suffix)),
        root.join(format!("codex-zh-CN-{VERSION}")).join(suffix),
        root.join(format!("codex-zh-CN-{VERSION}"))
            .join("launchers")
            .join(suffix.strip_prefix("launchers/").unwrap_or(suffix)),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

async fn download_verified(
    http: &reqwest::Client,
    root: &Path,
    preferred_host: &str,
) -> Result<()> {
    let release = fetch_release(http, preferred_host).await?;
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
    let bytes = download_asset(http, asset, preferred_host).await?;
    std::fs::create_dir_all(root)?;
    let _ = std::fs::remove_file(root.join(SAFE_PATCH_STAMP));
    let archive = root.join(ASSET);
    std::fs::write(&archive, bytes)?;
    let result = verify_and_unpack(&archive, root, digest).await;
    let _ = std::fs::remove_file(&archive);
    result
}

async fn fetch_release(http: &reqwest::Client, preferred_host: &str) -> Result<Release> {
    let mut last_error = None;
    for source in release_sources(preferred_host) {
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
                        if release.assets.iter().any(|asset| asset.name == ASSET) {
                            return Ok(release);
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

fn release_sources(preferred_host: &str) -> Vec<String> {
    vec![format!(
        "{}{}",
        preferred_host.trim_end_matches('/'),
        LOCALIZATION_MANIFEST_PATH
    )]
}

async fn download_asset(
    http: &reqwest::Client,
    asset: &Asset,
    preferred_host: &str,
) -> Result<bytes::Bytes> {
    let urls = relay_asset_urls(preferred_host);
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
        "汉化包下载失败：所有中转线路均不可用。请确认服务端已部署 /client/codex-localization/download/:asset 代理路由{}",
        last_error
            .map(|error| format!("：{error}"))
            .unwrap_or_default()
    ))
}

fn relay_asset_urls(preferred_host: &str) -> Vec<String> {
    vec![format!(
        "{}{}/{}",
        preferred_host.trim_end_matches('/'),
        LOCALIZATION_DOWNLOAD_PATH,
        ASSET
    )]
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

fn installer_progress(line: &str, current: u8) -> u8 {
    let Some(rest) = line.strip_prefix("[step ") else {
        return current;
    };
    let Some((step, tail)) = rest.split_once('/') else {
        return current;
    };
    let Some((total, _)) = tail.split_once(']') else {
        return current;
    };
    match (step.parse::<u16>(), total.parse::<u16>()) {
        (Ok(step), Ok(total)) if total > 0 && step <= total => {
            current.max((15 + step * 75 / total) as u8)
        }
        _ => current,
    }
}

fn installer_subprogress(line: &str, stage: u8) -> Option<u8> {
    let text = line.strip_prefix("[progress-bar]")?.trim_start();
    let percentage = text
        .split('%')
        .next()?
        .split_whitespace()
        .last()?
        .parse::<u16>()
        .ok()?;
    (percentage <= 100).then(|| stage.saturating_add((percentage * 9 / 100) as u8))
}

async fn execute(
    script: &Path,
    action: &str,
    report: &(dyn Fn(LocalizationProgress) + Send + Sync),
) -> Result<String> {
    let mut command = tokio::process::Command::new("node");
    command
        .arg(script)
        .arg(action)
        .args(if action == "install" {
            Some("--no-relaunch")
        } else {
            None
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command
        .spawn()
        .context("无法启动 Node.js，请先安装 Node.js")?;
    let stdout = child.stdout.take().context("无法读取汉化进度")?;
    let mut stderr = child.stderr.take().context("无法读取汉化错误输出")?;
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });
    let operation = async {
        let mut lines = BufReader::new(stdout).lines();
        let mut percent = 15;
        let mut stage = 15;
        let mut recent = std::collections::VecDeque::with_capacity(12);
        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with("[step ") {
                stage = installer_progress(line, stage);
                percent = percent.max(stage);
            } else if let Some(subprogress) = installer_subprogress(line, stage) {
                if subprogress > percent {
                    percent = subprogress;
                    progress(report, percent, line);
                }
                continue;
            } else if line.starts_with("[progress-bar]") {
                continue;
            }
            progress(report, percent, line);
            if recent.len() == 12 {
                recent.pop_front();
            }
            recent.push_back(line.to_owned());
        }
        let status = child.wait().await?;
        let stderr = stderr_task.await??;
        if !status.success() {
            let reason = String::from_utf8_lossy(&stderr);
            let details = if reason.trim().is_empty() {
                recent.into_iter().collect::<Vec<_>>().join("\n")
            } else {
                reason.trim().to_owned()
            };
            bail!("汉化操作失败：{details}");
        }
        Ok(match action {
            "install" => "汉化副本已安装，请点击“启动汉化版”打开。".into(),
            _ => "英文界面已恢复。".into(),
        })
    };
    tokio::time::timeout(Duration::from_secs(600), operation)
        .await
        .context("汉化操作超时")?
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
        let result = tokio::runtime::Runtime::new().unwrap().block_on(run(
            Path::new("."),
            &http,
            "delete",
            crate::state::SITE_HOST,
            &|_| {},
        ));
        assert!(result.is_err());
    }

    #[test]
    fn relay_download_url_uses_selected_host_only() {
        let urls = relay_asset_urls("https://example.com/");
        assert_eq!(urls.len(), 1);
        assert!(urls[0].starts_with("https://example.com/"));
        assert!(urls.iter().all(|url| url.ends_with(ASSET)));
        assert!(urls.iter().all(|url| !url.contains("github.com")));
    }

    #[test]
    fn finds_launcher_in_release_launchers_directory() {
        let root = std::env::temp_dir().join(format!(
            "jokerdeck-codex-localization-test-{}",
            std::process::id()
        ));
        let launchers = root.join("launchers");
        std::fs::create_dir_all(&launchers).unwrap();
        let launcher = launchers.join("launch-codex-zh-cn.ps1");
        std::fs::write(&launcher, "Write-Output ok").unwrap();

        assert_eq!(
            find_file(&root, "launch-codex-zh-cn.ps1").as_deref(),
            Some(launcher.as_path())
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_compatibility_patch_skips_unsafe_injection_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "jokerdeck-bundle-compatibility-test-{}",
            std::process::id()
        ));
        let scripts = root.join("scripts");
        let resources = root.join("resources");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("menu-hardcoded-zh-CN.json"), "[]").unwrap();
        let script = scripts.join("patch-codex-zh-cn.mjs");
        std::fs::write(
            &script,
            "const insertAt = text.lastIndexOf(\"};export\");\nconst zhPatched = patchZhBundle(zhContent, menuTitleMap);\nconst WEBVIEW_TEXT_PATCHES = [[\"title:`Featured`\", \"title:`精选`\"]];\nfunction patchWebviewBundles(asarPath, replacements = WEBVIEW_TEXT_PATCHES) {\n}\n  const webviewPatchCount = patchWebviewBundles(asarPath);",
        )
        .unwrap();
        patch_bundle_compatibility(&root).unwrap();
        let patched = std::fs::read_to_string(&script).unwrap();
        assert!(patched.contains("const zhPatched = { buffer: zhContent, count: 0 };"));
        assert!(patched.contains("function patchWebviewLocaleGate(asarPath)"));
        assert!(!patched.contains("text.lastIndexOf(\"}}))();export\")"));
        patch_bundle_compatibility(&root).unwrap();
        assert_eq!(std::fs::read_to_string(&script).unwrap(), patched);
        std::fs::write(&script, "unknown").unwrap();
        assert!(patch_bundle_compatibility(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn webview_patch_rejects_unrecognized_global_replacements() {
        let source = r#"const WEBVIEW_TEXT_PATCHES = [["title:`Featured`", "title:`精选`"]];"#;
        assert_eq!(patch_webview_text_replacements(source).unwrap(), source);
        assert!(patch_webview_text_replacements(
            r#"const WEBVIEW_TEXT_PATCHES = [["title:`Featured`", "title:`精选`"], ["未知", "未知"]];"#
        )
        .is_err());
    }

    #[test]
    fn parses_installer_steps_without_regressing_progress() {
        assert_eq!(installer_progress("[step 4/8] 备份原始文件", 15), 52);
        assert_eq!(installer_progress("[step 1/8] 查找目录", 52), 52);
        assert_eq!(installer_progress("[step invalid] 其他信息", 52), 52);
        assert_eq!(
            installer_subprogress("[progress-bar] [##] 50% (5/10)", 71),
            Some(75)
        );
        assert_eq!(installer_subprogress("[progress-bar] | 处理中", 71), None);
    }

    #[test]
    fn native_menu_locale_preserves_existing_entries_and_is_idempotent() {
        let root =
            std::env::temp_dir().join(format!("jokerdeck-native-menu-test-{}", std::process::id()));
        let resources = root.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        let file = resources.join("native-menu-zh-CN.json");
        std::fs::write(&file, r#"{"existing":"保留","windowsMenuBar.file":"File"}"#).unwrap();
        patch_native_menu_locale(&root).unwrap();
        let first = std::fs::read(&file).unwrap();
        let locale: serde_json::Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(locale["existing"], "保留");
        assert_eq!(locale["windowsMenuBar.file"], "文件");
        assert_eq!(locale["electron.appMenu.edit.undo"], "撤销");
        patch_native_menu_locale(&root).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), first);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_menu_detects_stale_and_missing_entries() {
        let root =
            std::env::temp_dir().join(format!("jokerdeck-menu-asar-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let asar_path = root.join("app.asar");
        let expected_path = root.join("menu.json");
        let menu = r#"{"windowsMenuBar.file":"文件"}"#.as_bytes();
        let index = serde_json::json!({
            "files": {"native-menu-locales": {"files": {
                "zh-CN.json": {"offset": "0", "size": menu.len()}
            }}}
        })
        .to_string();
        let mut asar = 4u32.to_le_bytes().to_vec();
        let size = index.len() as u32;
        let padded = (size + 3) & !3;
        asar.extend_from_slice(&(8 + padded).to_le_bytes());
        asar.extend_from_slice(&(4 + padded).to_le_bytes());
        asar.extend_from_slice(&size.to_le_bytes());
        asar.extend_from_slice(index.as_bytes());
        asar.resize(16 + padded as usize, 0);
        asar.extend_from_slice(menu);
        std::fs::write(&asar_path, asar).unwrap();
        std::fs::write(&expected_path, menu).unwrap();
        assert!(menu_matches_in_asar(&asar_path, &expected_path).unwrap());
        std::fs::write(&expected_path, r#"{"windowsMenuBar.file":"File"}"#).unwrap();
        assert!(!menu_matches_in_asar(&asar_path, &expected_path).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
