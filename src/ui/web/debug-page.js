"use strict";
// The debugger window of xljs --debug. The script runs in a worker
// (debug-worker.js) that waits for each call to Go with Atomics.wait; this
// page passes the calls on as WebView2 messages and puts the answers into
// the shared buffers. (A synchronous XMLHttpRequest answered from Go does
// not come back in every WebView2.) The page needs to be cross-origin
// isolated for SharedArrayBuffer (ui.WindowOptions.Isolated).
(async () => {
  const wv = window.chrome && window.chrome.webview;
  const $ = (id) => document.getElementById(id);
  // ?autorun (tests): run at once, close at the end and tell Go how far the
  // page got (in the trace of a test that hangs).
  const autorun = new URLSearchParams(location.search).has("autorun");
  const note = (text) => {
    if (autorun && wv) wv.postMessage("trace:" + text);
  };
  let L = {};
  const fmt = (s, ...args) => String(s).replace(/\{(\d+)\}/g, (m, i) => (args[i] === undefined ? m : args[i]));
  const setStatus = (text, cls) => {
    $("status").textContent = text;
    $("status").className = "status " + (cls || "");
  };
  const showError = (text) => {
    $("error").textContent = text;
    $("error").hidden = !text;
  };
  let failed = false;
  const fail = (text) => {
    setStatus("xljs: " + String(text).split("\n")[0], "error");
    showError(String(text));
    // The run cannot go on: Go ends it with this error instead of waiting
    // (and with ?autorun the window closes, as at the end of a run).
    if (wv && !failed) {
      wv.postMessage("call:" + JSON.stringify({ op: "fail", error: String(text) }));
      if (autorun) wv.postMessage("close");
    }
    failed = true;
  };

  function status(state, detail) {
    switch (state) {
      case "running":
        setStatus(L.running);
        $("run").disabled = true;
        $("break-first").disabled = true;
        break;
      case "done":
        if (detail.error) {
          setStatus(fmt(L.failed, detail.code), "error");
          showError(detail.error);
        } else {
          setStatus(fmt(L.finished, detail.code), "ok");
        }
        $("run").textContent = L.close;
        $("run").disabled = false;
        $("run").onclick = () => wv.postMessage("close");
        $("run").focus();
        if (autorun) wv.postMessage("close");
        break;
    }
  }

  if (!wv) return;
  note("loaded");
  if (!self.crossOriginIsolated) {
    fail("the debugger window is not cross-origin isolated (SharedArrayBuffer is not available)");
    return;
  }
  const ctrl = new SharedArrayBuffer(8);
  const data = new SharedArrayBuffer(1 << 16, { maxByteLength: 1 << 30 });
  const ctrlView = new Int32Array(ctrl);
  const encoder = new TextEncoder();

  /** Puts Go's answer to the worker's call where the worker waits for it. */
  function answer(text) {
    const bytes = encoder.encode(text);
    if (bytes.length > data.byteLength) data.grow(Math.min(data.maxByteLength, Math.max(bytes.length, data.byteLength * 2)));
    new Uint8Array(data).set(bytes);
    ctrlView[1] = bytes.length;
    Atomics.store(ctrlView, 0, 1);
    Atomics.notify(ctrlView, 0);
  }

  let worker;
  try {
    const base = new URL(".", location.href);
    const load = async (name) => {
      const r = await fetch(name);
      if (!r.ok) throw new Error(`${name}: ${r.status}`);
      return { url: new URL(name, base).href, text: await r.text() };
    };
    const [main, ...sources] = await Promise.all(["debug-worker.js", "debug/prelude.js", "debug/bundle.js", "debug.js"].map(load));
    note("fetched " + [main, ...sources].map((s) => s.text.length).join(" "));
    // A worker from the text: its requests would not reach Go in every WebView2.
    const blob = new Blob([main.text + "\n//# sourceURL=" + main.url], { type: "text/javascript" });
    worker = new Worker(URL.createObjectURL(blob), { name: "xljs" });
    worker.postMessage({ type: "init", ctrl, data, sources });
    note("worker started");
  } catch (e) {
    fail(e.stack || e);
    return;
  }

  // Go's messages start with \x02 (the wrapper echoes the page's own):
  // "R" + an answer, or an event (JSON).
  wv.addEventListener("message", (e) => {
    if (typeof e.data !== "string" || e.data[0] !== "\x02") return;
    const text = e.data.slice(1);
    if (text[0] === "R") answer(text.slice(1));
    else worker.postMessage({ type: "event", text });
  });

  worker.onerror = (e) => fail(e.message || "worker error");
  let calls = 0;
  worker.onmessage = (e) => {
    const m = e.data;
    switch (m.type) {
      case "call":
        if (calls++ === 0) note("first call");
        wv.postMessage("call:" + m.text);
        break;
      case "status":
        status(m.state, m.detail);
        break;
      case "failed":
        fail(m.error);
        break;
      case "ready":
        note("ready");
        L = m.labels;
        document.title = m.title;
        $("name").textContent = m.script;
        $("hint").textContent = L.hint;
        $("break-first-label").textContent = L.breakFirst;
        $("run").textContent = L.run;
        $("run").disabled = false;
        $("run").onclick = () => worker.postMessage({ type: "run", breakFirst: $("break-first").checked });
        $("run").focus();
        setStatus(L.ready);
        if (autorun) $("run").click();
        break;
    }
  };

  document.addEventListener("keydown", (e) => {
    if (e.key === "F5" || (e.ctrlKey && (e.key === "r" || e.key === "R"))) {
      // No reload (see debug.reloaded); F5 runs.
      e.preventDefault();
      if (e.key === "F5" && !$("run").disabled) $("run").click();
    }
  });
})();
