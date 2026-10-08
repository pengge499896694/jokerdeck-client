import { useEffect, useLayoutEffect, useRef, useState, type FormEvent, type MouseEvent as ReactMouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ArrowRight, Check, ChevronRight, CircleHelp, Copy, Download, Eye, EyeOff,
  KeyRound, LayoutDashboard, LogOut, Maximize2, Minimize2, Minus, Monitor, RefreshCw, CreditCard,
  ChartNoAxesCombined, Store, History, Search, Sparkles,
  Settings2, ShieldCheck, Stethoscope, UserRound, Wallet, X, Power, RotateCcw, Bell, Users, Folder, Globe, Receipt, SlidersHorizontal,
} from "lucide-react";
import { api, isPreview, type Bootstrap, type UserInfo, type ProxyStatus, type HostHealth, type CliReport,
  type DiagReport, type PlazaGroup, type ApplyResult, type PublicAuthSettings,
  type GroupModels, type ModelProbe, type ToolConfigView, type UpdateInfo,
  type CodexSession, type CodexSessionDetail, type DesktopFeatureStatus, type ProviderSyncResult, type CodexEnhancementStatus, type UpdateProgress } from "./api";

type SiteTab = typeof allSiteTabs[number]["id"];
type Tab = "setup" | "tools" | "computer" | "providers" | "sessions" | "enhance" | "account" | SiteTab;
type AuthPage = "register" | "forgot-password" | "reset-password";
type CodexPreset = {
  claude: boolean; codex: boolean; groupId?: number; claudeModel?: string; codexModel?: string;
  localization: boolean; computerUse: boolean; browser: boolean;
};
const logo = new URL("../icon-source.png", import.meta.url).href;
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const formatBytes = (value: number) => {
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
};
const tabs = [
  { id: "setup", title: "控制台", icon: LayoutDashboard },
  { id: "providers", title: "服务商管理", icon: Globe },
  { id: "computer", title: "电脑与浏览器", icon: Monitor },
  { id: "tools", title: "工具与修复", icon: Stethoscope },
  { id: "sessions", title: "会话管理", icon: History },
  { id: "enhance", title: "Codex 增强", icon: Sparkles },
  { id: "account", title: "账户中心", icon: UserRound },
] as const;
const siteTabs = [
  { id: "dashboard", title: "网站概览", icon: LayoutDashboard },
  { id: "keys", title: "API Key", icon: KeyRound },
  { id: "usage", title: "使用记录", icon: ChartNoAxesCombined },
  { id: "self-monitor", title: "自助渠道监控", icon: Stethoscope },
  { id: "subscriptions", title: "我的订阅", icon: CreditCard },
  { id: "store", title: "店铺销售", icon: Store },
  { id: "profile", title: "账户与安全", icon: UserRound },
  { id: "balance-notifications", title: "余额邮箱提醒", icon: Bell },
  { id: "purchase", title: "在线充值", icon: Wallet },
] as const;
const adminTabs = [
  { id: "admin-dashboard", title: "管理概览", icon: LayoutDashboard },
  { id: "admin-users", title: "用户管理", icon: Users },
  { id: "admin-groups", title: "分组管理", icon: Folder },
  { id: "admin-channels", title: "渠道管理", icon: Globe },
  { id: "admin-accounts", title: "上游账户", icon: KeyRound },
  { id: "admin-subscriptions", title: "订阅管理", icon: CreditCard },
  { id: "admin-orders", title: "支付订单", icon: Receipt },
  { id: "admin-redeem", title: "兑换码", icon: KeyRound },
  { id: "admin-usage", title: "用量记录", icon: ChartNoAxesCombined },
  { id: "admin-feedback", title: "用户反馈", icon: Bell },
  { id: "admin-settings", title: "系统设置", icon: SlidersHorizontal },
] as const;
const financeTab = { id: "finance", title: "财务中心", icon: ChartNoAxesCombined } as const;
const allSiteTabs = [...siteTabs, ...adminTabs, financeTab];

