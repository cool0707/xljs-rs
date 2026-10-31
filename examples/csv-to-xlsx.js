// CSV（Shift_JIS / UTF-8 自動判定）を選んで Excel ブックに変換する。
// 複数選んだ場合はファイルごとにシートを分ける。
const handles = await showOpenFilePicker({
  multiple: true,
  types: [{ description: "CSV / TSV", accept: [".csv", ".tsv", ".txt"] }],
});
if (handles.length === 0) exit();

const book = workbooks.create();
for (const h of handles) {
  const rows = csv.read(h); // 数値・日付は自動で変換される（"00123" のようなコードは文字列のまま）
  const name = path.basename(h.name, path.extname(h.name)).slice(0, 31);
  book.addSheet(name).write(rows);
  console.log(`${h.name}: ${rows.length} 行`);
}

const first = handles[0];
const dest = await showSaveFilePicker({
  suggestedName: path.basename(first.name, path.extname(first.name)) + ".xlsx",
  startIn: first,
  types: [{ description: "Excel ブック", accept: [".xlsx"] }],
});
if (dest) {
  book.save(dest);
  if (confirm("保存しました。開きますか？")) shell.openPath(dest);
}
