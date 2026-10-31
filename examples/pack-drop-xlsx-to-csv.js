// exe にドロップした（または引数で渡した）Excel ファイルの先頭シートを
// Shift_JIS の CSV に変換して同じフォルダに保存する。
// `xljs pack pack-drop-xlsx-to-csv.js` で exe にしておくとドラッグ＆ドロップで使える。
let targets = xljs.args.map((p) => fs.file(p));
if (targets.length === 0) {
  targets = await showOpenFilePicker({ multiple: true, types: [{ description: "Excel", accept: [".xlsx", ".xlsm"] }] });
}
const created = [];
for (const h of ui.withProgress(targets, "CSV に変換中")) {
  const book = workbooks.open(h);
  const rows = book.sheet(0).toArrays({ text: true }); // 表示どおりの文字列で出力
  const dest = path.join(path.dirname(h), path.basename(h.name, path.extname(h.name)) + ".csv");
  csv.write(dest, rows, { encoding: "cp932" });
  book.close();
  console.log("作成:", dest);
  created.push(dest);
}
// 作成した CSV をまとめて選択した状態でエクスプローラーを開く
if (created.length) shell.showItemInFolder(created);
