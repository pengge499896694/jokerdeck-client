import { invoke } from "@tauri-apps/api/core";

export interface UserInfo {
  id: number;
  email: string;
  balance: number;
  frozen_balance: number;
  total_recharged: number;
  allowed_groups: number[];
  role: string;
  status: string;
}

export interface LoginResult {
  requires_2fa: boolean;
  temp_token?: string;
  email_masked?: string;
  user?: UserInfo;
}

export interface Group {
  id: number;
  name: string;
  multiplier?: number;
}

export interface HostHealth {
  host: string;
  healthy: boolean;
  latency_ms?: number;
}

export interface GroupKey {
  group_id: number;
  name: string;
  multiplier?: number;
}

export interface ProxyStatus {
  running: boolean;
  port: number;
  base_url: string;
  active_host?: string;
  preferred_host?: string;
  hosts: HostHealth[];
  groups: GroupKey[];
  active_group?: GroupKey;
  auto_fallback: boolean;
}

export interface ApplyResult {
  proxy_port: number;
  base_url: string;
  groups: number;
  files: string[];
  active_group_id?: number;
  warnings: string[];
}

export interface RestoreConfigResult {
  files: string[];
  warnings: string[];
  status: ProxyStatus;
}

/** One model inside a group. */
export interface PlazaModel {
  name: string;
  platform: string;
}
/** A group the user can use, with the models it serves. */
export interface PlazaGroup {
  id: number;
  name: string;
  platform: string;
  description: string;
  multiplier?: number;
  is_exclusive?: boolean;
  models: PlazaModel[];
}

export interface GroupModels {
  group_id: number;
  claude_models: string[];
  codex_models: string[];
  codex_error?: string;
}

export interface ModelProbe {
  ok: boolean;
  detail: string;
}

export interface ToolConfigView {
  path: string;
  content: string;
}

export interface CliStatus {
  installed: boolean;
  version?: string;
  path?: string;
}
export interface CliReport {
  node: CliStatus;
  npm: CliStatus;
  claude: CliStatus;
  codex: CliStatus;
  codex_desktop: CliStatus;
}
export interface InstallResult {
  ok: boolean;
  log: string;
}

export interface CodexSession {
  id: string;
  title: string;
  updated_at: number;
  size: number;
}
export interface CodexSessionDetail {
  messages: { role: "user" | "assistant"; text: string }[];
  truncated: boolean;
}
export interface CodexEnhancementStatus {
  plugins_enabled: boolean;
  cache_available: boolean;
  cache_registered: boolean;
  cached_plugins: number;
  model_catalog_active: boolean;
  model_count: number;
}

export interface DiagItem {
  name: string;
  ok: boolean;
  detail: string;
}
export interface DiagReport {
  items: DiagItem[];
  overall_ok: boolean;
}

export interface UpdateInfo {
  current: string;
  latest?: string;
  update_available: boolean;
  url?: string;
  notes?: string;
  error?: string;
}

export interface Bootstrap {
  logged_in: boolean;
  last_email?: string;
  user?: UserInfo;
  version: string;
  /** Persisted choices, so the UI can prefill its pickers. */
  preferred_group_id?: number;
  claude_model?: string;
  codex_model?: string;
  computer_use: boolean;
  auto_fallback: boolean;
  saved_password?: string;
  remember_password: boolean;
  site_url: string;
  desktop_supported: boolean;
}

export interface PublicAuthSettings {
  registration_enabled: boolean;
  email_verify_enabled: boolean;
  registration_email_suffix_whitelist?: string[];
  registration_email_domain_quota_enabled?: boolean;
  password_reset_enabled?: boolean;
  promo_code_enabled?: boolean;
  invitation_code_enabled?: boolean;
  turnstile_enabled: boolean;
  turnstile_site_key?: string;
  tencent_captcha_enabled: boolean;
  aliyun_captcha_enabled: boolean;
}

/** `invoke` only exists inside the Tauri runtime. */
const IN_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in (window as any);

/**
 * Call a Tauri command. Outside the Tauri runtime (previewing the UI with
 * `pnpm dev` in a browser) we surface a plain-language message instead of an
 * opaque "Cannot read properties of undefined (reading 'invoke')".
 */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IN_TAURI) {
    throw new Error(
      "当前是浏览器预览模式，只能查看界面。登录、余额、一键配置等功能需要客户端环境：" +
        "请用 `pnpm tauri dev` 启动，或安装客户端后再试。"
    );
  }
  return invoke<T>(cmd, args);
}

/** True when running in a plain browser — UI is viewable but nothing works. */
export const isPreview = !IN_TAURI;

