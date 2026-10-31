"use strict";
// A small Excel number format for the pages (chart axes, tooltips and
// labels): sections (positive;negative;zero), literals ("円", \x),
// currency ([$¥-411]), digits (0 # ?), thousands separators and scaling,
// decimals, percent and dates. What it does not understand gives null
// (the chart's own formatting). The script's side is src/api/numfmt.rs.
(() => {
  const DATE = /(^|[^"\\])(yy|m|d|h|s|aaa|ddd)/i;

  /** The sections of a format, quotes kept. */
  function sections(fmt) {
    const out = [];
    let cur = "", q = false;
    for (let i = 0; i < fmt.length; i++) {
      const c = fmt[i];
      if (c === '"') q = !q;
      if (c === "\\" && !q) { cur += c + (fmt[++i] || ""); continue; }
      if (c === ";" && !q) { out.push(cur); cur = ""; continue; }
      cur += c;
    }
    out.push(cur);
    return out;
  }

  /** Splits a section into tokens: { lit } for literal text, { code } for the rest. */
  function tokens(s) {
    const out = [];
    const lit = (t) => { if (t) out.push({ lit: t }); };
    let code = "";
    const flush = () => { if (code) out.push({ code }); code = ""; };
    for (let i = 0; i < s.length; i++) {
      const c = s[i];
      if (c === '"') {
        const j = s.indexOf('"', i + 1);
        flush(); lit(s.slice(i + 1, j < 0 ? s.length : j)); i = j < 0 ? s.length : j;
      } else if (c === "\\") { flush(); lit(s[++i] || ""); }
      else if (c === "_") { flush(); lit(" "); i++; }
      else if (c === "*") { i++; }
      else if (c === "[") {
        const j = s.indexOf("]", i);
        const inner = s.slice(i + 1, j < 0 ? s.length : j);
        i = j < 0 ? s.length : j;
        const cur = /^\$([^-]*)/.exec(inner);
        if (cur) { flush(); lit(cur[1]); }
        else if (/^h+$|^m+$|^s+$/i.test(inner)) code += inner; // elapsed time: shown as the plain unit
      } else if (/[0#?.,%]/.test(c) || (/[Ee]/.test(c) && /^[+-]/.test(s.slice(i + 1))) || (/[+-]/.test(c) && /[Ee]$/.test(code))) code += c;
      else if (/[ymdhsaAP/:]/i.test(c)) code += c;
      else { flush(); lit(c); }
    }
    flush();
    return out;
  }

  function number(code, v) {
    const pct = (code.match(/%/g) || []).length;
    v *= Math.pow(100, pct);
    const m = /([0#?,]*)(\.([0#?]*))?/.exec(code.replace(/%/g, "").replace(/^[^0#?.,]*/, ""));
    let int = m[1] || "", frac = m[3] || "";
    // Commas after the last digit scale by 1000 each.
    const tail = /,+$/.exec(int);
    if (tail && !m[2]) { v /= Math.pow(1000, tail[0].length); int = int.slice(0, -tail[0].length); }
    const group = int.includes(",");
    const minInt = (int.match(/0/g) || []).length;
    const minFrac = (frac.match(/0/g) || []).length, maxFrac = frac.length;
    const exp = /E[+-](0*)/i.exec(code);
    if (exp) {
      const [mant, e] = v.toExponential(maxFrac).split("e");
      const n = Number(e);
      return mant + "E" + (n < 0 ? "-" : "+") + String(Math.abs(n)).padStart(exp[1].length, "0");
    }
    let s = Math.abs(v).toFixed(maxFrac);
    let [ip, fp = ""] = s.split(".");
    while (fp.length > minFrac && fp.endsWith("0")) fp = fp.slice(0, -1);
    ip = ip.padStart(minInt, "0");
    if (group) ip = ip.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    return (v < 0 ? "-" : "") + (ip || "0") + (fp ? "." + fp : "") + "%".repeat(pct);
  }

  const pad = (n, k) => String(n).padStart(k, "0");

  const DATE_PART = /yyyy|yy|mmmmm|mmmm|mmm|mm|m|dddd|ddd|dd|d|aaaa|aaa|hh|h|ss|s|AM\/PM|A\/P|./gi;

  /** A date in the tokens of a section (literals kept in place). */
  function date(toks, d, lang) {
    const parts = [];
    for (const t of toks) {
      if (t.lit !== undefined) parts.push({ lit: t.lit });
      else for (const p of t.code.match(DATE_PART) || []) parts.push(/[a-z]/i.test(p) ? { p } : { lit: p });
    }
    const codes = parts.filter((x) => x.p);
    const hasAmPm = codes.some((x) => /^(AM\/PM|A\/P)$/i.test(x.p));
    const h = d.getHours();
    const intl = (o, l) => new Intl.DateTimeFormat(l || lang, o).format(d);
    return parts.map((x) => {
      if (!x.p) return x.lit;
      const k = codes.indexOf(x);
      const prev = k > 0 ? codes[k - 1].p : "", next = k + 1 < codes.length ? codes[k + 1].p : "";
      const minute = /^m{1,2}$/i.test(x.p) && (/^h/i.test(prev) || /^s/i.test(next));
      switch (x.p.toLowerCase()) {
        case "yyyy": return String(d.getFullYear());
        case "yy": return pad(d.getFullYear() % 100, 2);
        case "mmmmm": return intl({ month: "narrow" });
        case "mmmm": return intl({ month: "long" });
        case "mmm": return intl({ month: "short" });
        case "mm": return minute ? pad(d.getMinutes(), 2) : pad(d.getMonth() + 1, 2);
        case "m": return String(minute ? d.getMinutes() : d.getMonth() + 1);
        case "dddd": return intl({ weekday: "long" });
        case "ddd": return intl({ weekday: "short" });
        case "dd": return pad(d.getDate(), 2);
        case "d": return String(d.getDate());
        case "aaaa": return intl({ weekday: "long" }, "ja");
        case "aaa": return intl({ weekday: "short" }, "ja");
        case "hh": return pad(hasAmPm ? h % 12 || 12 : h, 2);
        case "h": return String(hasAmPm ? h % 12 || 12 : h);
        case "ss": return pad(d.getSeconds(), 2);
        case "s": return String(d.getSeconds());
        case "am/pm": return h < 12 ? "AM" : "PM";
        case "a/p": return h < 12 ? "A" : "P";
      }
      return x.p;
    }).join("");
  }

  /** Date of an Excel serial number (the 1900 system). */
  const fromSerial = (n) => {
    const days = Math.floor(n), ms = Math.round((n - days) * 86400000);
    const d = new Date(1899, 11, 30 + days);
    d.setMilliseconds(ms);
    return d;
  };

  /**
   * v formatted with the Excel format fmt, or null when the format is
   * empty, "General" or text ("@"), or v is not a number or date.
   */
  function format(fmt, v, lang) {
    if (!fmt || /^(general|@)$/i.test(fmt.trim())) return null;
    const isDate = v instanceof Date;
    if (!isDate && (typeof v !== "number" || !isFinite(v))) return null;
    const secs = sections(fmt);
    let s = secs[0], val = v;
    if (!isDate) {
      if (v < 0 && secs.length > 1 && secs[1] !== "") { s = secs[1]; val = -v; }
      else if (v === 0 && secs.length > 2 && secs[2] !== "") s = secs[2];
    }
    const toks = tokens(s);
    const code = toks.filter((t) => t.code).map((t) => t.code).join("");
    if (DATE.test(code.replace(/E[+-]/gi, ""))) {
      const d = isDate ? v : val >= 0 && val < 2958466 ? fromSerial(val) : null;
      return d && !isNaN(d) ? date(toks, d, lang || "ja") : null;
    }
    if (isDate) return null;
    if (!/[0#?]/.test(code)) return toks.map((t) => t.lit || "").join("") || null;
    // Literals before the first digit and after the last; those in between are dropped.
    const first = toks.findIndex((t) => t.code && /[0#?]/.test(t.code));
    const last = toks.length - 1 - [...toks].reverse().findIndex((t) => t.code && /[0#?]/.test(t.code));
    const lits = (a) => a.map((t) => t.lit || "").join("");
    return lits(toks.slice(0, first)) + number(code, val) + lits(toks.slice(last + 1));
  }

  window.xljsNumfmt = { format };
})();
