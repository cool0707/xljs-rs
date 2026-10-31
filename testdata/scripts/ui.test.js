import { ok, equal, deepEqual } from "./_assert.js";

// Answers come from ui.test.answers.json; an exhausted queue means "cancel".
fs.mkdir("in");
fs.writeText("in/a.xlsx", "");
fs.writeText("in/b.csv", "");

const picked = await showOpenFilePicker({ multiple: true, types: [{ description: "Excel", accept: [".xlsx"] }] });
deepEqual(picked.map((h) => h.name), ["a.xlsx", "b.csv"]);
ok(picked[0] instanceof FileSystemFileHandle);
const file = await picked?.[0]?.getFile();
equal(file.name, "a.xlsx");

const single = await showOpenFilePicker(); // multiple: false keeps only the first
equal(single.length, 1);
deepEqual(await showOpenFilePicker(), [], "cancel returns []");

const dir = await showDirectoryPicker();
equal(dir.name, "in");
equal(await showDirectoryPicker(), null, "cancel returns null");

const save = await showSaveFilePicker({ suggestedName: "out.xlsx" });
equal(save.name, "result.xlsx");
equal(await showSaveFilePicker(), null);

equal(confirm("1?"), true);
equal(confirm("2?"), false);
equal(confirm("3?"), false, "exhausted queue");
equal(prompt("name?", "def"), "太郎");
equal(prompt("again?"), null);
alert("hello");
alert({ any: "value" });

// Progress: manual API.
const p = ui.progress({ title: "manual", total: 3 });
p.update(1, "one");
p.increment("two");
equal(p.done, 2);
equal(p.cancelled, false);
p.close();
p.close(); // idempotent

// With onCancel: "break", withProgress over an array just stops when
// cancelled (cancelAfter: 2 updates). By default the script ends
// (TestProgressCancelEndsScript).
const seen = [];
for (const x of ui.withProgress([1, 2, 3, 4, 5], { title: "sync", onCancel: "break" })) seen.push(x);
deepEqual(seen, [1, 2]);
equal(ui.cancelled, true);

// withProgress over an async iterable (walk) completes normally.
const names = [];
for await (const h of ui.withProgress(dir.values(), { title: "async" })) {
  names.push(h.name);
}
deepEqual(names.sort(), ["a.xlsx", "b.csv"]);
equal(ui.cancelled, false);

// Breaking out early closes the indicator.
for (const x of ui.withProgress([1, 2, 3], "break")) { if (x === 1) break; }

// Explorer / default app (the mock UI only logs them).
shell.showItemInFolder(picked);
shell.showInFolder("in/a.xlsx", fs.file("in/b.csv"));
shell.openPath(dir);
shell.open("https://example.com/");
let threw = false;
try { shell.showItemInFolder("in/missing.txt"); } catch (e) { threw = true; }
equal(threw, true, "missing files are reported");
threw = false;
try { shell.openExternal("not a url"); } catch (e) { threw = true; }
equal(threw, true);

console.log("ui ok");
