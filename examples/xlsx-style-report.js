// 書式付きの帳票を作るサンプル。
// フォント・塗りつぶし・罫線・配置・表示形式・セル結合・ウィンドウ枠の固定・
// フィルター・列幅の自動調整を一通り使っている。
const sales = [
  { 日付: new Date(2026, 3, 1), 支店: "東京", 担当: "山田", 商品: "ノートPC", 数量: 3, 単価: 128000 },
  { 日付: new Date(2026, 3, 1), 支店: "大阪", 担当: "佐藤", 商品: "モニター", 数量: 5, 単価: 32800 },
  { 日付: new Date(2026, 3, 2), 支店: "東京", 担当: "鈴木", 商品: "キーボード", 数量: 12, 単価: 6980 },
  { 日付: new Date(2026, 3, 3), 支店: "名古屋", 担当: "田中", 商品: "ノートPC", 数量: 1, 単価: 128000 },
  { 日付: new Date(2026, 3, 3), 支店: "大阪", 担当: "佐藤", 商品: "マウス", 数量: 20, 単価: 2480 },
  { 日付: new Date(2026, 3, 4), 支店: "東京", 担当: "山田", 商品: "モニター", 数量: 2, 単価: 32800 },
].map((r) => ({ ...r, 金額: r.数量 * r.単価 }));

const book = workbooks.create(); // 日本語環境では既定フォントが游ゴシックになる

// ---- 明細シート: write() のオプションだけで表を整える ----
const detail = book.addSheet("明細");
detail.write(sales, {
  table: true,                               // 見出しの太字・塗り・罫線、列幅自動、先頭行固定、フィルター
  columnStyles: {
    日付: { format: "yyyy/m/d", align: "center" },
    数量: { format: "#,##0" },
    単価: { format: "#,##0" },
    金額: { format: "#,##0", bold: true },
  },
});

// ---- 集計シート: セル単位・範囲単位で書式を指定する ----
const summary = book.addSheet("支店別集計");
summary.setValue("A1", "2026年4月 支店別売上");
summary.merge("A1:D1");
summary.style("A1:D1", {
  font: { size: 16, bold: true, color: "#FFFFFF" },
  fill: "#305496",
  alignment: { horizontal: "center", vertical: "center" },
});
summary.setRowHeight(1, 30);

const byBranch = Object.entries(Object.groupBy(sales, (r) => r.支店)).map(([支店, list]) => ({
  支店,
  件数: list.length,
  数量: list.reduce((s, r) => s + r.数量, 0),
  金額: list.reduce((s, r) => s + r.金額, 0),
}));
summary.write(byBranch, {
  start: "A3",
  headerStyle: { bold: true, background: "#D9E1F2", align: "center" },
  columnStyles: { 金額: { format: "¥#,##0" }, 数量: { format: "#,##0" } },
});

// 合計行（数式で合計）
const last = 3 + byBranch.length; // 最終データ行
const total = last + 1;
summary.setValue(`A${total}`, "合計");
summary.cell(`B${total}`).formula = `=SUM(B4:B${last})`;
summary.cell(`C${total}`).formula = `=SUM(C4:C${last})`;
summary.cell(`D${total}`).formula = `=SUM(D4:D${last})`;
summary.style(`D4:D${total}`, { format: "¥#,##0" });

// 罫線: 表全体を外枠（中線）＋内側（細い灰色線）で囲んでから、
// 見出しの下を中線、合計行の上を二重線にする（後から指定したものが優先）
summary.range(`A3:D${total}`).border({ outline: "medium", inside: { style: "thin", color: "#BFBFBF" } });
summary.style("A3:D3", { border: { bottom: "medium" } });
summary.style(`A${total}:D${total}`, { bold: true, background: "#FFF2CC", border: { top: "double" } });

// 注記: 斜体・灰色・折り返し
summary.setValue(`A${total + 2}`, "※ 金額は税抜。数式は Excel で開いたときに再計算されます。");
summary.merge(`A${total + 2}:D${total + 3}`);
summary.style(`A${total + 2}`, { italic: true, color: "#7F7F7F", wrap: true, valign: "top" });

summary.setColumnWidth("A", 14).setColumnWidth("B:C", 10).setColumnWidth("D", 16);

// ---- 保存してエクスプローラーで表示 ----
const dest = await showSaveFilePicker({
  suggestedName: "売上レポート.xlsx",
  types: [{ description: "Excel ブック", accept: [".xlsx"] }],
});
if (dest) {
  book.save(dest);
  shell.showItemInFolder(dest);
}
