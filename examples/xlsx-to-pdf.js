// Excel ファイルを PDF に変換して同じフォルダに保存する（Excel で印刷したときと同じページ設定で）。
// exe にドロップした（または引数で渡した）ファイル、なければ選んだファイルが対象。
// 用紙・向き・余白・拡大縮小・印刷範囲・印刷タイトル・改ページ・ヘッダー/フッターはブックの設定に従い、グラフも描く。
// 設定を変えたいときは toPdf の 2 つ目の引数で（例: { paper: "A4", landscape: true, fitToWidth: 1 }）。
let targets = xljs.args.map((p) => fs.file(p));
if (targets.length === 0) {
  targets = await showOpenFilePicker({ multiple: true, types: [{ description: "Excel", accept: [".xlsx", ".xlsm"] }] });
}
const created = [];
for (const h of ui.withProgress(targets, "PDF に変換中")) {
  const book = workbooks.open(h);
  const dest = path.join(path.dirname(h), path.basename(h.name, path.extname(h.name)) + ".pdf");
  // 非表示でないすべてのシートを 1 つの PDF に。シートを選ぶなら { sheets: ["請求書"] }
  await book.toPdf(dest);
  book.close();
  console.log("作成:", dest);
  created.push(dest);
}
if (created.length) shell.showItemInFolder(created);
