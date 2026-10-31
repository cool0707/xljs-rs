// The custom title bar (titleBar: "custom"): injected into every page of a
// window whose OS title bar is hidden (src/ui/native/webview.rs), after
// `window.__xljsTitleBar = { title, icon, texts, nonClient }`.
//
// The bar is put at the top of <body> unless the page has its own element
// with data-xljs-titlebar. Either way, buttons with
// data-xljs-window="minimize|maximize|close" control the window. Dragging
// uses the CSS app-region (WebView2 non-client region support); on runtimes
// without it the bar asks the host to start the drag.
(() => {
  "use strict";
  const cfg = window.__xljsTitleBar;
  const wv = window.chrome && window.chrome.webview;
  if (!cfg || !wv || window.top !== window || window.__xljsTitleBarState) return;
  const send = (cmd) => wv.postMessage("\x01window:" + cmd);

  // The host calls this when the window is maximized/restored or
  // (de)activated.
  window.__xljsTitleBarState = (maximized, active) => {
    const root = document.documentElement;
    root.classList.toggle("xljs-maximized", !!maximized);
    root.classList.toggle("xljs-inactive", !active);
    for (const b of document.querySelectorAll('[data-xljs-window="maximize"]')) {
      b.title = maximized ? cfg.texts.restore : cfg.texts.maximize;
      if (b.classList.contains("xljs-tb-btn")) b.textContent = maximized ? "" : "";
    }
  };

  const css = `
:root { --xljs-titlebar-height: 32px; }
.xljs-titlebar {
  flex: none; display: flex; align-items: stretch; height: var(--xljs-titlebar-height);
  margin: 0; padding: 0; box-sizing: border-box; position: relative; z-index: 2147483000;
  background: var(--xljs-titlebar-bg, #f3f3f3); color: var(--xljs-titlebar-fg, #1b1b1b);
  font: 12px/1 "Yu Gothic UI", "Meiryo UI", "Segoe UI", system-ui, sans-serif;
  -webkit-user-select: none; user-select: none; cursor: default;
  app-region: drag; -webkit-app-region: drag;
}
:root.xljs-inactive .xljs-titlebar .xljs-tb-title, :root.xljs-inactive .xljs-titlebar .xljs-tb-btn { opacity: .55; }
.xljs-tb-title {
  flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; padding: 0 12px;
  overflow: hidden; white-space: nowrap;
}
.xljs-tb-title span { overflow: hidden; text-overflow: ellipsis; }
.xljs-tb-title i { color: var(--xljs-titlebar-accent, #005fb8); font-size: 14px; }
[data-xljs-titlebar] { app-region: drag; -webkit-app-region: drag; }
[data-xljs-titlebar] button, [data-xljs-titlebar] input, [data-xljs-titlebar] select, [data-xljs-titlebar] a,
[data-xljs-titlebar] [data-xljs-window], .xljs-tb-btn { app-region: no-drag; -webkit-app-region: no-drag; }
.xljs-tb-btn {
  width: 46px; min-width: 0; height: 100%; margin: 0; padding: 0; border: 0; border-radius: 0;
  background: transparent; color: inherit; outline: none;
  font: 10px/1 "Segoe Fluent Icons", "Segoe MDL2 Assets"; cursor: default;
}
.xljs-tb-btn:hover { background: var(--xljs-titlebar-hover, rgba(0, 0, 0, .06)); }
.xljs-tb-btn:active { background: var(--xljs-titlebar-active, rgba(0, 0, 0, .1)); }
.xljs-tb-btn.xljs-tb-close:hover { background: #c42b1c; color: #fff; opacity: 1; }
.xljs-tb-btn.xljs-tb-close:active { background: #c84031; color: #fff; }
@media (prefers-color-scheme: dark) {
  .xljs-titlebar { background: var(--xljs-titlebar-bg, #202020); color: var(--xljs-titlebar-fg, #f3f3f3); }
  .xljs-tb-title i { color: var(--xljs-titlebar-accent, #4cc2ff); }
  .xljs-tb-btn:hover { background: var(--xljs-titlebar-hover, rgba(255, 255, 255, .08)); }
  .xljs-tb-btn:active { background: var(--xljs-titlebar-active, rgba(255, 255, 255, .05)); }
}`;

  function build() {
    const head = document.head || document.documentElement;
    const style = document.createElement("style");
    style.textContent = css;
    head.appendChild(style);
    let bar = document.querySelector("[data-xljs-titlebar]");
    if (!bar) {
      bar = document.createElement("header");
      bar.className = "xljs-titlebar";
      bar.setAttribute("data-xljs-titlebar", "");
      const title = document.createElement("div");
      title.className = "xljs-tb-title";
      if (cfg.icon) {
        if (!document.querySelector('link[href$="fontawesome/css/all.min.css"]')) {
          const link = document.createElement("link");
          link.rel = "stylesheet";
          link.href = "/_xljs/fontawesome/css/all.min.css";
          head.appendChild(link);
        }
        const i = document.createElement("i");
        i.className = cfg.icon;
        title.appendChild(i);
      }
      const text = document.createElement("span");
      text.textContent = cfg.title;
      title.appendChild(text);
      bar.appendChild(title);
      const buttons = cfg.fixedSize ? [["minimize", ""], ["close", ""]] : [["minimize", ""], ["maximize", ""], ["close", ""]];
      for (const [cmd, glyph] of buttons) {
        const b = document.createElement("button");
        b.type = "button";
        b.tabIndex = -1;
        b.className = "xljs-tb-btn xljs-tb-" + cmd;
        b.setAttribute("data-xljs-window", cmd);
        b.title = cfg.texts[cmd];
        b.textContent = glyph;
        bar.appendChild(b);
      }
      document.body.insertBefore(bar, document.body.firstChild);
    }
    for (const b of document.querySelectorAll("[data-xljs-window]")) {
      if (!b.title) b.title = cfg.texts[b.getAttribute("data-xljs-window")] || "";
    }
    if (!cfg.nonClient) {
      // No app-region support: the host moves the window.
      bar.addEventListener("mousedown", (e) => {
        if (e.button === 0 && e.detail === 1 && !e.target.closest("[data-xljs-window], button, input, select, textarea, a")) {
          e.preventDefault();
          send("drag");
        }
      });
      bar.addEventListener("dblclick", (e) => {
        if (!cfg.fixedSize && !e.target.closest("[data-xljs-window], button, input, select, textarea, a")) send("maximize");
      });
    }
    send("state");
  }

  document.addEventListener("click", (e) => {
    const b = e.target instanceof Element && e.target.closest("[data-xljs-window]");
    if (b) send(b.getAttribute("data-xljs-window"));
  });
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", build);
  else build();
})();
