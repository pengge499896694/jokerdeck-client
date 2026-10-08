//! Add a shortcut to the application's existing deletion confirmation, never a new delete API.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MARKER: &str = "jokerdeck-sidebar-delete-v1";
const HELPER: &str = include_str!("sidebar_delete.js");

fn patch_source(source: &str) -> Result<String> {
    if source.contains(MARKER) {
        if source.matches("function jokerdeckDeleteAction(").count() != 1
            || source
                .matches("jdDeleteAction:jokerdeckDeleteAction(")
                .count()
                != 1
            || source.contains("let m;t[6]!==f||t[7]!==p?(m=[...f,...p]")
        {
            bail!("会话删除补丁不完整，请重新创建修改副本");
        }
        return Ok(source.to_owned());
    }
    if !source.contains("id:`delete-thread`") || !source.contains("DeleteThreadDialog") {
        bail!("此 ChatGPT / Codex 版本未提供可复用的会话删除确认功能");
    }
    // Guard each structural anchor. An application update must never silently target a different action.
    let replacements = [
        ("primaryAction:n,archive:t!=null&&(at||ge)?dt:t,getMenuItems:", "primaryAction:n,jdDeleteAction:jokerdeckDeleteAction(W(`row-actions`).find(item=>item.id===`delete-thread`),L0),archive:t!=null&&(at||ge)?dt:t,getMenuItems:"),
        ("onMenuOpenChange:c,pinAction:l}=e", "onMenuOpenChange:c,pinAction:l,jdDeleteAction}=e"),
        ("let m;t[6]!==f||t[7]!==p?(m=[...f,...p],t[6]=f,t[7]=p,t[8]=m):m=t[8];", "let m=[...f,...p,...jdDeleteAction?[jdDeleteAction]:[]];"),
        ("className:s1,isVisible:u,actions:m", "className:jdDeleteAction?s1.replace(`w-[52px]`,`w-[78px]`):s1,isVisible:u||jdDeleteAction!=null,actions:m"),
        ("Lt=+!!kt+(le&&A?1:0)", "Lt=+!!kt+(le&&A?1:0)+(!oa(R)&&!s&&!se?1:0)"),
    ];
    for (anchor, _) in replacements {
        if source.matches(anchor).count() != 1 {
            bail!("ChatGPT / Codex 会话列表结构已变化，未修改应用；请更新客户端适配");
        }
    }
    let mut patched = source.to_owned();
    for (anchor, replacement) in replacements {
        patched = patched.replacen(anchor, replacement, 1);
    }
    patched.push('\n');
    patched.push_str(HELPER);
    Ok(patched)
}

fn index(data: &[u8]) -> Result<(Value, usize)> {
    if data.len() < 16 {
        bail!("ASAR 头部过短");
    }
    let number = |start| u32::from_le_bytes(data[start..start + 4].try_into().unwrap()) as usize;
    let header_size = number(4);
    let length = number(12);
    let payload = header_size.checked_add(8).context("ASAR 大小溢出")?;
    if number(0) != 4
        || length == 0
        || length > 32 * 1024 * 1024
        || payload > data.len()
        || 16 + length > payload
    {
        bail!("ASAR 头部格式无效");
    }
    Ok((serde_json::from_slice(&data[16..16 + length])?, payload))
}

fn bundle_path(index: &Value, data: &[u8], payload: usize) -> Result<String> {
    let files = index["files"]["webview"]["files"]["assets"]["files"]
        .as_object()
        .context("缺少 WebView assets")?;
    let mut candidates = Vec::new();
    // Updates can move the sidebar into app-shared; identify the actual action owner.
    for (name, entry) in files {
        if !name.ends_with(".js")
            || (!name.starts_with("app-initial-") && !name.starts_with("app-shared-"))
        {
            continue;
        }
        let source = content(data, payload, entry)?;
        if source.contains("id:`delete-thread`") && source.contains("DeleteThreadDialog") {
            candidates.push(name);
        }
    }
    if candidates.len() != 1 {
        bail!("无法唯一定位 ChatGPT 会话界面 bundle");
    }
    Ok(candidates[0].to_string())
}

