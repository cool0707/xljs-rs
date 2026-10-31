// フォルダ内の Excel ファイル（サブフォルダ含む）の「明細」シートを集計し、
// 顧客ごとの件数・合計を新しいブックに書き出す。
const dir = await showDirectoryPicker({ title: "集計するフォルダを選択" });
if (!dir) exit();

const files = dir.walk("*.xlsx").toArray();
if (files.length === 0) {
  alert("Excel ファイルが見つかりませんでした");
  exit();
}

/** @type {any[]} */
const rows = [];
const errors = [];
// キャンセルされるとスクリプトはここで終わる（途中までの結果は保存しない）
for (const h of ui.withProgress(files, { title: "集計中" })) {
  try {
    const book = workbooks.open(h);
    const sheet = book.sheets["明細"] ?? book.sheets[0];
    for (const r of sheet.toObjects()) rows.push({ ...r, ファイル: h.relativePath });
    book.close();
  } catch (e) {
    errors.push(`${h.relativePath}: ${e.message}`);
  }
}

const byCustomer = Object.groupBy(rows, (r) => r.顧客名 ?? "(未設定)");
const summary = Object.entries(byCustomer).map(([name, list]) => ({
  顧客名: name,
  件数: list.length,
  合計: list.reduce((s, r) => s + (Number(r.金額) || 0), 0),
}));

const dest = await showSaveFilePicker({
  suggestedName: "集計結果.xlsx",
  startIn: dir,
  types: [{ description: "Excel ブック", accept: [".xlsx"] }],
});
if (!dest) exit();

const out = workbooks.create();
out.addSheet("集計").write(summary).setColumnWidth("A", 24);
out.addSheet("明細").write(rows);
if (errors.length) out.addSheet("エラー").write(errors.map((e) => [e]));
out.save(dest);

alert(`${files.length} ファイル、${rows.length} 件を集計しました` + (errors.length ? `\n（読めなかったファイル: ${errors.length}）` : ""));
shell.showItemInFolder(dest); // 保存したファイルをエクスプローラーで選択表示
