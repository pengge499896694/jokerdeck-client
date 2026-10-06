更新内容：

- macOS Claude Code 改用官方 native installer，绕过 npm optional native binary 缺失。
- macOS Codex 汉化和恢复英文后通过系统应用启动流程重启，并确认进程已运行。
- 关闭本地代理时同步恢复 CLI 配置；启用代理时注册登录启动项，在下次登录时自动清理异常退出留下的失效本地代理地址。
