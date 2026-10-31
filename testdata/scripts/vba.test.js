import { ok, equal, deepEqual, throws } from "./_assert.js";

// macro.xlsm is written by internal/vba (go test -run TestWriteFixture -update).
const book = path.join(xljs.scriptDir, "macro.xlsm");
const vba = workbooks.readVBA(book);
equal(vba.name, "VBAProject");
equal(vba.codePage, 932);
deepEqual(vba.modules.map((m) => [m.name, m.type, m.sheet]), [
  ["ThisWorkbook", "document", null],
  ["Sheet1", "document", "売上"],
  ["Sheet2", "document", "設定"],
  ["Module1", "standard", null],
  ["集計", "standard", null],
  ["Class1", "class", null],
  ["UserForm1", "form", null],
]);
const m1 = vba.modules.find((m) => m.name === "Module1");
ok(m1.code.startsWith('Attribute VB_Name = "Module1"\nOption Explicit\n'));
ok(m1.code.includes('Set ws = Worksheets("売上")'));
ok(!m1.code.includes("\r"));
ok(vba.text.startsWith("# ") && vba.text.includes("macro.xlsm"));
ok(vba.text.includes("```vba\nOption Explicit\n\n' 売上を集計する\nSub 売上集計()"));

// The same from a handle, a File and bytes.
equal(workbooks.readVBA(fs.file(book)).modules.length, 7);
const bytes = fs.readBytes(book);
ok(workbooks.readVBA(new File([bytes], "コピー.xlsm")).text.includes("コピー.xlsm"));
equal(workbooks.readVBA(bytes).modules[4].name, "集計");

// The workbook itself still opens.
const wb = workbooks.open(book);
deepEqual(wb.sheets.map((s) => s.name), ["売上", "設定"]);
wb.close();

// No macros: null. Not a workbook: an error.
workbooks.create().save("plain.xlsx");
equal(workbooks.readVBA("plain.xlsx"), null);
fs.writeText("a.csv", "a,b\n");
throws(() => workbooks.readVBA("a.csv"), /a\.csv/);

console.log("vba ok");
