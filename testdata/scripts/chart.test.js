import { ok, equal, deepEqual, throws, rejects } from "./_assert.js";

// ui.chart without windows (mock): the rows are shown as a table.
const rows = [
  { 月: "4月", 売上: 1200, 原価: 800, メモ: "a" },
  { 月: "5月", 売上: 1500, 原価: 900, メモ: "b" },
];
const view = ui.chart({ type: "column", data: rows, title: "月別売上" });
ok(view.closed instanceof Promise);
await view.closed;
view.update({ data: rows.slice(0, 1) });
view.close();
ui.chart({ type: "line", data: rows, x: "月", y: [{ key: "売上", type: "column" }, { key: "原価", axis: "y2" }] }, "組み合わせ");
ui.chart({ type: "pie", data: rows });
ui.chart({ type: "gantt", data: [{ 作業: "設計", 開始: new Date(2026, 10, 2), 終了: new Date(2026, 10, 6), 進捗: 0.5 }], title: "工程" });

// Checks: the type, the columns and the handlers.
throws(() => ui.chart({ type: "radar", data: rows }), /radar/);
throws(() => ui.chart({ data: rows, x: "日" }), /列 "日"/);
throws(() => ui.chart({ data: rows, y: ["粗利"] }), /粗利/);
throws(() => ui.chart({ data: [{ 名前: "x", 値: "a" }] }), /数値の列/);
throws(() => ui.chart({ type: "bubble", data: rows, y: "売上" }), /大きさ/);
throws(() => ui.chart({ type: "gantt", data: [{ 名前: "x" }] }), /開始日/);
throws(() => ui.chart({ data: rows, editable: true }), /ガントチャート/);
throws(() => ui.chart({ data: rows, onClick: "x" }), /関数/);
throws(() => ui.chart(null), /グラフの定義/);
ui.chart({ data: [] }); // no rows: nothing to check

// Excel's charts.
const book = workbooks.open(path.join(xljs.scriptDir, "..", "xlsx", "charts.xlsx"));
const charts = book.sheets["売上"].charts;
equal(charts.length, 9);
const [col, combo, pct, pie, donut, area, d3, radar, line] = charts;

equal(col.title, "月別売上");
equal(col.type, "column");
equal(col.range, "F2"); // openpyxl writes one-cell anchors (Excel: "F2:N17")
const s = col.toSpec();
deepEqual(s.data, [{ 月: "4月", 売上: 1200, 原価: 800 }, { 月: "5月", 売上: 1500, 原価: 900 }, { 月: "6月", 売上: 900, 原価: 700 }]);
equal(s.x, "月");
// The format of the values: the cells' (原価 has none). A series without a
// color of its own gets the automatic color from the workbook's theme
// (openpyxl writes the Office 2007 theme: accent2 is #C0504D).
deepEqual(s.y, [{ key: "売上", color: "#1F4E79", format: '#,##0"円"' }, { key: "原価", color: "#C0504D" }]);
deepEqual(s.axes, { x: { title: "月" }, y: { title: "金額", format: '#,##0"円"' } });
ok(s.data !== col.toSpec().data, "new rows each time");

const c2 = combo.toSpec();
equal(c2.type, "column");
deepEqual(c2.y.map((y) => [y.key, y.type, y.axis]), [["売上", undefined, undefined], ["原価", undefined, undefined], ["利益率", "line", "y2"]]);
equal(c2.axes.y2.title, "利益率");
equal(c2.axes.y2.format, "0%");
equal(c2.markers, false);

const p = pct.toSpec();
equal(p.type, "bar");
equal(p.stacked, "percent");
// Excel draws the first category of horizontal bars at the bottom: the rows are reversed.
deepEqual(p.data.map((r) => r.月), ["6月", "5月", "4月"]);

const pi = pie.toSpec();
equal(pi.type, "pie");
deepEqual(pi.colors, ["#C00000", "#00B050", "#0070C0"]);
equal(pi.title, "売上の内訳");
equal(donut.type, "donut");
equal(donut.title, "原価", "auto title of one series");
deepEqual(donut.toSpec().colors, ["#4F81BD", "#C0504D", "#9BBB59"], "slices: the theme's accents");
equal(area.toSpec().stacked, true);
equal(d3.type, "column");
deepEqual(d3.warnings, ["3-D グラフは平面で表示します"]);
equal(radar.supported, false);
equal(radar.type, null);
ok(/レーダーチャート/.test(radar.warnings[0]), radar.warnings[0]);
throws(() => radar.toSpec(), /レーダーチャートは表示できません/);
throws(() => ui.chart(radar), /レーダー/);
equal(line.toSpec().markers, false);
// The chart's color style (colors1.xml, "Change Colors"): accent6, then accent5.
deepEqual(line.toSpec().y.map((y) => y.color), ["#F79646", "#4BACC6"]);
equal(line.toSpec().axes.y.format, "#,##0");
ui.chart(col); // an ExcelChart as is

const [scatter, bubble] = book.sheets["散布"].charts;
const sc = scatter.toSpec();
equal(sc.type, "scatter");
deepEqual(sc.data, [{ x: 1, y: 2 }, { x: 2, y: 5 }, { x: 3, y: 3 }]);
const bu = bubble.toSpec();
equal(bu.type, "bubble");
deepEqual(bu.y, [{ key: "泡", color: "#4F81BD", size: "泡 (大きさ)" }]);
deepEqual(bu.data[1], { x: 2, 泡: 5, "泡 (大きさ)": 30 });

// Stacked horizontal bars with an unfilled first series: a Gantt chart.
const [gantt] = book.sheets["工程"].charts;
equal(gantt.type, "gantt");
const g = gantt.toSpec();
deepEqual({ name: g.name, start: g.start, end: g.end }, { name: "作業", start: "開始日", end: "終了" });
deepEqual(g.data, [
  { 作業: "設計", 開始日: new Date(2026, 10, 2), 終了: new Date(2026, 10, 6) },
  { 作業: "実装", 開始日: new Date(2026, 10, 9), 終了: new Date(2026, 10, 20) },
  { 作業: "試験", 開始日: new Date(2026, 10, 23), 終了: new Date(2026, 10, 27) },
]);
equal(gantt.toSpec({ gantt: false }).type, "bar");
throws(() => pie.toSpec({ gantt: true }), /ガントチャートとして読めません/);
deepEqual(book.sheets["工程"].charts.length, 1);
deepEqual(book.addSheet("新規").charts, []);
book.close();
deepEqual(workbooks.create().addSheet("x").charts, []);

// A chart field in a form (mock): shown as a table, no value unless an editable Gantt chart.
const tasks = [{ 名前: "設計", 開始: new Date(2026, 10, 2), 終了: new Date(2026, 10, 6) }];
const v = await ui.form({
  title: "グラフ付き",
  fields: [
    { name: "a" },
    { name: "g", type: "chart", label: "売上", chart: { data: rows } },
    { name: "t", type: "chart", chart: { type: "gantt", data: tasks, editable: true } },
  ],
  onSubmit(values, ctx) {
    ctx.chart("g", { type: "line" });
    throws(() => ctx.chart("a", {}), /グラフの項目/);
    throws(() => ctx.chart("g", { type: "x" }), /種類/);
  },
});
deepEqual(Object.keys(v), ["a", "t"]);
equal(v.t[0], tasks[0]);
await rejects(() => ui.form({ fields: [{ name: "c", type: "chart", chart: { type: "pie", data: rows, x: "日" } }] }), /列 "日"/);
console.log("chart ok");