fn pack(index: &Value, payload: &[u8]) -> Result<Vec<u8>> {
    let header = serde_json::to_vec(index)?;
    let padded = (header.len() + 3) & !3;
    let header_size = u32::try_from(8 + padded)?;
    let mut data = Vec::with_capacity(16 + padded + payload.len());
    data.extend_from_slice(&4u32.to_le_bytes());
    data.extend_from_slice(&header_size.to_le_bytes());
    data.extend_from_slice(&(header_size - 4).to_le_bytes());
    data.extend_from_slice(&u32::try_from(header.len())?.to_le_bytes());
    data.extend_from_slice(&header);
    data.resize(16 + padded, 0);
    data.extend_from_slice(payload);
    Ok(data)
}

fn content<'a>(data: &'a [u8], payload: usize, entry: &Value) -> Result<&'a str> {
    if entry["unpacked"] == true {
        bail!("不支持 unpacked WebView bundle");
    }
    let offset: usize = entry["offset"]
        .as_str()
        .context("bundle offset 无效")?
        .parse()?;
    let len = usize::try_from(entry["size"].as_u64().context("bundle size 无效")?)?;
    let start = payload.checked_add(offset).context("bundle offset 溢出")?;
    let end = start.checked_add(len).context("bundle size 溢出")?;
    Ok(std::str::from_utf8(
        data.get(start..end).context("bundle 超出 ASAR 边界")?,
    )?)
}
fn replace_entry(entry: &mut Value, payload: &mut Vec<u8>, text: &str) -> Result<()> {
    let bytes = text.as_bytes();
    entry["offset"] = json!(payload.len().to_string());
    entry["size"] = json!(bytes.len());
    let block_size = entry["integrity"]["blockSize"]
        .as_u64()
        .unwrap_or(4 * 1024 * 1024);
    if block_size == 0 || block_size > 64 * 1024 * 1024 {
        bail!("ASAR integrity blockSize 无效");
    }
    let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    entry["integrity"] = json!({"algorithm":"SHA256","hash":digest(bytes),"blockSize":block_size,
        "blocks":bytes.chunks(block_size as usize).map(digest).collect::<Vec<_>>()});
    payload.extend_from_slice(bytes);
    Ok(())
}
fn patch_csp(original: &str, origin: &str) -> Result<String> {
    if !origin.starts_with("http://127.0.0.1:") || !origin[17..].bytes().all(|b| b.is_ascii_digit())
    {
        bail!("统计服务地址无效");
    }
    let mut html = original.to_owned();
    const MARKER: &str = "<!-- jokerdeck-bridge-origin:";
    if let Some(start) = html.find(MARKER) {
        let end = start + html[start..].find(" -->").context("旧统计 CSP 标记无效")?;
        let old = html[start + MARKER.len()..end].to_owned();
        html.replace_range(start..end + 4, "");
        html = html.replacen(&format!("connect-src {old} "), "connect-src ", 1);
    }
    if html.matches("connect-src ").count() != 1 {
        bail!("Codex CSP 结构已变化，未放开统计服务访问");
    }
    html = html.replacen("connect-src ", &format!("connect-src {origin} "), 1);
    html.push_str(&format!("{MARKER}{origin} -->"));
    Ok(html)
}
fn patched_archive(data: &[u8]) -> Result<Option<Vec<u8>>> {
    let (mut tree, payload_start) = index(data)?;
    let mut native_main = None;
    if let Some(files) = tree["files"][".vite"]["files"]["build"]["files"].as_object() {
        for (name, entry) in files {
            if !name.starts_with("main-") || !name.ends_with(".js") {
                continue;
            }
            let original = content(data, payload_start, entry)?;
            if let Some(patched) = crate::computer_tools::patch_source(original)? {
                if native_main.is_some() {
                    bail!("无法唯一定位 Computer Use 主进程");
                }
                native_main = Some((name.clone(), patched, original.to_owned()));
            }
        }
    }
    if crate::computer_tools::enabled() && native_main.is_none() {
        bail!("当前安装包未找到可适配的 Computer Use 主进程，请更新版本适配");
    }
    let name = bundle_path(&tree, data, payload_start)?;
    let original = content(
        data,
        payload_start,
        &tree["files"]["webview"]["files"]["assets"]["files"][&name],
    )?;
    let base = original
        .split("\n// jokerdeck-desktop-features-v1")
        .next()
        .unwrap_or(original);
    let patched = format!(
        "{}{}",
        patch_source(&crate::desktop_locale::patch_source(base)?)?,
        crate::desktop_features::runtime_script()
    );
    let html = if let Some(origin) = crate::desktop_features::bridge_origin() {
        let original = content(
            data,
            payload_start,
            &tree["files"]["webview"]["files"]["index.html"],
        )?;
        Some(patch_csp(original, &origin)?)
    } else {
        None
    };
    if patched == original
        && native_main
            .as_ref()
            .is_none_or(|(_, patched, original)| patched == original)
        && html.as_ref().is_none_or(|text| {
            content(
                data,
                payload_start,
                &tree["files"]["webview"]["files"]["index.html"],
            )
            .ok()
                == Some(text.as_str())
        })
    {
        return Ok(None);
    }
    let base_length = tree["jokerdeck_payload_base"]
        .as_u64()
        .map(usize::try_from)
        .transpose()?
        .unwrap_or(data.len() - payload_start);
    if base_length > data.len() - payload_start {
        bail!("修改副本的 ASAR payload 边界无效");
    }
    tree["jokerdeck_payload_base"] = json!(base_length);
    let mut payload = data[payload_start..payload_start + base_length].to_vec();
    if let Some(html) = html {
        replace_entry(
            &mut tree["files"]["webview"]["files"]["index.html"],
            &mut payload,
            &html,
        )?;
    }
    replace_entry(
        &mut tree["files"]["webview"]["files"]["assets"]["files"][&name],
        &mut payload,
        &patched,
    )?;
    if let Some((name, patched, _)) = native_main {
        replace_entry(
            &mut tree["files"][".vite"]["files"]["build"]["files"][&name],
            &mut payload,
            &patched,
        )?;
    }
    Ok(Some(pack(&tree, &payload)?))
}

