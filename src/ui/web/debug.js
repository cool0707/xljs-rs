// xljs debugger runtime (xljs --debug): runs the prelude and the script in
// WebView2 (a worker of the debugger window, see debug-worker.js), so that
// its developer tools can debug them, while the natives stay in Go behind a
// synchronous bridge (internal/api/remote.go describes the wire format;
// internal/engine/debug.go the session).
//
// xljsDebug.start(transport, hooks) prepares a run:
//   transport.call(text) -> text   synchronous request to Go
//   transport.listen(fn)           fn(text) for each event from Go
//   hooks.status(state, detail)    "ready", "running", "done"
//   hooks.console                  the console of the developer tools (or undefined)
// and returns { run(breakFirst), fatal(error), t(key), rejected(), handled() }. The prelude and the bundled script are
// globalThis.__xljsPrelude and globalThis.__xljsMain.
(() => {
  "use strict";
  const G = globalThis;
  // Taken before the prelude replaces the globals.
  const timeout = G.setTimeout, untimeout = G.clearTimeout;
  const interval = G.setInterval, uninterval = G.clearInterval;
  const typedArrays = {
    Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array,
    Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView,
  };
  const errorTypes = { Error, TypeError, RangeError, SyntaxError, ReferenceError, EvalError, URIError };

  /** Thrown to unwind the script after exit() or the end of the run. */
  class Stop {
    toString() { return "xljs: stopped"; }
  }

  function start(transport, hooks = {}) {
    const status = hooks.status || (() => {});
    const pageConsole = hooks.console;

    // ------------------------------------------------------------ values

    const fnIds = new Map(); // page function -> id
    const fns = new Map(); // id -> page function
    const refs = new Map(); // id -> stub of a Go value
    const refIds = new WeakMap(); // stub -> id
    const promises = new Map(); // id -> { resolve, reject }
    let nextFn = 1;

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

    function enc(v, depth = 0) {
      if (depth > 100) throw new TypeError("xljs: value is nested too deeply (circular?)");
      switch (typeof v) {
        case "undefined": case "symbol": return { $u: 0 };
        case "number":
          if (Number.isNaN(v)) return { $num: "NaN" };
          if (!Number.isFinite(v)) return { $num: v > 0 ? "Infinity" : "-Infinity" };
          if (Object.is(v, -0)) return { $num: "-0" };
          return v;
        case "bigint": return { $big: String(v) };
        case "string": case "boolean": return v;
        case "function": {
          if (refIds.has(v)) return { $ref: refIds.get(v) };
          let id = fnIds.get(v);
          if (!id) {
            id = nextFn++;
            fnIds.set(v, id);
            fns.set(id, v);
          }
          return { $fn: id };
        }
      }
      if (v === null) return null;
      if (refIds.has(v)) return { $ref: refIds.get(v) };
      if (v instanceof Date) return { $date: isNaN(v.getTime()) ? null : v.getTime() };
      if (v instanceof ArrayBuffer) return { $bytes: toBase64(new Uint8Array(v)), $t: "ArrayBuffer" };
      if (ArrayBuffer.isView(v)) {
        return { $bytes: toBase64(new Uint8Array(v.buffer, v.byteOffset, v.byteLength)), $t: v.constructor.name };
      }
      if (v instanceof Error) return { $err: { name: String(v.name), message: String(v.message), stack: String(v.stack || "") } };
      if (Array.isArray(v)) {
        const out = [];
        for (let i = 0; i < v.length; i++) out.push(enc(v[i], depth + 1));
        return out;
      }
      const o = {};
      for (const k of Object.keys(v)) o[k] = enc(v[k], depth + 1);
      return { $o: o };
    }

    function makeError(e) {
      const C = errorTypes[e.name] || Error;
      const err = new C(e.message);
      if (err.name !== e.name) Object.defineProperty(err, "name", { value: e.name, writable: true, configurable: true });
      return err;
    }

    function dec(v) {
      if (v === null || typeof v !== "object") return v;
      if (Array.isArray(v)) return v.map(dec);
      if ("$o" in v) {
        const o = {};
        for (const k of Object.keys(v.$o)) o[k] = dec(v.$o[k]);
        return o;
      }
      if ("$u" in v) return undefined;
      if ("$num" in v) return Number(v.$num);
      if ("$big" in v) return BigInt(v.$big);
      if ("$date" in v) return new Date(v.$date === null ? NaN : v.$date);
      if ("$bytes" in v) {
        const b = fromBase64(v.$bytes);
        return v.$t === "ArrayBuffer" ? b.buffer : new typedArrays[v.$t](b.buffer);
      }
      if ("$err" in v) return makeError(v.$err);
      if ("$fn" in v) return fns.get(v.$fn);
      if ("$p" in v) {
        let p = refs.get(v.$p);
        if (!p) {
          p = new Promise((resolve, reject) => promises.set(v.$p, { resolve, reject }));
          refs.set(v.$p, p);
          refIds.set(p, v.$p);
        }
        return p;
      }
      if ("$ref" in v) {
        let x = refs.get(v.$ref);
        if (!x) {
          const id = v.$ref;
          x = v.$k === "fn"
            ? function () { return request({ op: "invoke", ref: id, args: Array.from(arguments, (a) => enc(a)) }); }
            : Object.freeze(Object.create(null));
          refs.set(id, x);
          refIds.set(x, id);
        }
        return x;
      }
      throw new TypeError("xljs: untagged object from Go");
    }

    // ------------------------------------------------------------ bridge

    let stopped = false;

    function request(msg) {
      if (stopped) throw new Stop();
      const res = JSON.parse(transport.call(JSON.stringify(msg)));
      if (res.stop) {
        // The run has ended (the "done" event is on its way).
        stopped = true;
        throw new Stop();
      }
      if ("error" in res) throw dec(res.error);
      return dec(res.value);
    }

    const native = request({ op: "init" });
    // exit() ends the run at once (goja interrupts the script).
    native.exit = (code) => {
      request({ op: "exit", code: code == null ? 0 : Number(code) });
      stopped = true;
      throw new Stop();
    };

    // ------------------------------------------------------------ the end
    //
    // The run ends when the script's function has settled, no timer is
    // left and Go has nothing pending (open windows) that could still call
    // into the page. The page reports when it is idle, with the number of
    // events it has handled; Go decides.

    const timers = new Set();
    const unhandled = new Map(); // promise -> reason (see rejected/handled)
    let started = false, settled = false, failure;
    let seen = 0, checking = false;

    function check() {
      if (checking || stopped || !started) return;
      checking = true;
      timeout(() => {
        checking = false;
        if (stopped || timers.size > 0) return;
        try {
          let error = null;
          if (failure !== undefined) error = describe(failure);
          else if (unhandled.size > 0) error = native.t("engine.unhandledRejection", describe(unhandled.values().next().value));
          request({ op: "idle", seen, settled, error });
        } catch (e) {
          if (!(e instanceof Stop)) throw e;
        }
      }, 0);
    }

    function describe(e) {
      if (e instanceof Error) {
        const s = String(e.stack || "");
        return s.includes(e.message) ? s : `${e.name}: ${e.message}\n${s}`;
      }
      return String(e);
    }

    /** Reports an uncaught error: the run fails like it does in goja. */
    function fatal(e) {
      if (stopped || e instanceof Stop) return;
      try {
        request({ op: "fail", error: describe(e) });
      } catch (e) {
        if (!(e instanceof Stop)) throw e;
      }
    }

    transport.listen((text) => {
      const ev = JSON.parse(text);
      if (ev.ev === "done") {
        stopped = true;
        for (const id of timers) { untimeout(id); uninterval(id); }
        timers.clear();
        status("done", ev);
        return;
      }
      if (stopped) return;
      seen++;
      check();
      switch (ev.ev) {
        case "call": {
          const fn = fns.get(ev.fn);
          fn(...ev.args.map(dec));
          break;
        }
        case "settle": {
          const p = promises.get(ev.p);
          promises.delete(ev.p);
          if (p) (ev.ok ? p.resolve : p.reject)(dec(ev.value));
          break;
        }
      }
    });

    // Timers keep the run going; their callbacks are not wrapped in
    // try/catch so that "pause on uncaught exceptions" works.
    function timer(repeat) {
      const set = repeat ? interval : timeout;
      return function (fn, ms, ...args) {
        if (typeof fn !== "function") throw new TypeError(native.t("engine.callbackNotFunction"));
        const id = set(() => {
          if (stopped) return;
          if (!repeat) timers.delete(id);
          check();
          fn(...args);
        }, ms);
        timers.add(id);
        return id;
      };
    }
    function untimer(id) {
      if (!timers.delete(id)) return;
      untimeout(id);
      uninterval(id);
      check();
    }
    const define = (name, value) =>
      Object.defineProperty(G, name, { value, writable: true, configurable: true, enumerable: false });
    define("setTimeout", timer(false));
    define("setInterval", timer(true));
    define("setImmediate", (fn, ...args) => G.setTimeout(fn, 0, ...args));
    define("clearTimeout", untimer);
    define("clearInterval", untimer);
    define("clearImmediate", untimer);

    // Use the prelude's versions so that the script sees what it sees in
    // goja (its Blob and File carry paths; goja has no Intl).
    for (const name of ["Blob", "File", "TextEncoder", "TextDecoder", "DOMException", "Intl"]) delete G[name];

    G.__xljsPrelude(native);

    // console goes to the program's output and to the developer tools.
    if (pageConsole) {
      const xc = G.console, both = {};
      for (const k of Object.keys(xc)) {
        both[k] = function () {
          if (typeof pageConsole[k] === "function") pageConsole[k].apply(pageConsole, arguments);
          return xc[k].apply(xc, arguments);
        };
      }
      define("console", both);
    }

    status("ready");
    return {
      fatal,
      /** Localized message for key. */
      t: (key, ...args) => native.t(key, ...args),
      /** Notes a rejection that nothing handles (unhandledrejection). */
      rejected(promise, reason) {
        if (!(reason instanceof Stop)) unhandled.set(promise, reason);
      },
      /** A rejection noted with rejected() got a handler after all. */
      handled(promise) { unhandled.delete(promise); },
      /** Runs the script; with breakFirst the debugger stops before it. */
      run(breakFirst) {
        if (started) return;
        started = true;
        status("running");
        if (native.appMode) native.openMain();
        if (breakFirst) {
          // Step into (F11) the next call to get to the script's first line.
          debugger; // eslint-disable-line no-debugger
        }
        const done = G.__xljsMain(native.setExports);
        done.then(
          () => { settled = true; check(); },
          (e) => {
            settled = true;
            if (!(e instanceof Stop)) failure = e;
            check();
          });
        check();
      },
    };
  }

  G.xljsDebug = { start, Stop };
})();
