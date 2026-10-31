import { ok, equal, deepEqual, throws } from "./_assert.js";

// Tests run with the Japanese locale: new workbooks default to 游ゴシック.
const wb = workbooks.create();
deepEqual(wb.defaultFont, { name: "游ゴシック", size: 11 });
equal(workbooks.create({ font: "Meiryo UI" }).defaultFont.name, "Meiryo UI");

const ws = wb.addSheet("s");
ws.write([["a", 1], ["b", 2], ["c", 3]]);

// Shorthands and the full form give the same result; styles merge.
ws.style("A1", { bold: true, color: "#f00", background: "yellow", align: "center", format: "0.0" });
ws.cell("A1").style({ italic: true, font: { size: 14 } });
deepEqual(ws.cell("A1").getStyle(), {
  font: { name: "游ゴシック", size: 14, bold: true, italic: true, color: "#FF0000" },
  fill: "#FFFF00",
  alignment: { horizontal: "center" },
  numberFormat: "0.0",
});
// replace: true discards the previous style.
ws.cell("A1").style({ underline: true }, { replace: true });
deepEqual(ws.cell("A1").getStyle(), { font: { name: "游ゴシック", size: 11, underline: "single" } });
// Removing fill / format.
ws.style("B1", { fill: "#ABCDEF", format: "#,##0" });
ws.style("B1", { fill: null, format: "General" });
deepEqual(ws.cell("B1").getStyle(), { font: { name: "游ゴシック", size: 11 } });

// Borders: outline + inside, and per side.
ws.range("A1:B3").border({ outline: "medium", inside: { style: "dashed", color: "#999" } });
const b = (ref) => ws.cell(ref).getStyle().border;
deepEqual(b("A1"), {
  left: { style: "medium", color: "#000000" }, right: { style: "dashed", color: "#999999" },
  top: { style: "medium", color: "#000000" }, bottom: { style: "dashed", color: "#999999" },
});
equal(b("B3").right.style, "medium");
equal(b("B3").bottom.style, "medium");
equal(b("A2").left.style, "medium");
equal(b("A2").right.style, "dashed");
// A later border on a sub-range wins and the touching neighbor is updated.
ws.style("A3:B3", { border: { top: "double" } });
equal(b("A3").top.style, "double");
equal(b("A2").bottom.style, "double");
ws.style("A2", { border: "none" });
equal(ws.cell("A2").getStyle().border, undefined);
ws.style("A2", { border: { style: "thick", color: "red" } });
equal(b("A2").top.color, "#FF0000");

// Alignment, rotation, wrap.
ws.style("C1", { alignment: { horizontal: "right", vertical: "middle", wrap: true, indent: 2, rotation: 45 } });
deepEqual(ws.cell("C1").getStyle().alignment, { horizontal: "right", vertical: "center", wrap: true, indent: 2, rotation: 45 });

// Values keep their types after styling; dates keep a date format.
ws.setValue("D1", new Date(2026, 0, 2));
ws.style("D1", { bold: true });
ok(ws.getValue("D1") instanceof Date, "styled date stays a date");
equal(ws.getValue("B2"), 2);

// Merge, freeze, filter, sizes.
ws.merge("E1:F2");
ws.freeze("A2");
ws.freeze({ rows: 1, columns: 1 });
ws.unfreeze();
ws.autoFilter("A1:B3");
ws.setRowHeight(1, 30);
ws.setColumnWidth("A:B", 12);
equal(ws.getColumnWidth("A"), 12);
ws.setValue("G1", "とても長い見出しのテキストです");
ws.autoFit("G");
ok(ws.getColumnWidth("G") > 25, "autoFit widens for wide text: " + ws.getColumnWidth("G"));

// Range values.
deepEqual(ws.range("A2:B3").values, [["b", 2], ["c", 3]]);
ws.range("H1").values = [[1, 2], [3, 4]];
equal(ws.getValue("I2"), 4);

// write() formatting options.
const t = wb.addSheet("table");
t.write([{ 名前: "a", 金額: 1200 }, { 名前: "b", 金額: 30 }], { table: true, columnStyles: { 金額: { format: "#,##0" } } });
equal(t.cell("A1").getStyle().font.bold, true);
equal(t.cell("A1").getStyle().fill, "#DDEBF7");
equal(t.cell("B2").getStyle().numberFormat, "#,##0");
equal(t.cell("B3").getStyle().border.bottom.style, "thin");
equal(t.cell("B2").text, "1,200");

// Errors are reported in Japanese.
throws(() => ws.style("A1", { color: "nope" }), /色の指定/);
throws(() => ws.style("A1", { border: "wavy" }), /罫線の種類/);
throws(() => ws.style("1A:B", { bold: true }), /セル範囲/);

wb.save("styled.xlsx");
const back = workbooks.open("styled.xlsx");
equal(back.sheet("table").cell("A1").getStyle().fill, "#DDEBF7");
console.log("style ok");