/// Only call on a managed application copy after stopping its processes.
pub(crate) async fn apply(archive: PathBuf) -> Result<bool> {
    tokio::task::spawn_blocking(move || {
        let data = fs::read(&archive).with_context(|| format!("无法读取 {}", archive.display()))?;
        let Some(patched) = patched_archive(&data)? else {
            return Ok(false);
        };
        let backup = archive.with_extension("asar.jokerdeck-delete-backup");
        if !backup.exists() {
            fs::copy(&archive, &backup)?;
        }
        let staging = archive.with_extension("asar.jokerdeck-delete-tmp");
        fs::write(&staging, patched)?;
        fs::rename(&staging, &archive).context("无法写入会话删除补丁")?;
        Ok(true)
    })
    .await?
}

pub(crate) async fn macos_copy(original: &Path) -> Result<PathBuf> {
    let archive = original.join("Contents/Resources/app.asar");
    // ASAR 可能有数百 MB，Intel Mac 上读盘和哈希不能阻塞异步线程。
    let version = tokio::task::spawn_blocking(move || -> Result<String> {
        let bytes = fs::read(&archive)?;
        let (tree, _) = index(&bytes)?;
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&tree)?)))
    })
    .await??;
    let archive = original.join("Contents/Resources/app.asar");
    let root = dirs::data_local_dir()
        .context("无法定位应用目录")?
        .join("jokerdeck/desktop-customizations")
        .join(&version[..16]);
    let app = root.join(original.file_name().context("应用名称无效")?);
    let ready = root.join(MARKER);
    fs::create_dir_all(&root)?;
    if ready.is_file() {
        // Refresh only the archive; the original binary and other resources are unchanged.
        let destination = app.join("Contents/Resources/app.asar");
        tokio::task::spawn_blocking(move || fs::copy(archive, destination)).await??;
    } else {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            tokio::process::Command::new("/usr/bin/ditto")
                .arg(original)
                .arg(&app)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("创建 ChatGPT 修改副本超时")??;
        if !output.status.success() {
            bail!(
                "无法创建 ChatGPT 修改副本：{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    apply(app.join("Contents/Resources/app.asar")).await?;
    // Electron validates the ASAR header separately from macOS code signing.
    let signed_archive = app.join("Contents/Resources/app.asar");
    let hash = tokio::task::spawn_blocking(move || {
        crate::codex_desktop::asar_header_hash(&signed_archive)
    })
    .await??;
    let plist = app.join("Contents/Info.plist");
    let integrity = tokio::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :ElectronAsarIntegrity"])
        .arg(&plist)
        .output()
        .await?;
    if integrity.status.success() {
        let mut updated = false;
        for path in ["Resources/app.asar", "resources/app.asar"] {
            let key = format!(":ElectronAsarIntegrity:{path}:hash");
            let probe = tokio::process::Command::new("/usr/libexec/PlistBuddy")
                .args(["-c", &format!("Print {key}")])
                .arg(&plist)
                .output()
                .await?;
            if probe.status.success() {
                let output = tokio::process::Command::new("/usr/libexec/PlistBuddy")
                    .args(["-c", &format!("Set {key} {hash}")])
                    .arg(&plist)
                    .output()
                    .await?;
                if !output.status.success() {
                    bail!("无法更新 ChatGPT 副本的 ASAR integrity");
                }
                updated = true;
            }
        }
        if !updated {
            bail!("ChatGPT 副本的 ASAR integrity 格式已变化，未启动修改版本");
        }
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new("/usr/bin/codesign")
            .args([
                "--force",
                // Nested native services are unchanged: keep their original signatures.
                "--preserve-metadata=entitlements,flags,runtime",
                "--sign",
                "-",
            ])
            .arg(&app)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("ChatGPT 修改副本签名超时")??;
    if !output.status.success() {
        bail!(
            "ChatGPT 副本签名失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(ready, b"v1")?;
    Ok(app)
}

pub(crate) async fn windows_copy() -> Result<PathBuf> {
    let detected = crate::cli_manager::detect_codex_desktop().await;
    let path = PathBuf::from(detected.path.context("未找到官方 Codex Desktop 安装目录")?);
    let base = if path.is_file() {
        path.parent().context("安装目录无效")?.to_owned()
    } else {
        path
    };
    let source = [base.clone(), base.join("app")]
        .into_iter()
        .find(|root| root.join("resources/app.asar").is_file())
        .context("官方 Codex 安装目录缺少 app.asar")?;
    let app = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let hash = crate::codex_desktop::asar_header_hash(&source.join("resources/app.asar"))?;
        let root = dirs::data_local_dir()
            .context("无法定位本地应用目录")?
            .join("jokerdeck/desktop-customizations")
            .join(&hash[..16]);
        let app = root.join("app");
        let ready = root.join("copy-ready-v1");
        if !ready.is_file() {
            copy_tree(&source, &app, 0)?;
            fs::write(ready, b"v1")?;
        }
        Ok(app)
    })
    .await??;
    apply(app.join("resources/app.asar")).await?;
    let repair_app = app.clone();
    tokio::task::spawn_blocking(move || crate::codex_desktop::repair_windows_exes(&repair_app))
        .await??;
    Ok(app)
}

fn copy_tree(source: &Path, destination: &Path, depth: usize) -> Result<()> {
    if depth > 32 {
        bail!("Codex 安装目录层级异常");
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            bail!("安装目录包含链接，未创建修改副本");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                bail!("安装目录包含 reparse point，未创建修改副本");
            }
        }
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &target, depth + 1)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> String {
        [
            "id:`delete-thread`;DeleteThreadDialog;",
            "c=s?.get(`enable_i18n`,!1),t[0]=s,t[1]=c",
            "primaryAction:n,archive:t!=null&&(at||ge)?dt:t,getMenuItems:",
            "onMenuOpenChange:c,pinAction:l}=e",
            "let m;t[6]!==f||t[7]!==p?(m=[...f,...p],t[6]=f,t[7]=p,t[8]=m):m=t[8];",
            "className:s1,isVisible:u,actions:m",
            "Lt=+!!kt+(le&&A?1:0)",
        ]
        .join("\n")
    }
    #[test]
    #[ignore = "Requires a local official bundle; writes only a review artifact"]
    fn verifies_installed_bundle_without_modifying_it() {
        let path = std::env::var_os("JOKERDECK_SIDEBAR_TEST_ASAR").expect("ASAR path required");
        if std::env::var_os("JOKERDECK_FEATURES_TEST_BRIDGE").is_some() {
            crate::desktop_features::enable_test_bridge();
        }
        let data = fs::read(path).unwrap();
        let result = patched_archive(&data)
            .unwrap()
            .expect("fresh source bundle required");
        let (tree, start) = index(&result).unwrap();
        let name = bundle_path(&tree, &result, start).unwrap();
        let entry = &tree["files"]["webview"]["files"]["assets"]["files"][&name];
        let offset = entry["offset"].as_str().unwrap().parse::<usize>().unwrap();
        let length = entry["size"].as_u64().unwrap() as usize;
        let output =
            std::env::var_os("JOKERDECK_SIDEBAR_TEST_OUTPUT").expect("output path required");
        fs::write(output, &result[start + offset..start + offset + length]).unwrap();
        if let Some(files) = tree["files"][".vite"]["files"]["build"]["files"].as_object() {
            for (name, entry) in files {
                if name.starts_with("main-") && name.ends_with(".js") {
                    let source = content(&result, start, entry).unwrap();
                    assert!(source.contains("/*jokerdeck-native-cua-v1*/"));
                    if let Some(output) = std::env::var_os("JOKERDECK_CUA_TEST_OUTPUT") {
                        fs::write(output, source).unwrap();
                    }
                }
            }
        }
        assert!(patched_archive(&result).unwrap().is_none());
    }
    #[test]
    fn csp_allows_only_current_loopback_bridge() {
        let original="<meta content=\"default-src 'none'; connect-src 'self' https://chatgpt.com; style-src 'self' 'unsafe-inline'\">";
        let first = patch_csp(original, "http://127.0.0.1:32000").unwrap();
        let second = patch_csp(&first, "http://127.0.0.1:33000").unwrap();
        assert!(!second.contains("32000"));
        assert!(second.contains("33000"));
        assert!(second.contains("default-src 'none'"));
        assert!(!second.contains("http://*"));
        assert!(patch_csp(original, "https://evil.example").is_err());
    }
    #[test]
    fn guards_versions_and_is_idempotent() {
        let patched = patch_source(&source()).unwrap();
        assert_eq!(patch_source(&patched).unwrap(), patched);
        assert!(patch_source(&source().replace("pinAction:l", "pinAction:changed")).is_err());
        assert!(patch_source(&(source() + MARKER)).is_err());
        assert!(patch_source(&(source() + "Lt=+!!kt+(le&&A?1:0)")).is_err());
    }
    #[test]
    fn preserves_other_assets_and_updates_integrity() {
        let source = source();
        let payload = format!("keep{source}");
        let tree = json!({"files":{"webview":{"files":{"assets":{"files":{
            "app-initial-test.js":{"offset":"4","size":source.len()},
            "other.js":{"offset":"0","size":4}
        }}}}}});
        let original = pack(&tree, payload.as_bytes()).unwrap();
        let result = patched_archive(&original).unwrap().unwrap();
        let (tree, start) = index(&result).unwrap();
        assert_eq!(&result[start..start + 4], b"keep");
        let entry = &tree["files"]["webview"]["files"]["assets"]["files"]["app-initial-test.js"];
        let offset = entry["offset"].as_str().unwrap().parse::<usize>().unwrap();
        let content = &result[start + offset..];
        assert_eq!(
            entry["integrity"]["hash"],
            format!("{:x}", Sha256::digest(content))
        );
        assert!(patched_archive(&result).unwrap().is_none());
        assert!(patched_archive(&original[..20]).is_err());
    }
}