function TitleBar({ flash, onClose }: { flash: (text: string) => void; onClose: () => void }) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (isPreview) return;
    const current = getCurrentWindow();
    current.isMaximized().then(setMaximized).catch(() => {});
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    current.onResized(() => current.isMaximized().then(setMaximized).catch(() => {}))
      .then((off) => { if (disposed) off(); else unsubscribe = off; }).catch(() => {});
    return () => { disposed = true; unsubscribe?.(); };
  }, []);
  const drag = (event: ReactMouseEvent<HTMLElement>) => {
    if (isPreview || event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
    getCurrentWindow().startDragging().catch((err) => flash(message(err)));
  };
  const toggleMaximize = () => getCurrentWindow().toggleMaximize()
    .then(() => getCurrentWindow().isMaximized()).then(setMaximized).catch((err) => flash(message(err)));
  return <header className="titlebar" onMouseDown={drag} onDoubleClick={isPreview ? undefined :
    (event) => { if (!(event.target as HTMLElement).closest("button")) toggleMaximize(); }}>
    <span className="titlebar-brand"><img src={logo} alt="" />jokerdeck-chatgpt</span>
    <span className="titlebar-status">{isPreview ? "界面预览" : "客户端"}</span>
    {!isPreview && <div className="window-controls">
      <button className="icon-button window-control" title="最小化到托盘" aria-label="最小化到托盘"
        onClick={() => getCurrentWindow().hide().catch((err) => flash(message(err)))}><Minus size={17} /></button>
      <button className="icon-button window-control" title={maximized ? "还原窗口" : "最大化"}
        aria-label={maximized ? "还原窗口" : "最大化"} onClick={toggleMaximize}>
        {maximized ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</button>
      <button className="icon-button window-control close" title="关闭窗口" aria-label="关闭窗口"
        onClick={onClose}><X size={17} /></button>
    </div>}
  </header>;
}

export default function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [ready, setReady] = useState(false);
  const [user, setUser] = useState<UserInfo | null>(null);
  const [authPage, setAuthPage] = useState<AuthPage | null>(null);
  const [resetLink, setResetLink] = useState("");
  const [resetCredentials, setResetCredentials] = useState<{ email: string; token: string }>();
  const [tab, setTab] = useState<Tab>("enhance");
  const [error, setError] = useState("");
  const [toast, setToast] = useState("");
  const [closePrompt, setClosePrompt] = useState(false);
  const [closing, setClosing] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress>({ downloaded: 0 });
  const [restarting, setRestarting] = useState(false);
  const [localeProgress, setLocaleProgress] = useState<{ percent: number; detail: string } | null>(null);
  const [restoreLocaleConfirm, setRestoreLocaleConfirm] = useState(false);
  const desktopName = navigator.platform.toLowerCase().includes("mac") ? "ChatGPT" : "Codex";
  const siteArea = useRef<HTMLDivElement>(null);
  const timer = useRef<number>();
  const flash = (text: string) => {
    setToast(text);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setToast(""), 6500);
  };
  const requestClose = async () => {
    if (closing) return;
    try {
      if ((await api.proxyStatus()).running) {
        await api.hideSite();
        setTab("setup");
        setClosePrompt(true);
      } else await api.quitApp(false);
    } catch (err) { flash(message(err)); }
  };
  const confirmClose = async () => {
    setClosing(true);
    try { await api.quitApp(true); }
    catch (err) { flash(message(err)); setClosing(false); }
  };
  const installUpdate = async () => {
    if (!update?.url) return;
    setUpdateInstalling(true);
    setUpdateProgress({ downloaded: 0 });
    try {
      const result = await api.installUpdate(update.url);
      setUpdate(null);
      flash(result.message);
    } catch (err) { flash(message(err)); }
    finally { setUpdateInstalling(false); }
  };
  const restartCodex = async (action: "launch" | "uninstall" = "launch") => {
    if (restarting) return;
    setRestoreLocaleConfirm(false);
    setRestarting(true);
    setLocaleProgress({ percent: 0, detail: action === "launch" ? "正在检测中文支持" : "正在恢复英文版本" });
    let off: (() => void) | undefined;
    try {
      // Register before invoking so fast native stages are not lost.
      off = await listen<{ percent: number; detail: string }>("codex-localization-progress", ({ payload }) => setLocaleProgress(payload));
      const result = await api.codexLocalization(action);
      setLocaleProgress({ percent: 100, detail: action === "launch" ? `${desktopName} 启动完成` : "英文版本已恢复" });
      flash(result);
      await refreshBoot();
    } catch (err) {
      setLocaleProgress((current) => ({ percent: current?.percent ?? 0, detail: `操作失败：${message(err)}` }));
      flash(message(err));
    } finally { off?.(); setRestarting(false); }
  };
  const runCodexPreset = async (preset: CodexPreset) => {
    if (restarting) return;
    setRestarting(true);
    setLocaleProgress({ percent: 3, detail: "正在应用 Codex 增强设置" });
    try {
      await api.applyConfig(preset.claude, preset.codex, preset.groupId, preset.claudeModel || undefined, preset.codexModel || undefined);
      await api.setExtensions(preset.computerUse);
      await api.configureComputerTools(preset.computerUse);
      await api.configureNativeBrowser(preset.browser);
      if (preset.localization) await api.codexLocalization("launch");
      else {
        await api.codexLocalization("uninstall").catch(() => {});
        const result = await api.restartCodex();
        if (!result.ok) throw new Error(result.log);
      }
      setLocaleProgress({ percent: 100, detail: `${desktopName} 已按习惯启动` });
      flash("Codex 已按选择完成配置并重启");
      await refreshBoot();
    } catch (err) {
      setLocaleProgress((current) => ({ percent: current?.percent ?? 0, detail: `操作失败：${message(err)}` }));
      flash(message(err));
    } finally { setRestarting(false); }
  };
  const refreshBoot = async () => {
    const result = await api.bootstrap();
    setBoot(result.logged_in ? { ...result, saved_password: undefined } : result);
    return result;
  };
  useEffect(() => {
    if (isPreview) { setReady(true); return; }
    refreshBoot().then((result) => setUser(result.user ?? null))
      .catch((err) => setError(message(err))).finally(() => setReady(true));
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    listen<string>("host-switched", (event) => flash(`线路已切换至 ${event.payload}`))
      .then((off) => disposed ? off() : unlisteners.push(off)).catch(() => {});
    listen("request-quit", () => { void requestClose(); })
      .then((off) => disposed ? off() : unlisteners.push(off)).catch(() => {});
    listen<UpdateProgress>("update-progress", (event) => setUpdateProgress(event.payload))
      .then((off) => disposed ? off() : unlisteners.push(off)).catch(() => {});
    return () => { disposed = true; unlisteners.forEach((off) => off()); window.clearTimeout(timer.current); };
  }, []);
  useEffect(() => {
    const preventMenu = (event: MouseEvent) => event.preventDefault();
    const shortcut = (event: KeyboardEvent) => {
      if (isPreview) return;
      if (event.key === "F12") {
        event.preventDefault();
        event.stopPropagation();
        api.openDevtools().catch((err) => flash(message(err)));
      } else if ((event.ctrlKey || event.metaKey) && event.shiftKey && ["i", "j", "c"].includes(event.key.toLowerCase())) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    document.addEventListener("contextmenu", preventMenu, true);
    window.addEventListener("keydown", shortcut, true);
    return () => {
      document.removeEventListener("contextmenu", preventMenu, true);
      window.removeEventListener("keydown", shortcut, true);
    };
  }, []);
  const siteTab = allSiteTabs.find((item) => item.id === tab);
  useEffect(() => {
    if (!user || !siteTab || isPreview) {
      if (!isPreview) api.hideSite().catch(() => {});
      return;
    }
    const area = siteArea.current;
    if (!area) return;
    let active = true;
    let revision = 0;
    const sync = (navigate = false) => {
      const current = ++revision;
      const bounds = area.getBoundingClientRect();
      if (bounds.width < 100 || bounds.height < 100) return;
      api.showSite(siteTab.id, {
        x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height,
      }, navigate).catch((err) => { if (active && current === revision) flash(message(err)); });
    };
    const observer = new ResizeObserver(() => sync());
    observer.observe(area);
    const onResize = () => sync();
    window.addEventListener("resize", onResize);
    sync(true);
    return () => {
      active = false;
      observer.disconnect();
      window.removeEventListener("resize", onResize);
    };
  }, [tab, user?.id]);
  useEffect(() => {
    if (!user || isPreview) return;
    api.checkUpdate().then((update) => {
      if (update.update_available) setUpdate(update);
    }).catch(() => {});
  }, [user?.id]);
  const loggedIn = async (next: UserInfo) => {
    setAuthPage(null);
    setUser(next);
    try { await refreshBoot(); } catch (err) { flash(message(err)); }
  };
  const logout = async () => {
    try { await api.closeSite(); await api.logout(); setUser(null); setTab("setup"); await refreshBoot(); }
    catch (err) { flash(message(err)); }
  };
  const returnToLogin = () => {
    setAuthPage(null);
    setResetCredentials(undefined);
    setResetLink("");
  };
  const useResetLink = (event: FormEvent) => {
    event.preventDefault();
    try {
      const url = new URL(resetLink.trim());
      const allowed = ["jokerdeck.cc.cd", "jokerdeck.de5.net", "jokere.duckdns.org",
        "api.jokere.asia", "sub2api.186-244-245-198.sslip.io"];
      if (url.protocol !== "https:" || url.port || !allowed.includes(url.hostname) || url.pathname !== "/reset-password") {
        throw new Error("请输入中转站邮件中的密码重置链接");
      }
      const email = url.searchParams.get("email");
      const token = url.searchParams.get("token");
      if (!email || !token) throw new Error("链接缺少邮箱或重置 token");
      setResetCredentials({ email, token });
      setAuthPage("reset-password");
    } catch (err) { flash(message(err)); }
  };
  return <div className="app">
    <TitleBar flash={flash} onClose={requestClose} />
    {!ready ? <main className="loading">加载中...</main> : !user ? authPage ?
      <AuthForm key={authPage} page={authPage} onBack={returnToLogin} onLogin={loggedIn} onResetLink={useResetLink}
        resetLink={resetLink} setResetLink={setResetLink} resetCredentials={resetCredentials} flash={flash} /> :
      <Login boot={boot} onLogin={loggedIn} onAuthPage={setAuthPage} initialError={error} /> :
      <div className="workspace">
        <aside className="sidebar">
          <div className="sidebar-brand"><img src={logo} alt="" /><div><strong>jokerdeck-chatgpt</strong><small>定制工具</small></div></div>
          <span className="nav-label">工作空间</span>
          <nav>{tabs.map(({ id, title, icon: Icon }) => <button key={id} id={id === "tools" ? "tools-nav" : undefined}
            className={`nav-link ${tab === id ? "active" : ""}`} onClick={() => setTab(id)}>
            <Icon size={18} /><span>{title}</span>{tab === id && <ChevronRight size={15} />}</button>)}</nav>
          <span className="nav-label site-nav-label">中转站</span>
          <nav className="site-nav">{siteTabs.map(({ id, title, icon: Icon }) =>
            <button key={id} className={`nav-link ${tab === id ? "active" : ""}`} title={title} onClick={() => setTab(id)}>
              <Icon size={18} /><span>{title}</span>{tab === id && <ChevronRight size={15} />}</button>)}</nav>
          {(user.role === "admin" || user.role === "finance") && <>
            <span className="nav-label site-nav-label">财务</span>
            <nav><button className={`nav-link ${tab === financeTab.id ? "active" : ""}`}
              onClick={() => setTab(financeTab.id)}><ChartNoAxesCombined size={18} /><span>财务中心</span></button></nav>
          </>}
          {user.role === "admin" && <>
            <span className="nav-label site-nav-label">管理</span>
            <nav>{adminTabs.map(({ id, title, icon: Icon }) => <button key={id}
              className={`nav-link ${tab === id ? "active" : ""}`} onClick={() => setTab(id)}>
              <Icon size={18} /><span>{title}</span>{tab === id && <ChevronRight size={15} />}</button>)}</nav>
          </>}
          <div className="sidebar-bottom">
            <div className="sidebar-user"><span className="avatar"><UserRound size={18} /></span>
              <span className="user-email" title={user.email}>{user.email}</span>
              <button className="icon-button" title="退出登录" aria-label="退出登录" onClick={logout}><LogOut size={17} /></button></div>
          </div>
        </aside>
        <main className={`main ${siteTab ? "site-main" : ""}`}>
          {siteTab ? <div ref={siteArea} className="site-area"><div className="site-loading"><RefreshCw size={28} className="spin" /><strong>正在打开中转站</strong><small>连接并加载页面...</small></div></div> : <>
          <div className="page-header"><div><span className="breadcrumb">工作空间 / {tabs.find((item) => item.id === tab)?.title}</span>
            <h1>{tabs.find((item) => item.id === tab)?.title}</h1></div>
            <div className="row wrap">
              <button className="btn" onClick={() => setTab("store")}><Store size={16} />店铺销售</button>
            </div></div>
          {tab === "setup" && localeProgress && <div className="section localization-progress" aria-live="polite">
            <div className="localization-progress-heading"><strong>{localeProgress.detail}</strong><span>{localeProgress.percent}%</span></div>
            <div className="progress-track" role="progressbar" aria-valuenow={localeProgress.percent} aria-valuemin={0} aria-valuemax={100}>
              <span style={{ width: `${localeProgress.percent}%` }} /></div>
          </div>}
          {tab === "setup" &&
            <Setup user={user} setUser={setUser} boot={boot} refreshBoot={refreshBoot} flash={flash}
              goTools={() => setTab("tools")} />}
          {tab === "tools" && <Tools boot={boot} flash={flash} refreshBoot={refreshBoot} goSetup={() => setTab("setup")} />}
          {tab === "computer" && <Tools key="computer" mode="computer" boot={boot} flash={flash} refreshBoot={refreshBoot} goSetup={() => setTab("setup")} />}
          {tab === "providers" && <ProviderControls flash={flash} goSetup={() => setTab("setup")} />}
          {tab === "sessions" && <Sessions flash={flash} />}
          {tab === "enhance" && <CodexEnhance boot={boot} flash={flash} goTools={() => setTab("tools")}
            goSessions={() => setTab("sessions")} restarting={restarting} onRestart={runCodexPreset} />}
          {tab === "account" && <Account user={user} version={boot?.version} flash={flash} onUpdate={setUpdate} />}
          </>}
        </main>
      </div>}
    {toast && <div className="toast" role="status">{toast}</div>}
    {restoreLocaleConfirm && <div className="dialog-backdrop"><div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="restore-locale-title">
      <h2 id="restore-locale-title">恢复英文版本</h2><p>将关闭当前 {desktopName} 并恢复英文界面，是否继续？</p>
      <div className="row"><button className="btn" onClick={() => setRestoreLocaleConfirm(false)}>取消</button>
        <button className="btn primary" onClick={() => void restartCodex("uninstall")}>确认恢复</button></div>
    </div></div>}
    {closePrompt && <div className="dialog-backdrop"><div className="dialog" role="alertdialog" aria-modal="true"
      aria-labelledby="close-title"><h2 id="close-title">代理仍在运行</h2>
      <p>请先关闭代理并恢复配置。确认后将自动恢复 Claude Code / Codex 配置、停止代理，再关闭客户端。</p>
      <div className="row wrap"><button className="btn" disabled={closing} onClick={() => setClosePrompt(false)}>取消</button>
        <button className="btn primary" disabled={closing} onClick={confirmClose}>{closing ? "处理中..." : "确认并退出"}</button></div>
    </div></div>}
    {update && <div className="dialog-backdrop"><div className="dialog" role="dialog" aria-modal="true"
      aria-labelledby="update-title"><h2 id="update-title">发现新版本 v{update.latest}</h2>
      <p className="update-notes">{update.notes || "新版本已发布，请更新客户端。"}</p>
      {updateInstalling && <div className="update-progress" aria-live="polite">
        <div className="progress-track"><span style={{ width: updateProgress.total ? `${Math.min(100, updateProgress.downloaded / updateProgress.total * 100)}%` : "100%" }} /></div>
        <small>{updateProgress.total ? `正在下载 ${formatBytes(updateProgress.downloaded)} / ${formatBytes(updateProgress.total)}` : "正在准备下载..."}</small>
      </div>}
      <div className="row wrap"><button className="btn" disabled={updateInstalling} onClick={() => setUpdate(null)}>稍后</button>
        <button className="btn primary" disabled={!update.url || updateInstalling} onClick={installUpdate}>
          <Download size={16} />{updateInstalling ? "下载中..." : "下载并安装"}</button></div>
    </div></div>}
  </div>;
}

function Login({ boot, onLogin, onAuthPage, initialError }: {
  boot: Bootstrap | null; onLogin: (user: UserInfo) => Promise<void>;
  onAuthPage: (page: AuthPage) => void; initialError: string;
}) {
  const [email, setEmail] = useState(boot?.last_email ?? "");
  const [password, setPassword] = useState(boot?.saved_password ?? "");
  const [remember, setRemember] = useState(boot?.remember_password ?? false);
  const [showPassword, setShowPassword] = useState(false);
  const [twofa, setTwofa] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(initialError);
  const [hosts, setHosts] = useState(boot?.hosts ?? []);
  const [preferredHost, setPreferredHost] = useState(boot?.preferred_host ?? boot?.site_url ?? "");
  const [health, setHealth] = useState<HostHealth[]>([]);
  const [probing, setProbing] = useState(!isPreview);
  const [switching, setSwitching] = useState(false);
  useEffect(() => {
    if (isPreview) return;
    let active = true;
    api.probeHosts().then((results) => {
      if (!active) return;
      setHosts((current) => current.length ? current : results.map(({ host }) => host));
      setHealth(results);
    }).catch((err) => { if (active) setError(message(err)); })
      .finally(() => { if (active) setProbing(false); });
    return () => { active = false; };
  }, []);
  const selectedHealth = health.find(({ host }) => host === preferredHost);
  const line = probing ? "正在检测线路" : selectedHealth?.healthy ? `已连接 ${preferredHost}`
    : selectedHealth ? `线路不可达：${preferredHost}` : "线路未连通，请检查网络或中转地址";
  const switchHost = async (host: string) => {
    if (switching || busy || host === preferredHost) return;
    setSwitching(true);
    try {
      await api.setPreferredHost(host);
      setPreferredHost(host);
      setError("");
    } catch (err) { setError(message(err)); }
    finally { setSwitching(false); }
  };
  const finish = async (user: UserInfo) => {
    await api.saveLogin(email.trim(), password, remember);
    setPassword("");
    await onLogin(user);
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true); setError("");
    try {
      if (twofa) {
        await finish(await api.submit2fa(twofa, code));
      } else {
        const result = await api.login(email.trim(), password);
        if (result.requires_2fa && result.temp_token) setTwofa(result.temp_token);
        else if (result.user) await finish(result.user);
        else throw new Error("登录响应无效");
      }
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); }
  };
  const toggleRemember = async (checked: boolean) => {
    setRemember(checked);
    if (!checked && !isPreview) {
      try { await api.forgetPassword(); } catch (err) { setError(message(err)); }
    }
  };
  return <main className="login-page">
    <div className="login-brand"><img src={logo} alt="jokerdeck" /><h1>jokerdeck</h1><span>中转服务</span></div>
    <div className="login-form">
      <div className="field login-host"><label htmlFor="login-host">连接地址</label>
        <select id="login-host" className="input" value={preferredHost} disabled={busy || switching || isPreview}
          onChange={(event) => void switchHost(event.target.value)}>
          {hosts.map((host) => <option key={host} value={host}>{host}</option>)}
        </select>
        <div className="line-status"><span className={`dot ${selectedHealth?.healthy ? "ok" : ""}`} />{line}</div>
      </div>
      <h2>{twofa ? "两步验证" : "登录账户"}</h2>
      <form onSubmit={submit}>
        {!twofa ? <>
          <div className="field"><label htmlFor="email">邮箱</label><input id="email" className="input" type="email"
            autoComplete="username" required value={email} disabled={busy} onChange={(event) => setEmail(event.target.value)} /></div>
          <div className="field"><label htmlFor="password">密码</label><div className="password-input">
            <input id="password" className="input" type={showPassword ? "text" : "password"} autoComplete="current-password"
              required value={password} disabled={busy} onChange={(event) => setPassword(event.target.value)} />
            <button type="button" className="icon-button" title={showPassword ? "隐藏密码" : "显示密码"}
              aria-label={showPassword ? "隐藏密码" : "显示密码"} onClick={() => setShowPassword(!showPassword)}>
              {showPassword ? <EyeOff size={18} /> : <Eye size={18} />}</button></div></div>
          <div className="login-options"><label className="check"><input type="checkbox" checked={remember}
            disabled={busy || boot?.desktop_supported === false} onChange={(event) => toggleRemember(event.target.checked)} />保存账号密码</label>
            <button className="text-button" type="button" disabled={busy} onClick={() => onAuthPage("forgot-password")}>忘记密码</button></div>
        </> : <div className="field"><label htmlFor="totp">动态验证码</label><input id="totp" className="input"
          autoComplete="one-time-code" inputMode="numeric" pattern="[0-9]{6}" maxLength={6} required
          value={code} disabled={busy} onChange={(event) => setCode(event.target.value.replace(/\D/g, ""))} /></div>}
        {error && <div className="alert error" role="alert">{error}</div>}
        <button className="btn primary block" type="submit" disabled={busy || isPreview}>
          {busy ? "登录中..." : twofa ? "验证并登录" : "登录"}<ArrowRight size={17} /></button>
      </form>
      <div className="login-links">{twofa ? <button className="text-button" disabled={busy} onClick={() => { setTwofa(null); setCode(""); }}>返回登录</button>
        : <button className="text-button" onClick={() => onAuthPage("register")}>注册账户</button>}
        <button className="text-button" onClick={() => onAuthPage("forgot-password")}>重置密码</button></div>
    </div>
    <div className="login-footer"><ShieldCheck size={15} />系统安全存储 <span>v{boot?.version ?? "0.1.0"}</span></div>
  </main>;
}

