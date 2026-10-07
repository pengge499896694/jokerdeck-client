// jokerdeck-sidebar-delete-v1
function jokerdeckDeleteAction(item, ui) {
  // Only expose actions the official application already authorizes for this exact thread and host.
  if (!item || item.enabled === false || typeof item.onSelect !== "function") return null;
  return {
    id: "jokerdeck-delete-thread",
    ariaLabel: "删除会话（需确认）",
    icon: ui.jsxs("svg", {
      width: 16, height: 16, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
      strokeWidth: 1.7, strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true,
      children: [ui.jsx("path", { d: "M3 6h18M9 6V4h6v2M5 6l1 14h12l1-14M10 10v6M14 10v6" })]
    }),
    // onSelect opens DeleteThreadDialog; its explicit confirmation performs thread/delete.
    onClick: () => item.onSelect()
  };
}
