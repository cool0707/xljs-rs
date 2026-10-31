// xljs prelude: builds the public JavaScript API on top of the Go natives.
// Evaluated once per runtime; the completion value is a function that
// receives the natives object. Must only use syntax goja supports natively
// (no async generators).
(function (native) {
  "use strict";
  const G = globalThis;
  const asyncIteratorSymbol = Symbol.asyncIterator || Symbol.for("Symbol.asyncIterator");
  const inspectSymbol = Symbol.for("xljs.inspect");

  /** Localized message for key (see internal/i18n/locales/*.json). */
  const T = (key, ...args) => native.t(key, ...args);

  const define = (obj, name, value) =>
    Object.defineProperty(obj, name, { value, writable: true, configurable: true, enumerable: false });
  const defineGlobal = (name, value) => define(G, name, value);

  // ---------------------------------------------------------------- util

  const pad2 = (n) => String(n).padStart(2, "0");
  // Day.js (set up at the end). Its objects are accepted wherever a Date is.
  let dayjsLib = null;
  const isDayjs = (v) => dayjsLib !== null && dayjsLib.isDayjs(v);
  /** v, with a Day.js object turned into a Date. */
  const plainDate = (v) => (isDayjs(v) ? v.toDate() : v);
  function formatDateLocal(d, withMs) {
    if (isNaN(d.getTime())) return "Invalid Date";
    let s = `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
    if (withMs && d.getMilliseconds()) s += "." + String(d.getMilliseconds()).padStart(3, "0");
    return s;
  }

  // Display width (East Asian wide characters count as 2).
  function strWidth(s) {
    let w = 0;
    for (const ch of String(s)) {
      const c = ch.codePointAt(0);
      if (c < 0x20) continue;
      w += (c >= 0x1100 && (c <= 0x115f || (c >= 0x2e80 && c <= 0xa4cf) || (c >= 0xac00 && c <= 0xd7a3) ||
        (c >= 0xf900 && c <= 0xfaff) || (c >= 0xfe30 && c <= 0xfe4f) || (c >= 0xff00 && c <= 0xff60) ||
        (c >= 0xffe0 && c <= 0xffe6) || (c >= 0x1f300 && c <= 0x1faff) || (c >= 0x20000 && c <= 0x3fffd))) ? 2 : 1;
    }
    return w;
  }

  // ---------------------------------------------------------------- inspect

  function inspect(value, opts) {
    const depthLimit = opts && opts.depth !== undefined ? opts.depth : 3;
    const seen = [];
    function fmt(v, depth, nested) {
      switch (typeof v) {
        case "string": return nested ? JSON.stringify(v) : v;
        case "number": return Object.is(v, -0) ? "-0" : String(v);
        case "bigint": return v + "n";
        case "boolean": case "undefined": return String(v);
        case "symbol": return v.toString();
        case "function": {
          if (/^class\s/.test(Function.prototype.toString.call(v))) return `[class ${v.name || "(anonymous)"}]`;
          return `[Function: ${v.name || "(anonymous)"}]`;
        }
      }
      if (v === null) return "null";
      if (typeof v[inspectSymbol] === "function") return v[inspectSymbol]();
      if (v instanceof Date) return formatDateLocal(v, true);
      if (v instanceof Error) return v.stack || `${v.name}: ${v.message}`;
      if (v instanceof RegExp) return String(v);
      if (seen.includes(v)) return "[Circular]";
      if (depth > depthLimit) return Array.isArray(v) ? "[Array]" : "[Object]";
      seen.push(v);
      try {
        let parts, open, close, prefix = "";
        if (Array.isArray(v) || ArrayBuffer.isView(v)) {
          const max = 100;
          parts = [];
          const len = v.length;
          for (let i = 0; i < Math.min(len, max); i++) parts.push(fmt(v[i], depth + 1, true));
          if (len > max) parts.push(`... ${len - max} more items`);
          open = "["; close = "]";
          if (ArrayBuffer.isView(v)) prefix = `${v.constructor.name}(${len}) `;
        } else if (v instanceof Map) {
          parts = [...v].map(([k, val]) => `${fmt(k, depth + 1, true)} => ${fmt(val, depth + 1, true)}`);
          open = "{"; close = "}"; prefix = `Map(${v.size}) `;
        } else if (v instanceof Set) {
          parts = [...v].map((x) => fmt(x, depth + 1, true));
          open = "{"; close = "}"; prefix = `Set(${v.size}) `;
        } else if (v instanceof ArrayBuffer) {
          return `ArrayBuffer { byteLength: ${v.byteLength} }`;
        } else if (v instanceof Promise) {
          return "Promise { ... }";
        } else {
          const keys = Object.keys(v);
          parts = keys.map((k) => {
            const key = /^[A-Za-z_$][\w$]*$/.test(k) || /^[^\x00-\x7f]+$/.test(k) ? k : JSON.stringify(k);
            return `${key}: ${fmt(v[k], depth + 1, true)}`;
          });
          const ctor = Object.getPrototypeOf(v) && Object.getPrototypeOf(v).constructor;
          if (ctor && ctor !== Object && ctor.name) prefix = ctor.name + " ";
          open = "{"; close = "}";
        }
        if (parts.length === 0) return prefix + open + close;
        const single = `${prefix}${open} ${parts.join(", ")} ${close}`;
        if (single.length <= 76 && !single.includes("\n")) return single;
        const ind = "  ".repeat(depth + 1);
        return `${prefix}${open}\n${ind}${parts.join(`,\n${ind}`)}\n${"  ".repeat(depth)}${close}`;
      } finally {
        seen.pop();
      }
    }
    return fmt(value, 0, false);
  }

  function formatArgs(args) {
    if (args.length === 0) return "";
    let first = args[0];
    let rest = Array.prototype.slice.call(args, 1);
    if (typeof first === "string" && /%[sdifjoO%]/.test(first)) {
      first = first.replace(/%([sdifjoO%])/g, (m, c) => {
        if (c === "%") return "%";
        if (rest.length === 0) return m;
        const a = rest.shift();
        switch (c) {
          case "s": return typeof a === "string" ? a : inspect(a);
          case "d": case "i": return String(c === "i" ? Math.trunc(Number(a)) : Number(a));
          case "f": return String(Number(a));
          case "j": try { return JSON.stringify(a); } catch (e) { return "[Circular]"; }
          default: return inspect(a);
        }
      });
    } else {
      first = inspect(first);
    }
    return [first].concat(rest.map((a) => inspect(a))).join(" ");
  }

  // ---------------------------------------------------------------- console

  const timers = new Map();
  const counts = new Map();
  let groupIndent = "";
  const out = (s) => native.stdout(groupIndent ? s.replace(/^/gm, groupIndent) + "\n" : s + "\n");
  const err = (s) => native.stderr(groupIndent ? s.replace(/^/gm, groupIndent) + "\n" : s + "\n");

  function table(data, columns) {
    if (data === null || typeof data !== "object") return out(inspect(data));
    const rows = Array.isArray(data) ? data.map((r, i) => [i, r]) : Object.entries(data);
    const cols = [];
    let hasValues = false;
    for (const [, r] of rows) {
      if (r !== null && typeof r === "object" && !(r instanceof Date)) {
        for (const k of Object.keys(r)) if (!cols.includes(k)) cols.push(k);
      } else hasValues = true;
    }
    const useCols = columns || cols;
    const header = ["(index)", ...useCols, ...(hasValues ? ["Values"] : [])];
    const cell = (v) => (v === undefined ? "" : typeof v === "string" ? v : inspect(v, { depth: 0 }));
    const body = rows.map(([k, r]) => {
      const isObj = r !== null && typeof r === "object" && !(r instanceof Date);
      return [String(k), ...useCols.map((c) => (isObj ? cell(r[c]) : "")), ...(hasValues ? [isObj ? "" : cell(r)] : [])];
    });
    const widths = header.map((h, i) => Math.max(strWidth(h), ...body.map((r) => strWidth(r[i]))));
    const line = (l, m, r) => l + widths.map((w) => "─".repeat(w + 2)).join(m) + r;
    const fmtRow = (r) => "│" + r.map((c, i) => " " + c + " ".repeat(widths[i] - strWidth(c)) + " ").join("│") + "│";
    out([line("┌", "┬", "┐"), fmtRow(header), line("├", "┼", "┤"), ...body.map(fmtRow), line("└", "┴", "┘")].join("\n"));
  }

  const consoleObj = {
    log: (...a) => out(formatArgs(a)),
    info: (...a) => out(formatArgs(a)),
    debug: (...a) => out(formatArgs(a)),
    warn: (...a) => err(formatArgs(a)),
    error: (...a) => err(formatArgs(a)),
    trace: (...a) => err("Trace: " + formatArgs(a) + "\n" + (new Error().stack || "").split("\n").slice(2).join("\n")),
    dir: (v, o) => out(inspect(v, o)),
    table,
    assert: (cond, ...a) => { if (!cond) err("Assertion failed" + (a.length ? ": " + formatArgs(a) : "")); },
    time: (label = "default") => { timers.set(label, Date.now()); },
    timeLog: (label = "default", ...a) => out(`${label}: ${Date.now() - (timers.get(label) || Date.now())}ms ${formatArgs(a)}`.trimEnd()),
    timeEnd: (label = "default") => { out(`${label}: ${Date.now() - (timers.get(label) || Date.now())}ms`); timers.delete(label); },
    count: (label = "default") => { const c = (counts.get(label) || 0) + 1; counts.set(label, c); out(`${label}: ${c}`); },
    countReset: (label = "default") => { counts.delete(label); },
    group: (...a) => { if (a.length) out(formatArgs(a)); groupIndent += "  "; },
    groupEnd: () => { groupIndent = groupIndent.slice(2); },
  };
  consoleObj.groupCollapsed = consoleObj.group;
  defineGlobal("console", consoleObj);

  // ---------------------------------------------------------------- polyfills

  if (!Array.fromAsync) {
    define(Array, "fromAsync", async function fromAsync(items, mapFn, thisArg) {
      const result = [];
      let i = 0;
      if (items != null && typeof items[asyncIteratorSymbol] === "function") {
        for await (const v of items) result.push(mapFn ? await mapFn.call(thisArg, v, i++) : v);
      } else {
        for (const v of (items != null && typeof items[Symbol.iterator] === "function" ? items : Array.from(items))) {
          const x = await v;
          result.push(mapFn ? await mapFn.call(thisArg, x, i++) : x);
        }
      }
      return result;
    });
  }

  if (typeof G.structuredClone !== "function") {
    defineGlobal("structuredClone", function structuredClone(value) {
      const memo = new Map();
      function clone(v) {
        if (v === null || typeof v !== "object") {
          if (typeof v === "function" || typeof v === "symbol") throw new TypeError(T("data.structuredClone"));
          return v;
        }
        if (memo.has(v)) return memo.get(v);
        let r;
        if (v instanceof Date) r = new Date(v.getTime());
        else if (v instanceof RegExp) r = new RegExp(v.source, v.flags);
        else if (v instanceof Map) { r = new Map(); memo.set(v, r); for (const [k, x] of v) r.set(clone(k), clone(x)); return r; }
        else if (v instanceof Set) { r = new Set(); memo.set(v, r); for (const x of v) r.add(clone(x)); return r; }
        else if (v instanceof ArrayBuffer) r = v.slice(0);
        else if (ArrayBuffer.isView(v)) r = new v.constructor(v);
        else if (Array.isArray(v)) { r = []; memo.set(v, r); v.forEach((x, i) => { r[i] = clone(x); }); return r; }
        else { r = {}; memo.set(v, r); for (const k of Object.keys(v)) r[k] = clone(v[k]); return r; }
        memo.set(v, r);
        return r;
      }
      return clone(value);
    });
  }

  if (typeof G.queueMicrotask !== "function") {
    defineGlobal("queueMicrotask", (fn) => { Promise.resolve().then(fn); });
  }

  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function bytesToBase64(bytes) {
    let s = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const a = bytes[i], b = bytes[i + 1], c = bytes[i + 2];
      s += B64[a >> 2] + B64[((a & 3) << 4) | ((b || 0) >> 4)] +
        (i + 1 < bytes.length ? B64[((b & 15) << 2) | ((c || 0) >> 6)] : "=") +
        (i + 2 < bytes.length ? B64[c & 63] : "=");
    }
    return s;
  }
  function base64ToBytes(s) {
    s = String(s).replace(/[\s=]/g, "");
    const out = new Uint8Array(Math.floor((s.length * 3) / 4));
    let j = 0;
    for (let i = 0; i < s.length; i += 4) {
      const n = (B64.indexOf(s[i]) << 18) | (B64.indexOf(s[i + 1]) << 12) |
        ((B64.indexOf(s[i + 2]) & 63) << 6) | (B64.indexOf(s[i + 3]) & 63);
      out[j++] = (n >> 16) & 255;
      if (i + 2 < s.length) out[j++] = (n >> 8) & 255;
      if (i + 3 < s.length) out[j++] = n & 255;
    }
    return out.slice(0, j);
  }
  if (typeof G.btoa !== "function") {
    defineGlobal("btoa", (s) => {
      s = String(s);
      const bytes = new Uint8Array(s.length);
      for (let i = 0; i < s.length; i++) {
        const c = s.charCodeAt(i);
        if (c > 255) throw new Error(T("data.btoa"));
        bytes[i] = c;
      }
      return bytesToBase64(bytes);
    });
    defineGlobal("atob", (s) => String.fromCharCode(...base64ToBytes(s)));
  }

  class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(s = "") { return new Uint8Array(native.encode(String(s), "utf8")); }
  }
  class TextDecoder {
    constructor(label = "utf-8", options = {}) {
      this._enc = native.normEncoding(label);
      this._label = String(label).toLowerCase();
      this.fatal = !!options.fatal;
      this.ignoreBOM = !!options.ignoreBOM;
    }
    get encoding() { return this._label; }
    decode(input) {
      if (input === undefined) return "";
      return native.decode(input, this._enc === "utf8bom" ? "utf8" : this._enc);
    }
  }
  if (typeof G.TextEncoder !== "function") defineGlobal("TextEncoder", TextEncoder);
  if (typeof G.TextDecoder !== "function") defineGlobal("TextDecoder", TextDecoder);

  // Locale formatting (goja has no Intl). Japanese conventions.
  function groupDigits(intStr) { return intStr.replace(/\B(?=(\d{3})+(?!\d))/g, ","); }
  function formatNumber(n, o = {}) {
    if (!isFinite(n)) return String(n);
    const minF = o.minimumFractionDigits !== undefined ? o.minimumFractionDigits : 0;
    const maxF = Math.max(minF, o.maximumFractionDigits !== undefined ? o.maximumFractionDigits : (o.style === "percent" ? 0 : 3));
    let v = o.style === "percent" ? n * 100 : n;
    let s = Math.abs(v).toFixed(maxF);
    if (maxF > minF) s = s.replace(/0+$/, (z) => z.slice(0, Math.max(0, minF - (s.split(".")[1] || "").replace(/0+$/, "").length))).replace(/\.$/, "");
    let [i, f] = s.split(".");
    if (f !== undefined && f.length < minF) f = f.padEnd(minF, "0");
    if (f === undefined && minF > 0) f = "0".repeat(minF);
    let r = (o.useGrouping === false ? i : groupDigits(i)) + (f ? "." + f : "");
    if (v < 0 && Number(r.replace(/,/g, "")) !== 0) r = "-" + r;
    if (o.style === "percent") r += "%";
    if (o.style === "currency") r = (o.currency === "JPY" || !o.currency ? "￥" : o.currency + " ") + r;
    return r;
  }
  define(Number.prototype, "toLocaleString", function (locales, options) { return formatNumber(Number(this), options || {}); });
  define(Date.prototype, "toLocaleDateString", function () {
    return isNaN(this) ? "Invalid Date" : `${this.getFullYear()}/${this.getMonth() + 1}/${this.getDate()}`;
  });
  define(Date.prototype, "toLocaleTimeString", function () {
    return isNaN(this) ? "Invalid Date" : `${this.getHours()}:${pad2(this.getMinutes())}:${pad2(this.getSeconds())}`;
  });
  define(Date.prototype, "toLocaleString", function () {
    return isNaN(this) ? "Invalid Date" : `${this.toLocaleDateString()} ${this.toLocaleTimeString()}`;
  });
  if (typeof G.Intl !== "object") {
    class NumberFormat {
      constructor(locales, options) {
        this._o = options || {};
        this.format = (n) => formatNumber(Number(n), this._o);
      }
      resolvedOptions() { return Object.assign({ locale: "ja-JP" }, this._o); }
    }
    class DateTimeFormat {
      constructor(locales, options) {
        this._o = options || {};
        this.format = (d) => this._format(d);
      }
      _format(d) {
        d = d === undefined ? new Date() : new Date(d);
        const o = this._o;
        if (o.timeStyle && !o.dateStyle) return d.toLocaleTimeString();
        if (o.hour !== undefined || o.timeStyle) return d.toLocaleString();
        return d.toLocaleDateString();
      }
      resolvedOptions() { return Object.assign({ locale: "ja-JP", timeZone: "local" }, this._o); }
    }
    class Collator {
      constructor(locales, options) {
        this._o = options || {};
        this.compare = (a, b) => this._compare(a, b);
      }
      _compare(a, b) {
        if (this._o.numeric) {
          const re = /(\d+)|(\D+)/g;
          const pa = String(a).match(re) || [], pb = String(b).match(re) || [];
          for (let i = 0; i < Math.min(pa.length, pb.length); i++) {
            const x = pa[i], y = pb[i];
            if (/^\d/.test(x) && /^\d/.test(y)) { const d = Number(x) - Number(y); if (d) return d < 0 ? -1 : 1; }
            else if (x !== y) return x < y ? -1 : 1;
          }
          return pa.length - pb.length;
        }
        return String(a).localeCompare(String(b));
      }
    }
    defineGlobal("Intl", { NumberFormat, DateTimeFormat, Collator });
  }

  defineGlobal("sleep", (ms) => new Promise((resolve) => setTimeout(resolve, ms)));

  // ---------------------------------------------------------------- errors

  class DOMException extends Error {
    constructor(message = "", name = "Error") {
      super(message);
      Object.defineProperty(this, "name", { value: name, configurable: true, writable: true });
    }
  }
  if (typeof G.DOMException !== "function") defineGlobal("DOMException", DOMException);
  const notFound = (p) => new DOMException(T("fs.notFound", p), "NotFoundError");
  const typeMismatch = (p) => new DOMException(T("fs.typeMismatch", p), "TypeMismatchError");

  // ---------------------------------------------------------------- Blob / File

  function toUint8(part) {
    if (part instanceof ArrayBuffer) return new Uint8Array(part);
    if (ArrayBuffer.isView(part)) return new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
    if (part instanceof Blob) return new Uint8Array(part._bytes());
    return new Uint8Array(native.encode(String(part), "utf8"));
  }
  function concatBytes(parts) {
    const arrays = parts.map(toUint8);
    const total = arrays.reduce((s, a) => s + a.length, 0);
    const outBuf = new Uint8Array(total);
    let off = 0;
    for (const a of arrays) { outBuf.set(a, off); off += a.length; }
    return outBuf.buffer;
  }

  class Blob {
    constructor(parts = [], options = {}) {
      this._buf = concatBytes(Array.from(parts));
      this._type = options.type ? String(options.type).toLowerCase() : "";
    }
    _bytes() { return this._buf; }
    get size() { return this._bytes().byteLength; }
    get type() { return this._type; }
    async arrayBuffer() { return this._bytes().slice(0); }
    async bytes() { return new Uint8Array(this._bytes().slice(0)); }
    /** text(encoding = "utf8"): xljs extension, accepts any supported encoding or "auto". */
    async text(encoding) { return native.decode(this._bytes(), encoding || "utf8"); }
    slice(start = 0, end = undefined, type = "") {
      const b = new Blob([], { type });
      b._buf = this._bytes().slice(start, end);
      return b;
    }
    [inspectSymbol]() { return `Blob { size: ${this.size}, type: ${JSON.stringify(this.type)} }`; }
  }

  class File extends Blob {
    constructor(parts, name, options = {}) {
      super(parts || [], options);
      this._name = String(name);
      this._lastModified = options.lastModified !== undefined ? Number(options.lastModified) : Date.now();
      this._path = null;
    }
    static _fromPath(path, st) {
      const f = new File([], native.pathBasename(path), { lastModified: st.mtimeMs, type: mimeOf(path) });
      f._path = path;
      f._size = st.size;
      f._buf = null;
      return f;
    }
    _bytes() {
      if (this._buf === null) this._buf = native.fsReadBytes(this._path);
      return this._buf;
    }
    get size() { return this._buf === null ? this._size : this._buf.byteLength; }
    get name() { return this._name; }
    get lastModified() { return this._lastModified; }
    get lastModifiedDate() { return new Date(this._lastModified); }
    /** Full path of the file on disk (xljs extension; null for in-memory files). */
    get path() { return this._path; }
    async text(encoding) {
      if (this._buf === null) return native.fsReadText(this._path, encoding || "utf8");
      return super.text(encoding);
    }
    [inspectSymbol]() { return `File { name: ${JSON.stringify(this.name)}, size: ${this.size}${this._path ? ", path: " + JSON.stringify(this._path) : ""} }`; }
  }
  if (typeof G.Blob !== "function") defineGlobal("Blob", Blob);
  if (typeof G.File !== "function") defineGlobal("File", File);

  const MIME = {
    ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ".xlsm": "application/vnd.ms-excel.sheet.macroEnabled.12",
    ".xls": "application/vnd.ms-excel",
    ".csv": "text/csv", ".tsv": "text/tab-separated-values", ".txt": "text/plain",
    ".json": "application/json", ".html": "text/html", ".xml": "application/xml",
    ".pdf": "application/pdf", ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
  };
  const mimeOf = (p) => MIME[native.pathExtname(p).toLowerCase()] || "";

  // ---------------------------------------------------------------- handles

  /** Converts a path-like value (string, handle, File with path) to a path string. */
  function toPath(x, what = "path") {
    if (typeof x === "string") return x;
    if (x instanceof FileSystemHandle) return x.path;
    if (x instanceof File && x.path) return x.path;
    if (x && typeof x.path === "string") return x.path;
    throw new TypeError(T("fs.expectedPath", what));
  }

  const sameName = native.platform === "windows"
    ? (a, b) => a.toLowerCase() === b.toLowerCase()
    : (a, b) => a === b;

  class FileSystemHandle {
    constructor(kind, path, relativePath) {
      define(this, "_kind", kind);
      define(this, "_path", native.pathResolve(path));
      define(this, "_rel", relativePath === undefined ? null : relativePath);
    }
    get kind() { return this._kind; }
    get name() { return native.pathBasename(this._path); }
    /** Absolute path (xljs extension). */
    get path() { return this._path; }
    /** Path relative to the directory a walk() started from (xljs extension). */
    get relativePath() { return this._rel === null ? this.name : this._rel; }
    async isSameEntry(other) { return other instanceof FileSystemHandle && other.kind === this.kind && sameName(other.path, this.path); }
    async queryPermission() { return "granted"; }
    async requestPermission() { return "granted"; }
    /** Whether the entry currently exists (xljs extension). */
    exists() {
      const st = native.fsStat(this._path);
      return !!st && (this._kind === "file" ? st.isFile : st.isDirectory);
    }
    toString() { return this._path; }
    toJSON() { return this._path; }
    [inspectSymbol]() { return `${this.constructor.name} { ${JSON.stringify(this._path)} }`; }
  }

  class FileSystemWritableFileStream {
    constructor(path, keep) {
      this._path = path;
      this._chunks = [];
      this._pos = 0;
      this._data = keep && native.fsStat(path) ? new Uint8Array(native.fsReadBytes(path)) : new Uint8Array(0);
      this._closed = false;
    }
    _ensure(size) {
      if (this._data.length >= size) return;
      const n = new Uint8Array(Math.max(size, this._data.length * 2));
      n.set(this._data);
      this._data = n;
      this._len = this._len || 0;
    }
    async write(data) {
      if (this._closed) throw new TypeError(T("fs.streamClosed"));
      if (data && typeof data === "object" && !(data instanceof Blob) && !(data instanceof ArrayBuffer) && !ArrayBuffer.isView(data) && data.type) {
        if (data.type === "seek") { this._pos = data.position; return; }
        if (data.type === "truncate") { this._truncate(data.size); return; }
        if (data.position !== undefined) this._pos = data.position;
        data = data.data;
      }
      const bytes = toUint8(data);
      const len = this._length();
      const end = this._pos + bytes.length;
      this._ensure(end);
      this._data.set(bytes, this._pos);
      this._pos = end;
      this._len = Math.max(len, end);
    }
    _length() { return this._len === undefined ? this._data.length : this._len; }
    _truncate(size) { this._ensure(size); this._len = size; if (this._pos > size) this._pos = size; }
    async seek(position) { this._pos = position; }
    async truncate(size) { this._truncate(size); }
    async close() {
      if (this._closed) return;
      this._closed = true;
      native.fsWriteBytes(this._path, this._data.subarray(0, this._length()), false);
    }
    async abort() { this._closed = true; }
  }

  /**
   * The encoding for writing text: UTF-8, but with a BOM for .csv / .tsv so
   * that Excel does not garble a CSV written with fs.writeText (the BOM is
   * only written when the file is created).
   */
  function textEncodingFor(p, encoding) {
    if (encoding) return encoding;
    return /\.(csv|tsv)$/i.test(String(p)) ? "utf8bom" : "utf8";
  }

  class FileSystemFileHandle extends FileSystemHandle {
    constructor(path, relativePath) { super("file", path, relativePath); }
    async getFile() {
      const st = native.fsStat(this._path);
      if (!st) throw notFound(this._path);
      if (!st.isFile) throw typeMismatch(this._path);
      return File._fromPath(this._path, st);
    }
    async createWritable(options = {}) {
      return new FileSystemWritableFileStream(this._path, !!options.keepExistingData);
    }
    // xljs extensions (synchronous)
    stat() { return native.fsStat(this._path); }
    readText(encoding = "auto") { return native.fsReadText(this._path, encoding); }
    readBytes() { return native.fsReadBytes(this._path); }
    writeText(text, encoding) { native.fsWriteText(this._path, String(text), textEncodingFor(this._path, encoding), false); return this; }
    appendText(text, encoding) { native.fsWriteText(this._path, String(text), textEncodingFor(this._path, encoding), true); return this; }
    writeBytes(data) { native.fsWriteBytes(this._path, toUint8(data), false); return this; }
  }

  function makeAsyncIterable(items, map) {
    let i = 0;
    const it = {
      next() {
        if (i >= items.length) return Promise.resolve({ done: true, value: undefined });
        return Promise.resolve({ done: false, value: map(items[i++]) });
      },
      return(value) { i = items.length; return Promise.resolve({ done: true, value }); },
    };
    it[asyncIteratorSymbol] = () => it;
    return it;
  }

  /** Result of walk(): iterable with for..of and for await..of. */
  class WalkResult {
    constructor(entries) { define(this, "_entries", entries); }
    [Symbol.iterator]() { return this._entries[Symbol.iterator](); }
    [asyncIteratorSymbol]() { return makeAsyncIterable(this._entries, (x) => x); }
    get length() { return this._entries.length; }
    toArray() { return this._entries.slice(); }
    map(fn) { return this._entries.map(fn); }
    filter(fn) { return this._entries.filter(fn); }
    forEach(fn) { this._entries.forEach(fn); }
    [inspectSymbol]() { return `WalkResult(${this._entries.length}) ${inspect(this._entries)}`; }
  }

  function walkEntries(root, opts = {}) {
    if (typeof opts === "string" || Array.isArray(opts)) opts = { glob: opts };
    const include = opts.glob !== undefined ? opts.glob : opts.include;
    const list = native.fsWalk(root, include === undefined ? [] : [].concat(include),
      opts.exclude === undefined ? [] : [].concat(opts.exclude),
      !!opts.directories, opts.files === false, !!opts.hidden);
    return new WalkResult(list.map((e) => e.isDirectory
      ? new FileSystemDirectoryHandle(e.path, e.rel)
      : new FileSystemFileHandle(e.path, e.rel)));
  }

  class FileSystemDirectoryHandle extends FileSystemHandle {
    constructor(path, relativePath) { super("directory", path, relativePath); }
    _child(name) {
      if (/[\\/]/.test(name) || name === "." || name === "..") throw new TypeError(T("fs.invalidName", name));
      return native.pathJoin(this._path, name);
    }
    async getFileHandle(name, options = {}) {
      const p = this._child(name);
      const st = native.fsStat(p);
      if (!st) {
        if (!options.create) throw notFound(p);
        native.fsWriteBytes(p, new Uint8Array(0), false);
      } else if (!st.isFile) throw typeMismatch(p);
      return new FileSystemFileHandle(p);
    }
    async getDirectoryHandle(name, options = {}) {
      const p = this._child(name);
      const st = native.fsStat(p);
      if (!st) {
        if (!options.create) throw notFound(p);
        native.fsMkdir(p);
      } else if (!st.isDirectory) throw typeMismatch(p);
      return new FileSystemDirectoryHandle(p);
    }
    async removeEntry(name, options = {}) {
      const p = this._child(name);
      if (!native.fsStat(p)) throw notFound(p);
      native.fsRemove(p, !!options.recursive);
    }
    async resolve(handle) {
      const rel = native.pathRelative(this._path, handle.path);
      if (rel === ".." || rel.startsWith(".." + native.pathSep) || native.pathIsAbsolute(rel)) return null;
      return rel === "." ? [] : rel.split(native.pathSep);
    }
    _list(opts = {}) {
      return native.fsReadDir(this._path, !!opts.hidden).map((e) => e.isDirectory
        ? new FileSystemDirectoryHandle(native.pathJoin(this._path, e.name))
        : new FileSystemFileHandle(native.pathJoin(this._path, e.name)));
    }
    entries() { return makeAsyncIterable(this._list(), (h) => [h.name, h]); }
    keys() { return makeAsyncIterable(this._list(), (h) => h.name); }
    values() { return makeAsyncIterable(this._list(), (h) => h); }
    [asyncIteratorSymbol]() { return this.entries(); }
    // xljs extensions
    /** Recursively lists entries. opts: { glob, exclude, directories, files, hidden } */
    walk(opts) { return walkEntries(this._path, opts); }
    /** Lists the direct children synchronously. opts: { hidden } */
    list(opts) { return this._list(opts); }
    /** Handle for a file in this directory (no existence check). */
    file(name) { return new FileSystemFileHandle(native.pathJoin(this._path, name)); }
    /** Handle for a sub directory (no existence check). */
    dir(name) { return new FileSystemDirectoryHandle(native.pathJoin(this._path, name)); }
  }

  defineGlobal("FileSystemHandle", FileSystemHandle);
  defineGlobal("FileSystemFileHandle", FileSystemFileHandle);
  defineGlobal("FileSystemDirectoryHandle", FileSystemDirectoryHandle);
  defineGlobal("FileSystemWritableFileStream", FileSystemWritableFileStream);

  // ---------------------------------------------------------------- fs / path

  // ---------------------------------------------------------------- json helpers

  /** Reads text from a path, handle, File or Blob. */
  function readSourceText(src, encoding) {
    if (src instanceof Blob && !(src instanceof File && src.path)) return native.decode(src._bytes(), encoding || "auto");
    return native.fsReadText(toPath(src), encoding || "auto");
  }
  function localISO(d) {
    if (isNaN(d.getTime())) return null;
    const off = -d.getTimezoneOffset();
    const sign = off >= 0 ? "+" : "-";
    const ms = d.getMilliseconds() ? "." + String(d.getMilliseconds()).padStart(3, "0") : "";
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}T${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}${ms}` +
      `${sign}${pad2(Math.floor(Math.abs(off) / 60))}:${pad2(Math.abs(off) % 60)}`;
  }
  const ISO_DATETIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?$/;
  function jsonParse(text, opts = {}) {
    const reviver = opts.dates
      ? (k, v) => (typeof v === "string" && ISO_DATETIME.test(v) ? new Date(v) : v)
      : undefined;
    return JSON.parse(text.replace(/^\uFEFF/, ""), reviver);
  }
  function jsonParseFile(src, text, opts, line) {
    try {
      return jsonParse(text, opts);
    } catch (e) {
      const where = (src instanceof Blob && !(src instanceof File && src.path) ? "Blob" : toPath(src)) + (line ? `:${line}` : "");
      throw new SyntaxError(T("json.parseError", where, e.message));
    }
  }
  /** JSON.stringify where Dates use local time with offset by default (dates: "iso" for UTC). */
  function jsonStringify(value, opts = {}, space) {
    const local = opts.dates !== "iso";
    return JSON.stringify(value, local ? function (k, v) {
      const raw = plainDate(this[k]);
      return raw instanceof Date ? localISO(raw) : v;
    } : null, space);
  }

  const fsObj = {
    file: (p) => new FileSystemFileHandle(toPath(p)),
    dir: (p) => new FileSystemDirectoryHandle(toPath(p)),
    /** Async variants mirroring the picker style. */
    getFileHandle: async (p) => {
      const h = new FileSystemFileHandle(toPath(p));
      if (!h.exists()) throw notFound(h.path);
      return h;
    },
    getDirectoryHandle: async (p) => {
      const h = new FileSystemDirectoryHandle(toPath(p));
      if (!h.exists()) throw notFound(h.path);
      return h;
    },
    exists: (p) => !!native.fsStat(toPath(p)),
    stat: (p) => native.fsStat(toPath(p)),
    isFile: (p) => { const s = native.fsStat(toPath(p)); return !!s && s.isFile; },
    isDirectory: (p) => { const s = native.fsStat(toPath(p)); return !!s && s.isDirectory; },
    readText: (p, encoding = "auto") => native.fsReadText(toPath(p), encoding),
    writeText: (p, text, encoding) => native.fsWriteText(toPath(p), String(text), textEncodingFor(toPath(p), encoding), false),
    appendText: (p, text, encoding) => native.fsWriteText(toPath(p), String(text), textEncodingFor(toPath(p), encoding), true),
    readBytes: (p) => native.fsReadBytes(toPath(p)),
    writeBytes: (p, data) => native.fsWriteBytes(toPath(p), toUint8(data), false),
    /** Reads a JSON file (path, handle, File or Blob). opts: { encoding = "auto", dates = false } */
    readJSON: (src, opts = {}) => jsonParseFile(src, readSourceText(src, opts.encoding), opts),
    /** Writes value as JSON. opts: { space = 2, encoding = "utf8", dates = "local" | "iso", append } (a number means space) */
    writeJSON: (dest, value, opts = {}) => {
      if (typeof opts === "number" || typeof opts === "string") opts = { space: opts };
      native.fsWriteText(toPath(dest), jsonStringify(value, opts, opts.space === undefined ? 2 : opts.space) + "\n", opts.encoding || "utf8", false);
    },
    /** Reads JSON Lines (one JSON value per line). */
    readJSONL: (src, opts = {}) => {
      const out = [];
      readSourceText(src, opts.encoding).split(/\r?\n/).forEach((l, i) => {
        if (l.trim() !== "") out.push(jsonParseFile(src, l, opts, i + 1));
      });
      return out;
    },
    /** Writes an array as JSON Lines. opts: { encoding = "utf8", dates, append } */
    writeJSONL: (dest, rows, opts = {}) => {
      const text = Array.from(rows || []).map((r) => jsonStringify(r, opts)).join("\n") + "\n";
      native.fsWriteText(toPath(dest), text, opts.encoding || "utf8", !!opts.append);
    },
    list: (p = ".", opts = {}) => native.fsReadDir(toPath(p), !!opts.hidden).map((e) => e.name),
    walk: (p, opts) => walkEntries(toPath(p), opts),
    glob: (pattern, opts = {}) => walkEntries(opts.cwd ? toPath(opts.cwd) : native.fsCwd(), { glob: pattern, exclude: opts.exclude, hidden: opts.hidden }).toArray(),
    mkdir: (p) => native.fsMkdir(toPath(p)),
    remove: (p, opts = {}) => native.fsRemove(toPath(p), !!opts.recursive),
    rename: (from, to) => native.fsRename(toPath(from), toPath(to)),
    move: (from, to) => native.fsRename(toPath(from), toPath(to)),
    copy: (from, to) => native.fsCopy(toPath(from), toPath(to)),
    cwd: () => native.fsCwd(),
    chdir: (p) => native.fsChdir(toPath(p)),
  };
  defineGlobal("fs", fsObj);

  const pathObj = {
    sep: native.pathSep,
    join: (...a) => native.pathJoin(...a.map((x) => toPath(x))),
    resolve: (...a) => native.pathResolve(...a.map((x) => toPath(x))),
    dirname: (p) => native.pathDirname(toPath(p)),
    basename: (p, ext) => native.pathBasename(toPath(p), ext || ""),
    extname: (p) => native.pathExtname(toPath(p)),
    normalize: (p) => native.pathNormalize(toPath(p)),
    isAbsolute: (p) => native.pathIsAbsolute(toPath(p)),
    relative: (from, to) => native.pathRelative(toPath(from), toPath(to)),
  };
  defineGlobal("path", pathObj);

  // ---------------------------------------------------------------- xljs

  const xljsObj = {
    version: native.version,
    args: native.args,
    script: native.script,
    scriptDir: native.pathDirname(native.script),
    platform: native.platform,
    ui: native.uiName,
    title: native.title,
    exit: (code = 0) => native.exit(code),
    env: (name) => native.getenv(String(name)),
    inspect,
  };
  defineGlobal("xljs", xljsObj);
  defineGlobal("exit", xljsObj.exit);

  // ---------------------------------------------------------------- dialogs & pickers

  const title = () => String(xljsObj.title || "xljs");
  const msg = (m) => (m === undefined ? "" : typeof m === "string" ? m : inspect(m));
  defineGlobal("alert", (message) => native.uiAlert(title(), msg(message)));
  defineGlobal("confirm", (message) => native.uiConfirm(title(), msg(message)));
  defineGlobal("prompt", (message, defaultValue) =>
    native.uiPrompt(title(), msg(message), defaultValue === undefined || defaultValue === null ? "" : String(defaultValue)));

  function toFilters(types, excludeAll) {
    const filters = [];
    for (const t of types || []) {
      let exts = [];
      const acc = t.accept;
      if (typeof acc === "string") exts = [acc];
      else if (Array.isArray(acc)) exts = acc;
      else if (acc && typeof acc === "object") for (const k of Object.keys(acc)) exts = exts.concat(acc[k]);
      const patterns = exts.map((e) => (e.startsWith("*") ? e : e.startsWith(".") ? "*" + e : "*." + e));
      if (patterns.length) filters.push({ description: t.description || "", patterns });
    }
    if (filters.length && !excludeAll) filters.push({ description: T("ui.allFiles"), patterns: ["*.*"] });
    return filters;
  }
  function startInPath(startIn) {
    if (!startIn) return "";
    if (startIn instanceof FileSystemFileHandle) return native.pathDirname(startIn.path);
    if (startIn instanceof FileSystemDirectoryHandle) return startIn.path;
    const s = String(startIn);
    return native.knownFolder(s) || s;
  }
  function pickerOptions(o = {}) {
    return {
      title: o.title || "",
      multiple: !!o.multiple,
      startIn: startInPath(o.startIn),
      suggestedName: o.suggestedName || "",
      filters: toFilters(o.types, o.excludeAcceptAllOption),
    };
  }
  /** Returns [] when cancelled. */
  async function showOpenFilePicker(options = {}) {
    return native.uiOpen(pickerOptions(options)).map((p) => new FileSystemFileHandle(p));
  }
  /** Returns null when cancelled. */
  async function showSaveFilePicker(options = {}) {
    // Without types, the extension of suggestedName becomes the file type
    // (and the default extension), so "集計" is saved as "集計.xlsx".
    const ext = native.pathExtname(String(options.suggestedName || ""));
    if (!options.types && /^\.[\w]+$/.test(ext)) options = Object.assign({}, options, { types: [{ description: ext.slice(1).toUpperCase(), accept: [ext] }] });
    const p = native.uiSave(pickerOptions(options));
    return p === null ? null : new FileSystemFileHandle(p);
  }
  /** Returns null when cancelled. */
  async function showDirectoryPicker(options = {}) {
    const p = native.uiDirectory(pickerOptions(options));
    return p === null ? null : new FileSystemDirectoryHandle(p);
  }
  defineGlobal("showOpenFilePicker", showOpenFilePicker);
  defineGlobal("showSaveFilePicker", showSaveFilePicker);
  defineGlobal("showDirectoryPicker", showDirectoryPicker);

  // ---------------------------------------------------------------- shell

  const isURL = (s) => typeof s === "string" && /^[a-z][a-z0-9+.-]*:\/\//i.test(s);
  const shellObj = {
    /**
     * Opens Explorer with the files selected. Accepts paths, handles or Files,
     * as separate arguments or arrays. Files in different folders open one
     * window per folder.
     */
    showItemInFolder(...items) {
      const paths = items.flat(Infinity).filter((x) => x !== null && x !== undefined).map((x) => toPath(x));
      native.shellShowInFolder(paths);
    },
    /** Opens a file or folder with its default application. */
    openPath(target) { native.shellOpen(toPath(target)); },
    /** Opens a URL in the default browser. */
    openExternal(url) {
      if (!isURL(url)) throw new TypeError(T("shell.expectedURL", url));
      native.shellOpen(String(url));
    },
  };
  shellObj.showInFolder = shellObj.showItemInFolder;
  shellObj.open = (target) => (isURL(target) ? shellObj.openExternal(target) : shellObj.openPath(target));
  defineGlobal("shell", shellObj);

  // ---------------------------------------------------------------- progress

  let lastCancelled = false;
  class Progress {
    constructor(options = {}) {
      if (typeof options === "string") options = { title: options };
      define(this, "_total", Math.max(0, Math.floor(Number(options.total) || 0)));
      define(this, "_done", 0);
      define(this, "_message", options.message ? String(options.message) : "");
      define(this, "_closed", false);
      define(this, "_n", native.uiProgress(options.title !== undefined ? String(options.title) : title(), this._total,
        this._message, options.cancellable !== false));
      lastCancelled = false;
    }
    get total() { return this._total; }
    set total(n) { this._total = Math.max(0, Math.floor(Number(n) || 0)); this._push(); }
    get done() { return this._done; }
    get message() { return this._message; }
    set message(m) { this._message = String(m); this._push(); }
    get cancelled() {
      if (this._closed) return lastCancelled;
      const c = this._n.cancelled();
      if (c) lastCancelled = true;
      return c;
    }
    _push() { if (!this._closed) this._n.update(this._done, this._total, this._message); }
    /** Sets the number of completed items and optionally the message. */
    update(done, message) {
      this._done = Math.max(0, Math.floor(Number(done) || 0));
      if (message !== undefined) this._message = String(message);
      this._push();
      return this;
    }
    increment(message) { return this.update(this._done + 1, message); }
    close() {
      if (this._closed) return;
      this.cancelled; // latch cancellation state
      this._closed = true;
      this._n.close();
    }
  }

  const newProgress = (opts) => new Progress(opts);

  // make(opts) returns the indicator: { cancelled, update(done, message), close() }.
  function* withProgressSync(iterable, opts, label, total, make = newProgress, stop = () => {}) {
    const p = make(Object.assign({}, opts, { total }));
    let i = 0;
    try {
      for (const item of iterable) {
        if (p.cancelled) {
          p.close();
          stop();
          return;
        }
        p.update(i, label(item, i));
        yield item;
        i++;
      }
      p.update(i, "");
    } finally {
      p.close();
    }
  }

  function withProgressAsync(iterable, opts, label, total, make = newProgress, stop = () => {}) {
    const iter = iterable[asyncIteratorSymbol]();
    const p = make(Object.assign({}, opts, { total }));
    let i = 0, finished = false;
    const finish = () => { if (!finished) { finished = true; p.close(); } };
    const it = {
      async next() {
        if (finished) return { done: true, value: undefined };
        if (p.cancelled) {
          finish();
          if (iter.return) await iter.return();
          stop();
          return { done: true, value: undefined };
        }
        let r;
        try { r = await iter.next(); } catch (e) { finish(); throw e; }
        if (r.done) { p.update(i, ""); finish(); return r; }
        p.update(i, label(r.value, i));
        i++;
        return r;
      },
      async return(value) {
        finish();
        if (iter.return) await iter.return();
        return { done: true, value };
      },
      async throw(e) { finish(); throw e; },
    };
    it[asyncIteratorSymbol] = () => it;
    return it;
  }

  function defaultLabel(item) {
    if (item instanceof FileSystemHandle) return item.relativePath;
    if (typeof item === "string" || typeof item === "number") return String(item);
    return "";
  }

  /**
   * Wraps an iterable and shows progress while it is consumed. When the
   * user cancels, stop() ends the script (or the form's run); with
   * onCancel: "break" the loop just ends (check ui.cancelled afterwards).
   */
  function withProgress(iterable, options = {}, make = newProgress, stop = () => native.exit(1)) {
    if (typeof options === "string") options = { title: options };
    const label = typeof options.label === "function" ? options.label : defaultLabel;
    let total = options.total;
    if (total === undefined) {
      if (typeof iterable.length === "number") total = iterable.length;
      else if (typeof iterable.size === "number") total = iterable.size;
      else total = 0;
    }
    if (options.onCancel === "break") stop = () => {};
    if (iterable && typeof iterable[Symbol.iterator] === "function") return withProgressSync(iterable, options, label, total, make, stop);
    if (iterable && typeof iterable[asyncIteratorSymbol] === "function") return withProgressAsync(iterable, options, label, total, make, stop);
    throw new TypeError(T("ui.expectedIterable"));
  }

  const uiObj = {
    alert: G.alert,
    confirm: G.confirm,
    prompt: G.prompt,
    showOpenFilePicker,
    showSaveFilePicker,
    showDirectoryPicker,
    progress: (options) => new Progress(options),
    withProgress: (iterable, options) => withProgress(iterable, options),
    form: (options) => showForm(options),
    table: (rows, options) => showTable(rows, options),
    /** Whether the most recent progress was cancelled by the user. */
    get cancelled() { return lastCancelled; },
    error: (message) => native.uiError(title(), msg(message)),
    Progress,
  };
  defineGlobal("ui", uiObj);

  // ---------------------------------------------------------------- app (HTML windows)
  //
  // Windows and the page runtime (internal/ui/web/xljs.js) exchange JSON
  // messages with an "xljs" key naming the kind: ready/init, invoke,
  // result/error, event, state, dom, dom-event, eval and unhandled. Values
  // travel through wireStringify/wireParse: JSON plus Date, handles, bytes
  // and Error, and files the page attaches ({"$attach": index}).

  /** JSON.stringify for messages to a page. */
  function wireStringify(m) {
    return JSON.stringify(m, function (k, v) {
      const raw = this[k];
      if (raw instanceof Date) return isNaN(raw.getTime()) ? null : { $date: raw.toISOString() };
      if (raw instanceof FileSystemHandle) return raw.kind === "file" ? { $file: raw.path } : { $dir: raw.path };
      if (raw instanceof File) return raw.path ? { $file: raw.path } : null;
      if (raw instanceof ArrayBuffer) return { $bytes: bytesToBase64(new Uint8Array(raw)) };
      if (ArrayBuffer.isView(raw)) return { $bytes: bytesToBase64(new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength)) };
      if (raw instanceof Error) return { $error: errorInfo(raw) };
      if (raw instanceof Map) return Object.fromEntries(raw);
      if (raw instanceof Set) return Array.from(raw);
      return v;
    });
  }

  /** Revives the special values of a parsed message; files are the attached paths. */
  function wireRevive(v, files) {
    if (Array.isArray(v)) return v.map((x) => wireRevive(x, files));
    if (v === null || typeof v !== "object") return v;
    const keys = Object.keys(v);
    if (keys.length === 1) {
      const x = v[keys[0]];
      switch (keys[0]) {
        case "$date": return new Date(x);
        case "$file": return new FileSystemFileHandle(String(x));
        case "$dir": return new FileSystemDirectoryHandle(String(x));
        case "$bytes": return base64ToBytes(x).buffer;
        case "$error": return errorFrom(x);
        case "$attach": {
          // A dropped folder arrives as a File too.
          const p = files && files[x];
          if (!p) return null;
          const st = native.fsStat(p);
          return st && st.isDirectory ? new FileSystemDirectoryHandle(p) : new FileSystemFileHandle(p);
        }
      }
    }
    const out = {};
    for (const k of keys) out[k] = wireRevive(v[k], files);
    return out;
  }

  function errorInfo(e) {
    if (!(e instanceof Error)) return { name: "Error", message: typeof e === "string" ? e : inspect(e) };
    return { name: String(e.name), message: String(e.message), stack: errorText(e) };
  }

  /** An Error rebuilt from errorInfo (the page's or the script's). */
  function errorFrom(info) {
    info = info || {};
    const e = new Error(info.message === undefined ? "" : String(info.message));
    if (info.name) define(e, "name", String(info.name));
    if (info.stack) define(e, "stack", String(info.stack));
    return e;
  }

  /** Shows an error that no one caught (in a handler called by a window) without ending the script. */
  function reportError(e) {
    native.uiError(title(), e instanceof Error ? errorText(e) : msg(e));
  }

  /** Calls fn and reports what it throws or rejects with. */
  function guarded(fn, self, args) {
    try {
      const r = fn.apply(self, args);
      if (r && typeof r.then === "function") r.then(undefined, reportError);
    } catch (e) {
      reportError(e);
    }
  }

  // Functions windows can call: the entry's exports and app.handle.
  let entryExports = null;
  native.setExports = (obj) => { entryExports = obj; };
  const appHandlers = new Map();
  const hasExport = (name) => entryExports !== null && Object.prototype.hasOwnProperty.call(entryExports, name);
  function appHandler(name) {
    if (appHandlers.has(name)) return appHandlers.get(name);
    const fn = hasExport(name) ? entryExports[name] : undefined;
    return typeof fn === "function" ? fn : null;
  }

  let wireID = 0;

  /** Elements of a window matching a CSS selector (win.$). */
  class AppElements {
    constructor(win, selector) {
      define(this, "_w", win);
      define(this, "_sel", selector);
    }
    get selector() { return this._sel; }
    _op(op, args) { this._w._send({ xljs: "dom", sel: this._sel, op, args }); return this; }
    _read(op, args) { return this._w._request({ xljs: "dom", sel: this._sel, op, args }); }
    text(value) { return arguments.length ? this._op("text", [value]) : this._read("text", []); }
    html(value) { return arguments.length ? this._op("html", [value]) : this._read("html", []); }
    val(value) { return arguments.length ? this._op("val", [value]) : this._read("val", []); }
    attr(name, value) { return arguments.length > 1 ? this._op("attr", [String(name), value]) : this._read("attr", [String(name)]); }
    css(name, value) {
      if (name !== null && typeof name === "object") return this._op("css", [name]);
      return arguments.length > 1 ? this._op("css", [{ [name]: value }]) : this._read("css", [String(name)]);
    }
    addClass(names) { return this._op("addClass", [String(names)]); }
    removeClass(names) { return this._op("removeClass", [String(names)]); }
    toggleClass(names, force) { return this._op("toggleClass", arguments.length > 1 ? [String(names), !!force] : [String(names)]); }
    show() { return this._op("show", []); }
    hide() { return this._op("hide", []); }
    /** Calls fn(event) for DOM events on the elements (also ones added later). options: { preventDefault } */
    on(type, fn, options = {}) {
      if (typeof fn !== "function") throw new TypeError(T("engine.callbackNotFunction"));
      const h = { hid: ++wireID, sel: this._sel, type: String(type), fn, preventDefault: !!options.preventDefault };
      this._w._dom.set(h.hid, h);
      this._w._send({ xljs: "dom", op: "on", sel: h.sel, type: h.type, hid: h.hid, preventDefault: h.preventDefault });
      return this;
    }
    /** Removes handlers added with on (all of them for type, or for this selector). */
    off(type, fn) {
      for (const h of Array.from(this._w._dom.values())) {
        if (h.sel !== this._sel || (type !== undefined && h.type !== String(type)) || (fn !== undefined && h.fn !== fn)) continue;
        this._w._dom.delete(h.hid);
        this._w._send({ xljs: "dom", op: "off", hid: h.hid });
      }
      return this;
    }
    [inspectSymbol]() { return `AppElements { ${JSON.stringify(this._sel)} }`; }
  }

  /** An HTML window opened by app.open (or the main window of @mode app). */
  class AppWindow {
    constructor(html, winTitle, width, height) {
      define(this, "_queue", []);        // messages until the page is ready
      define(this, "_ready", false);
      define(this, "_closed", false);
      define(this, "_pending", new Map()); // id -> { resolve, reject }
      define(this, "_listeners", new Map()); // event -> [fn]
      define(this, "_dom", new Map());   // hid -> handler
      define(this, "_store", {});
      const win = this;
      define(this, "_state", new Proxy(this._store, {
        set(t, k, v) {
          t[k] = v;
          if (typeof k === "string") win._send({ xljs: "state", key: k, value: v });
          return true;
        },
        deleteProperty(t, k) {
          delete t[k];
          if (typeof k === "string") win._send({ xljs: "state", key: k, deleted: true });
          return true;
        },
      }));
      define(this, "_n", native.appOpen(html, winTitle, width, height, (text, files) => this._receive(text, files)));
      /** Resolves when the window has been closed. */
      define(this, "closed", this._n.closed.then(() => {
        this._closed = true;
        this._rejectPending(T("app.windowClosed"));
        this._listeners.clear();
        this._dom.clear();
      }));
    }

    /** Shared values: assignments are sent to the page (xljs.state, data-bind). Only top-level keys are synchronized. */
    get state() { return this._state; }
    set state(values) { Object.assign(this._state, values); }

    close() { this._n.close(); }

    /** Calls the page's function (xljs.handle, or a global function) and resolves with its result. */
    invoke(name, ...args) { return this._request({ xljs: "invoke", name: String(name), args }); }

    /** Sends an event to the page (xljs.on). */
    emit(name, ...args) { this._send({ xljs: "event", name: String(name), args }); }

    /** Listens to events from the page (xljs.emit), and to "state" (key, value) when the page changes a value. */
    on(name, fn) {
      if (typeof fn !== "function") throw new TypeError(T("engine.callbackNotFunction"));
      name = String(name);
      if (!this._listeners.has(name)) this._listeners.set(name, []);
      this._listeners.get(name).push(fn);
      return this;
    }

    off(name, fn) {
      name = String(name);
      const list = this._listeners.get(name);
      if (!list) return this;
      if (fn === undefined) this._listeners.delete(name);
      else this._listeners.set(name, list.filter((f) => f !== fn));
      return this;
    }

    /** Elements matching selector; writes are sent at once, reads resolve with the first element's value. */
    $(selector) { return new AppElements(this, String(selector)); }

    /**
     * Runs fn (its source; no closures) in the page with args and resolves
     * with its result. A string is evaluated as an expression.
     */
    evaluate(fn, ...args) {
      const isFn = typeof fn === "function";
      return this._request({ xljs: "eval", code: isFn ? Function.prototype.toString.call(fn) : String(fn), fn: isFn, args });
    }

    _send(m) {
      if (this._closed) return;
      const text = wireStringify(m);
      if (this._ready) this._n.post(text);
      else this._queue.push({ m, text });
    }

    _request(m) {
      if (this._closed) return Promise.reject(new Error(T("app.windowClosed")));
      return new Promise((resolve, reject) => {
        m.id = ++wireID;
        this._pending.set(m.id, { resolve, reject });
        this._send(m);
      });
    }

    _rejectPending(message) {
      const pending = Array.from(this._pending.values());
      this._pending.clear();
      for (const p of pending) p.reject(new Error(message));
    }

    _fire(name, args) {
      for (const fn of (this._listeners.get(name) || []).slice()) guarded(fn, this, args);
    }

    _onReady() {
      if (this._ready) {
        // The page was reloaded: what was sent to the old one is lost.
        this._rejectPending(T("app.pageReloaded"));
      }
      const handlers = Array.from(this._dom.values()).map((h) => ({ hid: h.hid, sel: h.sel, type: h.type, preventDefault: h.preventDefault }));
      this._n.post(wireStringify({ xljs: "init", state: this._store, handlers }));
      this._ready = true;
      // The snapshot above covers the queued state and handler changes.
      const queue = this._queue.splice(0);
      for (const q of queue) {
        if (q.m.xljs === "state" || (q.m.xljs === "dom" && (q.m.op === "on" || q.m.op === "off"))) continue;
        this._n.post(q.text);
      }
    }

    _serve(id, name, args) {
      const reply = (m) => { m.id = id; this._send(m); };
      const fail = (e) => reply({ xljs: "error", error: errorInfo(e) });
      const fn = appHandler(name);
      if (!fn) {
        fail(new Error(T("app.noHandler", name)));
        return;
      }
      let r;
      try {
        r = fn.apply(this, args);
      } catch (e) {
        fail(e);
        return;
      }
      Promise.resolve(r).then((value) => reply({ xljs: "result", value }), fail);
    }

    _receive(text, files) {
      let m;
      try { m = JSON.parse(text); } catch (e) { return; }
      if (!m || typeof m.xljs !== "string" || this._closed) return;
      switch (m.xljs) {
        case "ready":
          this._onReady();
          break;
        case "invoke":
          this._serve(m.id, String(m.name), Array.isArray(m.args) ? wireRevive(m.args, files) : []);
          break;
        case "result":
        case "error": {
          const p = this._pending.get(m.id);
          if (!p) break;
          this._pending.delete(m.id);
          if (m.xljs === "result") p.resolve(wireRevive(m.value, files));
          else if (m.error && m.error.code === "noHandler") p.reject(new Error(T("app.noPageHandler", m.error.message)));
          else p.reject(errorFrom(m.error));
          break;
        }
        case "event":
          this._fire(String(m.name), Array.isArray(m.args) ? wireRevive(m.args, files) : []);
          break;
        case "state": {
          const key = String(m.key);
          const value = m.deleted ? undefined : wireRevive(m.value, files);
          if (m.deleted) delete this._store[key];
          else this._store[key] = value;
          this._fire("state", [key, value]);
          break;
        }
        case "dom-event": {
          const h = this._dom.get(m.hid);
          if (h) guarded(h.fn, this, [wireRevive(m.event, files)]);
          break;
        }
        case "unhandled":
          reportError(errorFrom(m.error));
          break;
      }
    }

    [inspectSymbol]() { return "AppWindow {}"; }
  }

  const appObj = {
    /** The main window of an `@mode app` script (null otherwise). */
    main: null,
    /**
     * Opens a window. html is relative to the script folder and must be
     * covered by @html/@assets. The process keeps running while it is open.
     */
    open(options = {}) {
      if (typeof options === "string") options = { html: options };
      return new AppWindow(options.html === undefined ? "" : String(options.html),
        options.title === undefined ? title() : String(options.title),
        Number(options.width) || 0, Number(options.height) || 0);
    },
    /**
     * Lets windows call fn by name (xljs.invoke), like an export of the
     * entry file. null removes it.
     */
    handle(name, fn) {
      name = String(name);
      if (fn === null || fn === undefined) {
        appHandlers.delete(name);
        return;
      }
      if (typeof fn !== "function") throw new TypeError(T("engine.callbackNotFunction"));
      if (hasExport(name)) throw new Error(T("app.handleExported", name));
      appHandlers.set(name, fn);
    },
  };
  defineGlobal("app", appObj);
  native.openMain = () => {
    appObj.main = new AppWindow(native.appHTML || "", title(), native.appWidth || 0, native.appHeight || 0);
  };

  // ---------------------------------------------------------------- forms & tables
  //
  // With the native UI both are built-in pages (internal/ui/web) that talk to
  // the script through JSON messages; other UIs answer native.uiForm and
  // native.uiTable. Form values travel in "wire" form: strings, except
  // checkbox (boolean) and files (array of paths).

  /** Opens a built-in page; onMessage(msg, files) gets its parsed messages. */
  function openPage(page, opts, onMessage) {
    const n = native.appOpen("_xljs/" + page, opts.title, opts.width, opts.height, (text, files) => {
      let m;
      try { m = JSON.parse(text); } catch (e) { return; }
      if (m && typeof m.type === "string") onMessage(m, files.filter((f) => f !== ""));
    });
    return { n, send: (m) => n.post(JSON.stringify(m)) };
  }

  /** Message of a caught error for display: "Name: message" and the script's stack frames. */
  function errorText(e) {
    if (!(e instanceof Error)) return native.displayText(typeof e === "string" ? e : inspect(e));
    const frames = String(e.stack || "").split("\n")
      .filter((l) => /^\s+at /.test(l) && !/xljs:prelude|\(native\)/.test(l))
      .map((l) => l.replace(/\(\d+\)\)$/, ")").replace(/(:\d+:\d+)\(\d+\)/, "$1"));
    return native.displayText([`${e.name}: ${e.message}`, ...frames].join("\n"));
  }

  /** Existing paths among the values returned by onSubmit (paths, handles, Files, arrays). */
  function outputsOf(r) {
    const out = [];
    for (const x of [r].flat(Infinity)) {
      if (x === null || x === undefined) continue;
      let p;
      try { p = toPath(x); } catch (e) { continue; }
      if (native.fsStat(p)) out.push(native.pathResolve(p));
    }
    return out;
  }

  const FIELD_TYPES = ["text", "password", "number", "date", "month", "time", "select", "radio", "checkbox",
    "textarea", "file", "files", "folder", "saveFile", "table"];

  /** Types of editable table cells. */
  const CELL_TYPES = ["text", "number", "date", "month", "time", "select", "checkbox"];

  /**
   * The table of a "table" field: { data, cols, select } (see ui.table) and,
   * when cells are editable, edit (per column: a normalized field for the
   * cell, or null), bad (cells whose last input did not parse: "row:col" ->
   * message) and validateRow.
   */
  function formTable(f, rows, columns) {
    const data = Array.from(rows || []);
    const sel = f.selectable === true ? "multiple" : f.selectable ? String(f.selectable) : "";
    if (sel && sel !== "single" && sel !== "multiple") throw new TypeError(T("table.badSelectable", sel));
    const cols = tableColumns(data, columns);
    const specs = Array.from(columns || []);
    const edit = cols.map((c, i) => {
      const s = specs[i] !== null && typeof specs[i] === "object" ? specs[i] : {};
      if (!(s.editable !== undefined ? s.editable : f.editable) || c.key === null) return null;
      let type = s.type;
      if (type === undefined) {
        // From the values: numbers, dates and booleans; otherwise text.
        const v = data.map((r) => plainDate(c.get(r))).find((x) => x !== null && x !== undefined && x !== "");
        type = typeof v === "number" ? "number" : typeof v === "boolean" ? "checkbox" : v instanceof Date ? "date" : "text";
      }
      if (!CELL_TYPES.includes(String(type))) throw new TypeError(T("form.cellType", c.label, type));
      const cf = normField(Object.assign({}, s, { name: c.label, type: String(type) }), i, -1);
      cf.col = i;
      cf.validate = typeof s.validate === "function" ? s.validate : null;
      return cf;
    });
    return {
      data, cols, select: sel,
      edit: edit.some(Boolean) ? edit : null, bad: new Map(),
      validateRow: typeof f.validateRow === "function" ? f.validateRow : null,
    };
  }

  /** Wire value of a cell for checking it again (empty cells are "", not a select's first option). */
  const cellToWire = (cf, v) => (v === null || v === undefined || v === "" ? (cf.type === "checkbox" ? false : "") : toWire(cf, v));

  /**
   * Applies an input to a cell: the row gets the value when it passes the
   * cell's checks; otherwise the message is kept until the next input.
   */
  function setCell(t, r, c, w) {
    const cf = t.edit && t.edit[c];
    if (!cf || !Number.isInteger(r) || r < 0 || r >= t.data.length) return false;
    const v = fromWire(cf, w);
    const e = checkValue(cf, w, v);
    if (e) t.bad.set(r + ":" + c, e);
    else {
      t.bad.delete(r + ":" + c);
      t.cols[c].set(t.data[r], v);
    }
    return true;
  }

  /** Errors of a row of an editable table: [{ row, col, message }] (col -1: the row as a whole). */
  async function rowErrors(t, r) {
    const out = [];
    const row = t.data[r];
    for (const cf of t.edit) {
      if (!cf) continue;
      let e = t.bad.get(r + ":" + cf.col) || "";
      if (!e) {
        const v = t.cols[cf.col].get(row);
        const w = cellToWire(cf, v);
        e = checkValue(cf, w, fromWire(cf, w));
        if (!e && cf.validate) e = await cf.validate(plainDate(v), row, r) || "";
      }
      if (e) out.push({ row: r, col: cf.col, message: String(e) });
    }
    if (t.validateRow) {
      const m = await t.validateRow(row, r);
      if (typeof m === "string" && m) out.push({ row: r, col: -1, message: m });
      else if (m && typeof m === "object") {
        for (const [k, msg] of Object.entries(m)) {
          if (!msg) continue;
          const col = t.cols.findIndex((c) => String(c.key) === k || c.label === k);
          if (!out.some((x) => x.col === col && col >= 0)) out.push({ row: r, col, message: String(msg) });
        }
      }
    }
    return out;
  }

  /** Errors of all rows of an editable table. */
  async function tableErrors(t) {
    const out = [];
    if (t.edit) for (let r = 0; r < t.data.length; r++) out.push(...(await rowErrors(t, r)));
    return out;
  }

  /** How the page edits the cells: per column { type, options, min, max, step, required } or null. */
  const editWire = (t) => (t.edit ? t.edit.map((cf) => cf && {
    type: cf.type, options: cf.options, min: cf.min, max: cf.max, step: cf.step, required: cf.required,
  }) : null);

  /** Rows and columns of a form table for the page. */
  const tableWire = (t) => ({
    columns: t.cols.map((c) => c.label),
    rows: t.data.map((r) => t.cols.map((c) => cellWire(c.get(r)))),
  });

  /** Rows and columns of a form table as text (console and mock). */
  const tableText = (t) => {
    const w = tableWire(t);
    return { columns: w.columns, rows: w.rows.map((r) => r.map((v) => (v === null ? "" : String(v)))) };
  };

  /**
   * The Font Awesome classes for an icon option: a bare name ("folder") is
   * the solid icon, classes ("fa-regular fa-folder") are kept.
   */
  function faClass(icon) {
    if (icon === undefined || icon === null || icon === "") return "";
    const words = String(icon).trim().split(/\s+/).filter((w) => /^[\w-]+$/.test(w));
    if (words.length === 1 && !words[0].startsWith("fa-")) return "fa-solid fa-" + words[0];
    return words.join(" ");
  }

  function normField(f, i, tab) {
    if (!f || typeof f !== "object" || f.name === undefined || f.name === null || f.name === "") {
      throw new TypeError(T("form.fieldName", i + 1));
    }
    const name = String(f.name);
    const type = f.type === undefined ? "text" : String(f.type);
    if (!FIELD_TYPES.includes(type)) throw new TypeError(T("form.fieldType", name, type));
    const options = Array.from(f.options || []).map((o) => (o !== null && typeof o === "object"
      ? { value: String(o.value), label: String(o.label === undefined ? o.value : o.label) }
      : { value: String(o), label: String(o) }));
    if ((type === "select" || type === "radio") && !options.length) throw new TypeError(T("form.noOptions", name));
    const accept = (typeof f.accept === "string" ? [f.accept] : Array.from(f.accept || []))
      .map((a) => String(a).replace(/^\*/, "")).map((a) => (a.startsWith(".") ? a : "." + a).toLowerCase());
    const num = (v) => (v === undefined || v === null || v === "" ? null : Number(v));
    const table = type === "table" ? formTable(f, f.rows, f.columns) : null;
    return {
      name, type, options, accept, table, tab, columns: f.columns,
      tableSpec: { selectable: f.selectable, editable: f.editable, validateRow: f.validateRow },
      label: f.label === undefined ? name : String(f.label),
      icon: faClass(f.icon),
      required: !!f.required,
      placeholder: f.placeholder ? String(f.placeholder) : "",
      help: f.help ? String(f.help) : "",
      tooltip: f.tooltip ? String(f.tooltip) : "",
      min: num(f.min), max: num(f.max), step: f.step === undefined ? null : f.step,
      rows: type === "table" ? Number(f.height) || 0 : Number(f.rows) || 0,
      pattern: f.pattern === undefined || f.pattern === null || f.pattern === "" ? null
        : f.pattern instanceof RegExp ? f.pattern : new RegExp("^(?:" + f.pattern + ")$"),
      patternMessage: f.patternMessage ? String(f.patternMessage) : "",
      remember: f.remember !== false && type !== "password" && type !== "table",
      validate: typeof f.validate === "function" ? f.validate : null,
      onChange: typeof f.onChange === "function" ? f.onChange : null,
      default: f.default,
    };
  }

  const ymd = (d) => `${String(d.getFullYear()).padStart(4, "0")}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;

  function toWire(f, v) {
    v = plainDate(v);
    if (f.type === "checkbox") return !!v;
    if (f.type === "table") {
      // Selected rows: the rows themselves or their indices.
      const n = f.table.data.length;
      const idx = [v].flat().map((x) => (typeof x === "number" ? x : f.table.data.indexOf(x)))
        .filter((x) => Number.isInteger(x) && x >= 0 && x < n);
      return f.table.select === "single" ? idx.slice(0, 1) : Array.from(new Set(idx));
    }
    if (f.type === "files") return [v].flat().filter((x) => x !== null && x !== undefined && x !== "").map((x) => toPath(x));
    // A select with a placeholder starts on it (""), otherwise on its first option.
    if (v === null || v === undefined) return f.type === "select" && !f.placeholder ? f.options[0].value : "";
    switch (f.type) {
      case "file": case "folder": case "saveFile": return toPath(v);
      case "date": return v instanceof Date ? (isNaN(v) ? "" : ymd(v)) : String(v);
      case "month": return v instanceof Date ? (isNaN(v) ? "" : ymd(v).slice(0, 7)) : String(v);
      case "time": return v instanceof Date ? (isNaN(v) ? "" : `${pad2(v.getHours())}:${pad2(v.getMinutes())}`) : String(v);
    }
    return String(v);
  }

  const wireStr = (w) => (w === null || w === undefined ? "" : String(w));
  const wireList = (w) => (Array.isArray(w) ? w.map(wireStr) : wireStr(w).split(/[;\r\n]+/))
    .map((s) => s.trim().replace(/^"(.*)"$/, "$1")).filter(Boolean);

  function wireEmpty(f, w) {
    if (f.type === "checkbox") return w !== true && w !== "true";
    if (f.type === "files") return wireList(w).length === 0;
    if (f.type === "table") return !Array.isArray(w) || w.length === 0;
    return wireStr(w).trim() === "";
  }

  function fromWire(f, w) {
    switch (f.type) {
      case "checkbox": return w === true || w === "true";
      case "files": return wireList(w).map((p) => new FileSystemFileHandle(p));
      case "table": {
        if (!f.table.select) return f.table.data.slice(); // editable: all rows
        const rows = (Array.isArray(w) ? w : []).filter((i) => Number.isInteger(i) && i >= 0 && i < f.table.data.length)
          .map((i) => f.table.data[i]);
        return f.table.select === "single" ? (rows.length ? rows[0] : null) : rows;
      }
      case "file": case "saveFile": { const p = wireStr(w).trim(); return p ? new FileSystemFileHandle(p) : null; }
      case "folder": { const p = wireStr(w).trim(); return p ? new FileSystemDirectoryHandle(p) : null; }
      case "number": {
        // Full-width digits and thousands separators as typed in Japanese.
        const s = wireStr(w).trim().replace(/[０-９．，－＋]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xfee0))
          .replace(/^(?=[+-]?[\d,]+(\.\d*)?$)([+-]?)(\d{1,3}(,\d{3})+)/, (m) => m.replace(/,/g, ""));
        return s === "" ? null : Number(s);
      }
      case "date": {
        const m = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(wireStr(w).trim().replace(/\//g, "-"));
        if (!m) return null;
        const d = new Date(+m[1], +m[2] - 1, +m[3]);
        return d.getMonth() === +m[2] - 1 ? d : null;
      }
      case "month": case "time": { const s = wireStr(w).trim(); return s === "" ? null : s; }
    }
    return wireStr(w);
  }

  /** Message for a wire value of a field or an editable cell that does not pass its checks ("" if it does). */
  function checkValue(f, w, v) {
    if (wireEmpty(f, w)) return f.required ? T(f.type === "table" ? "form.selectRow" : "form.required") : "";
    let e = "";
    switch (f.type) {
      case "number":
        if (!isFinite(v)) e = T("form.notNumber");
        else if (f.min !== null && v < f.min) e = T("form.min", f.min);
        else if (f.max !== null && v > f.max) e = T("form.max", f.max);
        break;
      case "date": if (v === null) e = T("form.notDate"); break;
      case "month": if (!/^\d{4}-\d{2}$/.test(v)) e = T("form.notMonth"); break;
      case "time": if (!/^\d{1,2}:\d{2}(:\d{2})?$/.test(v)) e = T("form.notTime"); break;
      case "select": case "radio": if (!f.options.some((o) => o.value === v)) e = T("form.badOption"); break;
      case "file": if (!fsObj.isFile(v)) e = T("form.fileNotFound", v.path); break;
      case "files": { const bad = v.find((h) => !fsObj.isFile(h)); if (bad) e = T("form.fileNotFound", bad.path); break; }
      case "folder": if (!fsObj.isDirectory(v)) e = T("form.folderNotFound", v.path); break;
    }
    if (!e && f.pattern && typeof v === "string" && !f.pattern.test(v)) e = f.patternMessage || T("form.pattern");
    return e;
  }

  /**
   * Checks the wire values; returns { ok, errors: { name: message }, error,
   * values, cells: { name: errors of an editable table } }.
   */
  async function checkForm(fields, wire, validate) {
    const errors = {}, values = {}, cells = {};
    for (const f of fields) {
      const w = wire[f.name];
      if (f.type === "table" && !f.table.select && !f.table.edit) continue; // shown only
      values[f.name] = fromWire(f, w);
      if (f.type === "table" && f.table.edit) {
        cells[f.name] = await tableErrors(f.table);
        if (cells[f.name].length) {
          errors[f.name] = T("form.tableErrors", cells[f.name].length);
          continue;
        }
        if (!f.table.select) continue;
      }
      const e = checkValue(f, w, values[f.name]);
      if (e) errors[f.name] = e;
    }
    // The fields' own validate, with all the values.
    for (const f of fields) {
      if (f.validate && f.name in values && !errors[f.name]) {
        const e = await f.validate(values[f.name], values);
        if (e) errors[f.name] = String(e);
      }
    }
    let error = "";
    if (!Object.keys(errors).length && typeof validate === "function") {
      const r = await validate(values);
      if (typeof r === "string") error = r;
      else if (r && typeof r === "object") {
        for (const [k, m] of Object.entries(r)) {
          if (!m) continue;
          if (fields.some((f) => f.name === k)) errors[k] = String(m);
          else error = error ? error + "\n" + m : String(m);
        }
      }
    }
    return { ok: !error && !Object.keys(errors).length, errors, error, values, cells };
  }

  /**
   * Message for a field as entered, while the form is shown ("" if it
   * passes): its checks and its own validate, not the form's validate.
   */
  async function checkField(fields, f, wire) {
    if (f.type === "table" && !f.table.select) return ""; // cells are checked on their own
    const w = wire[f.name], v = fromWire(f, w);
    const e = checkValue(f, w, v) || (f.validate ? await f.validate(v, rawValues(fields, wire)) : "");
    return e ? String(e) : "";
  }

  /** Values of the fields as entered, without checking them (buttons with validate: false). */
  function rawValues(fields, wire) {
    const values = {};
    for (const f of fields) if (!(f.type === "table" && !f.table.select && !f.table.edit)) values[f.name] = fromWire(f, wire[f.name]);
    return values;
  }

  // Remembered values: %APPDATA%\xljs\<script name>.json, { "<form id or title>": { field: wire value } }.
  function settingsFile() {
    const dir = native.configDir();
    return dir ? native.pathJoin(dir, native.pathBasename(native.script, native.pathExtname(native.script)) + ".json") : "";
  }
  function readSettings(p) {
    try {
      const all = p && native.fsStat(p) ? JSON.parse(native.fsReadText(p, "utf8")) : null;
      return all && typeof all === "object" && !Array.isArray(all) ? all : {};
    } catch (e) {
      return {};
    }
  }
  function loadRemembered(key) {
    const v = readSettings(settingsFile())[key];
    return v && typeof v === "object" ? v : {};
  }
  function saveRemembered(key, fields, wire) {
    const p = settingsFile();
    if (!p) return;
    const all = readSettings(p);
    all[key] = {};
    for (const f of fields) if (f.remember) all[key][f.name] = wire[f.name];
    try {
      native.fsMkdir(native.pathDirname(p));
      native.fsWriteText(p, JSON.stringify(all, null, 2) + "\n", "utf8", false);
    } catch (e) {
      // Remembering is a convenience; the form still works without it.
    }
  }

  /** The ctx argument of onSubmit. */
  class FormContext {
    constructor(impl) { define(this, "_i", impl); }
    /** Whether the user asked to stop (or closed the window). */
    get cancelled() { return this._i.cancelled(); }
    /** progress(done, total?, message?) or progress(done, message) */
    progress(done, total, message) {
      if (typeof total === "string") { message = total; total = undefined; }
      this._i.progress(Math.max(0, Math.floor(Number(done) || 0)),
        total === undefined || total === null ? undefined : Math.max(0, Math.floor(Number(total) || 0)),
        message === undefined ? undefined : String(message));
    }
    log(...args) { this._i.log(formatArgs(args), "info"); }
    error(...args) { this._i.log(formatArgs(args), "error"); }
    /** Replaces the rows (and optionally the columns) of a table field; the selection is cleared. */
    table(name, rows, columns) {
      const f = this._i.fields.find((x) => x.name === String(name) && x.type === "table");
      if (!f) throw new TypeError(T("form.noTable", name));
      if (columns !== undefined) f.columns = columns;
      f.table = formTable(f.tableSpec, rows, f.columns);
      this._i.table(f);
    }
    /** Sets the value of a field (as its default would be given). */
    setValue(name, value) {
      const f = this._i.fields.find((x) => x.name === String(name));
      if (!f) throw new TypeError(T("form.noField", name));
      this._i.setValue(f, toWire(f, value));
    }
    /**
     * Like ui.withProgress, shown in the form. Cancelling ends onSubmit
     * (the form shows that it was aborted) unless onCancel: "break".
     */
    withProgress(iterable, options = {}) {
      return withProgress(iterable, options, (o) => {
        const ctx = this;
        return {
          get cancelled() { return ctx.cancelled; },
          update(done, message) { ctx.progress(done, o.total, message); },
          close() {},
        };
      }, () => { throw new FormCancel(); });
    }
  }

  /** Thrown by ctx.withProgress when the run is aborted; ends onSubmit quietly. */
  class FormCancel {}

  function formTexts() {
    return {
      ok: T("form.ok"), cancel: T("form.cancel"), close: T("form.close"), browse: T("form.browse"),
      clear: T("form.clear"), dropHere: T("form.dropHere"), dropFolder: T("form.dropFolder"), abort: T("form.abort"), aborting: T("form.aborting"),
      running: T("form.running"), done: T("form.done"), aborted: T("form.aborted"), failed: T("form.failed"),
      openFolder: T("form.openFolder"), noRows: T("table.empty"),
      cellError: T("form.cellError"), rowError: T("form.rowError"), moreErrors: T("form.moreErrors"),
    };
  }

  async function showForm(options) {
    if (!options || typeof options !== "object") throw new TypeError(T("form.expectedOptions"));
    // Fields above the tabs, then each tab's fields (tab: its index, or -1).
    const fields = Array.from(options.fields || []).map((f, i) => normField(f, i, -1));
    const tabIcons = [];
    const tabs = Array.from(options.tabs || []).map((t, k) => {
      if (!t || typeof t !== "object") throw new TypeError(T("form.badTab", k + 1));
      for (const f of Array.from(t.fields || [])) fields.push(normField(f, fields.length, k));
      tabIcons.push(faClass(t.icon));
      return t.label === undefined ? String(k + 1) : String(t.label);
    });
    const seen = new Set();
    for (const f of fields) {
      if (seen.has(f.name)) throw new TypeError(T("form.duplicateName", f.name));
      seen.add(f.name);
    }
    const key = String(options.id || options.title || "form");
    const remembered = options.remember ? loadRemembered(key) : {};
    const wire = {};
    for (const f of fields) wire[f.name] = f.remember && f.name in remembered ? remembered[f.name] : toWire(f, f.default);
    // Buttons beside the submit button; each runs its onClick like onSubmit.
    const buttons = Array.from(options.buttons || []).map((b, k) => {
      if (!b || typeof b !== "object" || typeof b.onClick !== "function") throw new TypeError(T("form.badButton", k + 1));
      return {
        label: b.label === undefined ? String(k + 1) : String(b.label), icon: faClass(b.icon),
        tooltip: b.tooltip ? String(b.tooltip) : "",
        onClick: b.onClick, validate: b.validate !== false, close: !!b.close,
      };
    });
    const o = {
      title: options.title === undefined ? title() : String(options.title),
      description: options.description ? String(options.description) : "",
      submit: options.submit ? String(options.submit) : "",
      noSubmit: options.submit === false && buttons.length > 0,
      icon: faClass(options.icon), submitIcon: faClass(options.submitIcon),
      key, tabs, tabIcons, buttons,
    };
    return native.supportsWindows ? formWindow(o, fields, wire, options) : formPrompt(o, fields, wire, options);
  }

  /**
   * The values for a press of the submit button (b = null) or a button:
   * checked unless the button has validate: false. Returns { values } or
   * { errors, error }.
   */
  async function pressValues(o, b, fields, wire, options) {
    if (b && !b.validate) return { values: rawValues(fields, wire) };
    const c = await checkForm(fields, wire, options.validate);
    if (!c.ok) return { errors: c.errors, error: c.error, cells: c.cells };
    if (options.remember) saveRemembered(o.key, fields, wire);
    return { values: c.values };
  }

  // Console and mock: ask until the values pass, then run onSubmit (or the
  // button's onClick) with a progress indicator. After a button without
  // close: true the form is asked again.
  async function formPrompt(o, fields, wire, options) {
    let errors = {}, error = "", result = null;
    for (;;) {
      const cells = {};
      for (const f of fields) if (f.table && f.table.edit) cells[f.name] = await tableErrors(f.table);
      const spec = {
        title: o.title, description: o.description, error,
        submit: o.noSubmit ? "" : o.submit || T("form.ok"), buttons: o.buttons.map((b) => b.label),
        fields: fields.map((f) => Object.assign({
          name: f.name, label: f.label, type: f.type, options: f.options, required: f.required,
          placeholder: f.placeholder, help: f.help, accept: f.accept, value: wire[f.name], error: errors[f.name] || "",
          tab: f.tab >= 0 ? o.tabs[f.tab] : "",
        }, f.table ? Object.assign({ select: f.table.select }, tableText(f.table)) : {},
        f.table && f.table.edit ? {
          editable: f.table.edit.filter(Boolean).map((cf) => ({ index: cf.col, type: cf.type, options: cf.options })),
          cellErrors: cells[f.name],
        } : {})),
      };
      const r = native.uiForm(JSON.stringify(spec));
      if (r === null) return result;
      const answer = JSON.parse(r);
      // "$button": the index or label of the button pressed (none: the submit button).
      let b = o.noSubmit ? o.buttons[0] : null;
      if ("$button" in answer) {
        const x = answer.$button;
        delete answer.$button;
        if (x !== null && x !== undefined) {
          b = typeof x === "number" ? o.buttons[x] : o.buttons.find((y) => y.label === String(x));
          if (!b) throw new TypeError(T("form.noButton", x));
        }
      }
      // "$cells": edited cells, { field: [[row, column (index, key or label), value], ...] }.
      if ("$cells" in answer) {
        for (const [name, list] of Object.entries(answer.$cells || {})) {
          const t = fields.find((f) => f.name === name && f.table && f.table.edit);
          if (!t) throw new TypeError(T("form.noTable", name));
          for (const [row, col, value] of Array.from(list || [])) {
            const c = typeof col === "number" ? col : t.table.cols.findIndex((x) => String(x.key) === String(col) || x.label === String(col));
            setCell(t.table, row, c, value);
          }
        }
        delete answer.$cells;
      }
      // The answered values in field order; one that differs from what was
      // shown is a change by the user and runs the field's onChange (once
      // it passes its checks), whose setValue may fill later fields.
      const shown = Object.assign({}, wire);
      for (const f of fields) {
        if (!(f.name in answer) || JSON.stringify(answer[f.name]) === JSON.stringify(shown[f.name])) continue;
        wire[f.name] = answer[f.name];
        if (f.onChange && !(await checkField(fields, f, wire))) {
          const ctx = promptContext(o, fields, wire);
          try {
            await f.onChange(fromWire(f, wire[f.name]), rawValues(fields, wire), ctx);
          } finally {
            ctx._i.close();
          }
        }
      }
      const c = await pressValues(o, b, fields, wire, options);
      errors = c.errors || {};
      error = c.error || "";
      if (!c.values) continue;
      if (!b || b.validate || b.close) result = c.values;
      const handler = b ? b.onClick : options.onSubmit;
      if (typeof handler === "function") {
        const ctx = promptContext(o, fields, wire);
        try {
          await handler(c.values, ctx);
        } catch (e) {
          if (!(e instanceof FormCancel)) throw e;
        } finally {
          ctx._i.close();
        }
      }
      if (!b || b.close) return result;
    }
  }

  /** onSubmit's (and onChange's) ctx for the console and mock: a progress indicator once it reports progress. */
  function promptContext(o, fields, wire) {
    let p = null;
    return new FormContext({
      fields,
      table(f) {
        native.uiTable(JSON.stringify(Object.assign({ title: f.label, select: "" }, tableText(f.table))));
      },
      setValue(f, w) { wire[f.name] = w; },
      progress(done, total, message) {
        if (!p) p = new Progress({ title: o.title, total: total || 0 });
        else if (total !== undefined) p.total = total;
        p.update(done, message);
      },
      log: (text, level) => (level === "error" ? err : out)(text),
      cancelled: () => (p ? p.cancelled : false),
      close: () => { if (p) p.close(); },
    });
  }

  function formSize(o, fields, hasRun, options) {
    let h = 100;
    if (o.icon) h += 44; // the heading
    if (o.description) h += 14 + 21 * Math.min(8, o.description.split("\n").reduce((n, l) => n + 1 + Math.floor(l.length / 40), 0));
    // Fields above the tabs, plus the tallest tab.
    const heights = o.tabs.map(() => 0);
    let common = 0;
    for (const f of fields) {
      let fh = 10 + (f.type === "textarea" ? 10 + 21 * (f.rows || 4) : f.type === "files" ? 10 + 21 * Math.max(3, f.rows || 3)
        : f.type === "table" ? 34 + 27 * tableLines(f)
          : f.type === "checkbox" || f.type === "radio" ? 28 : 32);
      if (f.help) fh += 20;
      if (f.tab >= 0) heights[f.tab] += fh;
      else common += fh;
    }
    h += common + (o.tabs.length ? 48 + Math.max(...heights) : 0);
    if (hasRun) h += 240;
    return {
      width: Number(options.width) || 600,
      height: Number(options.height) || Math.min(Math.max(h, 220), 760),
    };
  }

  /** Visible rows of a table field: height, or up to 8 (at least 3). */
  const tableLines = (f) => f.rows || Math.min(8, Math.max(3, f.table.data.length));

  // Native UI: the form page. Resolves with the last submitted values once the window is closed.
  function formWindow(o, fields, wire, options) {
    const byName = new Map(fields.map((f) => [f.name, f]));
    const onSubmit = typeof options.onSubmit === "function" ? options.onSubmit : null;
    const hasRun = !!onSubmit || o.buttons.length > 0;
    return new Promise((resolve, reject) => {
      let result = null, failed = null, running = false, outputs = [];
      // ctx.setValue calls: their count and, per field, the last one.
      const sets = { n: 0, at: new Map() };
      // Cell edits and field checks are handled in order, and before a press of a button.
      let editQueue = Promise.resolve();
      const page = openPage("form.html", Object.assign({ title: o.title }, formSize(o, fields, hasRun, options)),
        (m, files) => { handle(m, files).catch(fail); });
      const finish = () => (failed ? reject(failed) : resolve(result));
      function fail(e) {
        if (!failed) failed = e;
        page.n.close();
      }
      page.n.closed.then(() => { if (!running) finish(); });

      async function handle(m, files) {
        switch (m.type) {
          case "ready":
            page.send({
              type: "init", title: o.title, icon: o.icon, description: o.description, texts: formTexts(),
              submit: o.submit, submitIcon: o.submitIcon, tabs: o.tabs, tabIcons: o.tabIcons,
              noSubmit: o.noSubmit, buttons: o.buttons.map((b) => b.label), buttonIcons: o.buttons.map((b) => b.icon),
              buttonTooltips: o.buttons.map((b) => b.tooltip),
              fields: fields.map((f) => ({
                name: f.name, label: f.label, icon: f.icon, type: f.type, options: f.options, required: f.required,
                placeholder: f.placeholder, help: f.help, tooltip: f.tooltip, change: !!f.onChange,
                accept: f.accept, min: f.min, max: f.max, step: f.step,
                rows: f.type === "table" ? tableLines(f) : f.rows, value: wire[f.name], tab: f.tab,
                table: f.table ? Object.assign({ select: f.table.select, editable: editWire(f.table) || undefined }, tableWire(f.table)) : undefined,
              })),
            });
            for (const f of fields) {
              if (f.table && f.table.edit) page.send({ type: "cellErrors", field: f.name, errors: await tableErrors(f.table) });
            }
            break;
          case "check": {
            // A field was changed (change: by the user) or is being typed in: its error, or "" to clear it.
            // A changed field that passes its checks runs its onChange.
            const f = byName.get(m.field);
            if (!f || running) return;
            editQueue = editQueue.then(async () => {
              takeValues(m);
              const error = await checkField(fields, f, wire);
              if (page.n.isClosed() || running) return;
              page.send({ type: "fieldError", field: f.name, error });
              if (!error && m.change && f.onChange) await changed(f);
            });
            await editQueue;
            break;
          }
          case "cell": {
            const f = byName.get(m.field);
            if (!f || !f.table || !f.table.edit || running) return;
            const t = f.table;
            editQueue = editQueue.then(async () => {
              if (t !== f.table || !setCell(t, m.row, m.col, m.value)) return;
              const errors = await rowErrors(t, m.row);
              if (!page.n.isClosed()) page.send({ type: "cellErrors", field: f.name, row: m.row, errors });
            });
            await editQueue;
            break;
          }
          case "pick": {
            const f = byName.get(m.field);
            const v = f ? await pickPath(f, m.value) : null;
            if (v !== null) page.send({ type: "picked", field: f.name, value: v, append: f.type === "files" });
            break;
          }
          case "drop": {
            const f = byName.get(m.field);
            const v = f ? droppedPath(f, files || [], typeof m.value === "string" ? m.value : "") : null;
            if (v !== null) page.send({ type: "picked", field: f.name, value: v, append: f.type === "files" });
            break;
          }
          case "submit": {
            // button: the index of the button pressed (none: the submit button).
            const b = typeof m.button === "number" ? o.buttons[m.button] : null;
            if (b === undefined || (!b && o.noSubmit) || running) return;
            await editQueue;
            takeValues(m);
            const c = await pressValues(o, b, fields, wire, options);
            if (page.n.isClosed()) return;
            if (!c.values) {
              page.send({ type: "errors", errors: c.errors, error: c.error });
              for (const [name, errors] of Object.entries(c.cells || {})) page.send({ type: "cellErrors", field: name, errors });
              return;
            }
            if (!b || b.validate || b.close) result = c.values;
            const handler = b ? b.onClick : onSubmit;
            const ok = handler ? await run(handler, c.values) : true;
            // Without onSubmit the submit button closes the form; a button with close: true does once it succeeds.
            if (b ? b.close && ok : !handler) page.n.close();
            break;
          }
          case "open":
            if (outputs.length) shellObj.showItemInFolder(outputs);
            break;
          case "close":
            page.n.close();
            break;
        }
      }

      /**
       * Takes the values of a message from the page, except those the
       * script set (ctx.setValue) after the page sent them: m.seq is the
       * last setValue the page had applied (none: take them all).
       */
      function takeValues(m) {
        if (!m.values) return;
        const stale = (f) => typeof m.seq === "number" && (sets.at.get(f.name) || 0) > m.seq;
        for (const f of fields) if (f.name in m.values && !stale(f)) wire[f.name] = m.values[f.name];
      }

      /** Runs a field's onChange; an error it throws is shown as the field's error. */
      async function changed(f) {
        const ctx = windowContext(page, fields, wire, sets);
        try {
          await f.onChange(fromWire(f, wire[f.name]), rawValues(fields, wire), ctx);
        } catch (e) {
          if (!(e instanceof FormCancel) && !page.n.isClosed()) {
            page.send({ type: "fieldError", field: f.name, error: errorText(e).split("\n")[0] });
          }
        }
        ctx._i.flush();
      }

      /** Runs onSubmit or a button's onClick; true if it ended without an error or being stopped. */
      async function run(handler, values) {
        running = true;
        page.n.clearCancel();
        page.send({ type: "running" });
        const ctx = windowContext(page, fields, wire, sets);
        let done;
        try {
          outputs = outputsOf(await handler(values, ctx));
          done = { type: "done", ok: true, cancelled: ctx.cancelled, output: outputs.length > 0 };
        } catch (e) {
          done = e instanceof FormCancel ? { type: "done", ok: true, cancelled: true, output: false }
            : { type: "done", ok: false, error: errorText(e) };
        }
        ctx._i.flush();
        running = false;
        if (page.n.isClosed()) finish();
        else page.send(done);
        return done.ok && !done.cancelled;
      }
    });
  }

  /**
   * onSubmit's (and onChange's) ctx for a form window: updates are sent at
   * most every 50 ms. sets counts the setValue calls (see takeValues).
   */
  function windowContext(page, fields, wire, sets) {
    const state = { done: 0, total: 0, message: "" };
    let progressDirty = false, lines = [], level = "info", last = 0, timer = 0;
    const flush = () => {
      if (timer) { clearTimeout(timer); timer = 0; }
      if (lines.length) { page.send({ type: "log", lines, level }); lines = []; }
      if (progressDirty) { page.send(Object.assign({ type: "progress" }, state)); progressDirty = false; }
      last = Date.now();
    };
    const soon = () => {
      if (Date.now() - last >= 50) flush();
      else if (!timer) timer = setTimeout(flush, 50);
    };
    return new FormContext({
      fields,
      table(f) {
        flush();
        page.send(Object.assign({ type: "table", field: f.name, select: f.table.select, editable: editWire(f.table) || undefined }, tableWire(f.table)));
        if (f.table.edit) {
          const t = f.table;
          tableErrors(t).then((errors) => {
            if (t === f.table && !page.n.isClosed()) page.send({ type: "cellErrors", field: f.name, errors });
          }, (e) => page.send({ type: "log", lines: [errorText(e)], level: "error" }));
        }
      },
      setValue(f, w) {
        flush();
        wire[f.name] = w;
        sets.at.set(f.name, ++sets.n);
        // quiet: checked on the page, but not a change by the user (no onChange).
        page.send({ type: "picked", field: f.name, value: w, append: false, quiet: true, seq: sets.n });
      },
      progress(done, total, message) {
        state.done = done;
        if (total !== undefined) state.total = total;
        if (message !== undefined) state.message = message;
        progressDirty = true;
        soon();
      },
      log(text, lvl) {
        if (lines.length && lvl !== level) flush();
        level = lvl;
        lines.push(text);
        soon();
      },
      cancelled: () => page.n.cancelRequested() || page.n.isClosed(),
      flush,
    });
  }

  function existingDir(p) {
    if (!p) return undefined;
    const st = native.fsStat(p);
    if (st && st.isDirectory) return p;
    const d = native.pathDirname(native.pathResolve(p));
    return native.fsStat(d) ? d : undefined;
  }

  /** Shows the picker for a file or folder field; returns the path(s) or null. */
  async function pickPath(f, current) {
    const cur = Array.isArray(current) ? current[current.length - 1] : current ? String(current) : "";
    const types = f.accept.length ? [{ description: f.accept.map((a) => "*" + a).join(";"), accept: f.accept }] : undefined;
    const startIn = existingDir(cur);
    switch (f.type) {
      case "file": { const [h] = await showOpenFilePicker({ types, startIn }); return h ? h.path : null; }
      case "files": { const hs = await showOpenFilePicker({ types, startIn, multiple: true }); return hs.length ? hs.map((h) => h.path) : null; }
      case "folder": { const h = await showDirectoryPicker({ startIn: cur && fsObj.isDirectory(cur) ? cur : startIn }); return h ? h.path : null; }
      case "saveFile": {
        const h = await showSaveFilePicker({ types, startIn, suggestedName: cur ? native.pathBasename(cur, "") : "" });
        return h ? h.path : null;
      }
    }
    return null;
  }

  /**
   * Value of a file or folder field for dropped paths (null to ignore the
   * drop). A save field takes a dropped file, or a dropped folder with the
   * file name it has (current).
   */
  function droppedPath(f, files, current) {
    const ok = (p) => !f.accept.length || f.accept.includes(native.pathExtname(p).toLowerCase());
    switch (f.type) {
      case "folder": {
        if (!files.length) return null;
        return fsObj.isDirectory(files[0]) ? files[0] : native.pathDirname(files[0]);
      }
      case "file": { const p = files.find((x) => fsObj.isFile(x) && ok(x)); return p || null; }
      case "files": { const ps = files.filter((x) => fsObj.isFile(x) && ok(x)); return ps.length ? ps : null; }
      case "saveFile": {
        const p = files[0];
        if (!p) return null;
        if (!fsObj.isDirectory(p)) return ok(p) ? p : null;
        const name = current ? native.pathBasename(current, "") : "";
        return name ? native.pathJoin(p, name) : null;
      }
    }
    return null;
  }

  // ---- tables

  const isRecord = (r) => r !== null && typeof r === "object" && !(r instanceof Date) && !Array.isArray(r);

  /** Sets a cell of an object or array row (editable form tables). */
  const setter = (key) => (r, v) => { if (r !== null && typeof r === "object") r[key] = v; };

  /** Columns as [{ key, label, width, get(row), set(row, value) }] (key is null for a list of plain values). */
  function tableColumns(rows, columns) {
    const sample = rows.find((r) => r !== null && r !== undefined);
    const arrays = Array.isArray(sample);
    if (columns) {
      return Array.from(columns).map((c, i) => {
        const spec = c !== null && typeof c === "object" ? c : { key: c };
        let key = spec.key === undefined ? i : spec.key;
        const label = String(spec.label === undefined ? key : spec.label);
        if (arrays && typeof key !== "number") key = /^\d+$/.test(String(key)) && spec.label !== undefined ? Number(key) : i;
        return { key, label, width: Number(spec.width) || 0, get: (r) => (r === null || r === undefined ? undefined : r[key]), set: setter(key) };
      });
    }
    if (arrays) {
      const n = rows.reduce((m, r) => (Array.isArray(r) ? Math.max(m, r.length) : m), 0);
      return Array.from({ length: n }, (_, i) => ({ key: i, label: native.xlsxColumnName(i + 1), width: 0, get: (r) => (r ? r[i] : undefined), set: setter(i) }));
    }
    if (sample === undefined || isRecord(sample)) {
      return keysOf(rows.filter(isRecord)).map((k) => ({ key: k, label: k, width: 0, get: (r) => (isRecord(r) ? r[k] : undefined), set: setter(k) }));
    }
    return [{ key: null, label: T("table.value"), width: 0, get: (r) => r }];
  }

  /** A cell for the page: number, string, boolean or null. */
  function cellWire(v) {
    v = plainDate(v);
    if (v === null || v === undefined) return null;
    if (typeof v === "number") return isFinite(v) ? v : String(v);
    if (typeof v === "string" || typeof v === "boolean") return v;
    if (v instanceof Date) {
      if (isNaN(v)) return "";
      return v.getHours() || v.getMinutes() || v.getSeconds() || v.getMilliseconds() ? formatDateLocal(v) : ymd(v);
    }
    if (v instanceof FileSystemHandle) return v.path;
    return inspect(v, { depth: 0 });
  }

  /** A cell for CSV/Excel: keeps numbers, dates and booleans. */
  function cellSave(v) {
    if (v === undefined) return null;
    if (v === null || typeof v === "number" || typeof v === "string" || typeof v === "boolean" || v instanceof Date) return v;
    if (v instanceof FileSystemHandle) return v.path;
    return inspect(v, { depth: 0 });
  }

  /** What ui.table returns when nothing is selected: the window, closed and close(). */
  class TableView {
    constructor(closed, close) {
      define(this, "closed", closed);
      define(this, "_close", close);
    }
    close() { this._close(); }
  }

  function tableTexts() {
    return {
      filter: T("table.filter"), count: T("table.count"), countFiltered: T("table.countFiltered"),
      loading: T("table.loading"), selected: T("table.selected"), copy: T("table.copy"), copied: T("table.copied"),
      saveCsv: T("table.saveCsv"), saveXlsx: T("table.saveXlsx"), saved: T("table.saved"), ok: T("form.ok"),
      cancel: T("form.cancel"), close: T("form.close"), empty: T("table.empty"),
    };
  }

  function showTable(rows, options = {}) {
    if (typeof options === "string") options = { title: options };
    options = options || {};
    const data = Array.from(rows || []);
    const cols = tableColumns(data, options.columns);
    const sel = options.selectable === true ? "multiple" : options.selectable ? String(options.selectable) : "";
    if (sel && sel !== "single" && sel !== "multiple") throw new TypeError(T("table.badSelectable", sel));
    if (options.onDoubleClick !== undefined && typeof options.onDoubleClick !== "function") {
      throw new TypeError(T("table.badHandler", "onDoubleClick"));
    }
    const t = options.title === undefined ? title() : String(options.title);
    if (native.supportsWindows) return tableWindow(data, cols, sel, t, options);
    const text = (v) => { const w = cellWire(v); return w === null ? "" : String(w); };
    const picked = native.uiTable(JSON.stringify({
      title: t, select: sel, columns: cols.map((c) => c.label), rows: data.map((r) => cols.map((c) => text(c.get(r)))),
    }));
    if (!sel) return new TableView(Promise.resolve(), () => {});
    return Promise.resolve((picked || []).filter((i) => i >= 0 && i < data.length).map((i) => data[i]));
  }

  function tableWindow(data, cols, sel, t, options) {
    let picked = [];
    const page = openPage("table.html", { title: t, width: Number(options.width) || 960, height: Number(options.height) || 600 },
      (m) => { handle(m).catch((e) => page.send({ type: "error", message: errorText(e) })); });

    async function handle(m) {
      switch (m.type) {
        case "ready": {
          page.send({ type: "init", select: sel, total: data.length, texts: tableTexts(), dblclick: !!options.onDoubleClick, columns: cols.map((c) => ({ label: c.label, width: c.width })) });
          const chunk = 2000;
          for (let i = 0; i < data.length; i += chunk) {
            page.send({ type: "rows", rows: data.slice(i, i + chunk).map((r) => cols.map((c) => cellWire(c.get(r)))) });
          }
          break;
        }
        case "ok":
          if (sel) picked = Array.from(m.indices || []).filter((i) => i >= 0 && i < data.length).map((i) => data[i]);
          if (sel === "single") picked = picked.slice(0, 1);
          page.n.close();
          break;
        case "close":
          page.n.close();
          break;
        case "dblclick": {
          const i = Number(m.index);
          if (!options.onDoubleClick || !(i >= 0 && i < data.length)) break;
          const row = data[i], col = cols[m.column];
          await options.onDoubleClick({
            row, index: i,
            column: col ? col.label : null, key: col ? col.key : null, value: col ? col.get(row) : undefined,
            close: () => page.n.close(),
          });
          break;
        }
        case "save":
          await saveTable(m.format === "xlsx" ? "xlsx" : "csv", Array.isArray(m.indices) ? m.indices : null);
          break;
      }
    }

    async function saveTable(format, indices) {
      const ext = "." + format;
      const h = await showSaveFilePicker({
        suggestedName: t.replace(/[\\/:*?"<>|]/g, "_") + ext,
        types: [{ description: format === "xlsx" ? "Excel" : "CSV", accept: [ext] }],
      });
      if (!h) return;
      const list = indices ? indices.filter((i) => i >= 0 && i < data.length).map((i) => data[i]) : data;
      const labels = cols.map((c) => c.label);
      const matrix = list.map((r) => cols.map((c) => cellSave(c.get(r))));
      if (format === "xlsx") {
        const book = workbooksObj.create();
        try {
          const name = t.replace(/[[\]:*?/\\]/g, "_").slice(0, 31) || "Sheet1";
          book.addSheet(name).write(matrix, { columns: labels, table: true });
          book.save(h);
        } finally {
          book.close();
        }
      } else {
        csvObj.write(h, matrix, { columns: labels });
      }
      page.send({ type: "saved", path: h.path });
    }

    if (!sel) return new TableView(page.n.closed, () => page.n.close());
    return page.n.closed.then(() => picked);
  }

  // ---------------------------------------------------------------- csv

  /**
   * The sheet name that name refers to: the exact name, or else the only
   * name equal to it ignoring case (as Excel does), surrounding spaces and
   * full/half width ("明細 ", "ＤＡＴＡ" for "DATA"). null when none.
   */
  function sheetNameLike(names, name) {
    if (names.includes(name)) return name;
    const norm = (x) => x.normalize("NFKC").trim().toLowerCase();
    const want = norm(name);
    const found = names.filter((n) => norm(n) === want);
    return found.length === 1 ? found[0] : null;
  }

  function rowsFromRecords(records, opts) {
    const header = opts.header === undefined ? true : opts.header;
    let recs = records;
    if (opts.skipEmptyLines !== false) recs = recs.filter((r) => !(r.length === 0 || (r.length === 1 && r[0] === "")));
    if (opts.trim) recs = recs.map((r) => r.map((s) => s.trim()));
    // Values are typed by default (like Excel opening a CSV, but keeping
    // codes such as "00123" as text); cast: false keeps every value a string.
    const raw = opts.cast === false;
    const cast = raw ? (x) => x : castValue;
    if (!header) {
      if (opts.columns) return recs.map((r) => objectFrom(opts.columns, r, cast));
      return raw ? recs : recs.map((r) => r.map(cast));
    }
    if (recs.length === 0) return [];
    // Header names are trimmed: "金額 " would otherwise not be r.金額.
    const keys = uniqueKeys(recs[0].map((h) => h.trim()));
    return recs.slice(1).map((r) => objectFrom(keys, r, cast));
  }
  function uniqueKeys(names) {
    const used = new Map();
    return names.map((raw, i) => {
      let k = raw === null || raw === undefined || String(raw) === "" ? `列${i + 1}` : String(raw);
      if (used.has(k)) {
        let n = used.get(k) + 1;
        while (used.has(`${k}_${n}`)) n++;
        used.set(k, n);
        k = `${k}_${n}`;
      }
      used.set(k, 1);
      return k;
    });
  }
  function objectFrom(keys, values, cast) {
    const o = {};
    keys.forEach((k, i) => { o[k] = i < values.length ? cast(values[i]) : (cast === castValue ? null : ""); });
    return o;
  }
  /**
   * Types a CSV value: numbers (not codes with a leading zero, nor more than
   * 15 digits, which a number cannot hold exactly), dates "2026/4/1" or
   * "2026-04-01" with an optional time, booleans; "" becomes null.
   */
  function castValue(s) {
    if (typeof s !== "string") return s;
    const t = s.trim();
    if (t === "") return null;
    if ((/^-?(0|[1-9]\d*)(\.\d+)?$/.test(t) || /^-?(0|[1-9]\d{0,2}(,\d{3})+)(\.\d+)?$/.test(t)) && t.replace(/\D/g, "").length <= 15) {
      return Number(t.replace(/,/g, ""));
    }
    if (t === "true" || t === "TRUE") return true;
    if (t === "false" || t === "FALSE") return false;
    const m = /^(\d{4})([\/-])(\d{1,2})\2(\d{1,2})(?:[ T](\d{1,2}):(\d{2})(?::(\d{2}))?)?$/.exec(t);
    if (m) {
      const [y, mo, d, h, mi, sec] = [m[1], m[3], m[4], m[5] || 0, m[6] || 0, m[7] || 0].map(Number);
      const date = new Date(y, mo - 1, d, h, mi, sec);
      if (date.getFullYear() === y && date.getMonth() === mo - 1 && date.getDate() === d && h < 24 && mi < 60 && sec < 60) return date;
    }
    return s;
  }
  function cellToString(v) {
    v = plainDate(v);
    if (v === null || v === undefined) return "";
    if (v instanceof Date) {
      if (isNaN(v)) return "";
      const d = `${v.getFullYear()}/${pad2(v.getMonth() + 1)}/${pad2(v.getDate())}`;
      if (v.getHours() || v.getMinutes() || v.getSeconds()) return `${d} ${pad2(v.getHours())}:${pad2(v.getMinutes())}:${pad2(v.getSeconds())}`;
      return d;
    }
    if (typeof v === "object") return JSON.stringify(v);
    return String(v);
  }
  function recordsFromRows(rows, opts) {
    rows = Array.from(rows || []);
    if (rows.length === 0) return opts.columns && opts.header !== false ? [opts.columns.slice()] : [];
    if (Array.isArray(rows[0])) {
      const recs = rows.map((r) => r.map(cellToString));
      if (opts.columns && opts.header !== false) recs.unshift(opts.columns.map(String));
      return recs;
    }
    const cols = opts.columns || keysOf(rows);
    const recs = rows.map((r) => cols.map((c) => cellToString(r[c])));
    if (opts.header !== false) recs.unshift(cols.map(String));
    return recs;
  }
  function keysOf(rows) {
    const cols = [], seen = new Set();
    for (const r of rows) for (const k of Object.keys(r || {})) if (!seen.has(k)) { seen.add(k); cols.push(k); }
    return cols;
  }
  function delimiterFor(target, opts) {
    if (opts.delimiter) return opts.delimiter;
    if (typeof target === "string" || target instanceof FileSystemHandle || (target instanceof File && target.path)) {
      const p = typeof target === "string" ? target : target.path || target.name;
      if (/\.(tsv|tab)$/i.test(p)) return "\t";
    }
    return ",";
  }
  const csvObj = {
    /** Parses CSV text. opts: { header=true, delimiter=",", columns, cast=true, trim, skipEmptyLines=true, comment } */
    parse(text, opts = {}) {
      return rowsFromRecords(native.csvParse(String(text), opts.delimiter || ",", opts.comment || ""), opts);
    },
    /** Converts rows (objects or arrays) to CSV text. opts: { header=true, columns, delimiter, eol="\r\n", quoteAll } */
    stringify(rows, opts = {}) {
      const crlf = opts.eol === undefined ? true : opts.eol === "\r\n";
      return native.csvStringify(recordsFromRows(rows, opts), opts.delimiter || ",", crlf, !!opts.quoteAll);
    },
    /** Reads a CSV file (path, handle or File). opts: parse options + encoding="auto" */
    read(src, opts = {}) {
      let text;
      if (src instanceof Blob && !(src instanceof File && src.path)) text = native.decode(src._bytes(), opts.encoding || "auto");
      else text = native.fsReadText(toPath(src), opts.encoding || "auto");
      return rowsFromRecords(native.csvParse(text, delimiterFor(src, opts), opts.comment || ""), opts);
    },
    /** Writes rows to a CSV file. opts: stringify options + encoding="utf8bom" */
    write(dest, rows, opts = {}) {
      const crlf = opts.eol === undefined ? true : opts.eol === "\r\n";
      const text = native.csvStringify(recordsFromRows(rows, opts), delimiterFor(dest, opts), crlf, !!opts.quoteAll);
      native.fsWriteText(toPath(dest), text, opts.encoding || "utf8bom", !!opts.append);
    },
  };
  defineGlobal("csv", csvObj);

  // ---------------------------------------------------------------- workbooks

  const XLSX_MIME = MIME[".xlsx"];

  /**
   * Normalizes a style spec. Besides the full form
   * { font, fill, border, alignment, numberFormat } it accepts shorthands:
   * bold, italic, underline, strike, color, fontSize, fontName,
   * background (= fill), align, valign, wrap, format (= numberFormat).
   */
  function normStyle(s) {
    if (!s || typeof s !== "object") return {};
    const out = {};
    const font = Object.assign({}, s.font);
    for (const k of ["bold", "italic", "underline", "strike"]) if (s[k] !== undefined) font[k] = s[k];
    if (s.color !== undefined) font.color = s.color;
    if (s.fontSize !== undefined) font.size = s.fontSize;
    if (s.fontName !== undefined) font.name = s.fontName;
    if (Object.keys(font).length) out.font = font;
    const fill = s.fill !== undefined ? s.fill : s.background;
    if (fill !== undefined) out.fill = fill;
    const al = Object.assign({}, s.alignment);
    if (s.align !== undefined) al.horizontal = s.align;
    if (s.valign !== undefined) al.vertical = s.valign;
    if (s.wrap !== undefined) al.wrap = s.wrap;
    if (Object.keys(al).length) out.alignment = al;
    if (s.border !== undefined) out.border = s.border;
    const nf = s.numberFormat !== undefined ? s.numberFormat : s.format;
    if (nf !== undefined) out.numberFormat = nf;
    if (s.protection !== undefined) out.protection = s.protection;
    return out;
  }

  /** Style used by write({ table: true }). */
  const TABLE_STYLE = {
    header: { bold: true, background: "#DDEBF7", align: "center", valign: "center", border: { all: { style: "thin", color: "#8EA9DB" } } },
    body: { border: { all: { style: "thin", color: "#BFBFBF" } } },
  };

  class Range {
    constructor(sheet, address) {
      define(this, "_sheet", sheet);
      define(this, "_address", String(address).toUpperCase());
    }
    get address() { return this._address; }
    get sheet() { return this._sheet; }
    /** Applies (merges) a style; opts.replace discards the existing style first. */
    style(spec, opts = {}) {
      native.xlsxStyle(this._sheet._book._h, this._sheet._name, this._address, normStyle(spec), !!opts.replace);
      return this;
    }
    font(spec) { return this.style({ font: spec }); }
    fill(color) { return this.style({ fill: color }); }
    border(spec) { return this.style({ border: spec }); }
    align(horizontal, vertical) { return this.style({ alignment: { horizontal, vertical } }); }
    numberFormat(format) { return this.style({ numberFormat: format }); }
    merge() { native.xlsxMerge(this._sheet._book._h, this._sheet._name, this._address, false); return this; }
    unmerge() { native.xlsxMerge(this._sheet._book._h, this._sheet._name, this._address, true); return this; }
    /** Values of the range as a 2-D array; assigning writes from the top-left cell. */
    get values() {
      const [a, b] = this._address.split(":");
      const [c1, r1] = native.xlsxParseRef(a);
      const [c2, r2] = native.xlsxParseRef(b || a);
      const rows = this._sheet.toArrays();
      const out = [];
      for (let r = Math.min(r1, r2); r <= Math.max(r1, r2); r++) {
        const row = rows[r - 1] || [];
        const line = [];
        for (let c = Math.min(c1, c2); c <= Math.max(c1, c2); c++) line.push(c - 1 < row.length ? row[c - 1] : null);
        out.push(line);
      }
      return out;
    }
    set values(rows) {
      native.xlsxWriteRows(this._sheet._book._h, this._sheet._name, this._address.split(":")[0], rows.map((r) => Array.from(r, plainDate)));
    }
    [inspectSymbol]() { return `Range ${this._sheet._name}!${this._address}`; }
  }

  class Cell {
    constructor(sheet, address) {
      define(this, "_sheet", sheet);
      define(this, "_address", address.toUpperCase());
    }
    get address() { return this._address; }
    get row() { return native.xlsxParseRef(this._address)[1]; }
    get column() { return native.xlsxParseRef(this._address)[0]; }
    /** Typed value: number, string, boolean, Date or null. */
    get value() { return native.xlsxGet(this._sheet._book._h, this._sheet._name, this._address); }
    set value(v) { native.xlsxSet(this._sheet._book._h, this._sheet._name, this._address, plainDate(v)); }
    /** Displayed text (number format applied). */
    get text() { return native.xlsxGetText(this._sheet._book._h, this._sheet._name, this._address); }
    /** Formula with leading "=", or null. */
    get formula() { return native.xlsxGetFormula(this._sheet._book._h, this._sheet._name, this._address); }
    set formula(f) { native.xlsxSetFormula(this._sheet._book._h, this._sheet._name, this._address, f === null || f === undefined ? "" : String(f)); }
    /** Evaluates the formula with excelize's calculation engine (subset of Excel functions). */
    calc() { return native.xlsxCalc(this._sheet._book._h, this._sheet._name, this._address); }
    /** Applies (merges) a style to the cell. */
    style(spec, opts) { new Range(this._sheet, this._address).style(spec, opts); return this; }
    /** The cell's current style ({ font, fill, border, alignment, numberFormat }). */
    getStyle() { return native.xlsxGetStyle(this._sheet._book._h, this._sheet._name, this._address); }
    [inspectSymbol]() { return `Cell ${this._sheet._name}!${this._address} ${inspect(this.value)}`; }
  }

  function refOf(a, b) {
    if (typeof a === "number") return native.xlsxCellName(b, a); // (row, column)
    return String(a);
  }

  class Worksheet {
    constructor(book, name) {
      define(this, "_book", book);
      define(this, "_name", name);
    }
    get book() { return this._book; }
    get name() { return this._name; }
    set name(v) {
      native.xlsxRenameSheet(this._book._h, this._name, String(v));
      this._name = String(v);
    }
    /** cell("B2") or cell(row, column) (1-based). */
    cell(a, b) { return new Cell(this, refOf(a, b)); }
    getValue(a, b) { return native.xlsxGet(this._book._h, this._name, refOf(a, b)); }
    setValue(a, b, c) {
      if (typeof a === "number") native.xlsxSet(this._book._h, this._name, refOf(a, b), plainDate(c));
      else native.xlsxSet(this._book._h, this._name, String(a), plainDate(b));
      return this;
    }
    /** All used rows as arrays of typed values. opts: { text: true } returns displayed strings. */
    toArrays(opts = {}) {
      const rows = native.xlsxReadAll(this._book._h, this._name, !!opts.text);
      return opts.skipEmpty ? rows.filter((r) => r.some((v) => v !== null && v !== "")) : rows;
    }
    /**
     * Rows as objects keyed by the header row.
     * opts: { header = 1 (row number), columns, skipEmpty = true, text = false }
     */
    toObjects(opts = {}) {
      const rows = native.xlsxReadAll(this._book._h, this._name, !!opts.text);
      const headerRow = opts.header === undefined ? 1 : opts.header;
      let keys, body;
      if (opts.columns) {
        keys = opts.columns;
        body = headerRow ? rows.slice(headerRow) : rows;
      } else {
        const h = rows[headerRow - 1] || [];
        keys = uniqueKeys(h.map((v) => (v === null ? "" : v instanceof Date ? formatDateLocal(v) : String(v).trim())));
        body = rows.slice(headerRow);
      }
      if (opts.skipEmpty !== false) body = body.filter((r) => r.some((v) => v !== null && v !== ""));
      return body.map((r) => {
        const o = {};
        keys.forEach((k, i) => { o[k] = i < r.length ? r[i] : null; });
        return o;
      });
    }
    /** Alias of toObjects(). */
    rows(opts) { return this.toObjects(opts); }
    /**
     * Writes rows (array of objects or array of arrays) starting at opts.start ("A1").
     * Objects get a header row unless opts.header === false.
     */
    write(rows, opts = {}) {
      rows = Array.from(rows || []);
      const start = String(opts.start || "A1").toUpperCase();
      let data, cols = null, hasHeader = false;
      if (rows.length && !Array.isArray(rows[0])) {
        cols = opts.columns || keysOf(rows);
        data = rows.map((r) => cols.map((c) => r[c]));
        if (opts.header !== false) { data.unshift(cols); hasHeader = true; }
      } else {
        data = rows;
        if (opts.columns && opts.header !== false) { data = [opts.columns].concat(data); hasHeader = true; cols = opts.columns; }
      }
      native.xlsxWriteRows(this._book._h, this._name, start, data.map((r) => Array.from(r, plainDate)));
      this._formatWritten(start, data, cols, hasHeader, opts);
      return this;
    }
    /** Applies the formatting options of write() to the written block. */
    _formatWritten(start, data, cols, hasHeader, opts) {
      const preset = opts.table ? (typeof opts.table === "object" ? opts.table : TABLE_STYLE) : null;
      const headerStyle = opts.headerStyle || (preset && preset.header);
      const bodyStyle = opts.style || (preset && preset.body);
      const nCols = data.reduce((m, r) => Math.max(m, r.length), 0);
      if (data.length === 0 || nCols === 0) return;
      const [c1, r1] = native.xlsxParseRef(start);
      const c2 = c1 + nCols - 1, r2 = r1 + data.length - 1;
      const ref = (c, r) => native.xlsxCellName(c, r);
      const bodyTop = hasHeader ? r1 + 1 : r1;
      if (bodyStyle) this.style(`${ref(c1, r1)}:${ref(c2, r2)}`, bodyStyle);
      if (opts.columnStyles && bodyTop <= r2) {
        for (const [key, spec] of Object.entries(opts.columnStyles)) {
          let idx = cols ? cols.indexOf(key) : -1;
          if (idx < 0 && /^\d+$/.test(key)) idx = Number(key);
          if (idx < 0 && /^[A-Z]+$/i.test(key)) idx = native.xlsxParseRef(key + "1")[0] - c1;
          if (idx >= 0 && idx < nCols) this.style(`${ref(c1 + idx, bodyTop)}:${ref(c1 + idx, r2)}`, spec);
        }
      }
      if (hasHeader && headerStyle) this.style(`${ref(c1, r1)}:${ref(c2, r1)}`, headerStyle);
      const want = (k) => opts[k] !== undefined ? opts[k] : !!preset;
      if (want("autoFit")) this.autoFit(`${native.xlsxColumnName(c1)}:${native.xlsxColumnName(c2)}`);
      if (want("freeze") && hasHeader) this.freeze(ref(1, r1 + 1));
      if (want("filter") && hasHeader) this.autoFilter(`${ref(c1, r1)}:${ref(c2, r2)}`);
    }
    /** Appends rows below the last used row (objects are written without a header). */
    append(rows, opts = {}) {
      const next = native.xlsxRowCount(this._book._h, this._name) + 1;
      const header = opts.header !== undefined ? opts.header : next === 1;
      return this.write(rows, Object.assign({}, opts, { start: "A" + next, header }));
    }
    /** Used range, e.g. "A1:D20". */
    get dimension() { return native.xlsxDimension(this._book._h, this._name); }
    get rowCount() { return native.xlsxRowCount(this._book._h, this._name); }
    /** setColumnWidth("A", 20) or setColumnWidth("A:C", 20) */
    setColumnWidth(cols, width) {
      const [a, b] = String(cols).split(":");
      native.xlsxSetColWidth(this._book._h, this._name, a, b || a, width);
      return this;
    }
    getColumnWidth(col) { return native.xlsxGetColWidth(this._book._h, this._name, String(col)); }
    setRowHeight(row, height) {
      native.xlsxSetRowHeight(this._book._h, this._name, row, height);
      return this;
    }
    /** Sets column widths from the cell contents. cols: "A:D" (default: all used columns). */
    autoFit(cols, opts = {}) {
      if (typeof cols === "object" && cols !== null) { opts = cols; cols = undefined; }
      let c1 = 1, c2 = 16384;
      if (cols) {
        const [a, b] = String(cols).toUpperCase().split(":");
        c1 = native.xlsxParseRef(a + "1")[0];
        c2 = native.xlsxParseRef((b || a) + "1")[0];
      }
      native.xlsxAutoFit(this._book._h, this._name, c1, c2, opts.min !== undefined ? opts.min : 6, opts.max !== undefined ? opts.max : 60);
      return this;
    }
    /** range("A1:D10") */
    range(address) { return new Range(this, address); }
    /** style("A1:D1", { bold: true, background: "#DDEBF7", border: "thin" }) */
    style(address, spec, opts) { new Range(this, address).style(spec, opts); return this; }
    merge(address) { new Range(this, address).merge(); return this; }
    unmerge(address) { new Range(this, address).unmerge(); return this; }
    /** Freezes the rows above and columns left of the cell: freeze("A2") freezes row 1. */
    freeze(cell) {
      if (cell && typeof cell === "object") cell = native.xlsxCellName((cell.columns || 0) + 1, (cell.rows || 0) + 1);
      native.xlsxFreeze(this._book._h, this._name, cell ? String(cell) : "");
      return this;
    }
    unfreeze() { native.xlsxFreeze(this._book._h, this._name, ""); return this; }
    /** Adds filter buttons to the header row of the range (default: the used range). */
    autoFilter(address) {
      native.xlsxAutoFilter(this._book._h, this._name, address || this.dimension);
      return this;
    }
    [inspectSymbol]() { return `Worksheet ${JSON.stringify(this._name)}`; }
  }

  class Workbook {
    constructor(h) { define(this, "_h", h); }
    /** Path the workbook was opened from or last saved to (null for new workbooks). */
    get path() { return native.xlsxPath(this._h); }
    get sheetNames() { return native.xlsxSheetNames(this._h); }
    /** Array of worksheets; sheets also accessible by name: book.sheets["売上"]. */
    /** Sheets by position, and by name (book.sheets["明細"], matched like sheet(name)). */
    get sheets() {
      const names = native.xlsxSheetNames(this._h);
      const arr = names.map((n) => new Worksheet(this, n));
      names.forEach((n, i) => { if (!(n in arr)) define(arr, n, arr[i]); });
      return new Proxy(arr, {
        get: (target, key, receiver) => {
          if (typeof key !== "string" || key in target) return Reflect.get(target, key, receiver);
          const n = sheetNameLike(names, key);
          return n === null ? undefined : target[names.indexOf(n)];
        },
      });
    }
    /** sheet("名前") or sheet(0); null when missing. */
    sheet(nameOrIndex) {
      const names = native.xlsxSheetNames(this._h);
      const n = typeof nameOrIndex === "number" ? names[nameOrIndex] : sheetNameLike(names, String(nameOrIndex));
      return n === undefined || n === null ? null : new Worksheet(this, n);
    }
    addSheet(name) {
      native.xlsxAddSheet(this._h, String(name));
      return new Worksheet(this, String(name));
    }
    removeSheet(sheet) {
      native.xlsxRemoveSheet(this._h, sheet instanceof Worksheet ? sheet.name : String(sheet));
    }
    /** Saves to dest (path or handle), or to the original path. */
    save(dest) {
      native.xlsxSave(this._h, dest === undefined || dest === null ? "" : toPath(dest, "save()"));
      return this;
    }
    toArrayBuffer() { return native.xlsxToBytes(this._h); }
    toBlob() { return new Blob([native.xlsxToBytes(this._h)], { type: XLSX_MIME }); }
    /** Default font of the workbook: { name, size } */
    get defaultFont() { return native.xlsxDefaultFont(this._h); }
    close() { native.xlsxClose(this._h); }
    [inspectSymbol]() { return `Workbook { path: ${JSON.stringify(this.path)}, sheets: ${inspect(this.sheetNames)} }`; }
  }

  const workbooksObj = {
    /** Opens an .xlsx from a path, handle, File or Blob. */
    open(src) {
      if (src instanceof Blob && !(src instanceof File && src.path)) return new Workbook(native.xlsxOpenBytes(src._bytes(), src.name || ""));
      if (src instanceof ArrayBuffer || ArrayBuffer.isView(src)) return new Workbook(native.xlsxOpenBytes(src, ""));
      return new Workbook(native.xlsxOpen(toPath(src, "workbooks.open()")));
    },
    /**
     * Reads the VBA code of a macro workbook (.xlsm, .xlsb, .xlam ...)
     * without opening it: { name, codePage, modules, text }, or null when it
     * has no macros. text is all modules as Markdown, for an AI chat.
     */
    readVBA(src) {
      if (src instanceof Blob && !(src instanceof File && src.path)) return native.vbaReadBytes(src._bytes(), src.name || "");
      if (src instanceof ArrayBuffer || ArrayBuffer.isView(src)) return native.vbaReadBytes(src, "");
      return native.vbaRead(toPath(src, "workbooks.readVBA()"));
    },
    /** Creates an empty workbook. The first addSheet() replaces the default sheet. */
    create(options) { return new Workbook(native.xlsxCreate(options)); },
    Workbook,
    Worksheet,
    Cell,
    Range,
    /** Converts a column number to letters (1 -> "A"). */
    columnName: (n) => native.xlsxColumnName(n),
    /** Builds a cell reference from (row, column), 1-based. */
    cellName: (row, column) => native.xlsxCellName(column, row),
  };
  defineGlobal("workbooks", workbooksObj);

  // ---------------------------------------------------------------- dayjs

  /**
   * Day.js with all bundled plugins, the locale of the message language
   * (English when Day.js has no bundled locale for it; dayjs.locale(name)
   * switches), console output like Dates, and dayjs.fromExcel(serial).
   */
  function setupDayjs(bundle) {
    const d = bundle.dayjs;
    for (const p of bundle.plugins) d.extend(p);
    d.locale(bundle.locales.includes(native.lang) ? native.lang : "en");
    define(d.prototype, inspectSymbol, function () {
      return `dayjs(${this.isValid() ? formatDateLocal(this.toDate(), true) : "Invalid Date"})`;
    });
    /** Excel's date serial number (1 = 1900/1/1, with its 1900 leap day bug) to a local date. */
    d.fromExcel = (serial) => {
      const n = Number(serial);
      if (!isFinite(n)) return d(NaN);
      const days = Math.floor(n), ms = Math.round((n - days) * 86400000);
      const base = days >= 60 ? new Date(1899, 11, 30) : new Date(1899, 11, 31);
      return d(base).add(days, "day").add(ms, "ms");
    };
    return d;
  }
  dayjsLib = setupDayjs(/*@dayjs@*/null);
  defineGlobal("dayjs", dayjsLib);
});
