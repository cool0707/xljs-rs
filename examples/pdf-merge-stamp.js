// 複数の PDF を 1 つにまとめ、表紙を付けてページ番号と「社外秘」の透かしを入れる。
// exe にドロップした（または引数で渡した）PDF、なければ選んだ PDF が対象（ファイル名の順に結合）。
// Excel や Acrobat は不要。文字のフォントは使った文字だけを PDF に埋め込む。
let files = xljs.args.map((p) => fs.file(p));
if (files.length === 0) {
  files = await showOpenFilePicker({ multiple: true, types: [{ description: "PDF", accept: [".pdf"] }] });
}
if (files.length === 0) exit();
files.sort((a, b) => a.name.localeCompare(b.name, "ja"));

const doc = pdf.merge(files);

// 表紙: 先頭に白紙のページ（A4 縦）を入れて、タイトルと日付を書く。
doc.insertBlankPage(1, { paper: "A4" });
doc.addText(1, "資料一式", { y: 100, align: "center", size: 28 });
doc.addText(1, files.map((f) => f.name).join("\n"), { y: 130, align: "center", size: 11, color: "#555" });
doc.addText(1, dayjs().format("YYYY年M月D日"), { position: "bottom-center", margin: 30 });

// ページ番号（表紙を除く）と透かし。
doc.addText("2-", (p) => `${p.number - 1} / ${p.count - 1}`, { position: "bottom-center", margin: 8, size: 9 });
doc.addText("2-", "社外秘", { position: "top-right", margin: 8, size: 12, color: "red" });
// 透かしはページの中央に（文字の中心で回転）。
doc.addText("all", "CONFIDENTIAL", { position: "center", size: 60, color: "gray", opacity: 0.15, rotate: 30 });

// 不要なページの削除・向きの修正の例（必要なら）:
// doc.removePages("3,5-6");
// doc.rotatePages(4, 90);

const dest = await showSaveFilePicker({ suggestedName: "結合.pdf" });
if (dest) {
  doc.save(dest);
  console.log("作成:", dest.path, `${doc.pageCount} ページ`);
  shell.showItemInFolder([dest.path]);
}
doc.close();
