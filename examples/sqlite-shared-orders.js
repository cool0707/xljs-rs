// ファイルサーバー上の SQLite データベースに、Excel の受注一覧を取り込んで集計する。
// 複数の人が同じデータベースを使っても壊れにくい「共有モード」で開く
// （ネットワークドライブや \\server\share 上なら自動で共有モード）。
// 万一壊れたときは、確認のダイアログが出て、自動バックアップと変更の記録から復旧できる。
const dbPath = xljs.args[0] || "受注.db";
const db = sqlite.open(dbPath, { shared: true });

db.exec(`CREATE TABLE IF NOT EXISTS 受注 (
  伝票番号 TEXT PRIMARY KEY,
  受注日 DATE NOT NULL,
  得意先 TEXT NOT NULL,
  金額 INTEGER NOT NULL,
  登録者 TEXT
)`);

const [h] = await showOpenFilePicker({ types: [{ description: "Excel", accept: [".xlsx", ".xlsm"] }] });
if (h) {
  const rows = workbooks.open(h).sheet(0).toObjects();
  const user = xljs.env("USERNAME") || "";
  // 1 回のトランザクションで取り込む（同じ伝票番号は置き換える）
  const n = db.insert("受注", rows.map((r) => ({
    伝票番号: String(r["伝票番号"]),
    受注日: r["受注日"],
    得意先: r["得意先"],
    金額: r["金額"],
    登録者: user,
  })), { replace: true });
  console.log(`${n} 件を取り込みました`);
}

// 月別・得意先別の集計
const summary = db.all(`
  SELECT strftime('%Y-%m', 受注日) AS 月, 得意先, count(*) AS 件数, sum(金額) AS 金額
  FROM 受注 GROUP BY 月, 得意先 ORDER BY 月, 金額 DESC`);
await ui.table(summary, { title: "月別・得意先別の受注" });

// 自動バックアップの一覧（共有モード）
for (const b of db.backups()) console.log(`バックアップ ${b.generation}: ${b.time.toLocaleString()}`);
db.close();