function AuthForm({ page, onBack, onLogin, onResetLink, resetLink, setResetLink, resetCredentials, flash }: {
  page: AuthPage;
  onBack: () => void;
  onLogin: (user: UserInfo) => Promise<void>;
  onResetLink: (event: FormEvent) => void;
  resetLink: string;
  setResetLink: (value: string) => void;
  resetCredentials?: { email: string; token: string };
  flash: (text: string) => void;
}) {
  const [settings, setSettings] = useState<PublicAuthSettings | null>(null);
  const [settingsError, setSettingsError] = useState("");
  const [email, setEmail] = useState(resetCredentials?.email ?? "");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [verifyCode, setVerifyCode] = useState("");
  const [invitationCode, setInvitationCode] = useState("");
  const [promoCode, setPromoCode] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [countdown, setCountdown] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  useEffect(() => {
    let active = true;
    if (page === "reset-password") return;
    api.publicAuthSettings()
      .then((value) => { if (active) setSettings(value); })
      .catch((err) => { if (active) setSettingsError(message(err)); });
    return () => { active = false; };
  }, [page]);

  useEffect(() => {
    if (countdown <= 0) return;
    const timer = window.setInterval(() => setCountdown((value) => Math.max(0, value - 1)), 1000);
    return () => window.clearInterval(timer);
  }, [countdown]);

  useEffect(() => {
    if (page === "reset-password") setEmail(resetCredentials?.email ?? "");
  }, [page, resetCredentials?.email]);

  const captchaUnavailable = Boolean(settings?.turnstile_enabled || settings?.tencent_captcha_enabled || settings?.aliyun_captcha_enabled);
  const unavailable = page !== "reset-password" && (!settings || captchaUnavailable ||
    (page === "register" && !settings.registration_enabled) ||
    (page === "forgot-password" && settings.password_reset_enabled === false));
  const title = page === "register" ? "注册账户" : page === "forgot-password" ? "找回密码" : "重置密码";
  const validateEmail = (value: string) => {
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value)) return "请输入有效的邮箱地址";
    const whitelist = settings?.registration_email_suffix_whitelist?.filter(Boolean) ?? [];
    const domain = value.toLowerCase().split("@")[1];
    if (page === "register" && !settings?.registration_email_domain_quota_enabled && whitelist.length > 0 &&
      !whitelist.some((suffix) => {
        const allowed = suffix.trim().toLowerCase().replace(/^@+/, "");
        return allowed.startsWith("*.") ? (domain === allowed.slice(2) || domain.endsWith(allowed.slice(1))) : domain === allowed;
      })) {
      return `当前仅支持以下邮箱后缀：${whitelist.join("、")}`;
    }
    return "";
  };
  const ensureAvailable = () => {
    if (isPreview) throw new Error("浏览器预览模式不能执行此操作，请启动 Tauri 客户端");
    if (settingsError) throw new Error(`无法读取认证设置：${settingsError}`);
    if (page !== "reset-password" && !settings) throw new Error("认证设置尚未加载");
    if (settings && page === "register" && !settings.registration_enabled) throw new Error("当前暂未开放注册");
    if (settings && page === "forgot-password" && settings.password_reset_enabled === false) throw new Error("当前暂未开放密码重置");
    if (captchaUnavailable) throw new Error("当前启用了 CAPTCHA，请前往中转站完成验证后再操作");
  };
  const sendCode = async () => {
    const emailError = validateEmail(email.trim());
    if (emailError) { setError(emailError); return; }
    if (busy || countdown > 0) return;
    setBusy(true); setError(""); setSuccess("");
    try {
      ensureAvailable();
      const result = await api.sendVerifyCode(email.trim());
      setCountdown(Math.max(1, result.countdown || 60));
      setSuccess(result.message || "验证码已发送，请检查邮箱");
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); }
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true); setError(""); setSuccess("");
    try {
      ensureAvailable();
      const normalizedEmail = email.trim();
      const emailError = validateEmail(normalizedEmail);
      if (emailError) throw new Error(emailError);
      if (page === "register") {
        if (password.length < 6) throw new Error("密码至少需要 6 位");
        if (password !== confirmPassword) throw new Error("两次输入的密码不一致");
        if (settings?.email_verify_enabled && !verifyCode.trim()) throw new Error("请输入邮箱验证码");
        const result = await api.register({
          email: normalizedEmail, password,
          verify_code: verifyCode.trim() || undefined,
          invitation_code: invitationCode.trim() || undefined,
          promo_code: promoCode.trim() || undefined,
        });
        if (!result.user) throw new Error("注册响应无效");
        await onLogin(result.user);
        return;
      }
      if (page === "forgot-password") {
        const result = await api.forgotPassword(normalizedEmail);
        setSuccess(result || "如果邮箱已注册，密码重置邮件将很快发送");
        return;
      }
      if (!resetCredentials?.token) throw new Error("请先粘贴密码重置链接");
      if (password.length < 6) throw new Error("新密码至少需要 6 位");
      if (password !== confirmPassword) throw new Error("两次输入的新密码不一致");
      const result = await api.resetPassword(normalizedEmail, resetCredentials.token, password);
      flash(result || "密码已重置，请使用新密码登录");
      onBack();
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); }
  };

  return <main className="login-page auth-page">
    <div className="login-brand"><img src={logo} alt="jokerdeck" /><h1>jokerdeck</h1><span>中转服务</span></div>
    <div className="login-form">
      <div className="auth-heading"><button type="button" className="icon-button" title="返回登录" aria-label="返回登录" onClick={onBack}><ArrowRight className="back-icon" size={18} /></button><h2>{title}</h2></div>
      {settingsError && page !== "reset-password" && <div className="alert error" role="alert">无法读取认证设置：{settingsError}</div>}
      {page === "register" && settings && !settings.registration_enabled && <div className="alert warning">当前暂未开放注册</div>}
      {page === "forgot-password" && settings?.password_reset_enabled === false && <div className="alert warning">当前暂未开放密码重置</div>}
      {captchaUnavailable && <div className="alert warning">当前启用了 CAPTCHA，请使用中转站原网页完成验证。</div>}
      <form onSubmit={submit}>
        <div className="field"><label htmlFor="auth-email">邮箱</label><input id="auth-email" className="input" type="email"
          autoComplete="email" required value={email} disabled={busy || page === "reset-password"} onChange={(event) => setEmail(event.target.value)} /></div>
        {page === "register" && settings?.email_verify_enabled && <div className="field">
          <label htmlFor="auth-code">邮箱验证码</label><div className="input-action-row"><input id="auth-code" className="input" inputMode="numeric"
            autoComplete="one-time-code" maxLength={8} required value={verifyCode} disabled={busy} onChange={(event) => setVerifyCode(event.target.value.trim())} />
            <button type="button" className="btn" disabled={busy || countdown > 0 || unavailable} onClick={sendCode}>{countdown > 0 ? `${countdown}s` : "发送验证码"}</button></div>
        </div>}
        {page === "register" && settings?.invitation_code_enabled && <div className="field"><label htmlFor="invitation-code">邀请码</label>
          <input id="invitation-code" className="input" required value={invitationCode} disabled={busy} onChange={(event) => setInvitationCode(event.target.value)} /></div>}
        {page === "register" && settings?.promo_code_enabled && <div className="field"><label htmlFor="promo-code">优惠码（选填）</label>
          <input id="promo-code" className="input" value={promoCode} disabled={busy} onChange={(event) => setPromoCode(event.target.value)} /></div>}
        {page !== "forgot-password" && <><div className="field"><label htmlFor="auth-password">{page === "reset-password" ? "新密码" : "密码"}</label><div className="password-input">
          <input id="auth-password" className="input" type={showPassword ? "text" : "password"} autoComplete="new-password" required value={password}
            disabled={busy} onChange={(event) => setPassword(event.target.value)} />
          <button type="button" className="icon-button" title={showPassword ? "隐藏密码" : "显示密码"} aria-label={showPassword ? "隐藏密码" : "显示密码"} onClick={() => setShowPassword(!showPassword)}>
            {showPassword ? <EyeOff size={18} /> : <Eye size={18} />}
          </button></div></div>
          <div className="field"><label htmlFor="auth-confirm-password">确认密码</label><input id="auth-confirm-password" className="input" type="password"
            autoComplete="new-password" required value={confirmPassword} disabled={busy} onChange={(event) => setConfirmPassword(event.target.value)} /></div></>}
        {page === "forgot-password" && <p className="muted auth-hint">提交后请检查邮箱。收到邮件后，将完整的重置链接粘贴到下方。</p>}
        {page === "forgot-password" && <div className="reset-link-form"><input className="input" type="url" placeholder="收到邮件后粘贴重置链接"
          value={resetLink} onChange={(event) => setResetLink(event.target.value)} /><button className="btn" type="button" disabled={busy} onClick={onResetLink}>继续重置</button></div>}
        {error && <div className="alert error" role="alert">{error}</div>}
        {success && <div className="alert success" role="status">{success}</div>}
        <button className="btn primary block" type="submit" disabled={busy || isPreview || unavailable}>
          {busy ? "处理中..." : title}<ArrowRight size={17} /></button>
      </form>
      <div className="login-links"><button className="text-button" type="button" disabled={busy} onClick={onBack}>返回登录</button>
        {page === "forgot-password" && <button className="text-button" type="button" disabled={busy} onClick={() => setResetLink("")}>清空链接</button>}</div>
    </div>
    <div className="login-footer"><ShieldCheck size={15} />Windows 安全加密存储</div>
  </main>;
}

