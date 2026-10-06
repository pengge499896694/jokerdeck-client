use anyhow::{bail, Context, Result};

#[cfg(target_os = "macos")]
const MACOS_LABEL: &str = "cc.jokerdeck.client.proxy-recovery";
#[cfg(windows)]
const WINDOWS_VALUE: &str = "jokerdeck-proxy-recovery";

/// Login startup is needed only while CLI settings reference our ephemeral listener.
pub fn set_enabled(enabled: bool) -> Result<()> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let dir = dirs::home_dir()
            .context("无法定位用户目录")?
            .join("Library/LaunchAgents");
        let path = dir.join(format!("{MACOS_LABEL}.plist"));
        if enabled {
            std::fs::create_dir_all(&dir)?;
            let executable = std::env::current_exe()?;
            let app = executable
                .ancestors()
                .find(|path| path.extension().is_some_and(|ext| ext == "app"))
                .context("无法定位客户端 .app")?;
            let app = xml_escape(&app.to_string_lossy());
            let contents = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\"><dict><key>Label</key><string>{MACOS_LABEL}</string>\
                 <key>ProgramArguments</key><array><string>/usr/bin/open</string>\
                 <string>-a</string><string>{app}</string></array>\
                 <key>RunAtLoad</key><true/></dict></plist>\n"
            );
            std::fs::write(path, contents)?;
        } else if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
    }
    #[cfg(windows)]
    {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let mut command = std::process::Command::new("reg.exe");
        if enabled {
            let executable = std::env::current_exe()?;
            command
                .args(["add", key, "/v", WINDOWS_VALUE, "/t", "REG_SZ", "/d"])
                .arg(format!("\"{}\"", executable.display()))
                .arg("/f");
        } else {
            command.args(["delete", key, "/v", WINDOWS_VALUE, "/f"]);
        }
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let output = command.output().context("无法设置登录启动项")?;
        if !output.status.success() && enabled {
            bail!(
                "无法设置登录启动项：{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
