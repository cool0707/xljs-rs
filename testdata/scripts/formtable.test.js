import { ok, equal, deepEqual, throws, rejects } from "./_assert.js";

// Table fields and tabs of ui.form. Answers: formtable.test.answers.json
// (a table's value is the indices of the selected rows).
const customers = [
  { 顧客名: "A社", 件数: 2, 合計: 130 },
  { 顧客名: "B社", 件数: 1, 合計: 50 },
  { 顧客名: "C社", 件数: 5, 合計: 1200 },
];

// 1st answer selects nothing (required) -> error; 2nd selects rows 0 and 2.
let v = await ui.form({
  title: "表",
  fields: [
    { name: "pick", label: "対象", type: "table", rows: customers, selectable: "multiple", required: true },
    { name: "one", type: "table", rows: customers, columns: ["顧客名"], selectable: "single", default: customers[1] },
    { name: "info", type: "table", rows: [[1, "x"]] },
  ],
});
deepEqual(v.pick, [customers[0], customers[2]]);
equal(v.one, customers[1], "default row");
ok(!("info" in v), "a table without selectable has no value");

// Tabs: the fields above the tabs and those of every tab, flat.
v = await ui.form({
  title: "タブ",
  fields: [{ name: "common", default: "共通" }],
  tabs: [
    { label: "基本", fields: [{ name: "a", required: true }] },
    { label: "詳細", fields: [{ name: "b", type: "number", default: 1 }, { name: "t", type: "table", rows: customers, selectable: true }] },
  ],
});
deepEqual(v, { common: "共通", a: "入力", b: 1, t: [customers[1]] });

await rejects(() => ui.form({ fields: [{ name: "x" }], tabs: [{ fields: [{ name: "x" }] }] }), /"x"/, "same name on a tab");
await rejects(() => ui.form({ tabs: [1] }), /タブ/);
await rejects(() => ui.form({ fields: [{ name: "t", type: "table", selectable: "some" }] }), /some/);

// ctx.table replaces a table's rows while onSubmit runs.
await ui.form({
  title: "結果",
  fields: [{ name: "result", label: "結果", type: "table", rows: [] }],
  async onSubmit(values, ctx) {
    ctx.table("result", customers.map((c) => ({ 顧客名: c.顧客名, 合計: c.合計 })));
    throws(() => ctx.table("missing", []), /missing/);
  },
});
console.log("formtable ok");
