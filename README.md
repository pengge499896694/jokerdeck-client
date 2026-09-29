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

## 发布

同步更新 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 的版本号，提交并推送到 `main`，然后推送相同版本的 tag：

```powershell
git tag v0.1.1
git push origin main
git push origin v0.1.1
```

GitHub Actions 会在 tag 推送后并行测试、构建 Windows x64 NSIS、macOS Intel DMG、macOS Apple Silicon DMG。三个构建全部成功才发布含安装包及 `latest.json` 的 GitHub Release。普通代码推送不会发版。客户端将从最新 Release 检查更新；目前没有 Windows 代码签名或 Apple 签名与公证，系统可能提示未验证开发者。
