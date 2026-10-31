import { ok, equal, deepEqual } from "./_assert.js";

// parse: quotes, embedded newlines and delimiters, duplicate/empty headers.
const text = 'id,名前,,名前\r\n1,"山田, 太郎","a ""q""",x\r\n2,"改行\nあり",,y\r\n\r\n';
deepEqual(csv.parse(text, { cast: false }), [
  { id: "1", 名前: "山田, 太郎", 列3: 'a "q"', 名前_2: "x" },
  { id: "2", 名前: "改行\nあり", 列3: "", 名前_2: "y" },
]);
deepEqual(csv.parse("a,b\n1,2", { header: false, cast: false }), [["a", "b"], ["1", "2"]]);
deepEqual(csv.parse("1,2\n3", { header: false, columns: ["x", "y"], cast: false }), [{ x: "1", y: "2" }, { x: "3", y: "" }]);

// Values are typed by default: numbers (codes with a leading zero and
// numbers too long for a double stay text), dates, booleans, "" → null.
deepEqual(csv.parse("n,s,b,z\n1,007,true,\n-2.5,\"1,234\",FALSE,0"), [
  { n: 1, s: "007", b: true, z: null },
  { n: -2.5, s: 1234, b: false, z: 0 },
]);
deepEqual(csv.parse("a,b\n1,2", { header: false }), [["a", "b"], [1, 2]]);
deepEqual(csv.parse("1,2\n3", { header: false, columns: ["x", "y"] }), [{ x: 1, y: 2 }, { x: 3, y: null }]);
const typed = csv.parse("日付,日時,不正,ID,JAN,郵便,電話\n2026/4/1,2026-04-01 09:30,2026/2/30,12345678901234567,4901234567894,123-4567,090-1234-5678");
ok(typed[0].日付 instanceof Date);
equal(typed[0].日付.getTime(), new Date(2026, 3, 1).getTime());
equal(typed[0].日時.getTime(), new Date(2026, 3, 1, 9, 30).getTime());
equal(typed[0].不正, "2026/2/30");
equal(typed[0].ID, "12345678901234567");
equal(typed[0].JAN, 4901234567894);
equal(typed[0].郵便, "123-4567");
equal(typed[0].電話, "090-1234-5678");
// Header names are trimmed.
deepEqual(csv.parse(" 金額 ,品名　\n1,a"), [{ 金額: 1, 品名: "a" }]);
deepEqual(csv.parse("﻿a\n1"), [{ a: 1 }], "BOM is stripped");
deepEqual(csv.parse("a\tb\nx\ty", { delimiter: "\t" }), [{ a: "x", b: "y" }]);
deepEqual(csv.parse("# c\na\n1", { comment: "#" }), [{ a: 1 }]);
deepEqual(csv.parse(" a , b \n 1 , 2 ", { trim: true, cast: false }), [{ a: "1", b: "2" }]);

// stringify: header from union of keys, quoting, dates, CRLF.
const rows = [
  { 日付: new Date(2026, 3, 1), 品名: 'ねじ "M3"', 数量: 10 },
  { 日付: new Date(2026, 3, 2, 9, 30), 品名: "a,b", 備考: "改行\nあり" },
];
equal(csv.stringify(rows),
  '日付,品名,数量,備考\r\n2026/04/01,"ねじ ""M3""",10,\r\n2026/04/02 09:30:00,"a,b",,"改行\nあり"\r\n');
equal(csv.stringify([[1, null, "x"]], { eol: "\n" }), "1,,x\n");
equal(csv.stringify([{ a: 1, b: 2 }], { columns: ["b"], header: false }), "2\r\n");
equal(csv.stringify([{ a: 1 }], { quoteAll: true }), '"a"\r\n"1"\r\n');

// write/read round trip in CP932 and UTF-8 (BOM), auto-detected on read.
csv.write("sjis.csv", rows, { encoding: "cp932" });
csv.write("utf8.csv", rows);
const bytes = new Uint8Array(fs.readBytes("utf8.csv"));
deepEqual([bytes[0], bytes[1], bytes[2]], [0xef, 0xbb, 0xbf], "default encoding writes a BOM");
for (const f of ["sjis.csv", "utf8.csv"]) {
  const back = csv.read(f);
  equal(back.length, 2, f);
  equal(back[0].品名, 'ねじ "M3"', f);
  equal(back[1].備考, "改行\nあり", f);
}
// Characters outside CP932 are replaced instead of failing.
csv.write("emoji.csv", [{ a: "😀" }], { encoding: "cp932" });
equal(csv.read("emoji.csv")[0].a, "?");

// TSV by extension, append mode, read from handle / File / Blob.
csv.write("t.tsv", [{ a: 1, b: 2 }]);
csv.write("t.tsv", [{ a: 3, b: 4 }], { header: false, append: true });
deepEqual(csv.read(fs.file("t.tsv")), [{ a: 1, b: 2 }, { a: 3, b: 4 }]);
const file = await fs.file("sjis.csv").getFile();
equal(csv.read(file)[0].数量, 10);
equal(csv.read(file)[0].日付.getTime(), new Date(2026, 3, 1).getTime(), "dates written by csv.write read back as Date");
deepEqual(csv.read(new Blob(["x\n1"])), [{ x: 1 }]);

console.log("csv ok");