function Setup({ user, setUser, boot, refreshBoot, flash, goTools }: {
  user: UserInfo; setUser: (user: UserInfo) => void; boot: Bootstrap | null;
  refreshBoot: () => Promise<Bootstrap>; flash: (text: string) => void; goTools: () => void;
}) {
  const [groups, setGroups] = useState<PlazaGroup[]>([]);
  const [groupId, setGroupId] = useState<number | undefined>(boot?.preferred_group_id);
  const [claudeModel, setClaudeModel] = useState(boot?.claude_model ?? "");
  const [codexModel, setCodexModel] = useState(boot?.codex_model ?? "");
  const [claude, setClaude] = useState(true);
  const [codex, setCodex] = useState(true);
  const [status, setStatus] = useState<ProxyStatus | null>(null);
  const [report, setReport] = useState<CliReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [result, setResult] = useState<ApplyResult | null>(null);
  const [restoreConfirm, setRestoreConfirm] = useState(false);
  const [restoreResult, setRestoreResult] = useState<{ files: string[]; warnings: string[] } | null>(null);
  const [hostBusy, setHostBusy] = useState(false);
  const [groupModels, setGroupModels] = useState<GroupModels | null>(null);
  const [modelsBusy, setModelsBusy] = useState(false);
  const [modelsError, setModelsError] = useState("");
  const [testing, setTesting] = useState<"claude" | "codex" | null>(null);
  const [probes, setProbes] = useState<Partial<Record<"claude" | "codex", ModelProbe>>>({});
  const [configView, setConfigView] = useState<(ToolConfigView & { which: string }) | null>(null);
  const [viewBusy, setViewBusy] = useState(false);
  const [setupProgress, setSetupProgress] = useState<{ percent: number; detail: string } | null>(null);
  const [guideOpen, setGuideOpen] = useState(false);
  const [guideIntro, setGuideIntro] = useState(false);
  const [guideStep, setGuideStep] = useState(0);
  const modelRevision = useRef(0);
  const group = groups.find((item) => item.id === groupId);
  const claudeModels = groupModels && groupModels.group_id === groupId ? groupModels.claude_models : [];
  const codexModels = groupModels && groupModels.group_id === groupId ? groupModels.codex_models : [];
  const selectedClaudeModel = claudeModels.includes(claudeModel) ? claudeModel : claudeModels[0] ?? "";
  const selectedCodexModel = codexModels.includes(codexModel) ? codexModel : codexModels[0] ?? "";
  const load = async () => {
    setLoading(true); setError("");
    try {
      const [plaza, available, current] = await Promise.all([api.plaza(), api.groups(), api.proxyStatus()]);
      const allowed = new Set(available.map((item) => item.id));
      const selectable = plaza.filter((item) => allowed.has(item.id));
      setGroups(selectable);
      setGroupId((id) => selectable.some((item) => item.id === id) ? id :
        selectable.length === 1 ? selectable[0].id : undefined);
      setStatus(current);
    } catch (err) { setError(message(err)); }
    finally { setLoading(false); }
  };
  useEffect(() => {
    load();
    api.detectClis().then(setReport).catch((err) => setError(message(err)));
    let pending = false;
    const interval = window.setInterval(async () => {
      if (pending) return;
      pending = true;
      try { setStatus(await api.proxyStatus()); } catch { /* Retain last known state offline. */ }
      finally { pending = false; }
    }, 5000);
    return () => window.clearInterval(interval);
  }, []);
  useEffect(() => {
    if (isPreview) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    listen<{ percent: number; detail: string }>("setup-progress", ({ payload }) => {
      if (!disposed) setSetupProgress(payload);
    }).then((off) => { if (disposed) off(); else unsubscribe = off; }).catch(() => {});
    return () => { disposed = true; unsubscribe?.(); };
  }, []);
  const loadModels = async (id: number) => {
    const revision = ++modelRevision.current;
    setGroupModels(null); setModelsBusy(true); setModelsError(""); setProbes({});
    setSetupProgress({ percent: 12, detail: "正在从 sub2api 拉取模型" });
    try {
      const models = await api.groupModels(id);
      if (revision === modelRevision.current) setGroupModels(models);
      if (revision === modelRevision.current) setSetupProgress({ percent: 100, detail: "模型列表已加载" });
    } catch (err) {
      if (revision === modelRevision.current) setModelsError(message(err));
    } finally {
      if (revision === modelRevision.current) setModelsBusy(false);
    }
  };
  useEffect(() => {
    if (groupId) loadModels(groupId);
    else { modelRevision.current++; setGroupModels(null); setModelsBusy(false); }
    return () => { modelRevision.current++; };
  }, [groupId]);
  const selectGroup = (id: number) => {
    setGroupId(id); setClaudeModel(""); setCodexModel(""); setResult(null);
    setClaude(true); setCodex(true);
  };
  const apply = async () => {
    if (busy) return;
    setBusy(true); setError(""); setResult(null); setRestoreResult(null);
    setSetupProgress({ percent: 5, detail: "准备一键配置" });
    try {
      const applied = await api.applyConfig(claude && !!claudeModels.length, codex && !!codexModels.length,
        groupId, selectedClaudeModel || undefined, selectedCodexModel || undefined);
      setResult(applied);
      setStatus(await api.proxyStatus());
      const next = await refreshBoot();
      setClaudeModel(next.claude_model ?? ""); setCodexModel(next.codex_model ?? "");
      flash("分组 Key、线路和模型配置已应用");
      setSetupProgress({ percent: 100, detail: "一键配置完成" });
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); }
  };
  const stopProxy = async () => {
    if (busy) return;
    setBusy(true); setError(""); setResult(null);
    try {
      setStatus(await api.stopProxy());
      const next = await refreshBoot();
      setGroupId(next.preferred_group_id);
      setClaudeModel(next.claude_model ?? "");
      setCodexModel(next.codex_model ?? "");
      flash("本地代理已关闭，工具配置已恢复；再次应用配置即可启动");
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); }
  };
  const restore = async () => {
    if (busy) return;
    setBusy(true); setError(""); setResult(null); setRestoreResult(null);
    try {
      const restored = await api.restoreConfig();
      setStatus(restored.status);
      setRestoreResult(restored);
      const next = await refreshBoot();
      setGroupId(next.preferred_group_id);
      setClaudeModel(next.claude_model ?? "");
      setCodexModel(next.codex_model ?? "");
      flash(restored.files.length ? "工具配置已恢复，本地代理已关闭" : "没有可恢复的客户端配置，代理已关闭");
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); setRestoreConfirm(false); }
  };
  const refreshAccount = async () => {
    try { setUser(await api.account()); } catch (err) { flash(message(err)); }
  };
  const switchHost = async (host: string) => {
    setHostBusy(true);
    try {
      setStatus(await api.setPreferredHost(host || undefined));
      flash(host ? "已优先使用所选线路，失败时仍会尝试其他线路" : "已恢复自动优选线路");
    } catch (err) { setError(message(err)); }
    finally { setHostBusy(false); }
  };
  const testModel = async (tool: "claude" | "codex") => {
    if (!groupId || testing) return;
    const model = tool === "claude" ? selectedClaudeModel : selectedCodexModel;
    setTesting(tool); setSetupProgress({ percent: 15, detail: `正在测试 ${tool === "claude" ? "Claude Code" : "Codex"} 模型` });
    setProbes((current) => ({ ...current, [tool]: undefined }));
    try {
      const result = await api.testGroupModel(groupId, tool, model);
      setProbes((current) => ({ ...current, [tool]: result }));
      setSetupProgress({ percent: 100, detail: "模型测试完成" });
    } catch (err) {
      setProbes((current) => ({ ...current, [tool]: { ok: false, detail: message(err) } }));
    } finally { setTesting(null); }
  };
  const showConfig = async (which: "claude" | "codex" | "catalog") => {
    setViewBusy(true);
    try {
      setConfigView({ ...(await api.readToolConfig(which)), which });
    } catch (err) { flash(message(err)); }
    finally { setViewBusy(false); }
  };
  const applied = result?.active_group_id === groupId || (status?.active_group?.group_id === groupId && !!status?.running);
  const finishGuide = () => {
    try { localStorage.setItem("jokerdeck.beginner-guide.v1", "1"); } catch {}
    setGuideOpen(false); setGuideIntro(false);
  };
  const openGuide = () => { setGuideStep(0); setGuideIntro(true); };
  return <>
    <div className="stats">
      <div className="stat"><span><Wallet size={17} />账户余额<button className="icon-button" title="刷新余额" aria-label="刷新余额" onClick={refreshAccount}><RefreshCw size={14} /></button></span>
        <strong>${user.balance.toFixed(2)}</strong><small>冻结 ${user.frozen_balance.toFixed(2)}</small></div>
      <div className="stat"><span><ShieldCheck size={17} />本地代理</span>
        <strong className={status?.running ? "positive" : ""}>{status?.running ? "运行中" : "已关闭"}</strong><small>{status?.running ? status.base_url : "应用配置以启动"}</small></div>
      <div className="stat"><span><KeyRound size={17} />实际使用分组</span>
        <strong>{status?.running ? status.active_group?.name ?? "未配置" : "未运行"}</strong><small>{status?.running && status.active_group?.multiplier != null ? `${status.active_group.multiplier}x 倍率` : "等待配置"}</small></div>
    </div>
    <section className="section">
      <div className="section-header"><h2>分组与模型</h2><div className="row">
        <button className="btn small" onClick={openGuide}><CircleHelp size={15} />新手引导</button>
        <button className="btn small" title="拉取当前分组模型" aria-label="拉取当前分组模型"
          disabled={!groupId || modelsBusy || busy} onClick={() => groupId && loadModels(groupId)}>
          <RefreshCw size={17} className={modelsBusy ? "spin" : ""} />刷新模型</button>
        <button className="btn small" title="刷新分组" aria-label="刷新分组"
          disabled={loading || busy} onClick={load}><RefreshCw size={17} />刷新分组</button></div></div>
      {setupProgress && <div className="setup-progress" aria-live="polite">
        <div className="setup-progress-heading"><strong>{setupProgress.detail}</strong><span>{setupProgress.percent}%</span></div>
        <div className="progress-track"><span style={{ width: `${setupProgress.percent}%` }} /></div>
      </div>}
      <div className="field"><label htmlFor="group">分组</label><select id="group" className="input" value={groupId ?? ""}
        disabled={busy || loading} onChange={(event) => selectGroup(Number(event.target.value))}>
        <option value="" disabled>{loading ? "加载中..." : "选择分组"}</option>
        <optgroup label="公开分组">
          {groups.filter((item) => !item.is_exclusive).map((item) => <option key={item.id} value={item.id}>{item.name}
            {item.multiplier != null ? ` · ${item.multiplier}x` : ""}</option>)}
        </optgroup>
        <optgroup label="专属分组">
          {groups.filter((item) => item.is_exclusive).map((item) => <option key={item.id} value={item.id}>{item.name}
            {item.multiplier != null ? ` · ${item.multiplier}x` : ""}</option>)}
        </optgroup>
        </select></div>
      {group && <div className="selection-summary"><span>{group.name}</span><span className={`tag ${applied ? "success" : "warning"}`}>
        {applied ? "代理当前分组" : "待应用"}</span>{group.description && <small>{group.description}</small>}</div>}
      {!loading && !groups.length && !error && <div className="alert warning">没有可配置分组，请在中转站检查授权或订阅。</div>}
      {modelsError && <div className="alert error" role="alert">{modelsError}</div>}
      {groupModels?.codex_error && <div className="alert warning">{groupModels.codex_error}</div>}
      <div className="model-grid" id="model-options">
        <div className="field"><label htmlFor="claude-model">Claude Code</label><div className="model-action">
          <select id="claude-model" className="input" disabled={busy || modelsBusy || !claudeModels.length} value={selectedClaudeModel}
            onChange={(event) => { setClaudeModel(event.target.value); setResult(null); setProbes((old) => ({ ...old, claude: undefined })); }}>
            {!claudeModels.length && <option value="">{modelsBusy ? "拉取中..." : "当前分组不支持"}</option>}
            {claudeModels.map((model) => <option key={model}>{model}</option>)}</select>
          <button type="button" className="btn" title="测试所选 Claude Code 模型，可能产生费用" disabled={!selectedClaudeModel || !!testing || busy} onClick={() => testModel("claude")}>
            <Stethoscope size={16} />测试（计费）</button></div>
          {probes.claude && <small className={`probe-result ${probes.claude.ok ? "positive" : "probe-fail"}`}>{probes.claude.detail}</small>}</div>
        <div className="field"><label htmlFor="codex-model">Codex Desktop / CLI</label><div className="model-action">
          <select id="codex-model" className="input" disabled={busy || modelsBusy || !codexModels.length} value={selectedCodexModel}
            onChange={(event) => { setCodexModel(event.target.value); setResult(null); setProbes((old) => ({ ...old, codex: undefined })); }}>
            {!codexModels.length && <option value="">{modelsBusy ? "拉取中..." : "当前分组不支持"}</option>}
            {codexModels.map((model) => <option key={model}>{model}</option>)}</select>
          <button type="button" className="btn" title="测试所选 Codex 模型，可能产生费用" disabled={!selectedCodexModel || !!testing || busy} onClick={() => testModel("codex")}>
            <Stethoscope size={16} />测试（计费）</button></div>
          {probes.codex && <small className={`probe-result ${probes.codex.ok ? "positive" : "probe-fail"}`}>{probes.codex.detail}</small>}</div>
      </div>
      <div className="tool-options" id="agent-options">
        <label className="tool-choice"><input type="checkbox" checked={claude && !!claudeModels.length}
          disabled={busy || !claudeModels.length} onChange={(event) => setClaude(event.target.checked)} />
          <span><strong>Claude Code</strong><small>{report?.claude.installed ? `已安装 · ${report.claude.version ?? ""}` : "未检测到 CLI"}</small></span></label>
        <label className="tool-choice"><input type="checkbox" checked={codex && !!codexModels.length}
          disabled={busy || !codexModels.length} onChange={(event) => setCodex(event.target.checked)} />
          <span><strong>Codex Desktop / CLI</strong><small>{report?.codex_desktop.installed ? "已检测到 Desktop" : "未检测到 Desktop"} · {report?.codex.installed ? "CLI 已安装" : "CLI 未安装"}</small></span></label>
      </div>
    </section>
    <section className="section">
      <div className="section-header"><h2>应用配置</h2><span className="tag">桌面端</span></div>
      <p className="muted">自动创建或复用当前分组的 Key，配置支持的工具与模型。</p>
      {error && <div className="alert error" role="alert">{error}</div>}
      {result && <div className="alert success"><Check size={17} />配置已应用{codex && "，Codex 模型目录已同步"}。
        <span>已打开的 Claude Code / Codex 会话需重新启动。</span></div>}
      {result?.warnings?.map((warning, index) => <div className="alert warning" key={index}>{warning}</div>)}
      {restoreResult && <div className="alert success">{restoreResult.files.length
        ? `已恢复 ${restoreResult.files.length} 个配置文件，代理已关闭。` : "没有检测到客户端写入的配置，代理已关闭。"}
        <span>已打开的 Claude Code / Codex 会话需重新启动。</span></div>}
      {restoreResult?.warnings.map((warning, index) => <div className="alert warning" key={index}>{warning}</div>)}
      {restoreConfirm && <div className="alert warning restore-confirm" role="group" aria-label="确认恢复配置">
        将撤回客户端写入的 Claude Code / Codex 配置，并关闭本地代理。其他配置项会保留。
        <div className="row wrap"><button type="button" className="btn small" disabled={busy} onClick={() => setRestoreConfirm(false)}>取消</button>
          <button type="button" className="btn small" disabled={busy} onClick={restore}>确认恢复</button></div>
      </div>}
      <div className="apply-footer"><span className="muted">{group ? group.name : "尚未选择分组"}</span><div className="row wrap">
        <button className="btn" disabled={busy || boot?.desktop_supported === false} onClick={() => setRestoreConfirm(true)}>
          <RotateCcw size={17} />恢复配置</button>
        <button className="btn" disabled={busy || !status?.running} onClick={stopProxy}>
          <Power size={17} />关闭代理</button>
        <button id="apply-config" className="btn primary" disabled={busy || loading || modelsBusy || !group || boot?.desktop_supported === false
          || !(claude && claudeModels.length || codex && codexModels.length)} onClick={apply}>
          {busy ? <RefreshCw size={17} className="spin" /> : <Check size={17} />}{busy ? "处理中..." : "一键应用配置"}</button></div></div>
    </section>
    <section className="section"><div className="section-header"><h2>当前配置文件</h2></div>
      <div className="row wrap">
        <button className="btn" disabled={viewBusy} onClick={() => showConfig("claude")}><Eye size={16} />Claude Code</button>
        <button className="btn" disabled={viewBusy} onClick={() => showConfig("codex")}><Eye size={16} />Codex</button>
        <button className="btn" disabled={viewBusy} onClick={() => showConfig("catalog")}><Eye size={16} />Codex 模型目录</button>
      </div>
      {configView && <div className="config-view"><div className="row"><strong>{configView.path}</strong>
        <button className="icon-button" title="关闭配置预览" aria-label="关闭配置预览" onClick={() => setConfigView(null)}><X size={16} /></button></div>
        <pre className="log">{configView.content}</pre></div>}
    </section>
    <section className="section"><div className="section-header"><h2>线路状态</h2><span className="tag">{status?.hosts.length ?? 0} 条线路</span></div>
      <div className="field"><label htmlFor="preferred-host">优先线路</label>
        <select className="input" id="preferred-host" value={status?.preferred_host ?? ""}
          disabled={hostBusy || !status?.hosts.length} onChange={(event) => switchHost(event.target.value)}>
          <option value="">自动优选</option>
          {status?.hosts.map((host) => <option value={host.host} key={host.host}>{host.host}</option>)}
        </select></div>
      {status?.hosts.map((host) => <div className="list-row" key={host.host}><span className="row"><span className={`dot ${host.healthy ? "ok" : ""}`} />
        <span className="break">{host.host}</span>{status.active_host === host.host && <span className="tag success">当前</span>}
        {status.preferred_host === host.host && <span className="tag">优先</span>}</span>
        <small>{host.latency_ms != null ? `${host.latency_ms} ms` : "未检测"}</small></div>)}</section>
    {guideIntro && <GuideIntro onStart={() => { setGuideIntro(false); setGuideOpen(true); }}
      onSkip={finishGuide} />}
    {guideOpen && <BeginnerGuide step={guideStep} setStep={setGuideStep}
      onTools={() => { finishGuide(); goTools(); }} onClose={finishGuide} />}
  </>;
}

