import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

const root = document.getElementById("root") as HTMLElement;

/** True once React has committed its first render. */
let mounted = false;

function escapeHtml(s: string) {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c] as string));
}

function describe(detail: unknown) {
  return detail instanceof Error
    ? `${detail.message}\n\n${detail.stack ?? ""}`
    : String(detail);
}

/**
 * Before React mounts, replace the page with the error — otherwise a startup
 * failure just looks like a blank white window.
 * After mount, never tear down the UI: append a dismissible banner instead, so
 * a harmless background failure can't blank a working screen.
 */
function report(label: string, detail: unknown) {
  const text = describe(detail);
  console.error(`[jokerdeck] ${label}:`, detail);

  if (mounted) {
    const bar = document.createElement("div");
    bar.style.cssText =
      "position:fixed;left:12px;right:12px;bottom:12px;z-index:9999;background:#fef0f0;" +
      "border:1px solid #fde2e2;border-left:3px solid #f56c6c;border-radius:4px;" +
      "padding:10px 14px;font:12px/1.6 Consolas,Monaco,monospace;color:#f56c6c;" +
      "max-height:140px;overflow:auto;cursor:pointer";
    bar.textContent = `⚠ ${label}：${text.split("\n")[0]}   （点击关闭，详情见控制台）`;
    bar.onclick = () => bar.remove();
    document.body.appendChild(bar);
    return;
  }

  root.innerHTML = `
    <div style="font-family:Consolas,Monaco,monospace;padding:24px;color:#f56c6c;background:#fef0f0;
                height:100%;overflow:auto;white-space:pre-wrap;font-size:13px;line-height:1.6">
      <div style="font-size:16px;font-weight:700;margin-bottom:10px">⚠ ${label}</div>
      <div>把下面这段发给开发者即可定位问题：</div>
      <hr style="border:none;border-top:1px solid #fde2e2;margin:12px 0" />
      ${escapeHtml(text)}
      <hr style="border:none;border-top:1px solid #fde2e2;margin:12px 0" />
      <div style="color:#909399">UA: ${escapeHtml(navigator.userAgent)}</div>
    </div>`;
}

window.addEventListener("error", (e) => report("页面出错", e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => report("后台请求出错", e.reason));

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("渲染出错:", error, info);
  }
  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 24, fontFamily: "Consolas, monospace", color: "#f56c6c" }}>
          <h3>界面渲染出错</h3>
          <pre style={{ whiteSpace: "pre-wrap", fontSize: 13 }}>
            {String(this.state.error?.stack ?? this.state.error)}
          </pre>
        </div>
      );
    }
    return this.props.children;
  }
}

try {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>
  );
  mounted = true;
} catch (err) {
  report("启动失败", err);
}