export const api = {
  bootstrap: () => call<Bootstrap>("get_bootstrap"),
  login: (email: string, password: string) => call<LoginResult>("login", { email, password }),
  publicAuthSettings: () => call<PublicAuthSettings>("public_auth_settings"),
  register: (payload: { email: string; password: string; verify_code?: string; promo_code?: string; invitation_code?: string; aff_code?: string }) =>
    call<LoginResult>("register", {
      email: payload.email, password: payload.password, verifyCode: payload.verify_code,
      promoCode: payload.promo_code, invitationCode: payload.invitation_code, affCode: payload.aff_code,
    }),
  sendVerifyCode: (email: string) => call<{ message: string; countdown: number }>("send_verify_code", { email }),
  forgotPassword: (email: string) => call<string>("forgot_password", { email }),
  resetPassword: (email: string, token: string, new_password: string) =>
    call<string>("reset_password", { email, token, newPassword: new_password }),
  submit2fa: (temp_token: string, totp_code: string) =>
    call<UserInfo>("submit_2fa", { tempToken: temp_token, totpCode: totp_code }),
  logout: () => call<void>("logout"),
  account: () => call<UserInfo>("get_account"),
  groups: () => call<Group[]>("list_groups"),
  /** Groups + their models, straight from the relay's model plaza. */
  plaza: () => call<PlazaGroup[]>("fetch_plaza"),
  groupModels: (groupId: number) => call<GroupModels>("fetch_group_models", { groupId }),
  testGroupModel: (groupId: number, tool: "claude" | "codex", model: string) =>
    call<ModelProbe>("test_group_model", { groupId, tool, model }),
  readToolConfig: (which: "claude" | "codex" | "catalog") =>
    call<ToolConfigView>("read_tool_config", { which }),
  applyConfig: (configure_claude: boolean, configure_codex: boolean, group_id?: number, claude_model?: string, codex_model?: string) =>
    call<ApplyResult>("apply_config", { configureClaude: configure_claude, configureCodex: configure_codex, groupId: group_id, claudeModel: claude_model, codexModel: codex_model }),
  proxyStatus: () => call<ProxyStatus>("proxy_status"),
  stopProxy: () => call<ProxyStatus>("stop_proxy"),
  restoreConfig: () => call<RestoreConfigResult>("restore_config"),
  quitApp: (restore: boolean) => call<void>("quit_app", { restore }),
  probeHosts: () => call<HostHealth[]>("probe_hosts"),
  setPreferredHost: (host?: string) => call<ProxyStatus>("set_preferred_host", { host }),
  setAutoFallback: (enabled: boolean) => call<void>("set_auto_fallback", { enabled }),
  setExtensions: (computer_use: boolean) => call<void>("set_extensions", { computerUse: computer_use }),
  detectClis: () => call<CliReport>("detect_clis"),
  installCli: (which: string) => call<InstallResult>("install_cli", { which }),
  restartCodex: () => call<InstallResult>("restart_codex"),
  codexLocalization: (action: "install" | "uninstall" | "launch") =>
    call<string>("codex_localization", { action }),
  codexSessions: () => call<CodexSession[]>("list_codex_sessions"),
  codexSession: (id: string) => call<CodexSessionDetail>("read_codex_session", { id }),
  deleteCodexSession: (id: string) => call<void>("delete_codex_session", { id }),
  codexEnhancementStatus: () => call<CodexEnhancementStatus>("codex_enhancement_status"),
  enableCodexMarketplace: () => call<string>("enable_codex_marketplace"),
  registerCodexPluginCache: () => call<string>("register_codex_plugin_cache"),
  diagnostics: () => call<DiagReport>("run_diagnostics"),
  checkUpdate: () => call<UpdateInfo>("check_update"),
  installUpdate: (url: string) => call<{ ok: boolean; message: string }>("install_update", { url }),
  openUrl: (url: string) => call<void>("open_url", { url }),
  openDevtools: () => call<void>("open_devtools"),
  openSite: (page: string) => call<void>("open_site", { page }),
  showSite: (page: string, bounds: { x: number; y: number; width: number; height: number }, navigate: boolean) =>
    call<void>("show_site", { page, ...bounds, navigate }),
  hideSite: () => call<void>("hide_site"),
  closeSite: () => call<void>("close_site"),
  saveLogin: (email: string, password: string, remember: boolean) =>
    call<void>("save_login", { email, password, remember }),
  forgetPassword: () => call<void>("forget_password"),
  setSiteUrl: (url: string) => call<void>("set_site_url", { url }),
};