function Sessions({ flash }: { flash: (text: string) => void }) {
  const [items, setItems] = useState<CodexSession[]>([]);
  const [selected, setSelected] = useState("");
  const [detail, setDetail] = useState<CodexSessionDetail | null>(null);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [reading, setReading] = useState(false);
  const refresh = async () => {
    setLoading(true);
    try { setItems(await api.codexSessions()); }
    catch (err) { flash(message(err)); }
    finally { setLoading(false); }
  };
  useEffect(() => { void refresh(); }, []);
  const select = async (id: string) => {
    setSelected(id); setDetail(null); setReading(true);
    try { setDetail(await api.codexSession(id)); }
    catch (err) { flash(message(err)); }
    finally { setReading(false); }
  };
  const filtered = items.filter((item) => `${item.title} ${item.id}`.toLowerCase().includes(query.toLowerCase()));
  return <section className="section sessions-section">
    <div className="section-header"><h2>最近 100 条本地会话</h2><button className="icon-button" title="刷新会话"
      aria-label="刷新会话" disabled={loading} onClick={refresh}><RefreshCw size={17} className={loading ? "spin" : ""} /></button></div>
    <div className="session-search"><Search size={17} /><input className="input" type="search"
      placeholder="搜索标题或 ID" aria-label="搜索会话" value={query} onChange={(event) => setQuery(event.target.value)} /></div>
    <div className="sessions-layout">
      <div className="session-list" aria-label="本地会话列表">
        {filtered.map((item) => <div key={item.id} className={`session-item ${selected === item.id ? "active" : ""}`}>
          <button type="button" className="session-item-main" onClick={() => void select(item.id)}>
            <strong>{item.title}</strong><small>{new Date(item.updated_at * 1000).toLocaleString()} · {(item.size / 1024).toFixed(0)} KB</small>
          </button>
        </div>)}
        {!loading && !filtered.length && <p className="muted">{query ? "没有匹配的会话" : "没有本地会话"}</p>}
      </div>
      <div className="session-preview">
        {reading ? <span className="muted">读取中...</span> : detail ? <>
          {detail.truncated && <div className="alert warning">会话过长，仅显示已读取部分。</div>}
          {detail.messages.length ? detail.messages.map((item, index) => <article className="session-message" key={index}>
            <strong>{item.role === "user" ? "用户" : "Codex"}</strong><pre>{item.text}</pre>
          </article>) : <span className="muted">没有可预览的消息</span>}
        </> : <span className="muted">选择会话查看消息</span>}
      </div>
    </div>
  </section>;
}

