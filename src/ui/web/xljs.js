// xljs page runtime: injected into every page of a window before its own
// scripts run (internal/ui/webview_windows.go). Defines the global `xljs`
// object and the data-bind / data-model / data-show / data-action
// attributes. The script's side is AppWindow in internal/api/prelude.js;
// messages are JSON objects with an "xljs" key naming the kind.
(() => {
  "use strict";
  const wv = window.chrome && window.chrome.webview;
  // Only the script's pages (not the built-in ones under /_xljs/).
  if (!wv || window.xljs || location.origin !== "https://app.xljs" || location.pathname.startsWith("/_xljs/")) return;
  const canAttach = typeof wv.postMessageWithAdditionalObjects === "function";

  // ------------------------------------------------------------ values
  //
  // JSON plus Date, file/folder handles, bytes and Error. Files (File,
  // FileList) are attached to the message so that the script gets their
  // paths; without support for that (WebView2 < 113) they become null.

  /** A file or folder of the script's side: only its path is known here. */
  class XljsHandle {
    constructor(kind, path) {
      this.kind = kind;
      this.path = path;
      this.name = String(path).split(/[\\/]/).filter(Boolean).pop() || String(path);
    }
    toString() { return this.path; }
  }

  function stringify(m, files) {
    return JSON.stringify(m, function (k, v) {
      const raw = this[k];
      if (raw instanceof Date) return isNaN(raw.getTime()) ? null : { $date: raw.toISOString() };
      if (raw instanceof XljsHandle) return raw.kind === "file" ? { $file: raw.path } : { $dir: raw.path };
      if (raw instanceof File) {
        if (!canAttach) return null;
        if (!files.includes(raw)) files.push(raw);
        return { $attach: files.indexOf(raw) };
      }
      if (raw instanceof FileList) return Array.from(raw);
      if (raw instanceof ArrayBuffer) return { $bytes: toBase64(new Uint8Array(raw)) };
      if (ArrayBuffer.isView(raw)) return { $bytes: toBase64(new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength)) };
      if (raw instanceof Error) return { $error: errorInfo(raw) };
      if (raw instanceof Map) return Object.fromEntries(raw);
      if (raw instanceof Set) return Array.from(raw);
      return v;
    });
  }

  function revive(v) {
    if (Array.isArray(v)) return v.map(revive);
    if (v === null || typeof v !== "object") return v;
    const keys = Object.keys(v);
    if (keys.length === 1) {
      const x = v[keys[0]];
      switch (keys[0]) {
        case "$date": return new Date(x);
        case "$file": return new XljsHandle("file", x);
        case "$dir": return new XljsHandle("directory", x);
        case "$bytes": return fromBase64(x).buffer;
        case "$error": return errorFrom(x);
      }
    }
    const out = {};
    for (const k of keys) out[k] = revive(v[k]);
    return out;
  }

  function toBase64(bytes) {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(s);
  }

  function fromBase64(s) {
    const bin = atob(s);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  const errorInfo = (e) => (e instanceof Error
    ? { name: e.name, message: e.message, stack: e.stack, code: e.code }
    : { name: "Error", message: String(e) });

  /** Errors that came from the script; unhandled ones are reported to it. */
  const remote = new WeakSet();
  function errorFrom(info) {
    info = info || {};
    const e = new Error(info.message === undefined ? "" : String(info.message));
    if (info.name) e.name = String(info.name);
    if (info.stack) e.stack = String(info.stack);
    remote.add(e);
    return e;
  }

  // ------------------------------------------------------------ messages

  function send(m) {
    const files = [];
    const s = stringify(m, files);
    if (files.length) wv.postMessageWithAdditionalObjects(s, files);
    else wv.postMessage(s);
  }

  let nextID = 0;
  const pending = new Map(); // id -> { resolve, reject }
  const handlers = new Map(); // name -> fn (xljs.handle)
  const listeners = new Map(); // event -> [fn]
  const store = {};
  let resolveReady;
  const ready = new Promise((r) => { resolveReady = r; });

  function fire(name, args) {
    for (const fn of (listeners.get(name) || []).slice()) {
      try {
        const r = fn(...args);
        if (r && typeof r.then === "function") r.catch((e) => console.error(e));
      } catch (e) {
        console.error(e);
      }
    }
  }

  /** Answers a request of the script with the (awaited) result of fn. */
  function answer(id, fn) {
    let r;
    try {
      r = fn();
    } catch (e) {
      send({ xljs: "error", id, error: errorInfo(e) });
      return;
    }
    Promise.resolve(r).then(
      (value) => send({ xljs: "result", id, value }),
      (e) => send({ xljs: "error", id, error: errorInfo(e) }));
  }

  function callPage(name, args) {
    const fn = handlers.has(name) ? handlers.get(name) : window[name];
    if (typeof fn !== "function") throw Object.assign(new Error(name), { code: "noHandler" });
    return fn(...args);
  }

  wv.addEventListener("message", (e) => {
    // The WebView2 wrapper echoes the page's own messages back; messages
    // from the script start with \x02.
    if (typeof e.data !== "string" || e.data[0] !== "\x02") return;
    let m;
    try { m = JSON.parse(e.data.slice(1)); } catch (err) { return; }
    if (!m || typeof m.xljs !== "string") return;
    switch (m.xljs) {
      case "init":
        for (const k of Object.keys(store)) delete store[k];
        Object.assign(store, revive(m.state || {}));
        dom.clear();
        for (const h of m.handlers || []) dom.add(h);
        bindAll(document);
        pushModels(document);
        resolveReady();
        break;
      case "invoke":
        answer(m.id, () => callPage(String(m.name), revive(m.args || [])));
        break;
      case "result":
      case "error": {
        const p = pending.get(m.id);
        if (!p) break;
        pending.delete(m.id);
        if (m.xljs === "result") p.resolve(revive(m.value));
        else p.reject(errorFrom(m.error));
        break;
      }
      case "event":
        fire(String(m.name), revive(m.args || []));
        break;
      case "state":
        if (m.deleted) delete store[m.key];
        else store[m.key] = revive(m.value);
        bindKey(m.key);
        fire("state", [m.key, store[m.key]]);
        break;
      case "dom":
        if (m.op === "on") dom.add(m);
        else if (m.op === "off") dom.remove(m.hid);
        else if (m.id) answer(m.id, () => domRead(m.sel, m.op, revive(m.args || [])));
        else domWrite(m.sel, m.op, revive(m.args || []));
        break;
      case "eval":
        answer(m.id, () => {
          const v = new Function("return (" + m.code + "\n)")();
          return m.fn ? v(...revive(m.args || [])) : v;
        });
        break;
    }
  });

  // An error of the script that the page did not handle is shown by the script.
  window.addEventListener("unhandledrejection", (e) => {
    if (remote.has(e.reason)) send({ xljs: "unhandled", error: errorInfo(e.reason) });
  });

  // ------------------------------------------------------------ state

  const state = new Proxy(store, {
    set(t, k, v) {
      t[k] = v;
      if (typeof k === "string") {
        send({ xljs: "state", key: k, value: v });
        bindKey(k);
      }
      return true;
    },
    deleteProperty(t, k) {
      delete t[k];
      if (typeof k === "string") {
        send({ xljs: "state", key: k, deleted: true });
        bindKey(k);
      }
      return true;
    },
  });

  /** Value of a data-bind path ("total" or "summary.total"; "!busy" negates). */
  function lookup(path) {
    const not = path.startsWith("!");
    let v = store;
    for (const part of path.replace(/^!/, "").split(".")) v = v === null || v === undefined ? undefined : v[part];
    return not ? !v : v;
  }

  const display = (v) => (v === null || v === undefined ? ""
    : v instanceof Date ? v.toLocaleString()
      : typeof v === "number" ? v.toLocaleString()
        : typeof v === "object" ? JSON.stringify(v) : String(v));

  /** Current value of a form control (checkbox: boolean, number: number or null, multiple: array). */
  function readValue(el) {
    if (el instanceof HTMLInputElement) {
      if (el.type === "checkbox") return el.checked;
      if (el.type === "radio") return el.checked ? el.value : null;
      if (el.type === "number" || el.type === "range") return el.value === "" ? null : el.valueAsNumber;
      if (el.type === "file") return el.multiple ? Array.from(el.files) : el.files[0] || null;
      return el.value;
    }
    if (el instanceof HTMLSelectElement && el.multiple) return Array.from(el.selectedOptions, (o) => o.value);
    if ("value" in el && !(el instanceof HTMLLIElement)) return el.value;
    return el.textContent;
  }

  function writeValue(el, v) {
    if (el instanceof HTMLInputElement && el.type === "checkbox") el.checked = typeof v === "boolean" ? v : !!v && v === el.value;
    else if (el instanceof HTMLInputElement && el.type === "radio") el.checked = v !== null && v !== undefined && String(v) === el.value;
    else if (el instanceof HTMLInputElement && el.type === "file") { /* read-only */ }
    else if (el instanceof HTMLSelectElement && el.multiple) {
      const set = new Set((Array.isArray(v) ? v : [v]).map(String));
      for (const o of el.options) o.selected = set.has(o.value);
    } else if ("value" in el && !(el instanceof HTMLLIElement) && !(el instanceof HTMLButtonElement)) {
      el.value = v instanceof Date ? v.toISOString().slice(0, 10) : v === null || v === undefined ? "" : v;
    } else el.textContent = display(v);
  }

  const keyOf = (path) => path.replace(/^!/, "").split(".")[0];

  function bindEl(el) {
    if (el.dataset.bind !== undefined) {
      const v = lookup(el.dataset.bind);
      if (el instanceof HTMLProgressElement || el instanceof HTMLMeterElement) {
        if (v === null || v === undefined) el.removeAttribute("value");
        else el.value = Number(v);
      } else if (el instanceof HTMLInputElement || el instanceof HTMLSelectElement || el instanceof HTMLTextAreaElement) {
        writeValue(el, v);
      } else el.textContent = display(v);
    }
    if (el.dataset.model !== undefined && el.dataset.model in store) writeValue(el, store[el.dataset.model]);
    if (el.dataset.show !== undefined) el.hidden = !lookup(el.dataset.show);
  }

  const boundSel = "[data-bind],[data-model],[data-show]";

  function bindAll(root) {
    if (root.matches && root.matches(boundSel)) bindEl(root);
    if (root.querySelectorAll) root.querySelectorAll(boundSel).forEach(bindEl);
  }

  function bindKey(key) {
    document.querySelectorAll(boundSel).forEach((el) => {
      for (const a of ["bind", "model", "show"]) {
        if (el.dataset[a] !== undefined && keyOf(el.dataset[a]) === key) {
          bindEl(el);
          return;
        }
      }
    });
  }

  /** Sends the initial values of data-model controls the script has no value for. */
  function pushModels(root) {
    root.querySelectorAll("[data-model]").forEach((el) => {
      const key = el.dataset.model;
      if (key in store || (el.type === "radio" && !el.checked)) return;
      state[key] = readValue(el);
    });
  }

  function onModelInput(e) {
    const el = e.target;
    if (!(el instanceof Element) || el.dataset.model === undefined) return;
    if (el.type === "radio" && !el.checked) return;
    const key = el.dataset.model;
    const v = readValue(el);
    // input and change both arrive for most controls.
    if ((v === null || typeof v !== "object") && store[key] === v) return;
    state[key] = v;
  }
  document.addEventListener("input", onModelInput, true);
  document.addEventListener("change", onModelInput, true);

  new MutationObserver((records) => {
    for (const r of records) for (const n of r.addedNodes) if (n.nodeType === 1) bindAll(n);
  }).observe(document, { childList: true, subtree: true });

  // ------------------------------------------------------------ forms and actions

  /** Values of a form's named controls as an object (repeated names become arrays). */
  function formValues(form) {
    const out = {};
    for (const el of form.elements) {
      if (!el.name || el.disabled || el instanceof HTMLButtonElement || el.type === "submit" || el.type === "button" || el.type === "reset") continue;
      if (el.type === "radio") {
        if (el.checked) out[el.name] = el.value;
        else if (!(el.name in out)) out[el.name] = null;
        continue;
      }
      const v = readValue(el);
      if (el.name in out && el.type !== "checkbox") out[el.name] = [].concat(out[el.name], v);
      else out[el.name] = v;
    }
    return out;
  }

  async function runAction(el, e) {
    e.preventDefault();
    if (el.classList.contains("xljs-busy")) return;
    const form = el instanceof HTMLFormElement ? el : el.form || el.closest("form");
    if (form && !form.reportValidity()) return;
    let args = [];
    if (el.dataset.args !== undefined) {
      const a = JSON.parse(el.dataset.args);
      args = Array.isArray(a) ? a : [a];
    } else if (form) args = [formValues(form)];
    el.classList.add("xljs-busy");
    const disable = "disabled" in el && !el.disabled;
    if (disable) el.disabled = true;
    try {
      const r = await invoke(el.dataset.action, ...args);
      if (el.dataset.result) state[el.dataset.result] = r;
    } catch (err) {
      if (!remote.has(err)) throw err;
      send({ xljs: "unhandled", error: errorInfo(err) });
    } finally {
      el.classList.remove("xljs-busy");
      if (disable) el.disabled = false;
    }
  }

  document.addEventListener("click", (e) => {
    const el = e.target instanceof Element && e.target.closest("[data-action]");
    if (el && !(el instanceof HTMLFormElement)) runAction(el, e);
  });
  document.addEventListener("submit", (e) => {
    const form = e.target;
    if (form.dataset.action !== undefined) runAction(form, e);
    else e.preventDefault(); // a submitted form would reload the page
  });

  // ------------------------------------------------------------ win.$ (simple DOM)

  const dom = (() => {
    const regs = new Map(); // hid -> { sel, type, preventDefault }
    const types = new Set();
    function listen(type) {
      if (types.has(type)) return;
      types.add(type);
      document.addEventListener(type, (e) => {
        if (!(e.target instanceof Element)) return;
        for (const [hid, h] of regs) {
          if (h.type !== type) continue;
          const el = e.target.closest(h.sel);
          if (!el) continue;
          if (h.preventDefault || type === "submit") e.preventDefault();
          send({ xljs: "dom-event", hid, event: eventValue(e, el) });
        }
      }, true);
    }
    return {
      add(h) {
        regs.set(h.hid, { sel: h.sel, type: h.type, preventDefault: !!h.preventDefault });
        listen(h.type);
      },
      remove(hid) { regs.delete(hid); },
      clear() { regs.clear(); },
    };
  })();

  function eventValue(e, el) {
    const v = { type: e.type, id: el.id, name: el.getAttribute("name") || "", dataset: Object.assign({}, el.dataset) };
    if ("value" in el && !(el instanceof HTMLLIElement)) v.value = readValue(el);
    else v.value = el.textContent;
    if (el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio")) v.checked = el.checked;
    if (e instanceof KeyboardEvent) Object.assign(v, { key: e.key, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey });
    if (e instanceof MouseEvent) Object.assign(v, { button: e.button, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey });
    if (el instanceof HTMLFormElement) v.values = formValues(el);
    if (e instanceof DragEvent && e.dataTransfer) v.files = Array.from(e.dataTransfer.files);
    return v;
  }

  function domWrite(sel, op, args) {
    const els = document.querySelectorAll(sel);
    const [a, b] = args;
    for (const el of els) {
      switch (op) {
        case "text": el.textContent = display(a); break;
        case "html": el.innerHTML = a === null || a === undefined ? "" : String(a); break;
        case "val": writeValue(el, a); break;
        case "attr":
          if (b === null || b === undefined || b === false) el.removeAttribute(a);
          else el.setAttribute(a, b === true ? "" : String(b));
          break;
        case "css":
          for (const [k, v] of Object.entries(a || {})) {
            if (k.includes("-")) el.style.setProperty(k, v === null ? "" : String(v));
            else el.style[k] = v === null ? "" : v;
          }
          break;
        case "addClass": el.classList.add(...String(a).split(/\s+/).filter(Boolean)); break;
        case "removeClass": el.classList.remove(...String(a).split(/\s+/).filter(Boolean)); break;
        case "toggleClass":
          for (const c of String(a).split(/\s+/).filter(Boolean)) {
            if (args.length > 1) el.classList.toggle(c, !!b);
            else el.classList.toggle(c);
          }
          break;
        case "show":
          el.hidden = false;
          if (el.style.display === "none") el.style.display = "";
          break;
        case "hide": el.hidden = true; break;
      }
    }
  }

  function domRead(sel, op, args) {
    const el = document.querySelector(sel);
    if (!el) return null;
    switch (op) {
      case "text": return el.textContent;
      case "html": return el.innerHTML;
      case "val": return readValue(el);
      case "attr": return el.getAttribute(args[0]);
      case "css": return getComputedStyle(el).getPropertyValue(args[0].replace(/[A-Z]/g, (c) => "-" + c.toLowerCase()));
    }
    return null;
  }

  // ------------------------------------------------------------ xljs

  function invoke(name, ...args) {
    return new Promise((resolve, reject) => {
      const id = ++nextID;
      pending.set(id, { resolve, reject });
      send({ xljs: "invoke", id, name: String(name), args });
    });
  }

  const xljs = Object.freeze({
    /** Calls a function of the script (an export of the entry file or app.handle) and resolves with its result. */
    invoke,
    /** Lets the script call fn by name (win.invoke). */
    handle(name, fn) {
      if (fn === null || fn === undefined) handlers.delete(String(name));
      else handlers.set(String(name), fn);
    },
    /** Sends an event to the script (win.on). */
    emit(name, ...args) { send({ xljs: "event", name: String(name), args }); },
    /** Listens to the script's events (win.emit), and to "state" (key, value) when the script changes a value. */
    on(name, fn) {
      if (!listeners.has(name)) listeners.set(name, []);
      listeners.get(name).push(fn);
    },
    off(name, fn) {
      const list = listeners.get(name);
      if (!list) return;
      if (fn === undefined) listeners.delete(name);
      else listeners.set(name, list.filter((f) => f !== fn));
    },
    /** Values shared with the script (win.state). Only top-level keys are synchronized. */
    state,
    /** Resolves once the script has sent its state. */
    ready,
    /** Values of a form's controls, as data-action passes them. */
    formValues,
  });
  Object.defineProperty(window, "xljs", { value: xljs, enumerable: true });

  const start = () => send({ xljs: "ready" });
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
