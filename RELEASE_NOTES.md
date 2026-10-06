更新内容：

- 修复 macOS 最小化到托盘后无法通过左键或右键菜单恢复窗口。
- macOS 托盘左键直接显示窗口，右键保留菜单，并重新激活应用到前台。
- 修复 Apple Silicon 上的 x64 Rosetta npm 跳过 `darwin-arm64` optional dependency，导致 Claude Code 安装失败。
- 安装脚本返回网页时直接回退 npm，并校验原生包与 Claude Code 命令。
- 工具安装日志右上角新增一键复制按钮，便于提交完整错误日志。