function CodexEnhance({ boot, flash, goTools, goSessions, restarting, onRestart }: {
  boot: Bootstrap | null; flash: (text: string) => void; goTools: () => void; goSessions: () => void;
  restarting: boolean; onRestart: (preset: CodexPreset) => Promise<void>;
}) {
  const [busy, setBusy] = useState("");
  const [status, setStatus] = useState<CodexEnhancementStatus | null>(null);
  const [features, setFeatures] = useState<DesktopFeatureStatus | null>(null);
  const [sync, setSync] = useState<ProviderSyncResult | null>(null);
  const [featureError, setFeatureError] = useState("");
  const [syncLog, setSyncLog] = useState("");
  const [confirmation, setConfirmation] = useState<{ action: "sync" | "restore"; backup?: string } | null>(null);
  const [restartOpen, setRestartOpen] = useState(false);
  const [preset, setPreset] = useState<CodexPreset>({ claude: true, codex: true, localization: true, computerUse: false, browser: false });
  const [groups, setGroups] = useState<PlazaGroup[]>([]);
  const [groupModels, setGroupModels] = useState<GroupModels | null>(null);
  const [rememberPreset, setRememberPreset] = useState(false);
  const nodeReady = Boolean(features?.node_version);
  const themes = [
    { id: "native", name: "原生", colors: ["#f5f5f5", "#202020", "#8b8b8b"] },
    { id: "rose", name: "玫瑰柔光", colors: ["#fff7fb", "#a93f79", "#3d2340"] },
    { id: "midnight", name: "午夜蓝", colors: ["#131927", "#8faeff", "#eef3ff"] },
    { id: "jade", name: "翡翠晨光", colors: ["#f3faf7", "#177d59", "#193b30"] },
  ];
  const updateFeatures = async (theme: string, overlay: boolean, autoSync: boolean) => {
    if (busy) return;
    setBusy("features"); setFeatureError("");
    try { setFeatures(await api.configureDesktopFeatures(theme, overlay, autoSync)); flash("设置已保存，已安装增强的 Codex 将自动更新"); }
    catch (err) { setFeatureError(message(err)); }
    finally { setBusy(""); }
  };
  const provider = async (operation: "install" | "status" | "sync" | "restore", backup?: string) => {
    setConfirmation(null); setBusy(`provider-${operation}`); setFeatureError("");
    setSyncLog(operation === "install" ? "正在下载并安装 Provider 同步组件..." : operation === "sync" ? "正在备份并同步会话与 SQLite 索引..." : "正在检查 Provider 数据...");
    try {
      const result = await api.providerSyncAction(operation, backup);
      setSyncLog(operation === "sync" ? `同步完成：会话 ${result.sessionFilesUpdated ?? 0} 个，SQLite ${result.sqliteRowsUpdated ?? 0} 行${result.backupDir ? "；已生成备份" : ""}${result.skippedLockedRolloutFiles?.length ? "；有正在使用的会话被跳过，请停止写入后重试" : ""}` : operation === "restore" ? "备份已恢复，当前 Provider 配置保留" : "组件及 Provider 状态检查完成");
      setSync(operation === "sync" || operation === "restore" ? await api.providerSyncAction("status") : result);
      setFeatures(await api.desktopFeatureStatus());
    } catch (err) { setFeatureError(message(err)); setSyncLog("操作未完成，请查看提示；上游备份会保留"); }
    finally { setBusy(""); }
  };
  useEffect(() => {
    api.desktopFeatureStatus().then(setFeatures).catch((err) => setFeatureError(message(err)));
    api.plaza().then(setGroups).catch(() => {});
    try {
      const saved = localStorage.getItem("jokerdeck.codex-restart-preset.v1");
      if (saved) setPreset(JSON.parse(saved));
    } catch {}
  }, []);
  const openRestart = async () => {
    if (restarting) return;
    try {
      const saved = localStorage.getItem("jokerdeck.codex-restart-preset.v1");
      if (saved) {
        const value = JSON.parse(saved) as Partial<CodexPreset>;
        if (typeof value.claude === "boolean" && typeof value.codex === "boolean" &&
          typeof value.localization === "boolean" && typeof value.computerUse === "boolean" &&
          typeof value.browser === "boolean") {
          await onRestart(value as CodexPreset);
          return;
        }
      }
    } catch {}
    setRestartOpen(true);
  };
  const submitRestart = async () => {
    if (!preset.claude && !preset.codex) { setFeatureError("至少选择 Claude Code 或 Codex"); return; }
    if (rememberPreset) localStorage.setItem("jokerdeck.codex-restart-preset.v1", JSON.stringify(preset));
    setRestartOpen(false);
    await onRestart(preset);
  };
  const selectedGroup = groups.find((item) => item.id === preset.groupId);
  const loadPresetModels = async (id: number) => {
    setPreset((current) => ({ ...current, groupId: id, claudeModel: "", codexModel: "" }));
    try { setGroupModels(await api.groupModels(id)); } catch (err) { setFeatureError(message(err)); }
  };
  const refresh = async () => {
    try { setStatus(await api.codexEnhancementStatus()); }
    catch (err) { flash(message(err)); }
  };
  useEffect(() => { void refresh(); }, []);
  const action = async (name: string, run: () => Promise<string>) => {
    setBusy(name);
    try { flash(await run()); await refresh(); }
    catch (err) { flash(message(err)); }
    finally { setBusy(""); }
  };
  return <>
    <section className="section">
      <div className="section-header"><div><h2>一键重启 Codex</h2><p className="subtext">首次选择一次，之后可直接按习惯启动。</p></div>
        <button className="btn primary" disabled={restarting || boot?.desktop_supported === false} onClick={() => void openRestart()}>
          <RefreshCw size={16} className={restarting ? "spin" : ""} />{restarting ? "处理中..." : "一键重启 Codex"}</button>
      </div>
      {boot?.desktop_supported === false && <div className="alert warning">当前系统未检测到可适配的 Codex Desktop。</div>}
    </section>
    <section className="section"><div className="section-header"><h2>Codex 换肤</h2><span className="tag">Windows 汉化副本</span></div>
      <div className="theme-grid">{themes.map((theme) => <button className={`theme-card ${features?.theme === theme.id ? "selected" : ""}`} key={theme.id}
        disabled={!!busy || !features || navigator.platform.toLowerCase().includes("mac")}
        onClick={() => features && void updateFeatures(theme.id, features.overlay, features.auto_sync)}
        aria-pressed={features?.theme === theme.id}>
        <span className="theme-preview" style={{ background: theme.colors[0] }}><span style={{ background: theme.colors[1] }} /><span style={{ background: theme.colors[2] }} /></span>
        <strong>{theme.name}</strong>{features?.theme === theme.id && <Check size={15} />}
      </button>)}</div>
      <small className="subtext">选择后启动或重启汉化版 Codex 生效；原生主题可恢复默认颜色。仅修改客户端管理的副本。</small>
    </section>
    <section className="section">
      <div className="section-header"><h2>服务商切换 · 会话保护</h2><span className="tag">{features?.provider ?? "检测中"}</span></div>
      <p className="subtext">将历史会话和 SQLite 索引对齐到当前 Provider，找回切换后隐藏的会话。修改前自动备份，保留历史模型与聊天内容。</p>
      <div className="row wrap">
        {!features?.sync_installed && <button className="btn primary" disabled={!!busy || !features || !nodeReady} onClick={() => void provider("install")}><Download size={16} />{busy === "provider-install" ? "安装中..." : "安装同步组件"}</button>}
        <button className="btn" disabled={!!busy || !features?.sync_installed || !nodeReady} onClick={() => void provider("status")}><Search size={16} />检查同步状态</button>
        <button className="btn primary" disabled={!!busy || !features?.sync_installed || !nodeReady} onClick={() => setConfirmation({ action: "sync" })}><RefreshCw size={16} className={busy === "provider-sync" ? "spin" : ""} />同步历史会话</button>
        <button className="btn" disabled={!!busy} onClick={() => action("node-upgrade", async () => {
          const result = await api.installCli("node");
          if (!result.ok) throw new Error(result.log || "Node.js 升级失败");
          setFeatures(await api.desktopFeatureStatus());
          return result.log || "Node.js 已安装，请重新检查同步组件";
        })}><Download size={16} />安装 / 升级 Node.js</button>
        <button className="btn" onClick={goTools}><Monitor size={16} />环境检查</button>
      </div>
      {!nodeReady && features && <div className="alert warning" role="status">未找到支持 SQLite backup 的 Node.js 24+。请先升级 Node.js，再安装同步组件。</div>}
      <label className="feature-toggle"><input type="checkbox" checked={features?.auto_sync ?? false} disabled={!!busy || !features?.sync_installed}
        onChange={(event) => features && void updateFeatures(features.theme, features.overlay, event.target.checked)} /><span>应用 Codex 配置后自动同步 Provider</span></label>
      {sync?.rolloutCounts && <div className="provider-counts">{Object.entries(sync.rolloutCounts).map(([name, count]) => <span className="tag" key={name}>{name} · {count}</span>)}</div>}
      {!!sync?.backups?.length && <div className="provider-backups"><strong>可恢复备份</strong>{sync.backups.slice(0, 3).map((backup, index) => {
        const path = backup.path ?? backup.backupDir ?? backup.name ?? "";
        const id = path.split(/[\/]/).filter(Boolean).pop() ?? "";
        return <div className="list-row" key={id || index}><small className="subtext">{backup.createdAt ?? id}</small><button className="btn small" disabled={!!busy || !id} onClick={() => setConfirmation({ action: "restore", backup: id })}><RotateCcw size={14} />恢复</button></div>;
      })}</div>}
      {syncLog && <p className="subtext" aria-live="polite">{syncLog}</p>}
      <small className="subtext">基于 codex-provider-sync 0.5.0，需要 Node.js 24+。元数据同步不能保证跨账号或 Provider 的加密会话仍可继续。</small>
    </section>
    <section className="section"><div className="section-header"><h2>Token 与费用</h2><ChartNoAxesCombined size={18} /></div>
      <label className="feature-toggle"><input type="checkbox" checked={features?.overlay ?? true} disabled={!!busy || !features}
        onChange={(event) => features && void updateFeatures(features.theme, event.target.checked, features.auto_sync)} /><span>在 Codex 输入框底部显示用量</span></label>
      <div className="usage-preview"><span><ChartNoAxesCombined size={14} />今日 <strong>12.4K</strong><small>$0.42</small></span><span>本会话 <strong>3.8K</strong><small>≈$0.08</small></span></div>
      <small className="subtext">上方为样式示例。实际显示今日账户 Token / 扣费、本会话 Token，以及近期账单匹配费用；悬浮查看累计、输入、缓存与输出明细。未匹配的费用显示 —，尚未完整统计时标记 ≈。</small>
    </section>
    {featureError && <div className="alert error" role="alert">{featureError}</div>}
    {confirmation && <div className="dialog-backdrop"><div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="provider-confirm-title">
      <h2 id="provider-confirm-title">{confirmation.action === "sync" ? "同步历史 Provider" : "恢复历史备份"}</h2>
      <p>{confirmation.action === "sync" ? "将历史会话与 SQLite 索引同步到当前 Provider，修改前自动备份。建议先停止正在进行的对话。" : "将恢复所选备份中的会话与索引，当前 Provider 配置保持不变。建议先关闭 Codex。"}</p>
      <div className="row"><button className="btn" onClick={() => setConfirmation(null)}>取消</button><button className="btn primary" onClick={() => void provider(confirmation.action, confirmation.backup)}>确认操作</button></div>
    </div></div>}

    <section className="section"><div className="section-header"><h2>插件市场</h2>
      <span className={`tag ${status?.plugins_enabled ? "success" : "warning"}`}>{status?.plugins_enabled ? "插件功能已启用" : "插件功能未启用"}</span></div>
      <div className="row wrap">
        <button className="btn" disabled={!!busy || status?.plugins_enabled || boot?.desktop_supported === false}
          onClick={() => action("marketplace", api.enableCodexMarketplace)}><Sparkles size={16} />启用插件市场</button>
      </div>
      <small className="subtext">API Key 模式下的官方在线市场可能仍需 ChatGPT 登录；此操作不会绕过账号权限。</small>
    </section>
    <section className="section"><div className="section-header"><h2>模型白名单</h2>
      <span className={`tag ${status?.model_catalog_active ? "success" : "warning"}`}>
        {status?.model_catalog_active ? `已同步 ${status.model_count} 个模型` : "未同步"}</span></div>
      <button className="btn" disabled={!!busy || !boot?.preferred_group_id || boot?.desktop_supported === false}
        onClick={() => action("models", async () => {
          await api.applyConfig(false, true, boot?.preferred_group_id, undefined, boot?.codex_model);
          return "中转模型目录已同步到 Codex；重启后刷新模型选择列表";
        })}><RefreshCw size={16} className={busy === "models" ? "spin" : ""} />同步中转模型</button>
      {!boot?.preferred_group_id && <small className="subtext">请先在控制台选择分组并应用配置。</small>}
    </section>
    <section className="section"><div className="section-header"><h2>官方远端插件缓存</h2>
      <span className={`tag ${status?.cache_registered ? "success" : "warning"}`}>
        {status?.cache_registered ? `已注册 ${status.cached_plugins} 个` : status?.cache_available ? "待注册" : "本机无缓存"}</span></div>
      <button className="btn" disabled={!!busy || !status?.cache_available || status.cache_registered || boot?.desktop_supported === false}
        onClick={() => action("cache", api.registerCodexPluginCache)}>
        <RefreshCw size={16} className={busy === "cache" ? "spin" : ""} />修复缓存注册</button>
    </section>
    <section className="section"><div className="section-header"><h2>会话</h2><History size={18} /></div>
      <button className="btn" onClick={goSessions}><History size={16} />管理本地会话</button>
    </section>
    {restartOpen && <div className="dialog-backdrop"><div className="dialog restart-dialog" role="dialog" aria-modal="true" aria-labelledby="restart-title">
      <h2 id="restart-title">配置并重启 Codex</h2>
      <p>选择本次要启用的能力。勾选“保留习惯”后，下次点击将直接执行。</p>
      <div className="field"><label htmlFor="restart-group">分组</label><select id="restart-group" className="input" value={preset.groupId ?? ""} onChange={(event) => void loadPresetModels(Number(event.target.value))}>
        <option value="">选择分组</option>{groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}</select></div>
      {selectedGroup && <div className="model-grid">
        <div className="field"><label htmlFor="restart-claude-model">Claude 模型</label><select id="restart-claude-model" className="input" value={preset.claudeModel ?? ""} onChange={(event) => setPreset((p) => ({ ...p, claudeModel: event.target.value }))}>{(groupModels?.claude_models ?? []).map((model) => <option key={model}>{model}</option>)}</select></div>
        <div className="field"><label htmlFor="restart-codex-model">GPT / Codex 模型</label><select id="restart-codex-model" className="input" value={preset.codexModel ?? ""} onChange={(event) => setPreset((p) => ({ ...p, codexModel: event.target.value }))}>{(groupModels?.codex_models ?? []).map((model) => <option key={model}>{model}</option>)}</select></div>
      </div>}
      <div className="tool-options restart-options">
        <label className="tool-choice"><input type="checkbox" checked={preset.claude} onChange={(e) => setPreset((p) => ({ ...p, claude: e.target.checked }))} /><span><strong>启用 Claude Code</strong><small>写入 Claude Code 配置</small></span></label>
        <label className="tool-choice"><input type="checkbox" checked={preset.codex} onChange={(e) => setPreset((p) => ({ ...p, codex: e.target.checked }))} /><span><strong>启用 GPT / Codex</strong><small>写入 Codex 模型与分组</small></span></label>
        <label className="tool-choice"><input type="checkbox" checked={preset.localization} onChange={(e) => setPreset((p) => ({ ...p, localization: e.target.checked }))} /><span><strong>中文界面</strong><small>新版客户端不兼容时保留官方语言设置</small></span></label>
        <label className="tool-choice"><input type="checkbox" checked={preset.computerUse} onChange={(e) => setPreset((p) => ({ ...p, computerUse: e.target.checked }))} /><span><strong>强开 Computer Use</strong><small>同时启用本地电脑与浏览器能力</small></span></label>
        <label className="tool-choice"><input type="checkbox" checked={preset.browser} onChange={(e) => setPreset((p) => ({ ...p, browser: e.target.checked }))} /><span><strong>Browser 兼容</strong><small>启用 Codex++ 原生 Browser 适配</small></span></label>
      </div>
      <label className="feature-toggle"><input type="checkbox" checked={rememberPreset} onChange={(e) => setRememberPreset(e.target.checked)} /><span>保留习惯，下次直接一键重启</span></label>
      <div className="row"><button className="btn" onClick={() => setRestartOpen(false)}>取消</button><button className="btn primary" onClick={() => void submitRestart()}>保存并重启</button></div>
    </div></div>}
  </>;
}

