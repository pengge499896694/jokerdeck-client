更新内容：

- macOS Claude Code 安装改用阿里云 npm 镜像和用户目录，避免安装源返回 HTML 或写入 `/usr/local` 失败。
- 识别 macOS 新版 ChatGPT Desktop 中的 Codex 界面，并单独提示不支持 Codex 汉化的 ChatGPT Classic。
- Codex Desktop 手动下载入口新增 Apple Silicon / Intel 官方 DMG 直链，避免旧文档页面返回 403。
