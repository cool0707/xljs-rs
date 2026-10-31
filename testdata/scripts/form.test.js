import { ok, equal, deepEqual, rejects } from "./_assert.js";

// Answers come from form.test.answers.json: each answer lists the values
// that differ from the defaults; an exhausted queue means "cancel".
fs.mkdir("data");
fs.writeText("data/a.xlsx", "");
fs.writeText("data/b.xlsx", "");

const fields = [
  { name: "src", label: "集計フォルダ", type: "folder", required: true },
  { name: "out", label: "出力先", type: "saveFile", accept: [".xlsx"], default: "集計.xlsx" },
  { name: "mode", label: "集計単位", type: "select", options: ["顧客別", "商品別"] },
  { name: "kind", type: "radio", options: [{ value: "a", label: "A" }, { value: "b", label: "B" }], default: "b" },
  { name: "tax", label: "税込で集計", type: "checkbox" },
  { name: "count", type: "number", min: 1, max: 10, default: 3 },
  { name: "day", type: "date", default: new Date(2026, 3, 1) },
  { name: "month", type: "month" },
  { name: "code", pattern: "[A-Z]{3}", patternMessage: "英大文字 3 文字" },
  { name: "inputs", type: "files", accept: ["xlsx"] },
  { name: "secret", type: "password" },
];

// 1st answer: invalid values -> errors; 2nd: valid.
let validated = 0;
const v = await ui.form({
  title: "集計",
  description: "フォルダ内の Excel を集計します",
  fields,
  remember: true,
  validate(values) {
    validated++;
    if (values.mode === "商品別" && !values.tax) return { tax: "商品別は税込のみ" };
  },
});
equal(validated, 2, "validate runs once the fields pass");
ok(v.src instanceof FileSystemDirectoryHandle);
equal(v.src.name, "data");
ok(v.out instanceof FileSystemFileHandle);
equal(v.out.name, "集計.xlsx");
equal(v.mode, "商品別");
equal(v.kind, "b");
equal(v.tax, true);
equal(v.count, 5);
ok(v.day instanceof Date);
equal(v.day.getTime(), new Date(2026, 3, 1).getTime());
equal(v.month, "2026-04");
equal(v.code, "ABC");
deepEqual(v.inputs.map((h) => h.name), ["a.xlsx", "b.xlsx"]);
equal(v.secret, "pw");

// remember: the values come back as defaults (the password is not stored).
const saved = fs.readJSON(path.join(xljs.env("XLJS_CONFIG_DIR"), "form.test.json"));
equal(saved["集計"].src, "data");
equal(saved["集計"].secret, undefined, "passwords are not remembered");
const again = await ui.form({ title: "集計", fields, remember: true });
equal(again.src.name, "data");
equal(again.count, 5);
equal(again.secret, "", "password starts empty");

// Cancel.
equal(await ui.form({ title: "取消", fields: [{ name: "x" }] }), null);

// onSubmit runs with a context; the form returns the values afterwards.
const seen = [];
const r = await ui.form({
  title: "実行",
  fields: [{ name: "n", type: "number", default: 3 }],
  async onSubmit(values, ctx) {
    for (const i of ctx.withProgress([1, 2, 3])) seen.push(i);
    ctx.progress(3, 3, "done");
    ctx.log("finished", values.n);
    equal(ctx.cancelled, false);
  },
});
deepEqual(seen, [1, 2, 3]);
equal(r.n, 3);

// Errors in onSubmit reach the caller (no window to show them in).
await rejects(() => ui.form({ fields: [], onSubmit() { throw new Error("boom"); } }), /boom/);

// Aborting ctx.withProgress ends onSubmit (the rest does not run) without an error.
const aborted = [];
let reachedEnd = false;
const r2 = await ui.form({
  title: "中止",
  fields: [{ name: "n", type: "number", default: 1 }],
  onSubmit(values, ctx) {
    for (const i of ctx.withProgress([1, 2, 3])) aborted.push(i);
    reachedEnd = true;
  },
});
deepEqual(aborted, [1]);
equal(reachedEnd, false);
equal(r2.n, 1);

// Buttons: "$button" in an answer names the one pressed (index or label);
// after a button without close the form is asked again.
const clicks = [];
const r3 = await ui.form({
  title: "ボタン",
  fields: [{ name: "n", type: "number", required: true }, { name: "memo" }],
  buttons: [
    { label: "読込", validate: false, onClick(values, ctx) { clicks.push("load:" + values.n); ctx.setValue("n", 7); ctx.setValue("memo", "loaded"); } },
    { label: "確認", onClick(values, ctx) { clicks.push("check:" + values.n); ctx.log("checked"); } },
    { label: "完了", close: true, onClick(values) { clicks.push("finish:" + values.memo); } },
  ],
  onSubmit() { clicks.push("submit"); },
});
deepEqual(clicks, ["load:null", "check:7", "finish:x"]);
equal(r3.n, 8);

// submit: false hides the submit button; closing the form returns the values of the last checked press.
const r4 = await ui.form({
  title: "ボタンのみ",
  submit: false,
  fields: [{ name: "x", default: "a" }],
  buttons: [{ label: "実行", onClick(values) { clicks.push("run:" + values.x); } }],
});
deepEqual(clicks.slice(3), ["run:a"]);
equal(r4.x, "a");

// A field's own validate gets its value and all the values; it runs once the field passes its checks.
const fieldChecks = [];
const r5 = await ui.form({
  title: "項目の検証",
  fields: [
    { name: "from", type: "date", required: true },
    { name: "to", type: "date", validate(v, values) { fieldChecks.push(v === null ? null : v.getDate()); if (v && values.from && v < values.from) return "開始日より後にしてください"; } },
  ],
});
deepEqual(fieldChecks, [null, 1, 9]);
equal(r5.to.getDate(), 9);

// onChange runs for a field the answer changes (once it passes its checks);
// its setValue fills a later field unless the answer changes that too.
fs.writeText("data/売上_04.csv", "");
fs.writeText("data/仕入_04.csv", "");
const changes = [];
const pair = {
  title: "変更時",
  fields: [
    { name: "sales", type: "file", tooltip: "売上の CSV", onChange(h, values, ctx) {
      changes.push(h ? h.name : null);
      if (h && !values.purchase) ctx.setValue("purchase", h.path.replace("売上_", "仕入_"));
    } },
    { name: "purchase", type: "file" },
    { name: "kind", type: "select", options: ["a", "b"], placeholder: "選んでください" },
  ],
};
const r6 = await ui.form(pair);
deepEqual(changes, ["売上_04.csv"]);
equal(r6.purchase.name, "仕入_04.csv");
equal(r6.kind, "", "a select with a placeholder starts empty");
const r7 = await ui.form(pair);
deepEqual(changes, ["売上_04.csv", null], "no onChange for a missing file; clearing runs it");
equal(r7.purchase.name, "b.xlsx");

// Bad definitions.
await rejects(() => ui.form({ fields: [], buttons: [{ label: "x" }] }), /onClick/);
await rejects(() => ui.form({ fields: [{ name: "a", type: "colour" }] }), /colour/);
await rejects(() => ui.form({ fields: [{ name: "a" }, { name: "a" }] }), /重複/);
await rejects(() => ui.form({ fields: [{ name: "a", type: "select" }] }), /options/);
console.log("form ok");
