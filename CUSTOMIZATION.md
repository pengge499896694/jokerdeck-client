# jokerdeck-chatgpt 定制能力

## 使用

- 管理端：系统设置 → 用户默认值 → 客户端服务商权限。默认锁定；开放后仅有充值记录的用户可切换。后端与客户端需一起更新。
- 客户端：工具与修复 → 服务商管理。外部服务商要求 Responses API，填写 Base URL、API Key、Model。先关闭 Codex。回到配置页应用配置可恢复默认服务商。
- 客户端：工具与修复 → 原生 Computer Use / Browser → 开启。随后使用客户端的启动按钮启动定制副本。
- 定制副本：会话行增加删除按钮，使用原应用的确认对话框；原生 macOS ChatGPT Classic 与 chatgpt.com 网页不属于该补丁目标。

## 实现范围

Computer Use 与 Browser 使用原应用的插件、服务管理和 native pipe，不注册缺少会话元数据的独立 MCP。仅修改本地功能可用性；原生操作审批、账户访问校验、服务端校验与 macOS 系统授权保留。浏览器扩展需要安装并连接，macOS 需要辅助功能、屏幕录制权限。

Windows/macOS 启动独立修改副本，不覆盖官方安装包。ASAR 与可执行文件 integrity 更新后，macOS 副本重新签名。未知安装包结构明确拒绝修改，应用升级后可能需要新的适配。已验证 Windows 26.930.3748 包；macOS Intel/ARM 需要对应系统的 CI 构建及真机验收，不能将本机 Windows 测试等同于跨平台操作测试。

服务商切换保持 `model_provider = "jokerdeck"` 与同一个 CODEX_HOME，不改 auth.json，不迁移或删除历史。原有其他 provider 的历史可使用已有 provider-sync 工具另行同步。外部服务商不提供售后、不在自有中转账单中估算费用。

Token 来自本地 session usage，费用来自中转实际账单。最多分页读取 2,000 条，查询总时限 8 秒；不完整费用标记为近似值，外部服务商不展示中转费用。

客户端只监听 localhost，不修改系统代理、DNS 或路由。Intel Mac 的状态刷新不再持 Store 锁同步访问 Keychain 或系统进程，并限制阻塞探测时间和并发；代理运行后卡死的问题仍需真机复测。

## 验证

- `cargo test --locked --lib`（src-tauri）
- `pnpm build`
- `go test -tags unit ./internal/service -run '^TestClientProvider'`（source/backend）
- `.github/workflows/compatibility.yml`：Windows、macOS Intel、macOS ARM 编译和测试。
- 本地安装包验证：设置 `JOKERDECK_SIDEBAR_TEST_ASAR`、`JOKERDECK_SIDEBAR_TEST_OUTPUT`、`JOKERDECK_CUA_TEST_OUTPUT`，执行 ignored test `verifies_installed_bundle_without_modifying_it`。仅生成审阅 JS，不改安装包。随后执行 `node --check` 及 `node tools/verify-native-cua.cjs <main-js>`。

Codex++ 原生 Browser 扩展识别兼容代码保留 AGPL-3.0-only 来源与许可证；该上游模块只支持 Windows。Mac 的原生功能接入属于本项目适配，并非上游已有功能的一比一拷贝。具体来源见 CODEXPLUSPLUS-NOTICE.md。
