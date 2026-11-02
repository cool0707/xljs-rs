import { ok, equal, deepEqual, throws } from "./_assert.js";

// ---- local database
const db = sqlite.open("local.db");
equal(db.shared, false);
db.exec("CREATE TABLE item(id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL, sold DATE, at DATETIME, ok BOOLEAN, data BLOB)");
const r = db.run("INSERT INTO item(name, price, sold, at, ok, data) VALUES(?, ?, ?, ?, ?, ?)",
  "ねじ", 12.5, new Date(2026, 3, 1), new Date(2026, 3, 1, 9, 30), true, new Uint8Array([1, 2, 3]));
deepEqual(r, { changes: 1, lastInsertRowid: 1 });
db.run("INSERT INTO item(name, price) VALUES(:name, $price)", { name: "ナット", price: 3 });
db.run("INSERT INTO item(name) VALUES(?)", [dayjs("2026-05-01").toDate().getFullYear()]);
const first = db.get("SELECT * FROM item WHERE id = ?", 1);
equal(first.name, "ねじ");
ok(first.sold instanceof Date && first.sold.getTime() === new Date(2026, 3, 1).getTime(), "DATE column as Date");
equal(first.at.getTime(), new Date(2026, 3, 1, 9, 30).getTime());
equal(first.ok, true);
ok(first.data instanceof Uint8Array);
deepEqual(Array.from(first.data), [1, 2, 3]);
equal(db.get("SELECT sold FROM item WHERE id = 1 AND sold = '2026-04-01'").sold.getDate(), 1, "dates are stored as text");
equal(db.get("SELECT * FROM item WHERE id = 99"), null);
deepEqual(db.values("SELECT id, name FROM item ORDER BY id"), [[1, "ねじ"], [2, "ナット"], [3, "2026"]]);
deepEqual(db.all("SELECT 1 AS x, 2 AS x"), [{ x: 1, x_2: 2 }]);
equal(typeof db.get("SELECT 9007199254740993 AS n").n, "bigint");
throws(() => db.run("INSERT INTO item(name) VALUES(?)"), /パラメータは 1 個/);
throws(() => db.run("INSERT INTO item(price) VALUES(1)"), /NOT NULL/);
throws(() => db.all("SELECT * FROM missing_t"), /テーブル missing_t がありません/);

// transactions
throws(() => db.transaction(() => {
  db.run("DELETE FROM item");
  throw new Error("やめる");
}), /やめる/);
equal(db.get("SELECT count(*) AS n FROM item").n, 3, "rolled back");
equal(db.transaction(() => {
  db.run("INSERT INTO item(name) VALUES('a')");
  try {
    db.transaction(() => { db.run("INSERT INTO item(name) VALUES('b')"); throw new Error("inner"); });
  } catch (_) {}
  return "done";
}), "done");
deepEqual(db.values("SELECT name FROM item WHERE id > 3"), [["a"]]);
throws(() => db.transaction(async () => {}), /async/);
equal(db.inTransaction, false);

// insert with a new table typed from the values
const n = db.insert("sales", [
  { 日付: new Date(2026, 3, 1), 品名: "ねじ", 数量: 10, 単価: 1.5 },
  { 日付: new Date(2026, 3, 2), 品名: "ボルト", 数量: 3 },
]);
equal(n, 2);
deepEqual(db.columns("sales").map((c) => [c.name, c.type]), [["日付", "DATE"], ["品名", "TEXT"], ["数量", "INTEGER"], ["単価", "REAL"]]);
const sales = db.all("SELECT * FROM sales ORDER BY 日付");
ok(sales[1].日付 instanceof Date);
equal(sales[1].単価, null);
db.insert("sales", [[new Date(2026, 3, 3), "座金", 1, 0.5]]);
equal(db.get("SELECT sum(数量) AS s FROM sales").s, 14);
deepEqual(db.tables, ["item", "sales"]);
deepEqual(db.check(), { ok: true, problems: [] });
db.backup("copy.db");
db.close();
throws(() => db.all("SELECT 1"), /閉じられています/);
const copy = sqlite.open("copy.db", { readonly: true });
equal(copy.get("SELECT count(*) AS n FROM sales").n, 3);
copy.close();
equal(sqlite.check("copy.db").ok, true);

// ---- shared database
const a = sqlite.open("shared.db", { shared: true });
const b = sqlite.open("shared.db", { timeout: 100 });
ok(a.shared && b.shared, "the second open sees the shared folder");
ok(fs.isDirectory("shared.db.xljs-shared"));
a.exec("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)");
a.insert("t", Array.from({ length: 300 }, (_, i) => ({ id: i + 1, v: "x".repeat(200) })));
a.run("UPDATE t SET v = 'last' WHERE id = 300");
equal(sqlite.backups("shared.db").length, 1);
ok(sqlite.backups("shared.db")[0].time instanceof Date);
a.transaction(() => {
  throws(() => b.all("SELECT * FROM t"), /他の人が使用中/);
});
equal(b.get("SELECT count(*) AS n FROM t").n, 300);
b.close();
a.close();

// Damage the file: the next call asks (the mock says yes), repairs from
// the backup and the change log, and runs again.
const bytes = new Uint8Array(fs.readBytes("shared.db"));
for (let i = 4096; i < bytes.length - 4096; i++) bytes[i] = (i * 7) & 255;
fs.writeBytes("shared.db", bytes);
equal(sqlite.check("shared.db").ok, false);
const c = sqlite.open("shared.db");
equal(c.get("SELECT v FROM t WHERE id = 300").v, "last");
equal(c.get("SELECT count(*) AS n FROM t").n, 300);
c.close();
equal(sqlite.check("shared.db").ok, true);
equal(sqlite.repair("shared.db").status, "ok");
ok(fs.list("shared.db.xljs-shared/broken").length === 1, "the damaged file is kept");
console.log("sqlite ok");
