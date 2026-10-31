// ==xljs==
// @mode   app
// @html   ui/index.html
// @assets ui
// @title  アプリのテスト
// @size   640x480
// ==/xljs==
import { ok, equal, throws } from "./_assert.js";

// @mode app opens the main window before the script runs. The mock UI
// closes every window right away (see engine_test.go for the log).
ok(app.main, "main window");
equal(xljs.title, "アプリのテスト");
await app.main.closed;
app.main.close(); // closing again is harmless

const sub = app.open({ html: "ui/sub page.html", title: "詳細", width: 300, height: 200 });
await sub.closed;
equal(app.open().closed instanceof Promise, true, "empty page");
throws(() => app.open({ html: "../outside.html" }), /相対パス/);
throws(() => app.open("ui/missing.html"), /見つかりません/);
throws(() => app.open("app.test.js"), /見つかりません/, "outside @assets");
console.log("app ok");
