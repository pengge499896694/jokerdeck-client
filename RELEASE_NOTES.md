更新内容：

- Codex Desktop 下载改为客户端内直接安装：macOS 流式下载官方 DMG，Windows 使用 Microsoft Store 官方包。
- 下载与安装过程使用客户端进度条和详情日志展示，不再打开官网页面。
- 修复 macOS 站点 WebView 覆盖关闭确认框的问题。
- Homebrew 官方脚本连接中断时自动切换 CDN 下载源。

- macOS 安装 Node.js 时先检测 Homebrew；未安装则自动下载安装并重新检测。
- Homebrew 安装失败时停止后续步骤，并在客户端显示具体错误。
