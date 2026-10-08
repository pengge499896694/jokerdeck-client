//! Opt-in native desktop feature adapter. Transport and approvals stay in the application.
use anyhow::{bail, Result};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
const MARKER: &str = "/*jokerdeck-native-cua-v1*/";
const HOOK: &str = "/*jokerdeck-native-cua-v1*/if(n.JOKERDECK_ENABLE_NATIVE_CUA===`1`&&(r===`win32`||r===`darwin`))e={...e,computerUse:!0,computerUseAutoInstall:!0,computerUseNodeRepl:!0,externalBrowserUse:!0,externalBrowserUseAllowed:!0,inAppBrowserUse:!0,inAppBrowserUseAllowed:!0,browserPane:!0};";
const ANCHOR: &str = "function Er(e,{buildFlavor:t=a.a.resolve(),env:n=S.default.env,platform:r=S.default.platform}={}){let i=";
const ANCHOR_26930: &str = "function ci(e,{buildFlavor:t=d.t.resolve(),env:n=P.default.env,platform:r=P.default.platform}={}){let i=";

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[derive(Serialize)]
pub struct Status {
    pub configured: bool,
    pub platform_supported: bool,
    pub detail: String,
}

pub fn status() -> Result<Status> {
    let supported = cfg!(any(windows, target_os = "macos"));
    Ok(Status { configured: enabled(), platform_supported: supported,
        detail: if !supported { "原生 Computer Use 适配支持 Windows 和 macOS 桌面系统。" }
            else if enabled() { "已保存开启设置；从客户端重启 Codex 后生效。应用接入 Computer Use / Browser 插件、native pipe 和会话元数据；macOS 需授予辅助功能与屏幕录制权限。" }
            else { "可开启原生 Computer Use 与 Browser。启动时核验安装包结构，不兼容的版本会明确报错。" }.into() })
}

pub fn patch_source(source: &str) -> Result<Option<String>> {
    if source.contains(MARKER) {
        if source.matches(MARKER).count() != 1 || source.matches(HOOK).count() != 1 {
            bail!("Computer Use 补丁不完整，请重新创建定制副本");
        }
        return Ok(Some(source.to_owned()));
    }
    let anchors: Vec<_> = [ANCHOR, ANCHOR_26930]
        .into_iter()
        .filter(|anchor| source.contains(anchor))
        .collect();
    if anchors.len() != 1 || source.matches(anchors[0]).count() != 1 {
        if enabled() {
            bail!("此 Codex 版本的 Computer Use 原生启动结构未适配，未修改原应用");
        }
        return Ok(None);
    }
    // Only local feature availability changes; account access and operation authorization stay intact.
    let anchor = anchors[0];
    let replacement = anchor.replace("{let i=", &format!("{{{HOOK}let i="));
    Ok(Some(source.replacen(anchor, &replacement, 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_adapter_is_opt_in_and_preserves_original_logic() {
        let source = format!("{ANCHOR}e;return i}}");
        let patched = patch_source(&source).unwrap().unwrap();
        assert!(patched.contains("n.JOKERDECK_ENABLE_NATIVE_CUA===`1`"));
        assert!(patched.ends_with("let i=e;return i}"));
        assert!(!patched.contains("codexLocalAccess:"));
        assert!(!patched.contains("workCloudAccess:"));
        assert_eq!(patch_source(&patched).unwrap().unwrap(), patched);
        assert!(patch_source(&(source.clone() + &source)).unwrap().is_none());
        assert!(patch_source(&patched.replace("computerUse:!0", "computerUse:!1")).is_err());
    }
}
