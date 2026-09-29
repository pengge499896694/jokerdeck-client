# jokerdeck 客户端

基于 Tauri 2 + React 的桌面客户端。登录后可选择分组与模型，创建或复用分组专用 Key，启动本地代理，并配置 Claude Code、Codex CLI 与 Codex Desktop。

## 开发

需要 Node.js、pnpm、Rust 和 Windows WebView2。

```powershell
pnpm install
pnpm tauri dev
```

构建 Windows 安装包：

```powershell
pnpm tauri build
```

仅运行 `pnpm dev` 是浏览器预览，无法调用 Tauri 的登录与本地配置功能。使用与配置说明见 [使用说明.md](使用说明.md)。

仓库仅包含客户端源码与必要的图标资源。依赖目录、构建输出和安装包不会提交。
