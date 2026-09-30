import { useEffect, useRef, useState, type FormEvent, type MouseEvent as ReactMouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowRight, Check, ChevronRight, CircleHelp, Download, Eye, EyeOff,
  KeyRound, LayoutDashboard, LogOut, Maximize2, Minimize2, Minus, Monitor, RefreshCw, CreditCard,
  ChartNoAxesCombined, Store, Languages,
  Settings2, ShieldCheck, Stethoscope, UserRound, Wallet, X, Power, RotateCcw, Bell, Users, Folder, Globe, Receipt, SlidersHorizontal,
} from "lucide-react";
import { api, isPreview, type Bootstrap, type UserInfo, type ProxyStatus, type CliReport,
  type DiagReport, type PlazaGroup, type ApplyResult, type PublicAuthSettings,
  type GroupModels, type ModelProbe, type ToolConfigView, type UpdateInfo } from "./api";

type SiteTab = typeof allSiteTabs[number]["id"];
type Tab = "setup" | "tools" | "account" | SiteTab;
type AuthPage = "register" | "forgot-password" | "reset-password";
const logo = new URL("../icon-source.png", import.meta.url).href;
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const tabs = [
  { id: "setup", title: "控制台", icon: LayoutDashboard },
  { id: "tools", title: "工具与修复", icon: Stethoscope },
  { id: "account", title: "账户中心", icon: UserRound },
] as const;
const siteTabs = [
  { id: "dashboard", title: "网站概览", icon: LayoutDashboard },
  { id: "keys", title: "API Key", icon: KeyRound },
  { id: "usage", title: "使用记录", icon: ChartNoAxesCombined },
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
    <span className="titlebar-brand"><img src={logo} alt="" />jokerdeck</span>
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
  const [tab, setTab] = useState<Tab>("setup");
  const [error, setError] = useState("");
  const [toast, setToast] = useState("");
  const [closePrompt, setClosePrompt] = useState(false);
  const [closing, setClosing] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
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
      if ((await api.proxyStatus()).running) setClosePrompt(true);
      else await api.quitApp(false);
    } catch (err) { flash(message(err)); }
  };
  const confirmClose = async () => {
    setClosing(true);
    try { await api.quitApp(true); }
    catch (err) { flash(message(err)); setClosing(false); }
  };
  const installUpdate = async () => {
    if (!update?.url) return;
    try {
      const result = await api.installUpdate(update.url);
      setUpdate(null);
      flash(result.message);
    } catch (err) { flash(message(err)); }
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
          <div className="sidebar-brand"><img src={logo} alt="" /><div><strong>jokerdeck</strong><small>中转服务</small></div></div>
          <span className="nav-label">工作空间</span>
          <nav>{tabs.map(({ id, title, icon: Icon }) => <button key={id}
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
          {siteTab ? <div ref={siteArea} className="site-area" /> : <>
          <div className="page-header"><div><span className="breadcrumb">工作空间 / {tabs.find((item) => item.id === tab)?.title}</span>
            <h1>{tabs.find((item) => item.id === tab)?.title}</h1></div>
            <button className="btn" onClick={() => setTab("store")}><Store size={16} />店铺销售</button></div>
          {tab === "setup" &&
            <Setup user={user} setUser={setUser} boot={boot} refreshBoot={refreshBoot} flash={flash} />}
          {tab === "tools" && <Tools boot={boot} flash={flash} goSetup={() => setTab("setup")} />}
          {tab === "account" && <Account user={user} version={boot?.version} flash={flash} onUpdate={setUpdate} />}
          </>}
        </main>
      </div>}
    {toast && <div className="toast" role="status">{toast}</div>}
    {closePrompt && <div className="dialog-backdrop"><div className="dialog" role="alertdialog" aria-modal="true"
      aria-labelledby="close-title"><h2 id="close-title">代理仍在运行</h2>
      <p>请先关闭代理并恢复配置。确认后将自动恢复 Claude Code / Codex 配置、停止代理，再关闭客户端。</p>
      <div className="row wrap"><button className="btn" disabled={closing} onClick={() => setClosePrompt(false)}>取消</button>
        <button className="btn primary" disabled={closing} onClick={confirmClose}>{closing ? "处理中..." : "确认并退出"}</button></div>
    </div></div>}
    {update && <div className="dialog-backdrop"><div className="dialog" role="dialog" aria-modal="true"
      aria-labelledby="update-title"><h2 id="update-title">发现新版本 v{update.latest}</h2>
      <p className="update-notes">{update.notes || "新版本已发布，请更新客户端。"}</p>
      <div className="row wrap"><button className="btn" onClick={() => setUpdate(null)}>稍后</button>
        <button className="btn primary" disabled={!update.url} onClick={installUpdate}>
          <Download size={16} />下载并安装</button></div>
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
  const [line, setLine] = useState("正在检测线路");
  useEffect(() => {
    if (isPreview) return;
    api.probeHosts().then((hosts) => {
      const reachable = hosts.find((host) => host.healthy);
      setLine(reachable ? `已连接 ${reachable.host}` : "线路未连通，请检查网络或中转地址");
    }).catch(() => setLine("线路检测失败"));
  }, []);
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
      <div className="line-status"><span className={`dot ${line.startsWith("已连接") ? "ok" : ""}`} />{line}</div>
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

function Setup({ user, setUser, boot, refreshBoot, flash }: {
  user: UserInfo; setUser: (user: UserInfo) => void; boot: Bootstrap | null;
  refreshBoot: () => Promise<Bootstrap>; flash: (text: string) => void;
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
  const loadModels = async (id: number) => {
    const revision = ++modelRevision.current;
    setGroupModels(null); setModelsBusy(true); setModelsError(""); setProbes({});
    try {
      const models = await api.groupModels(id);
      if (revision === modelRevision.current) setGroupModels(models);
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
    try {
      const applied = await api.applyConfig(claude && !!claudeModels.length, codex && !!codexModels.length,
        groupId, selectedClaudeModel || undefined, selectedCodexModel || undefined);
      setResult(applied);
      setStatus(await api.proxyStatus());
      const next = await refreshBoot();
      setClaudeModel(next.claude_model ?? ""); setCodexModel(next.codex_model ?? "");
      flash("分组 Key、线路和模型配置已应用");
    } catch (err) { setError(message(err)); }
    finally { setBusy(false); }
  };
  const stopProxy = async () => {
    if (busy) return;
    setBusy(true); setError(""); setResult(null);
    try {
      setStatus(await api.stopProxy());
      flash("本地代理已关闭；再次应用配置即可启动");
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
    setTesting(tool); setProbes((current) => ({ ...current, [tool]: undefined }));
    try {
      const result = await api.testGroupModel(groupId, tool, model);
      setProbes((current) => ({ ...current, [tool]: result }));
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
        <button className="icon-button" title="拉取当前分组模型" aria-label="拉取当前分组模型"
          disabled={!groupId || modelsBusy || busy} onClick={() => groupId && loadModels(groupId)}>
          <RefreshCw size={17} className={modelsBusy ? "spin" : ""} /></button>
        <button className="icon-button" title="刷新分组" aria-label="刷新分组"
          disabled={loading || busy} onClick={load}><RefreshCw size={17} /></button></div></div>
      <div className="field"><label htmlFor="group">分组</label><select id="group" className="input" value={groupId ?? ""}
        disabled={busy || loading} onChange={(event) => selectGroup(Number(event.target.value))}>
        <option value="" disabled>{loading ? "加载中..." : "选择分组"}</option>
        {groups.map((item) => <option key={item.id} value={item.id}>{item.name}
          {item.is_exclusive ? " · 专属" : ""}{item.multiplier != null ? ` · ${item.multiplier}x` : ""}</option>)}</select></div>
      {group && <div className="selection-summary"><span>{group.name}</span><span className={`tag ${applied ? "success" : "warning"}`}>
        {applied ? "代理当前分组" : "待应用"}</span>{group.description && <small>{group.description}</small>}</div>}
      {!loading && !groups.length && !error && <div className="alert warning">没有可配置分组，请在中转站检查授权或订阅。</div>}
      {modelsError && <div className="alert error" role="alert">{modelsError}</div>}
      {groupModels?.codex_error && <div className="alert warning">{groupModels.codex_error}</div>}
      <div className="model-grid">
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
      <div className="tool-options">
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
        <button className="btn primary" disabled={busy || loading || modelsBusy || !group || boot?.desktop_supported === false
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
  </>;
}

function Tools({ boot, flash, goSetup }: { boot: Bootstrap | null; flash: (text: string) => void; goSetup: () => void }) {
  const [report, setReport] = useState<CliReport | null>(null);
  const [diag, setDiag] = useState<DiagReport | null>(null);
  const [busy, setBusy] = useState("");
  const [log, setLog] = useState("");
  const detect = async () => {
    setBusy("detect");
    try { setReport(await api.detectClis()); } catch (err) { setLog(message(err)); }
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
  const diagnose = async () => {
    setBusy("diag"); setDiag(null);
    try { setDiag(await api.diagnostics()); } catch (err) { flash(message(err)); }
    finally { setBusy(""); }
  };
  const restartCodex = async () => {
    setBusy("restart-codex"); setLog("正在重启 Codex Desktop...");
    try {
      const result = await api.restartCodex();
      setLog(result.log || (result.ok ? "Codex Desktop 已重启" : "Codex Desktop 重启失败"));
      if (!result.ok) flash(result.log || "Codex Desktop 重启失败");
    } catch (err) { setLog(message(err)); }
    finally { setBusy(""); }
  };
  const localize = async (action: "install" | "uninstall" | "launch") => {
    if (action === "uninstall" && !window.confirm("恢复 Codex Desktop 英文界面？汉化补丁会从本机移除。")) return;
    setBusy(`locale-${action}`);
    setLog(action === "install" ? "正在下载并校验汉化包..." : "正在处理 Codex 汉化...");
    try { setLog(await api.codexLocalization(action)); }
    catch (err) { setLog(message(err)); }
    finally { setBusy(""); }
  };
  return <>
    <section className="section"><div className="section-header"><h2>工具安装</h2><button className="icon-button" title="重新检测" aria-label="重新检测"
      disabled={!!busy} onClick={detect}><RefreshCw size={17} className={busy === "detect" ? "spin" : ""} /></button></div>
      {([
        ["Node.js", "node", report?.node], ["npm", "", report?.npm], ["Claude Code", "claude", report?.claude],
        ["Codex CLI", "codex", report?.codex], ["Codex Desktop", "desktop", report?.codex_desktop],
      ] as const).map(([label, which, item]) => <div className="list-row" key={label}><span className="row"><Monitor size={18} />
        <span><strong>{label}</strong><small className="subtext">{item?.version ?? (item?.installed ? "已检测到" : "未检测到")}</small></span></span>
        {item?.installed ? <span className="tag success">已安装</span> : which ? <button className="btn small" disabled={!!busy || boot?.desktop_supported === false}
          onClick={() => which === "desktop" ? api.openUrl("https://developers.openai.com/codex/app").catch((err) => flash(message(err))) : install(which)}>
          <Download size={14} />{which === "desktop" ? "下载" : busy === which ? "安装中..." : "安装"}</button> : <span className="tag warning">缺失</span>}</div>)}
      {log && <pre className="log">{log}</pre>}
    </section>
    <section className="section"><div className="section-header"><h2>Codex Desktop</h2><Monitor size={18} /></div>
      <div className="row wrap">
        <button className="btn" disabled={!!busy || !report?.codex_desktop.installed || boot?.desktop_supported === false}
          onClick={restartCodex}><RefreshCw size={16} className={busy === "restart-codex" ? "spin" : ""} />
          {busy === "restart-codex" ? "重启中..." : "一键重启 Codex"}</button>
      </div>
    </section>
    <section className="section"><div className="section-header"><h2>Codex Desktop 汉化</h2></div>
      <p className="subtext">非官方中文补丁（xqnode/codex-zh-CN v0.1.2），会备份并修改 Codex Desktop 本地资源。可能需要管理员授权；Codex 更新后可能需要重新汉化。仅支持 Windows。</p>
      <div className="row wrap">
        <button className="btn primary" disabled={!!busy || !report?.codex_desktop.installed || !report?.node.installed || boot?.desktop_supported === false}
          onClick={() => localize("install")}><Languages size={16} />{busy === "locale-install" ? "汉化中..." : "一键汉化 Codex"}</button>
        <button className="btn" disabled={!!busy || boot?.desktop_supported === false} onClick={() => localize("launch")}>
          <Monitor size={16} />启动汉化版</button>
        <button className="btn" disabled={!!busy || boot?.desktop_supported === false} onClick={() => localize("uninstall")}>
          <RotateCcw size={16} />恢复英文</button>
      </div>
      {!report?.node.installed && <small className="subtext">需先安装 Node.js。</small>}
    </section>
    <section className="section"><div className="section-header"><h2>诊断与修复</h2><Stethoscope size={18} /></div>
      <div className="row wrap"><button className="btn primary" disabled={!!busy || boot?.desktop_supported === false} onClick={diagnose}>
        <Stethoscope size={16} />{busy === "diag" ? "检测中..." : "开始检测（少量计费）"}</button>
        <button className="btn" disabled={!!busy} onClick={goSetup}><Settings2 size={16} />重新应用配置</button></div>
      {diag && <div className="diagnostic"><div className={`alert ${diag.overall_ok ? "success" : "error"}`}>{diag.overall_ok ? "全部通过" : "检测发现问题"}</div>
        {diag.items.map((item, index) => <div className="list-row" key={index}><span className="row">
          <span className={`dot ${item.ok ? "ok" : "error"}`} />{item.name}</span><span className="detail">{item.detail}</span></div>)}</div>}
    </section>
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
