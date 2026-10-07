function (config) {
  if (!config.url || !config.key || window.__jokerdeckFeatures) return;
  window.__jokerdeckFeatures = true;
  const palettes = {
    rose: ["#fff7fb", "#fcecf5", "#3d2340", "#79627d", "#eed9e5", "#a93f79"],
    midnight: ["#131927", "#202a3b", "#eef3ff", "#aebbd2", "#344158", "#8faeff"],
    jade: ["#f3faf7", "#e3f2ea", "#193b30", "#5e7e70", "#cfe4d8", "#177d59"]
  };
  const style = document.createElement("style");
  style.id = "jokerdeck-feature-style";
  style.textContent = `
    body,main{transition:background-color .35s ease,color .25s ease,border-color .25s ease}
    .jd-usage{display:flex;align-items:center;gap:8px;margin-inline:8px auto;min-width:0;max-width:390px;color:var(--color-text-secondary,#64748b);font:11px/1.4 system-ui;letter-spacing:.1px}
    .jd-usage-chip{display:flex;gap:5px;align-items:center;padding:4px 8px;border-radius:9px;background:color-mix(in srgb,currentColor 6%,transparent);white-space:nowrap;transition:background .25s}
    .jd-usage-chip strong{font-weight:600;font-variant-numeric:tabular-nums;color:var(--color-text-primary,inherit)}
    .jd-usage[data-offline=true]{opacity:.65}.jd-usage svg{flex:none;opacity:.8}
    @media(max-width:760px){.jd-usage{gap:4px;font-size:10px}.jd-usage-chip{padding:3px 5px}.jd-usage .jd-cost{display:none}}
    @media(prefers-reduced-motion:reduce){body,main,.jd-usage-chip{transition:none}}
  `;
  document.head.appendChild(style);
  const themeStyle = document.createElement("style");
  themeStyle.id = "jokerdeck-codex-theme";
  document.head.appendChild(themeStyle);
  let snapshot = null, lastThread = null, stopped = false, timer;
  const animation = new WeakMap();
  const compact = (value) => Number.isFinite(value) ? new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value) : "—";
  const dollars = (value) => Number.isFinite(value) ? `$${value < .01 && value > 0 ? value.toFixed(4) : value.toFixed(2)}` : "—";
  const routeThread = () => {
    const matches = location.pathname.match(/\/(?:local|threads|thread)\/([0-9a-f-]{36})(?:\/|$)/i);
    return matches ? matches[1] : null;
  };
  function animate(node, value) {
    const previous = animation.get(node);
    if (previous?.frame) cancelAnimationFrame(previous.frame);
    if (!Number.isFinite(value)) { node.textContent = "—"; animation.delete(node); return; }
    const from = previous?.value ?? value;
    if (from === value || matchMedia("(prefers-reduced-motion: reduce)").matches) { node.textContent = compact(value); animation.set(node, { value }); return; }
    const start = performance.now();
    const step = (now) => {
      const fraction = Math.min(1, (now - start) / 450);
      const current = from + (value - from) * (1 - Math.pow(1 - fraction, 3));
      node.textContent = compact(current);
      animation.set(node, { value: current, frame: fraction < 1 ? requestAnimationFrame(step) : null });
    };
    requestAnimationFrame(step);
  }
  function applyTheme(theme) {
    const palette = palettes[theme];
    if (!palette) { themeStyle.textContent = ""; return; }
    const [bg, secondary, text, muted, border, accent] = palette;
    const css = `:root,:root.dark,html body{--color-background-primary:${bg};--color-background-secondary:${secondary};--color-background-secondary-soft-alpha:${secondary};--color-background-tertiary:${secondary};--color-text-primary:${text};--color-text-secondary:${muted};--color-text-tertiary:${muted};--color-border-primary:${border};--color-border-secondary:${border};--color-token-main-surface-primary:${bg};--color-token-main-surface-secondary:${secondary};--color-token-text-primary:${text};--color-token-text-secondary:${muted};--color-token-border-light:${border};--color-surface:${bg};--color-surface-secondary:${secondary};--color-accent:${accent};background-color:${bg};color:${text}}`;
    if (themeStyle.textContent !== css) themeStyle.textContent = css;
  }
  function mount() {
    const footer = document.querySelector("[data-composer-footer-responsive]");
    if (!footer || footer.querySelector(".jd-usage")) return;
    const bar = document.createElement("div");
    bar.className = "jd-usage";
    bar.setAttribute("role", "status");
    bar.innerHTML = '<span class="jd-usage-chip"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 19V9m8 10V4m8 15v-6"/></svg><span>今日</span><strong data-jd="today">—</strong><span class="jd-cost" data-jd="today-cost">—</span></span><span class="jd-usage-chip"><span>本会话</span><strong data-jd="session">—</strong><span class="jd-cost" data-jd="session-cost">—</span></span>';
    // Append within the official composer footer; never replace its permissions or send controls.
    footer.insertBefore(bar, footer.lastElementChild);
    render();
  }
  function render() {
    const bar = document.querySelector(".jd-usage");
    if (!bar) return;
    bar.hidden = snapshot?.overlay === false;
    bar.style.display = snapshot?.overlay === false ? "none" : "";
    const usage = snapshot?.session_tokens;
    animate(bar.querySelector('[data-jd="today"]'), snapshot?.today_tokens);
    animate(bar.querySelector('[data-jd="session"]'), usage?.total_tokens);
    bar.querySelector('[data-jd="today-cost"]').textContent = dollars(snapshot?.today_cost);
    const cost = bar.querySelector('[data-jd="session-cost"]');
    cost.textContent = Number.isFinite(snapshot?.session_cost) ? (snapshot.session_cost_complete ? "" : "≈") + dollars(snapshot.session_cost) : "—";
    cost.title = snapshot?.session_cost_complete ? "本会话已匹配的实际扣费" : "最近100条账单中匹配此会话的实际扣费，可能尚未完整结算";
    bar.dataset.offline = String(Boolean(snapshot?.error));
    bar.title = snapshot?.error || `账户累计 ${compact(snapshot?.total_tokens)} Token · ${dollars(snapshot?.total_cost)}\n本会话输入 ${compact(usage?.input_tokens)} · 缓存 ${compact(usage?.cached_input_tokens)} · 输出 ${compact(usage?.output_tokens)}\n费用来自中转实际扣费；未匹配账单显示 —`;
  }
  const observer = new MutationObserver(() => { mount(); if (routeThread() !== lastThread) { lastThread = routeThread(); snapshot = snapshot ? { ...snapshot, session_tokens: null, session_cost: null } : null; render(); } });
  observer.observe(document.body, { childList: true, subtree: true });
  async function refresh() {
    if (stopped) return;
    try {
      if (!document.hidden) {
        const thread = routeThread();
        const url = new URL(config.url); url.searchParams.set("key", config.key);
        if (thread) url.searchParams.set("thread", thread);
        const response = await fetch(url, { cache: "no-store", signal: AbortSignal.timeout(15000) });
        if (!response.ok) throw new Error("client 统计服务未连接");
        const data = await response.json();
        if (routeThread() === thread) { snapshot = data; lastThread = thread; applyTheme(data.theme); mount(); render(); }
      }
    } catch { snapshot = { ...snapshot, error: "client 未连接，账单暂不可用" }; render(); }
    finally { if (!stopped) timer = setTimeout(refresh, 8000); }
  }
  window.addEventListener("pagehide", () => { stopped = true; clearTimeout(timer); observer.disconnect(); }, { once: true });
  mount(); refresh();
}
