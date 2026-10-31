"use strict";
// Worker of the debugger window (debug-page.js): runs the prelude and the
// script with debug.js. A call to Go posts {type: "call"} to the page and
// waits with Atomics.wait until the page has put the answer into the
// shared buffers: ctrl = Int32Array [state (1 = answered), byte length],
// data = UTF-8 text (a growable SharedArrayBuffer).
(() => {
  // Taken before the prelude replaces them.
  const decoder = new TextDecoder();
  let ctrl, data, session, listener;
  const early = []; // events before debug.js listens

  const transport = {
    call(text) {
      Atomics.store(ctrl, 0, 0);
      postMessage({ type: "call", text });
      Atomics.wait(ctrl, 0, 0);
      const n = ctrl[1];
      // TextDecoder does not take views of shared memory.
      const bytes = new Uint8Array(n);
      bytes.set(new Uint8Array(data, 0, n));
      return decoder.decode(bytes);
    },
    listen(fn) {
      listener = fn;
      for (const text of early.splice(0)) fn(text);
    },
  };

  function init(m) {
    ctrl = new Int32Array(m.ctrl);
    data = m.data;
    // Evaluated under their own URLs, so that the developer tools show
    // them (and the bundle's source map) by name.
    for (const s of m.sources) (0, eval)(s.text + "\n//# sourceURL=" + s.url);
    session = xljsDebug.start(transport, {
      status: (state, detail) => postMessage({ type: "status", state, detail }),
      console: self.console,
    });
    const t = session.t;
    const labels = {};
    for (const k of ["ready", "running", "finished", "failed", "close", "hint", "breakFirst", "run"]) labels[k] = t("debug." + k, "{0}");
    postMessage({ type: "ready", labels, title: t("debug.windowTitle", xljs.title), script: xljs.script });
  }

  self.addEventListener("error", (e) => {
    if (!session) return;
    e.preventDefault(); // not the page's error: the run fails (debug.js)
    session.fatal(e.error !== undefined ? e.error : e.message);
  });
  self.addEventListener("unhandledrejection", (e) => {
    if (session) session.rejected(e.promise, e.reason);
  });
  self.addEventListener("rejectionhandled", (e) => {
    if (session) session.handled(e.promise);
  });

  self.onmessage = (e) => {
    const m = e.data;
    switch (m.type) {
      case "init":
        try {
          init(m);
        } catch (err) {
          postMessage({ type: "failed", error: String((err && err.stack) || err) });
        }
        break;
      case "event":
        if (listener) listener(m.text);
        else early.push(m.text);
        break;
      case "run":
        session.run(m.breakFirst);
        break;
    }
  };
})();
