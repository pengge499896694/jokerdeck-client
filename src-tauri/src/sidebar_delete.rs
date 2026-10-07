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

fn bundle_path(index: &Value) -> Result<String> {
    let files = index["files"]["webview"]["files"]["assets"]["files"]
        .as_object()
        .context("缺少 WebView assets")?;
    let candidates: Vec<_> = files
        .keys()
        .filter(|name| name.starts_with("app-initial-") && name.ends_with(".js"))
        .collect();
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
    let name = bundle_path(&tree)?;
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
        patch_source(base)?,
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
    let bytes = fs::read(&archive)?;
    let (tree, _) = index(&bytes)?;
    let version = format!("{:x}", Sha256::digest(serde_json::to_vec(&tree)?));
    let root = dirs::data_local_dir()
        .context("无法定位应用目录")?
        .join("jokerdeck/desktop-customizations")
        .join(&version[..16]);
    let app = root.join(original.file_name().context("应用名称无效")?);
    let ready = root.join(MARKER);
    fs::create_dir_all(&root)?;
    if ready.is_file() {
        // Refresh only the archive; the original binary and other resources are unchanged.
        std::fs::copy(&archive, app.join("Contents/Resources/app.asar"))?;
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
    let hash = crate::codex_desktop::asar_header_hash(&app.join("Contents/Resources/app.asar"))?;
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
                "--deep",
                "--preserve-metadata=entitlements,requirements,flags,runtime",
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

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> String {
        [
            "id:`delete-thread`;DeleteThreadDialog;",
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
        let name = bundle_path(&tree).unwrap();
        let entry = &tree["files"]["webview"]["files"]["assets"]["files"][&name];
        let offset = entry["offset"].as_str().unwrap().parse::<usize>().unwrap();
        let length = entry["size"].as_u64().unwrap() as usize;
        let output =
            std::env::var_os("JOKERDECK_SIDEBAR_TEST_OUTPUT").expect("output path required");
        fs::write(output, &result[start + offset..start + offset + length]).unwrap();
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
