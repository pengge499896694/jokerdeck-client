更新内容：

- 修复 macOS 上 Claude Code 和 Codex CLI 使用全局 npm 安装时遇到的 `/usr/local` 权限错误：Claude Code 改用官方原生安装器，Codex CLI 安装到用户目录。
- 修复 Codex Desktop 官方下载源的 TLS 连接问题；下载失败时显示错误和官方手动下载入口。
- 支持检测安装在 `~/Applications` 的 Codex.app，并明确区分 ChatGPT.app 与 Codex.app，避免错误启用 Codex 汉化。
