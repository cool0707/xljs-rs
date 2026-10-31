import { ok, equal, deepEqual, rejects } from "./_assert.js";

// Editable table fields. Answers: formedit.test.answers.json; edits come in
// "$cells": { field: [[row, column (index, key or label), value]] }.
const rows = [
  { コード: "A01", 金額: 100, 区分: "売上", 日付: new Date(2026, 3, 1), 済: false },
  { コード: "x", 金額: "abc", 区分: "その他", 日付: null, 済: true },
  { コード: "A03", 金額: -5, 区分: "返品", 日付: new Date(2026, 3, 3), 済: false },
];
const columns = [
  {
    key: "コード", pattern: "[A-Z]\\d{2}", patternMessage: "英大文字＋数字2桁",
    validate: (v, row, i) => (rows.some((r, j) => j !== i && r.コード === v) ? "重複しています" : ""),
  },
  { key: "金額", min: 0, required: true }, // number (from the values)
  { key: "区分", type: "select", options: ["売上", "返品"] },
  { key: "日付" }, // date
  { key: "済" }, // checkbox
  { key: "メモ", editable: false },
];

// 1st answer: the 5 errors of the rows; 2nd fixes row 2 but leaves row 3's
// row error; 3rd makes a duplicate; 4th passes.
const errorCounts = [];
const v = await ui.form({
  title: "修正",
  fields: [{
    name: "t", type: "table", rows, columns, editable: true,
    validateRow(row) { if (row.区分 === "返品" && !row.済) return "返品は済にしてください"; },
  }],
  validate() { errorCounts.push("validate"); },
});
equal(v.t.length, 3, "an editable table without selectable: all rows");
equal(v.t[1], rows[1], "the rows themselves are edited");
equal(rows[1].コード, "A02");
equal(rows[1].金額, 200);
equal(rows[1].区分, "売上");
equal(rows[2].金額, 5);
equal(rows[2].済, true);
equal(rows[0].コード, "A01");
ok(rows[1].日付 === null);
deepEqual(errorCounts, ["validate"], "validate runs once the table has no errors");

// Selectable and editable: the selected rows, with their edits.
const items = [{ 名前: "a", 数: 1 }, { 名前: "b", 数: 2 }];
const v2 = await ui.form({
  title: "選択",
  fields: [{ name: "t", type: "table", rows: items, selectable: true, columns: [{ key: "名前" }, { key: "数", editable: true }] }],
});
deepEqual(v2.t, [items[1]]);
equal(items[1].数, 1020, "full-width digits and thousands separators");
equal(items[0].数, 1);

await rejects(() => ui.form({ fields: [{ name: "t", type: "table", rows: items, columns: [{ key: "数", type: "files", editable: true }] }] }), /files/);
console.log("formedit ok");
