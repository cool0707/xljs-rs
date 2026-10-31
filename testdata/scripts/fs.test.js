import { ok, equal, deepEqual, rejects, throws } from "./_assert.js";

// Tests run in an empty temporary directory.
fs.mkdir("data/sub/deep");
fs.mkdir("data/skip");
fs.writeText("data/a.xlsx", "x");
fs.writeText("data/B.XLSX", "x");
fs.writeText("data/~$a.xlsx", "lock");
fs.writeText("data/note.txt", "x");
fs.writeText("data/sub/c.xlsx", "x");
fs.writeText("data/sub/deep/d.xlsx", "x");
fs.writeText("data/skip/e.xlsx", "x");
fs.mkdir("data/.hidden");
fs.writeText("data/.hidden/h.xlsx", "x");
fs.writeText("data/.dot.txt", "x");

const dir = fs.dir("data");
ok(dir instanceof FileSystemDirectoryHandle);
equal(dir.kind, "directory");
equal(dir.name, "data");
ok(path.isAbsolute(dir.path));

// walk: patterns without "/" match the base name at any depth, case-insensitively.
const rel = (list) => list.map((h) => h.relativePath.replaceAll("\\", "/")).sort();
deepEqual(rel(dir.walk({ glob: "*.xlsx", exclude: "skip" }).toArray()),
  ["B.XLSX", "a.xlsx", "sub/c.xlsx", "sub/deep/d.xlsx"]);
// Hidden entries ("." names, Office lock files "~$*", hidden/system
// attributes on Windows) and hidden folders' contents are left out by default.
deepEqual(rel(dir.walk({ glob: "*.xlsx", exclude: "sub", hidden: true }).toArray()),
  [".hidden/h.xlsx", "B.XLSX", "a.xlsx", "skip/e.xlsx", "~$a.xlsx"]);
equal(dir.walk({ hidden: true }).length, 9);
deepEqual(fs.list("data").sort(), ["B.XLSX", "a.xlsx", "note.txt", "skip", "sub"]);
equal(fs.list("data", { hidden: true }).length, 8);
equal(dir.list({ hidden: true }).length, 8);
equal(fs.glob("data/**/*.xlsx").length, 5);
equal(fs.glob("data/**/*.xlsx", { hidden: true }).length, 7);
deepEqual(rel(dir.walk({ glob: "sub/**/*.xlsx" }).toArray()), ["sub/c.xlsx", "sub/deep/d.xlsx"]);
deepEqual(rel(dir.walk({ glob: "*.xlsx", exclude: "sub/deep" }).toArray()).filter((p) => p.startsWith("sub")), ["sub/c.xlsx"]);
deepEqual(rel(dir.walk({ directories: true, files: false }).toArray()), ["skip", "sub", "sub/deep"]);
equal(dir.walk().length, 6);

// for await and for..of both work on walk().
let n = 0;
for await (const h of dir.walk("*.txt")) { equal(h.name, "note.txt"); n++; }
for (const h of dir.walk("*.txt")) { ok(h instanceof FileSystemFileHandle); n++; }
equal(n, 2);

// entries()/values() list one level.
const names = [];
for await (const [name, h] of dir.entries()) names.push(name + (h.kind === "directory" ? "/" : ""));
ok(names.includes("sub/") && names.includes("a.xlsx") && !names.includes("c.xlsx"));
equal(dir.list().length, 5);

// fs.glob relative to cwd.
deepEqual(rel(fs.glob("data/sub/**/*.xlsx")).map((p) => p.replace(/^data\//, "")), ["sub/c.xlsx", "sub/deep/d.xlsx"]);

// Directory handle API.
const sub = await dir.getDirectoryHandle("sub");
const c = await sub.getFileHandle("c.xlsx");
equal(c.name, "c.xlsx");
deepEqual(await dir.resolve(c), ["sub", "c.xlsx"]);
equal(await dir.resolve(fs.dir(".")), null);
await rejects(() => dir.getFileHandle("missing.txt"), /NotFoundError/);
await rejects(() => dir.getFileHandle("sub"), /TypeMismatchError/);
const created = await dir.getFileHandle("new.txt", { create: true });
ok(created.exists());
await dir.removeEntry("new.txt");
ok(!created.exists());
const nd = await dir.getDirectoryHandle("made", { create: true });
ok(nd.exists());
ok(await nd.isSameEntry(fs.dir("data/made")));

// File and writable stream.
const h = fs.file("data/out.txt");
const w = await h.createWritable();
await w.write("こんにちは");
await w.write(new Uint8Array([0x0a]));
await w.write(new Blob(["blob"]));
await w.close();
const f = await h.getFile();
ok(f instanceof File && f instanceof Blob);
equal(f.name, "out.txt");
equal(await f.text(), "こんにちは\nblob");
equal(f.size, 20);
equal(f.type, "text/plain");
ok(f.lastModified > 0);
const w2 = await h.createWritable({ keepExistingData: true });
await w2.write({ type: "write", position: 0, data: "コ" });
await w2.close();
equal(h.readText(), "コんにちは\nblob");

// Text encodings.
fs.writeText("sjis.txt", "表示テスト", "cp932");
equal(fs.readBytes("sjis.txt").byteLength, 10);
equal(fs.readText("sjis.txt"), "表示テスト"); // auto-detected
equal(fs.readText("sjis.txt", "shift_jis"), "表示テスト");
fs.writeText("bom.txt", "a", "utf8bom");
fs.appendText("bom.txt", "b", "utf8bom"); // BOM only once
equal(fs.readBytes("bom.txt").byteLength, 5);
equal(fs.readText("bom.txt"), "ab");
// .csv / .tsv get a BOM by default (Excel reads them as UTF-8); other files do not.
fs.writeText("hand.csv", "名前\r\n");
fs.appendText("hand.csv", "山田\r\n");
fs.dir(".").file("hand.tsv").writeText("x");
equal(new Uint8Array(fs.readBytes("hand.csv"))[0], 0xef);
equal(new Uint8Array(fs.readBytes("hand.tsv"))[0], 0xef);
equal(fs.readText("hand.csv"), "名前\r\n山田\r\n");
equal(fs.readBytes("data/note.txt").byteLength, 1);
fs.writeText("plain.csv", "a", "utf8");
equal(fs.readBytes("plain.csv").byteLength, 1);
equal(await (await fs.file("sjis.txt").getFile()).text("auto"), "表示テスト");

// fs helpers.
fs.writeJSON("x.json", { a: [1, 2] });
deepEqual(fs.readJSON("x.json"), { a: [1, 2] });
fs.copy("x.json", "y.json");
fs.rename("y.json", "z.json");
ok(fs.exists("z.json") && !fs.exists("y.json"));
ok(fs.isFile("z.json") && fs.isDirectory("data"));
fs.remove("z.json");
ok(!fs.exists("z.json"));
fs.remove("data/made", { recursive: true });
ok(!fs.exists("data/made"));
equal(fs.stat("nope"), null);

// path helpers accept handles.
equal(path.basename(h), "out.txt");
equal(path.extname("a/b.tar.gz"), ".gz");
equal(path.basename("a/b.xlsx", ".xlsx"), "b");
equal(path.join(dir, "x.txt"), path.resolve("data", "x.txt"));
throws(() => path.join({}), /パス/);

console.log("fs ok");
