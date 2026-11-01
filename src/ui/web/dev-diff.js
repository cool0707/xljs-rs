"use strict";
// Line differences for the development mode (dev.js): Myers' algorithm on
// the lines between the common start and end.
const devDiff = (() => {
  /**
   * The edit script from a to b (arrays of lines): [{ op: "=", "-", "+", a, b }]
   * where a and b are the 0-based line numbers in a and b (-1 if none).
   * Above maxCost differences it gives up on finding the shortest script and
   * replaces the middle as a whole.
   */
  function lines(a, b, maxCost = 4000) {
    let start = 0;
    while (start < a.length && start < b.length && a[start] === b[start]) start++;
    let endA = a.length, endB = b.length;
    while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) { endA--; endB--; }
    const out = [];
    for (let i = 0; i < start; i++) out.push({ op: "=", a: i, b: i });
    const mid = myers(a.slice(start, endA), b.slice(start, endB), maxCost);
    for (const e of mid) out.push({ op: e.op, a: e.a < 0 ? -1 : e.a + start, b: e.b < 0 ? -1 : e.b + start });
    for (let i = 0; i < a.length - endA; i++) out.push({ op: "=", a: endA + i, b: endB + i });
    return out;
  }

  function myers(a, b, maxCost) {
    const n = a.length, m = b.length, max = n + m;
    if (n === 0 || m === 0 || max === 0) return replaceAll(n, m);
    const off = max;
    let v = new Int32Array(2 * max + 2);
    const trace = [];
    for (let d = 0; d <= max; d++) {
      if (d > maxCost) return replaceAll(n, m);
      // Only the diagonals -d..d are kept for the way back.
      trace.push(v.slice(off - d, off + d + 2));
      for (let k = -d; k <= d; k += 2) {
        let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
        let y = x - k;
        while (x < n && y < m && a[x] === b[y]) { x++; y++; }
        v[off + k] = x;
        if (x >= n && y >= m) return backtrack(trace, a, b, d);
      }
    }
    return replaceAll(n, m);
  }

  function backtrack(trace, a, b, dEnd) {
    const out = [];
    let x = a.length, y = b.length;
    for (let d = dEnd; d > 0; d--) {
      const t = trace[d]; // v before step d, for diagonals -d..d (index k + d)
      const at = (k) => t[k + d];
      const k = x - y;
      const prevK = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
      const prevX = at(prevK), prevY = prevX - prevK;
      while (x > prevX && y > prevY) { x--; y--; out.push({ op: "=", a: x, b: y }); }
      if (x === prevX) { y--; out.push({ op: "+", a: -1, b: y }); }
      else { x--; out.push({ op: "-", a: x, b: -1 }); }
      x = prevX; y = prevY;
    }
    while (x > 0 && y > 0) { x--; y--; out.push({ op: "=", a: x, b: y }); }
    return out.reverse();
  }

  function replaceAll(n, m) {
    const out = [];
    for (let i = 0; i < n; i++) out.push({ op: "-", a: i, b: -1 });
    for (let j = 0; j < m; j++) out.push({ op: "+", a: -1, b: j });
    return out;
  }

  /** Splits text into lines (\r\n or \n; no last empty line). */
  function split(text) {
    const l = String(text || "").replace(/\r\n?/g, "\n").split("\n");
    if (l.length && l[l.length - 1] === "") l.pop();
    return l;
  }

  /**
   * Groups an edit script into hunks with context lines around the changes:
   * [{ skip: count } | { op, a, b }] (skip = unchanged lines left out).
   */
  function hunks(edits, context = 3) {
    const keep = new Uint8Array(edits.length);
    edits.forEach((e, i) => {
      if (e.op === "=") return;
      for (let j = Math.max(0, i - context); j <= Math.min(edits.length - 1, i + context); j++) keep[j] = 1;
    });
    const out = [];
    let skipped = 0;
    edits.forEach((e, i) => {
      if (keep[i]) {
        if (skipped) out.push({ skip: skipped });
        skipped = 0;
        out.push(e);
      } else skipped++;
    });
    if (skipped) out.push({ skip: skipped });
    return out;
  }

  return { lines, split, hunks };
})();
if (typeof module !== "undefined") module.exports = devDiff;
