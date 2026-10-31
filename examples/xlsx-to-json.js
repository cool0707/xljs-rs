// Excel の先頭シートを JSON（と JSON Lines）に書き出す。
// 日付はローカル時刻（例 2026-04-01T09:00:00+09:00）で出力される。
const [h] = await showOpenFilePicker({ types: [{ description: "Excel", accept: [".xlsx", ".xlsm"] }] });
if (!h) exit();

const rows = workbooks.open(h).sheet(0).toObjects();
const base = path.join(path.dirname(h), path.basename(h.name, path.extname(h.name)));
fs.writeJSON(base + ".json", rows);
fs.writeJSONL(base + ".jsonl", rows);

// 逆方向: JSON → Excel（dates: true で日時文字列を Date に戻す）
const back = fs.readJSON(base + ".json", { dates: true });
const out = workbooks.create();
out.addSheet("from-json").write(back);
out.save(base + ".from-json.xlsx");

shell.showItemInFolder([base + ".json", base + ".jsonl", base + ".from-json.xlsx"]);
