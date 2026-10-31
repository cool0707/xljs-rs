"use strict";
// Charts for the pages: one definition (a ChartSpec, see types/xljs.d.ts)
// drawn with AG Charts Community (bar, line, area, ...) or Frappe Gantt
// (type "gantt"). Used by chart.html (ui.chart), form.js (chart fields)
// and xljs.js (xljs.chart in the script's own pages). The functions that
// turn a spec into the libraries' options do not touch the DOM.
(() => {
  /** Office's default theme colors, as Excel draws a new chart. */
  const PALETTE = ["#4472C4", "#ED7D31", "#A5A5A5", "#FFC000", "#5B9BD5", "#70AD47",
    "#264478", "#9E480E", "#636363", "#997300", "#255E91", "#43682B"];
  const FONT = '"Yu Gothic UI", "Meiryo UI", "Segoe UI", system-ui, sans-serif';
  const CARTESIAN = ["column", "bar", "line", "area", "scatter", "bubble", "histogram"];
  const TYPES = CARTESIAN.concat(["pie", "donut", "gantt"]);

  const fill = (t, ...a) => String(t || "").replace(/\{(\d)\}/g, (_, i) => (a[i] === undefined ? "" : a[i]));
  const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v) && !(v instanceof Date);
  const pad = (n) => String(n).padStart(2, "0");

  class ChartError extends Error {
    constructor(code, message) { super(message); this.name = "ChartError"; this.code = code; }
  }

  /** The keys of the rows: object keys in the order first seen, or the positions of array rows. */
  function keysOf(rows) {
    const keys = [];
    for (const r of rows) {
      if (Array.isArray(r)) { for (let i = keys.length; i < r.length; i++) keys.push(i); }
      else if (isObj(r)) for (const k of Object.keys(r)) if (!keys.includes(k)) keys.push(k);
    }
    return keys;
  }
  const cell = (r, k) => (r === null || r === undefined || k === undefined || k === null ? undefined : r[k]);
  const hasKey = (keys, k) => keys.some((x) => String(x) === String(k));
  const keyOf = (keys, k) => keys.find((x) => String(x) === String(k));

  const GANTT_KEYS = {
    name: ["name", "名前", "タスク", "作業", "項目"],
    start: ["start", "開始", "開始日"],
    end: ["end", "終了", "終了日"],
    progress: ["progress", "進捗", "進捗率"],
    dependencies: ["dependencies", "前提", "依存"],
    id: ["id", "ID"],
  };

  /**
   * The spec with its defaults filled in: type, the keys of the series
   * (series: [{ key, label, type, axis, color, format, size }]) or of the
   * Gantt tasks. Throws a ChartError (code, message from texts) for what
   * cannot be drawn.
   */
  function resolve(spec, texts = {}) {
    if (!isObj(spec)) throw new ChartError("spec", texts.badSpec || "chart: expected an object");
    const type = spec.type === undefined ? "column" : String(spec.type);
    if (!TYPES.includes(type)) throw new ChartError("type", fill(texts.badType || "unknown chart type: {0}", type));
    const rows = Array.from(spec.data || []);
    const keys = keysOf(rows);
    const need = (k, what) => {
      if (k === undefined || k === null || !hasKey(keys, k)) {
        if (!rows.length && k !== undefined && k !== null) return k;
        throw new ChartError("key", fill(texts.noKey || "no column {0} ({1})", k === undefined || k === null ? "" : k, what));
      }
      return keyOf(keys, k);
    };
    if (type === "gantt") {
      const pick = (name) => {
        const v = spec[name];
        if (v !== undefined && v !== null) return need(v, name);
        const found = GANTT_KEYS[name].find((n) => hasKey(keys, n));
        return found === undefined ? undefined : keyOf(keys, found);
      };
      const g = { type, rows, name: pick("name"), start: pick("start"), end: pick("end"),
        progress: pick("progress"), dependencies: pick("dependencies"), id: pick("id") };
      if (g.name === undefined) g.name = keys[0];
      if (g.start === undefined && rows.length) throw new ChartError("key", fill(texts.noKey || "no column {0} ({1})", "", "start"));
      return g;
    }
    const x = spec.x !== undefined && spec.x !== null ? need(spec.x, "x") : keys[0];
    let ys = spec.y;
    if (ys === undefined || ys === null) {
      ys = keys.filter((k) => String(k) !== String(x) && rows.some((r) => typeof cell(r, k) === "number"));
      if (type === "histogram") ys = [];
      else if (type === "pie" || type === "donut") ys = ys.slice(0, 1);
    }
    const series = (Array.isArray(ys) ? ys : [ys]).map((s) => {
      const o = isObj(s) ? s : { key: s };
      const key = need(o.key, "y");
      const st = o.type === undefined ? type : String(o.type);
      if (!CARTESIAN.includes(st) && st !== type) throw new ChartError("type", fill(texts.badType || "unknown chart type: {0}", st));
      return {
        key, label: o.label === undefined ? String(key) : String(o.label), type: st,
        axis: o.axis === "y2" ? "y2" : "y", color: o.color ? String(o.color) : "",
        format: o.format ? String(o.format) : "",
        size: o.size === undefined || o.size === null ? undefined : need(o.size, "size"),
      };
    });
    if (!series.length && type !== "histogram") throw new ChartError("noSeries", texts.noSeries || "no numeric columns to draw");
    if (type === "bubble" && series.some((s) => s.size === undefined)) throw new ChartError("key", fill(texts.noKey || "no column {0} ({1})", "", "size"));
    return { type, rows, x, series };
  }

  // ------------------------------------------------------------ AG Charts

  const num = (v) => (typeof v === "number" ? (isFinite(v) ? v : undefined) : v === null || v === undefined || v === "" ? undefined : isFinite(Number(v)) ? Number(v) : undefined);
  const dateText = (d) => d.getFullYear() + "/" + (d.getMonth() + 1) + "/" + d.getDate() +
    (d.getHours() || d.getMinutes() ? " " + d.getHours() + ":" + pad(d.getMinutes()) : "");

  /** Deep merge of plain objects (b wins; arrays are replaced). */
  function merge(a, b) {
    if (!isObj(a) || !isObj(b)) return b === undefined ? a : b;
    const out = Object.assign({}, a);
    for (const k of Object.keys(b)) out[k] = merge(a[k], b[k]);
    return out;
  }

  /** AgChartOptions for a resolved spec (r) of a cartesian or pie chart. */
  function agOptions(spec, r, ctx = {}) {
    const lang = ctx.lang || "ja";
    const fmt = (f, v) => (f && window.xljsNumfmt ? window.xljsNumfmt.format(f, v, lang) : null);
    const axes = isObj(spec.axes) ? spec.axes : {};
    const ax = (k) => (isObj(axes[k]) ? axes[k] : {});
    const xs = r.rows.map((row) => cell(row, r.x));
    const polar = r.type === "pie" || r.type === "donut";
    // The kind of X: time (dates) for lines and points, numbers for points; categories otherwise.
    const types = new Set(r.series.map((s) => s.type).concat([r.type]));
    const onlyPoints = [...types].every((t) => t === "line" || t === "area" || t === "scatter" || t === "bubble" || t === "histogram");
    const allDates = xs.length > 0 && xs.every((v) => v instanceof Date || v === null || v === undefined) && xs.some((v) => v instanceof Date);
    const allNums = xs.length > 0 && xs.every((v) => typeof v === "number" || v === null || v === undefined) && xs.some((v) => typeof v === "number");
    const xKind = polar ? "category" : r.type === "histogram" ? "number"
      : allDates && onlyPoints ? "time" : allNums && [...types].every((t) => t === "scatter" || t === "bubble" || t === "histogram" || (t === "line" && types.has("scatter"))) ? "number" : "category";
    const xFormat = ax("x").format ? String(ax("x").format) : "";
    const xValue = (v) => {
      if (xKind === "time") return v instanceof Date ? v : undefined;
      if (xKind === "number") return num(v);
      if (v instanceof Date) return fmt(xFormat, v) ?? dateText(v);
      if (v === null || v === undefined) return "";
      return typeof v === "number" ? (fmt(xFormat, v) ?? String(v)) : String(v);
    };
    const data = r.rows.map((row, i) => {
      const o = { __i: i, x: xValue(cell(row, r.x)) };
      r.series.forEach((s, k) => {
        o["s" + k] = num(cell(row, s.key));
        if (s.size !== undefined) o["z" + k] = num(cell(row, s.size));
      });
      return o;
    });
    const colors = Array.isArray(spec.colors) && spec.colors.length ? spec.colors.map(String) : PALETTE;
    const stacked = spec.stacked === "percent" ? { stacked: true, normalizedTo: 100 } : spec.stacked ? { stacked: true } : {};
    const series = polar ? [Object.assign({
      type: r.type, angleKey: "s0", angleName: r.series[0].label, legendItemKey: "x", calloutLabelKey: "x",
      sectorLabelKey: "s0", fills: colors, strokes: colors.map(() => ctx.dark ? "#2b2b2b" : "#ffffff"),
      sectorLabel: { enabled: spec.labels === true },
      calloutLabel: { enabled: spec.labels !== false },
    }, r.type === "donut" ? { innerRadiusRatio: 0.6 } : {})]
      : r.type === "histogram" ? [{ type: "histogram", xKey: "x", xName: String(r.x), fill: colors[0], stroke: colors[0],
        ...(spec.bins ? { binCount: Number(spec.bins) } : {}) }]
        : r.series.map((s, k) => {
          const color = s.color || colors[k % colors.length];
          const base = { xKey: "x", yKey: "s" + k, yName: s.label, xName: String(r.x) };
          if (s.axis === "y2") base.yKeyAxis = "y2";
          const stack = s.type === r.type ? stacked : {};
          switch (s.type) {
            case "column": case "bar":
              return Object.assign(base, { type: "bar", direction: s.type === "bar" ? "horizontal" : "vertical", fill: color, stroke: color }, stack);
            case "line":
              return Object.assign(base, { type: "line", stroke: color, marker: { enabled: spec.markers !== false, fill: color, stroke: color } }, stack);
            case "area":
              return Object.assign(base, { type: "area", fill: color, stroke: color, fillOpacity: types.size > 1 ? 0.5 : 0.8 }, stack);
            case "scatter":
              return Object.assign(base, { type: "scatter", fill: color, stroke: color });
            case "bubble":
              return Object.assign(base, { type: "bubble", sizeKey: "z" + k, fill: color, stroke: color, fillOpacity: 0.7 });
          }
          return base;
        });
    // Formats: per series (tooltips and labels), per axis (axis labels), or the chart's.
    const seriesFormat = (key) => {
      const k = /^[sz](\d+)$/.exec(String(key || ""));
      const s = k ? r.series[Number(k[1])] : null;
      return (s && (s.format || ax(s.axis).format)) || spec.format || "";
    };
    const formatter = (p) => {
      let f;
      if (p.property === "x" || p.property === "legendItem" || p.property === "calloutLabel") f = xKind === "category" ? "" : xFormat;
      else if (p.source === "axis-label") {
        const bound = (p.boundSeries || []).map((b) => /^s(\d+)$/.exec(b.key)).filter(Boolean).map((m) => r.series[Number(m[1])]);
        const axis = bound.length && bound.every((s) => s.axis === "y2") ? "y2" : "y";
        // 100% stacks: the axis goes from 0 to 100; shown as percentages (an Excel format such as "0%" sees 0 to 1).
        if (spec.stacked === "percent" && axis === "y" && typeof p.value === "number") {
          const yf = String(ax("y").format || "");
          const out = fmt(/%/.test(yf.replace(/"[^"]*"/g, "")) ? yf : "0%", p.value / 100);
          return out === null ? undefined : out;
        }
        f = ax(axis).format || spec.format || "";
      } else f = seriesFormat(p.key);
      const out = fmt(f ? String(f) : "", p.value);
      return out === null ? undefined : out;
    };
    const title = (a) => (a.title ? { title: { text: String(a.title) } } : {});
    const range = (a) => Object.assign({}, a.min !== undefined && a.min !== null ? { min: a.min } : {}, a.max !== undefined && a.max !== null ? { max: a.max } : {});
    const horizontal = r.type === "bar";
    const opts = {
      data,
      series,
      theme: {
        baseTheme: ctx.dark ? "ag-default-dark" : "ag-default",
        palette: { fills: colors, strokes: colors },
        params: { fontFamily: FONT },
      },
      formatter,
      // The page's own background (the form's panel, or the script's page).
      background: { visible: false },
      legend: spec.legend === false ? { enabled: false }
        : { enabled: spec.legend !== undefined ? true : polar || series.length > 1, position: typeof spec.legend === "string" ? spec.legend : "bottom" },
    };
    if (spec.title) opts.title = { text: String(spec.title) };
    if (spec.subtitle) opts.subtitle = { text: String(spec.subtitle) };
    if (lang.startsWith("ja") && window.AG_CHARTS_LOCALE_JA_JP) opts.locale = { localeText: window.AG_CHARTS_LOCALE_JA_JP };
    if (!polar) {
      const yType = (a) => (a.log ? "log" : "number");
      opts.axes = {
        x: Object.assign({ type: xKind, position: horizontal ? "left" : "bottom" }, title(ax("x")), xKind === "category" ? {} : range(ax("x"))),
        y: Object.assign({ type: yType(ax("y")), position: horizontal ? "bottom" : "left" }, title(ax("y")), range(ax("y"))),
      };
      if (r.series.some((s) => s.axis === "y2")) {
        opts.axes.y2 = Object.assign({ type: yType(ax("y2")), position: horizontal ? "top" : "right" }, title(ax("y2")), range(ax("y2")));
      }
    }
    if (ctx.onClick) {
      opts.listeners = {
        seriesNodeClick: (e) => {
          const d = e.datum;
          if (!d || typeof d.__i !== "number") return;
          const m = /^s(\d+)$/.exec(String(e.yKey || e.angleKey || ""));
          ctx.onClick(d.__i, { series: m && r.series[Number(m[1])] ? r.series[Number(m[1])].label : null });
        },
      };
    }
    return isObj(spec.options) ? merge(opts, spec.options) : opts;
  }

  // ------------------------------------------------------------ Frappe Gantt

  /** A date of a cell: Date, "2026-04-01" or "2026/4/1 9:00", or an Excel serial number. */
  function toDate(v) {
    if (v instanceof Date) return isNaN(v) ? null : v;
    if (typeof v === "number" && isFinite(v) && v > 0 && v < 2958466) {
      const d = new Date(1899, 11, 30 + Math.floor(v));
      d.setMinutes(Math.round((v % 1) * 1440));
      return d;
    }
    if (typeof v !== "string") return null;
    const m = /^(\d{4})[-/](\d{1,2})[-/](\d{1,2})(?:[ T](\d{1,2}):(\d{2}))?/.exec(v.trim());
    if (!m) return null;
    const d = new Date(+m[1], +m[2] - 1, +m[3], +(m[4] || 0), +(m[5] || 0));
    return d.getMonth() === +m[2] - 1 ? d : null;
  }
  const hasTime = (d) => d.getHours() || d.getMinutes() || d.getSeconds();
  const ganttDate = (d) => d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate()) +
    (hasTime(d) ? " " + pad(d.getHours()) + ":" + pad(d.getMinutes()) : "");

  const VIEWS = { day: "Day", week: "Week", month: "Month", year: "Year" };

  /** The view modes with headers in the page's language. */
  function viewModes(lang) {
    const f = (o) => new Intl.DateTimeFormat(lang, o);
    const ym = f({ year: "numeric", month: "long" }), md = f({ month: "numeric", day: "numeric" }),
      mon = f({ month: "short" }), yr = f({ year: "numeric" });
    return [
      { name: "Day", padding: "7d", step: "1d", date_format: "YYYY-MM-DD",
        lower_text: (d) => String(d.getDate()),
        upper_text: (d, ld) => (!ld || d.getMonth() !== ld.getMonth() ? ym.format(d) : ""),
        thick_line: (d) => d.getDay() === 1 },
      { name: "Week", padding: "1m", step: "7d", date_format: "YYYY-MM-DD", column_width: 140,
        lower_text: (d) => md.format(d) + " - " + md.format(new Date(d.getFullYear(), d.getMonth(), d.getDate() + 6)),
        upper_text: (d, ld) => (!ld || d.getMonth() !== ld.getMonth() ? ym.format(d) : ""),
        thick_line: (d) => d.getDate() >= 1 && d.getDate() <= 7, upper_text_frequency: 4 },
      { name: "Month", padding: "2m", step: "1m", date_format: "YYYY-MM", column_width: 120,
        lower_text: (d) => mon.format(d),
        upper_text: (d, ld) => (!ld || d.getFullYear() !== ld.getFullYear() ? yr.format(d) : ""),
        thick_line: (d) => d.getMonth() % 3 === 0, snap_at: "7d" },
      { name: "Year", padding: "2y", step: "1y", date_format: "YYYY", column_width: 120,
        lower_text: (d) => yr.format(d),
        upper_text: (d, ld) => (!ld || Math.floor(d.getFullYear() / 10) !== Math.floor(ld.getFullYear() / 10) ? String(d.getFullYear() - (d.getFullYear() % 10)) : ""),
        snap_at: "30d" },
    ];
  }

  /**
   * { tasks, options, ratio } for Frappe Gantt from a resolved spec (r).
   * Tasks are identified by their row index; rows without a start date are
   * left out. An end date without a time is included (the bar covers that
   * day). Progress in 0..1 is shown as a percentage (ratio: true).
   */
  function ganttOptions(spec, r, ctx = {}) {
    const lang = ctx.lang || "ja";
    const texts = ctx.texts || {};
    const progressOf = (row) => num(cell(row, r.progress));
    const ratio = r.progress !== undefined && r.rows.every((row) => { const p = progressOf(row); return p === undefined || (p >= 0 && p <= 1); });
    // Dependencies name other tasks by id (the id column) or by name.
    const ref = new Map();
    r.rows.forEach((row, i) => {
      const k = cell(row, r.id !== undefined ? r.id : r.name);
      if (k !== undefined && k !== null && !ref.has(String(k))) ref.set(String(k), String(i));
    });
    const deps = (v) => (Array.isArray(v) ? v : v === null || v === undefined ? [] : String(v).split(/[,、;\s]+/))
      .map((x) => ref.get(String(x).trim())).filter((x) => x !== undefined);
    const tasks = [];
    r.rows.forEach((row, i) => {
      const start = toDate(cell(row, r.start));
      if (!start) return;
      let end = r.end !== undefined ? toDate(cell(row, r.end)) : null;
      if (!end || end < start) end = start;
      const p = progressOf(row);
      const t = { id: String(i), name: String(cell(row, r.name) ?? ""), start: ganttDate(start), end: ganttDate(end),
        progress: p === undefined ? 0 : Math.max(0, Math.min(100, ratio ? p * 100 : p)) };
      const d = r.dependencies !== undefined ? deps(cell(row, r.dependencies)).filter((x) => x !== t.id) : [];
      if (d.length) t.dependencies = d.join(", ");
      tasks.push(t);
    });
    const edit = spec.editable === true ? { dates: true, progress: true }
      : isObj(spec.editable) ? { dates: !!spec.editable.dates, progress: !!spec.editable.progress } : { dates: false, progress: false };
    // Holidays: the script's dates (with an optional name) and, unless weekends: false, Saturdays and Sundays.
    const holidays = [];
    if (spec.weekends !== false) holidays.push((d) => d.getDay() === 0 || d.getDay() === 6);
    for (const h of Array.from(spec.holidays || [])) {
      const d = toDate(isObj(h) ? h.date : h);
      if (!d) continue;
      const day = ganttDate(new Date(d.getFullYear(), d.getMonth(), d.getDate()));
      holidays.push(isObj(h) && h.name ? { date: day, name: String(h.name) } : day);
    }
    const view = VIEWS[String(spec.view || "day").toLowerCase()] || "Day";
    const dayFmt = new Intl.DateTimeFormat(lang, { month: "numeric", day: "numeric" });
    const options = {
      view_mode: view,
      view_modes: viewModes(lang),
      language: lang,
      readonly: !edit.dates && !edit.progress,
      readonly_dates: !edit.dates,
      readonly_progress: !edit.progress,
      holidays: { "var(--g-weekend-highlight-color)": holidays },
      today_button: true,
      view_mode_select: spec.viewSelect !== false,
      infinite_padding: false,
      // The first task (Frappe Gantt takes a date string).
      scroll_to: tasks.length ? tasks.reduce((a, t) => (t.start < a ? t.start : a), tasks[0].start).slice(0, 10) : "today",
      popup_on: "click",
      lines: "both",
      popup: (p) => {
        const t = p.task;
        const last = new Date(t._end.getTime() - 1000);
        p.set_title(escape(t.name));
        p.set_subtitle("");
        p.set_details(escape(dayFmt.format(t._start) + " - " + dayFmt.format(last) + " (" + fill(texts.days || "{0} days", t.actual_duration) + ")") +
          (r.progress !== undefined ? "<br>" + escape(fill(texts.progress || "Progress: {0}%", Math.round(t.progress))) : ""));
      },
      __texts: texts,
    };
    if (ctx.height) options.container_height = ctx.height;
    if (ctx.onClick) {
      // Frappe Gantt reports a click more than once.
      let last = { id: null, at: 0 };
      options.on_click = (t) => {
        const now = Date.now();
        if (last.id === t.id && now - last.at < 400) return;
        last = { id: t.id, at: now };
        ctx.onClick(Number(t.id), {});
      };
    }
    if (ctx.onChange) {
      // A drag reports each step (and moves the tasks that depend on the
      // task): the changes of a task are sent once it has stopped moving.
      const pending = new Map();
      const report = (id, changes) => {
        const p = pending.get(id) || { changes: {} };
        clearTimeout(p.timer);
        Object.assign(p.changes, changes);
        p.timer = setTimeout(() => { pending.delete(id); ctx.onChange(Number(id), p.changes); }, 250);
        pending.set(id, p);
      };
      options.on_date_change = (t, start, end) => {
        // end is the last moment of the last day (23:59:59) for whole days.
        const e = new Date(end.getTime() + 1000);
        const whole = !hasTime(start) && !hasTime(e);
        report(t.id, { start, end: whole ? new Date(e.getFullYear(), e.getMonth(), e.getDate() - 1) : end });
      };
      options.on_progress_change = (t, p) => {
        const v = Math.round(p);
        report(t.id, { progress: ratio ? v / 100 : v });
      };
    }
    return { tasks, options: isObj(spec.options) ? merge(options, spec.options) : options, ratio };
  }

  const escape = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

  /** Translates the buttons of Frappe Gantt's side header (Today, Mode and the view names). */
  function patchGantt() {
    const G = window.Gantt;
    if (!G || G.prototype.__xljs) return;
    const orig = G.prototype.make_side_header;
    G.prototype.make_side_header = function () {
      orig.apply(this, arguments);
      const t = this.options.__texts || {};
      if (this.$today_button && t.today) this.$today_button.textContent = t.today;
      const sel = this.$side_header && this.$side_header.querySelector("select");
      if (sel) {
        for (const o of sel.options) {
          if (o.disabled) { if (t.view) o.textContent = t.view; }
          else if (t.views && t.views[o.value]) o.textContent = t.views[o.value];
        }
      }
    };
    G.prototype.__xljs = true;
  }

  // ------------------------------------------------------------ loading and drawing

  let loading = null;
  /** Loads the libraries (once); base is the folder of the built-in files ("" in them, "/_xljs/" elsewhere). */
  function load(base = "") {
    if (window.agCharts && window.Gantt) return Promise.resolve();
    if (loading) return loading;
    const script = (src) => new Promise((ok, fail) => {
      const s = document.createElement("script");
      s.src = base + src;
      s.onload = ok;
      s.onerror = () => fail(new Error("cannot load " + src));
      document.head.append(s);
    });
    const css = document.createElement("link");
    css.rel = "stylesheet";
    css.href = base + "vendor/frappe-gantt/frappe-gantt.css";
    document.head.append(css);
    const libs = [];
    if (!window.xljsNumfmt) libs.push(script("numfmt.js"));
    if (!window.agCharts) libs.push(script("vendor/ag-charts/ag-charts-community.min.js").then(() => script("vendor/ag-charts/ag-charts-locale-ja-JP.js")));
    if (!window.Gantt) libs.push(script("vendor/frappe-gantt/frappe-gantt.umd.js"));
    loading = Promise.all(libs);
    return loading;
  }

  const dark = () => !!(window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches);

  /**
   * Draws spec in el. ctx: { lang, texts, onClick(index, info),
   * onChange(index, changes), height (Gantt, px) }. Returns { update(spec),
   * destroy() }; update replaces the chart's definition.
   */
  function create(el, spec, ctx = {}) {
    let ag = null, gantt = null;
    const c = Object.assign({ dark: dark() }, ctx);
    function draw(s) {
      const r = resolve(s, c.texts);
      if (r.type === "gantt") {
        if (ag) { ag.destroy(); ag = null; }
        patchGantt();
        const g = ganttOptions(s, r, c);
        el.textContent = "";
        el.classList.add("xljs-gantt");
        if (!g.tasks.length) {
          el.append(Object.assign(document.createElement("div"), { className: "xljs-chart-empty", textContent: (c.texts && c.texts.empty) || "" }));
          gantt = null;
          return;
        }
        gantt = new window.Gantt(el, g.tasks, g.options);
      } else {
        const o = agOptions(s, r, c);
        el.classList.remove("xljs-gantt");
        if (gantt || !ag) {
          el.textContent = "";
          gantt = null;
          ag = window.agCharts.AgCharts.create(Object.assign({ container: el }, o));
        } else ag.update(Object.assign({ container: el }, o));
      }
    }
    draw(spec);
    return {
      update(s) { draw(s); },
      destroy() {
        if (ag) ag.destroy();
        ag = gantt = null;
        el.textContent = "";
      },
    };
  }

  // Dates arrive from the script as { $date: milliseconds }.
  function revive(v) {
    if (Array.isArray(v)) return v.map(revive);
    if (v === null || typeof v !== "object") return v;
    const keys = Object.keys(v);
    if (keys.length === 1 && keys[0] === "$date") return new Date(v.$date);
    const o = {};
    for (const k of keys) o[k] = revive(v[k]);
    return o;
  }

  window.XljsChart = { load, create, resolve, agOptions, ganttOptions, revive, toDate, ChartError, PALETTE };
})();
