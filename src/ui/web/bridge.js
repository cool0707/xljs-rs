"use strict";
// Message bridge between a built-in page and the script. Messages are JSON;
// the script's side is in internal/api/prelude.js (ui.form, ui.table).
const host = (() => {
  const wv = window.chrome && window.chrome.webview;
  const handlers = [];
  if (wv) {
    wv.addEventListener("message", (e) => {
      // The WebView2 wrapper echoes the page's own messages back; messages
      // from the script start with \x02.
      if (typeof e.data !== "string" || e.data[0] !== "\x02") return;
      const msg = JSON.parse(e.data.slice(1));
      for (const fn of handlers) fn(msg);
    });
  }
  const canAttach = !!(wv && typeof wv.postMessageWithAdditionalObjects === "function");
  return {
    /** Sends msg to the script; files (File objects) arrive there as paths. */
    send(msg, files) {
      if (!wv) return;
      const s = JSON.stringify(msg);
      if (files && files.length && canAttach) wv.postMessageWithAdditionalObjects(s, Array.from(files));
      else wv.postMessage(s);
    },
    on(fn) { handlers.push(fn); },
    /** Whether dropped files can be passed to the script (WebView2 113+). */
    canAttach,
  };
})();

/** Replaces {0}, {1}, ... in s. */
const fmt = (s, ...args) => String(s).replace(/\{(\d+)\}/g, (m, i) => (args[i] === undefined ? m : args[i]));

// Dropping a file outside a drop target must not replace the page.
document.addEventListener("dragover", (e) => {
  e.preventDefault();
  if (!e.target.closest || !e.target.closest(".drop")) e.dataTransfer.dropEffect = "none";
});
document.addEventListener("drop", (e) => e.preventDefault());
// No browser shortcuts that would leave or reload the page.
document.addEventListener("keydown", (e) => {
  if (e.key === "F5" || (e.ctrlKey && (e.key === "r" || e.key === "R" || e.key === "p" || e.key === "P"))) e.preventDefault();
});
