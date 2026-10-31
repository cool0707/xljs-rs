// ==xljs==
// @mode   app
// @html   ui/bridge.html
// @assets ui
// @title  連携のテスト
// ==/xljs==
// The mock UI plays the page (bridge.test.answers.json); engine_test.go
// checks its log.
import { ok, equal, deepEqual, throws } from "./_assert.js";

export function add(a, b) { return a + b; }
export async function info(d, file, dir, bytes) {
  ok(d instanceof Date && file instanceof FileSystemFileHandle && dir instanceof FileSystemDirectoryHandle);
  return { year: d.getFullYear(), file: file.name, dir: dir.kind, size: new Uint8Array(bytes).length };
}
export function fail() { throw new RangeError("だめです"); }
export function whoami() { return this === app.main; }
export const notCallable = 1;

app.handle("dynamic", (x) => x * 2);
throws(() => app.handle("add", () => 0), /export/);
throws(() => app.handle("x", 1), /関数/);

const win = app.main;
win.state.progress = 0;
win.state = { message: "準備中", when: new Date(2026, 3, 1, 9, 0) };
equal(win.state.message, "準備中");
win.$("#title").text("見出し").addClass("big");
win.$("#run").on("click", async (e) => {
  equal(e.type, "click");
  win.state.progress = 100;
  win.emit("done", { ok: true });
  console.log("render ->", await win.invoke("render", [1, 2]));
});
win.on("hello", (name) => console.log("hello", name));
// Errors in handlers are shown, and the script goes on.
win.on("boom", () => { throw new Error("ハンドラのエラー"); });
win.on("state", (key, value) => console.log("state", key, value));

// Reads are answered by the page (null from the mock).
equal(await win.$("#title").text(), null);
equal(await win.evaluate((a) => a + 1, 1), null);
equal(await win.evaluate("document.title"), null);

await win.closed;
deepEqual(win.state.month, "2026-04");
win.state.after = 1; // ignored
win.$("#title").text("x");
await win.invoke("render").then(() => ok(false, "resolved"), (e) => ok(/閉じられて/.test(e.message), e.message));
console.log("bridge ok");