function GuideIntro({ onStart, onSkip }: { onStart: () => void; onSkip: () => void }) {
  return <div className="dialog-backdrop"><div className="dialog guide-intro" role="dialog" aria-modal="true"
    aria-labelledby="guide-intro-title">
    <div className="guide-kicker">快速上手</div><h2 id="guide-intro-title">需要新手引导吗？</h2>
    <p>新手引导会用高亮框一步步指向分组、模型、Agent 和一键配置按钮。熟悉客户端可以直接跳过。</p>
    <div className="row wrap guide-intro-actions">
      <button className="btn" onClick={onSkip}>我是老手，直接跳过</button>
      <button className="btn primary" onClick={onStart}>开始新手引导<ArrowRight size={16} /></button>
    </div>
  </div></div>;
}

function BeginnerGuide({ step, setStep, onTools, onClose }: {
  step: number; setStep: (step: number) => void; onTools: () => void; onClose: () => void;
}) {
  const steps = [
    { title: "选择分组", text: "先选择已开通的分组，客户端会自动拉取该分组的模型。", target: "group" },
    { title: "选择模型", text: "确认 Claude Code 或 Codex 使用的模型，也可以点击旁边的测试按钮检查连通性。", target: "model-options" },
    { title: "选择 Agent", text: "勾选要配置的 Claude Code 或 Codex Agent。", target: "agent-options" },
    { title: "一键应用配置", text: "点击这里创建或复用分组 Key、启动本地代理并写入工具配置。", target: "apply-config" },
    { title: "安装和汉化环境", text: "最后到“工具与修复”安装环境、Claude Code、Codex CLI，或一键汉化 Codex Desktop。", target: "tools-nav" },
  ];
  const current = steps[step];
  const [rect, setRect] = useState<DOMRect | null>(null);
  useLayoutEffect(() => {
    const target = document.getElementById(current.target);
    if (!target) return;
    target.scrollIntoView({ behavior: "smooth", block: "center", inline: "nearest" });
    const update = () => setRect(target.getBoundingClientRect());
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => { window.removeEventListener("resize", update); window.removeEventListener("scroll", update, true); };
  }, [current.target]);
  const next = () => {
    if (step === steps.length - 1) onTools();
    else setStep(step + 1);
  };
  const tooltipStyle = rect ? {
    left: `${Math.min(Math.max(rect.left, 16), window.innerWidth - 336)}px`,
    top: `${rect.bottom + 14 < window.innerHeight - 170 ? rect.bottom + 14 : Math.max(16, rect.top - 164)}px`,
  } : undefined;
  return <div className="guide-layer" aria-live="polite">
    {rect && <div className="guide-spotlight" style={{
      left: `${rect.left - 6}px`, top: `${rect.top - 6}px`,
      width: `${rect.width + 12}px`, height: `${rect.height + 12}px`,
    }} />}
    <div className="guide-tooltip" style={tooltipStyle} role="dialog" aria-modal="false"
      aria-labelledby="beginner-guide-title">
      <div className="guide-kicker">新手引导 · {step + 1}/{steps.length}</div>
      <h2 id="beginner-guide-title">{current.title}</h2><p>{current.text}</p>
      <div className="guide-dots">{steps.map((_, index) => <span key={index} className={index === step ? "active" : ""} />)}</div>
      <div className="row wrap">
        <button className="btn small" onClick={onClose}>跳过</button>
        {step > 0 && <button className="btn small" onClick={() => setStep(step - 1)}>上一步</button>}
        <button className="btn primary small" onClick={next}>{step === steps.length - 1 ? "打开工具与修复" : "下一步"}<ArrowRight size={15} /></button>
      </div>
    </div>
  </div>;
}

function ProviderControls({ flash, goSetup }: { flash: (text: string) => void; goSetup: () => void }) {
  const [policy, setPolicy] = useState<Awaited<ReturnType<typeof api.clientProviderPolicy>> | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const refresh = async () => {
    try { setPolicy(await api.clientProviderPolicy()); setError(""); }
    catch (err) { setPolicy(null); setError(message(err)); }
  };
  useEffect(() => { if (!isPreview) void refresh(); }, []);
  const changePolicy = async () => {
    if (!policy || busy) return;
    setBusy(true);
    try { await api.setClientProviderPolicy(!policy.allow_provider_switch); await refresh(); }
    catch (err) { flash(message(err)); }
    finally { setBusy(false); }
  };
  const switchProvider = async () => {
    if (!policy?.eligible || busy) return;
    setBusy(true);
    try {
      await api.switchExternalProvider(baseUrl.trim(), apiKey, model.trim());
      setApiKey(""); await refresh();
      flash("服务商已切换，会话存储保持原路径；重新启动 Codex 生效。外部服务商不提供售后。");
    } catch (err) { flash(message(err)); }
    finally { setBusy(false); }
  };
  return <section className="section"><div className="section-header"><h2>服务商管理</h2><span className="tag">{policy?.provider.external ? "外部服务商" : "jokerdeck-chatgpt"}</span></div>
    {error && <p className="subtext">权限检测失败，外部服务商切换已锁定：{error}</p>}
    {policy?.is_admin && <button className="btn" disabled={busy} onClick={() => void changePolicy()}>{policy.allow_provider_switch ? "禁止用户更换服务商" : "允许充值用户更换服务商"}</button>}
    <p className="subtext">{policy?.eligible ? "管理员已开放服务商切换。" : "默认使用 jokerdeck-chatgpt；管理员开放后，充值过的用户可更换服务商。"}外部服务商不提供售后。</p>
    {policy?.eligible && <div>
      <div className="field"><label htmlFor="provider-url">Responses API Base URL</label><input id="provider-url" className="input" value={baseUrl} onChange={event => setBaseUrl(event.target.value)} placeholder="https://provider.example/v1" autoComplete="off" /></div>
      <div className="field"><label htmlFor="provider-api-key">API Key</label><input id="provider-api-key" className="input" type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} autoComplete="new-password" /></div>
      <div className="field"><label htmlFor="provider-model">Model</label><input id="provider-model" className="input" value={model} onChange={event => setModel(event.target.value)} placeholder="服务商支持的模型 ID" /></div>
      <button className="btn primary" disabled={busy || !baseUrl.trim() || !apiKey.trim() || !model.trim()} onClick={() => void switchProvider()}>{busy ? "处理中..." : "切换外部服务商"}</button>
    </div>}
    <div className="row wrap"><button className="btn" disabled={busy} onClick={() => void refresh()}>刷新权限</button><button className="btn" disabled={busy} onClick={goSetup}>返回配置页使用默认服务商</button></div>
    <small className="subtext">切换前关闭 Codex 和正在运行的 Codex CLI；配置使用同一 provider ID 和会话目录，不移动、不删除聊天记录。默认服务商通过配置页“一键应用配置”恢复。</small>
  </section>;
}

