import { ok, equal, deepEqual, throws } from "./_assert.js";

const data = { 名前: "山田", 数量: 3, 日付: new Date(2026, 3, 1, 9, 5), tags: ["a", "b"], nil: null };

// Dates are written as local time with offset by default.
fs.writeJSON("a.json", data);
const text = fs.readText("a.json");
ok(text.endsWith("\n") && text.includes('\n  "名前": "山田"'), "pretty printed with 2 spaces");
ok(/"日付": "2026-04-01T09:05:00[+-]\d{2}:\d{2}"/.test(text), text);
equal(new Date(fs.readJSON("a.json").日付).getTime(), data.日付.getTime());

// dates: true revives ISO date-time strings.
const back = fs.readJSON("a.json", { dates: true });
ok(back.日付 instanceof Date);
equal(back.日付.getTime(), data.日付.getTime());
deepEqual(back.tags, ["a", "b"]);
equal(typeof fs.readJSON("a.json").日付, "string");

// dates: "iso" keeps JSON.stringify's UTC format; space option.
fs.writeJSON("b.json", { d: new Date(Date.UTC(2026, 0, 2)) }, { dates: "iso", space: 0 });
equal(fs.readText("b.json"), '{"d":"2026-01-02T00:00:00.000Z"}\n');
fs.writeJSON("c.json", [1, 2], 4); // legacy third argument = space
equal(fs.readText("c.json"), "[\n    1,\n    2\n]\n");

// Encodings: BOM and CP932 input are handled.
fs.writeText("bom.json", '{"a":"日本"}', "utf8bom");
deepEqual(fs.readJSON("bom.json"), { a: "日本" });
fs.writeText("sjis.json", '{"a":"表示"}', "cp932");
deepEqual(fs.readJSON("sjis.json"), { a: "表示" });
fs.writeJSON("out_sjis.json", { a: "表示" }, { encoding: "cp932" });
equal(fs.readBytes("out_sjis.json").byteLength, 18);

// Handles, Files and Blobs as sources.
deepEqual(fs.readJSON(fs.file("bom.json")), { a: "日本" });
deepEqual(fs.readJSON(await fs.file("bom.json").getFile()), { a: "日本" });
deepEqual(fs.readJSON(new Blob(['{"x":1}'])), { x: 1 });

// JSON Lines.
fs.writeJSONL("rows.jsonl", [{ id: 1 }, { id: 2, d: new Date(2026, 0, 1) }]);
fs.writeJSONL("rows.jsonl", [{ id: 3 }], { append: true });
const rows = fs.readJSONL("rows.jsonl", { dates: true });
equal(rows.length, 3);
ok(rows[1].d instanceof Date);
equal(rows[2].id, 3);

// Round trip with a worksheet.
const wb = workbooks.create();
wb.addSheet("s").write(fs.readJSONL("rows.jsonl", { dates: true }));
ok(wb.sheet(0).getValue("B3") instanceof Date, "revived dates become date cells");

// Parse errors are reported in Japanese.
fs.writeText("bad.json", "{oops}");
throws(() => fs.readJSON("bad.json"), /JSON|oops|予期/);

// ES module import of JSON works through the bundler.
console.log("json ok");