function Tools({ boot, flash, refreshBoot, goSetup, mode = "maintenance" }: {
  mode?: "maintenance" | "computer";
  boot: Bootstrap | null;
  flash: (text: string) => void;
  refreshBoot: () => Promise<Bootstrap>;
  goSetup: () => void;
}) {
  const [report, setReport] = useState<CliReport | null>(null);
  const [diag, setDiag] = useState<DiagReport | null>(null);
  const [busy, setBusy] = useState("");
  const [log, setLog] = useState("");
  const [downloadProgress, setDownloadProgress] = useState<{ percent: number; detail: string } | null>(null);
  const [downloadDetails, setDownloadDetails] = useState<string[]>([]);
  const [desktopRunning, setDesktopRunning] = useState<boolean | null>(null);
  const [confirmStop, setConfirmStop] = useState(false);
  const [extensionBusy, setExtensionBusy] = useState(false);
  const [nativeBrowser, setNativeBrowser] = useState<Awaited<ReturnType<typeof api.nativeBrowserStatus>> | null>(null);
  const [nativeBrowserBusy, setNativeBrowserBusy] = useState(false);
  const [computerTools, setComputerTools] = useState<Awaited<ReturnType<typeof api.computerToolsStatus>> | null>(null);
  const [computerToolsBusy, setComputerToolsBusy] = useState(false);
  useEffect(() => { if (!isPreview) api.computerToolsStatus().then(setComputerTools).catch(err => flash(message(err))); }, []);
  const toggleComputerTools = async () => {
    if (!computerTools || computerToolsBusy) return;
    setComputerToolsBusy(true);
    try { setComputerTools(await api.configureComputerTools(!computerTools.configured)); flash("设置已保存，请通过客户端重启 Codex 生效"); }
    catch (err) { flash(message(err)); }
    finally { setComputerToolsBusy(false); }
  };
  const refreshNativeBrowser = async () => {
    try { setNativeBrowser(await api.nativeBrowserStatus()); }
    catch (err) { flash(message(err)); }
  };
  useEffect(() => { if (!isPreview) void refreshNativeBrowser(); }, []);
  const toggleNativeBrowser = async () => {
    if (!nativeBrowser || nativeBrowserBusy) return;
    setNativeBrowserBusy(true);
    try {
      await api.configureNativeBrowser(!nativeBrowser.enabled);
      await refreshNativeBrowser();
    } catch (err) { flash(message(err)); }
    finally { setNativeBrowserBusy(false); }
  };
  const refreshDesktop = async () => {
    if (isPreview) return;
    try { setDesktopRunning(await api.codexDesktopRunning()); }
    catch (err) { setDesktopRunning(null); flash(message(err)); }
  };
  useEffect(() => {
    if (isPreview) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    listen<{ percent: number; detail: string }>("codex-desktop-download-progress", ({ payload }) => {
      if (disposed) return;
      setDownloadProgress(payload);
      setDownloadDetails((lines) => [...lines.slice(-79), payload.detail]);
    }).then((off) => { if (disposed) off(); else unsubscribe = off; }).catch(() => {});
    return () => { disposed = true; unsubscribe?.(); };
  }, []);
  const detect = async () => {
    setBusy("detect");
    try {
      const [clis, running] = await Promise.all([api.detectClis(), api.codexDesktopRunning()]);
      setReport(clis);
      setDesktopRunning(running);
    } catch (err) { setLog(message(err)); }
    finally { setBusy(""); }
  };
  useEffect(() => { detect(); }, []);
  const install = async (which: string) => {
    setBusy(which); setLog("安装中...");
    try {
      const result = await api.installCli(which);
      setLog(result.log || (result.ok ? "安装完成" : "安装失败"));
      setReport(await api.detectClis());
    } catch (err) { setLog(message(err)); }
    finally { setBusy(""); }
  };
  const copyInstallLog = async () => {
    try {
      await writeText(log);
      flash("安装日志已复制");
    } catch (error) {
      flash(`复制日志失败：${message(error)}`);
    }
  };
  const downloadCodexDesktop = async () => {
    setBusy("desktop-download");
    setLog("正在准备 Codex Desktop 安装...");
    setDownloadProgress({ percent: 0, detail: "准备安装" });
    setDownloadDetails([]);
    try {
      const result = await api.downloadCodexDesktop();
      setLog(result.log || (result.ok ? "安装完成" : "安装失败"));
      setReport(await api.detectClis());
    } catch (err) {
      const detail = message(err);
      setLog(detail);
      setDownloadProgress({ percent: 0, detail: "Codex Desktop 下载失败" });
      setDownloadDetails((lines) => [...lines.slice(-79), detail]);
    }
    finally { setBusy(""); }
  };
  const diagnose = async () => {
    setBusy("diag"); setDiag(null);
    try { setDiag(await api.diagnostics()); } catch (err) { flash(message(err)); }
    finally { setBusy(""); }
  };
  const toggleComputerUse = async () => {
    if (!boot || extensionBusy) return;
    setExtensionBusy(true);
    try {
      await api.setExtensions(!boot.computer_use);
      await refreshBoot();
      flash(boot.computer_use ? "Anthropic Beta 标记已关闭" : "Anthropic Beta 标记已启用；此标记不代表本地工具已可用");
    } catch (err) { flash(message(err)); }
    finally { setExtensionBusy(false); }
  };
  const controlDesktop = async (action: "start" | "stop") => {
    setConfirmStop(false);
    setBusy(action);
    try {
      if (action === "stop") {
        await api.stopCodexDesktop();
        flash("Codex Desktop 已关闭");
      } else {
        const result = await api.restartCodex();
        if (!result.ok) throw new Error(result.log);
        flash(result.log);
      }
      await refreshDesktop();
    } catch (err) { flash(message(err)); await refreshDesktop(); }
    finally { setBusy(""); }
  };
  return <>
    <section className="section"><div className="section-header"><h2>Codex Desktop</h2>
      <button className="icon-button" title="刷新运行状态" aria-label="刷新运行状态"
        disabled={!!busy} onClick={() => void refreshDesktop()}><RefreshCw size={17} /></button></div>
      <div className="list-row"><span className="row"><Monitor size={18} />运行状态</span>
        <span className={`tag ${desktopRunning ? "success" : desktopRunning === false ? "warning" : ""}`}>
          {desktopRunning === null ? "检测中" : desktopRunning ? "运行中" : "未运行"}</span></div>
      <div className="row wrap">
        <button className="btn primary" disabled={!!busy || !report?.codex_desktop.installed || boot?.desktop_supported === false}
          onClick={() => void controlDesktop("start")}><Power size={16} />{busy === "start" ? "启动中..." : desktopRunning ? "重启" : "启动"}</button>
        <button className="btn" disabled={!!busy || !desktopRunning} onClick={() => setConfirmStop(true)}>
          <X size={16} />关闭</button>
      </div>
    </section>
    {mode === "computer" && <>
    <section className="section extension-section"><div className="section-header"><h2>原生 Computer Use / Browser</h2><span className="tag">{computerTools?.configured ? "开启" : "关闭"}</span></div>
      <p className="subtext">{computerTools?.detail ?? "正在检测..."}</p>
      <div className="row wrap"><button className="btn primary" disabled={computerToolsBusy || !computerTools || (!computerTools.configured && !computerTools.platform_supported)} onClick={() => void toggleComputerTools()}>{computerToolsBusy ? "处理中..." : computerTools?.configured ? "关闭原生增强" : "开启电脑和浏览器功能"}</button><button className="btn" disabled={computerToolsBusy} onClick={() => void api.computerToolsStatus().then(setComputerTools).catch(err => flash(message(err)))}>重新检测</button></div>
      <small className="subtext">从客户端启动定制副本生效，原应用保持完整。原有操作审批及系统权限继续生效，服务端能力仍需实际调用验证。外部浏览器需安装并连接浏览器扩展。</small>
    </section>
    <section className="section extension-section"><div className="section-header"><h2>Codex++ 原生 Browser 兼容</h2><span className="tag">{nativeBrowser?.enabled ? "兼容模式开启" : "兼容模式关闭"}</span></div>
      <p className="subtext">沿用 Codex++ 1.6.0 的原生 CUA runtime 适配、版本结构检查、备份与恢复。需要安装并连接原生 Edge / Chrome 扩展。</p>
      <div className="row wrap"><button className="btn primary" disabled={!nativeBrowser || nativeBrowserBusy} onClick={() => void toggleNativeBrowser()}>{nativeBrowserBusy ? "处理中..." : nativeBrowser?.enabled ? "关闭兼容" : "开启兼容"}</button><button className="btn" disabled={nativeBrowserBusy} onClick={() => void refreshNativeBrowser()}>检测连接</button></div>
      <p className="subtext">Runtime：{nativeBrowser?.runtime.detail ?? "尚未检测"}</p>
      <p className="subtext">扩展连接：{nativeBrowser?.connection.browsers.map(browser => `${browser.family}${browser.recognized ? "（已识别）" : "（未知扩展）"}`).join("、") || "未检测到已连接扩展"}</p>
      <small className="subtext">受控标签页请求可能携带 x-browser-agent 标识。关闭兼容后，扩展保留的标识设置仍需在扩展内关闭。电脑操作是否可用需由实际工具调用验证。</small>
    </section>
    <section className="section extension-section"><div className="section-header"><h2>Anthropic Computer Use Beta 标记</h2><span className="tag">{boot?.computer_use ? "已启用标记" : "未启用"}</span></div>
      <p className="subtext">为代理请求添加 Anthropic Beta 标记。本地电脑操作和 Browser 执行需要独立的 runtime 与工具接入。</p>
      <div className="row wrap">
        <button className={`btn ${boot?.computer_use ? "" : "primary"}`} disabled={!boot || extensionBusy || boot.desktop_supported === false} onClick={() => void toggleComputerUse()}>
          <ShieldCheck size={16} />{extensionBusy ? "处理中..." : boot?.computer_use ? "关闭标记" : "启用标记"}
        </button>
        {boot?.computer_use && <span className="tag success">请求头已注入</span>}
      </div>
      <small className="subtext">此开关只添加请求标记，不安装或注册电脑操作工具。</small>
    </section>
    </>}
    {mode === "maintenance" && <>
    <section className="section"><div className="section-header"><h2>工具安装</h2><button className="icon-button" title="重新检测" aria-label="重新检测"
      disabled={!!busy} onClick={detect}><RefreshCw size={17} className={busy === "detect" ? "spin" : ""} /></button></div>
      {([
        ["Node.js", "node", report?.node], ["npm", "", report?.npm], ["Claude Code", "claude", report?.claude],
        ["Codex CLI", "codex", report?.codex], ["Codex Desktop", "desktop", report?.codex_desktop],
        ...(navigator.platform.toLowerCase().includes("mac") && report?.chatgpt_desktop.installed && !report?.codex_desktop.installed
          ? [["ChatGPT Desktop", "", report.chatgpt_desktop] as const] : []),
      ] as const).map(([label, which, item]) => <div className="list-row" key={label}><span className="row"><Monitor size={18} />
        <span><strong>{label}</strong><small className="subtext">{item?.version ?? (item?.installed ? "已检测到" : "未检测到")}</small></span></span>
        {item?.installed ? <span className="tag success">已安装</span> : which ? <button className="btn small" disabled={!!busy || boot?.desktop_supported === false}
          onClick={() => which === "desktop" ? void downloadCodexDesktop() : install(which)}>
          <Download size={14} />{which === "desktop" ? (busy === "desktop-download" ? "安装中..." : "下载并安装") : busy === which ? "安装中..." : "安装"}</button> : <span className="tag warning">缺失</span>}</div>)}
      {log && <div className="install-log"><button type="button" className="icon-button" title="复制安装日志"
        aria-label="复制安装日志" onClick={() => void copyInstallLog()}><Copy size={16} /></button>
        <pre className="log">{log}</pre></div>}
      {navigator.platform.toLowerCase().includes("mac") && report?.chatgpt_desktop.installed && !report?.codex_desktop.installed &&
        <small className="subtext">已检测到 {report.chatgpt_desktop.path}，但不是支持 Codex 的新版 ChatGPT Desktop（旧版 Classic 不支持此处汉化）。请安装新版后重新检测。</small>}
      {navigator.platform.toLowerCase().includes("mac") && !report?.codex_desktop.installed &&
        <small className="subtext">下载遇到网络问题？<a href="https://persistent.oaistatic.com/codex-app-prod/Codex.dmg" target="_blank" rel="noopener noreferrer">Apple Silicon DMG</a> · <a href="https://persistent.oaistatic.com/codex-app-prod/Codex-latest-x64.dmg" target="_blank" rel="noopener noreferrer">Intel DMG</a> · <a href="https://learn.chatgpt.com/docs/app" target="_blank" rel="noopener noreferrer">官方应用页</a>。安装后点击重新检测。</small>}
      {downloadProgress && <div className="localization-progress" aria-live="polite">
        <div className="localization-progress-heading"><strong>{downloadProgress.detail}</strong><span>{downloadProgress.percent}%</span></div>
        <div className="progress-track" role="progressbar" aria-valuenow={downloadProgress.percent} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${downloadProgress.percent}%` }} /></div>
        <details open={busy === "desktop-download"}><summary>下载详情</summary>
          <pre className="log localization-log">{downloadDetails.join("\n")}</pre>
        </details>
      </div>}
    </section>
    <section className="section"><div className="section-header"><h2>诊断与修复</h2><Stethoscope size={18} /></div>
      <div className="row wrap"><button className="btn primary" disabled={!!busy || boot?.desktop_supported === false} onClick={diagnose}>
        <Stethoscope size={16} />{busy === "diag" ? "检测中..." : "开始检测（少量计费）"}</button>
        <button className="btn" disabled={!!busy} onClick={goSetup}><Settings2 size={16} />重新应用配置</button></div>
      {diag && <div className="diagnostic"><div className={`alert ${diag.overall_ok ? "success" : "error"}`}>{diag.overall_ok ? "全部通过" : "检测发现问题"}</div>
        {diag.items.map((item, index) => <div className="list-row" key={index}><span className="row">
          <span className={`dot ${item.ok ? "ok" : "error"}`} />{item.name}</span><span className="detail">{item.detail}</span></div>)}</div>}
    </section>
    </>}
    {confirmStop && <div className="dialog-backdrop"><div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="stop-codex-title">
      <h2 id="stop-codex-title">关闭 Codex Desktop</h2>
      <p>将强制关闭 Codex Desktop。请先保存正在编辑的内容。</p>
      <div className="row"><button className="btn" onClick={() => setConfirmStop(false)}>取消</button>
        <button className="btn danger" onClick={() => void controlDesktop("stop")}><Power size={16} />确认关闭</button></div>
    </div></div>}
  </>;
}

function Account({ user, version, flash, onUpdate }: {
  user: UserInfo; version?: string; flash: (text: string) => void; onUpdate: (update: UpdateInfo) => void;
}) {
  return <>
    <section className="section"><div className="section-header"><h2>账户信息</h2><UserRound size={18} /></div>
      <div className="list-row"><span>邮箱</span><span className="break">{user.email}</span></div>
      <div className="list-row"><span>余额</span><strong>${user.balance.toFixed(2)}</strong></div>
      <div className="list-row"><span>客户端版本</span><span>v{version ?? "0.1.0"}</span></div>
    </section>
    <section className="section"><div className="section-header"><h2>支持</h2><CircleHelp size={18} /></div>
      <button className="btn" onClick={() => api.checkUpdate().then((update) => {
        if (update.update_available) {
          onUpdate(update);
          return;
        }
        flash(update.error ?? "暂无可用更新");
      }).catch((err) => flash(message(err)))}><RefreshCw size={16} />检查更新</button>
    </section>
  </>;
}
