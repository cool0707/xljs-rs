// フォームで集計の条件を入力し、フォルダ内の Excel ファイルを集計する。
// フォームは実行後も開いたままで、進捗とログを表示する（ネイティブ UI の場合）。
// 前回の入力内容は remember: true で次回の初期値になる。
const values = await ui.form({
  title: "売上集計",
  icon: "chart-column", // Font Awesome のアイコン（ページ先頭にタイトルと表示）
  description: "フォルダ内（サブフォルダ含む）の Excel ファイルの「明細」シートを顧客ごとに集計します。",
  fields: [
    { name: "src", label: "集計フォルダ", type: "folder", required: true, icon: "folder-open" },
    { name: "out", label: "出力先", icon: "file-excel", type: "saveFile", accept: [".xlsx"], default: "集計結果.xlsx", required: true },
    { name: "key", label: "集計単位", type: "radio", options: ["顧客名", "商品名"], default: "顧客名" },
    { name: "from", label: "対象期間（開始）", type: "date", help: "空欄なら全期間" },
    { name: "details", label: "明細シートも出力する", type: "checkbox", default: true },
  ],
  submit: "集計",
  submitIcon: "play",
  remember: true,
  validate(v) {
    if (v.out && !v.out.name.toLowerCase().endsWith(".xlsx")) return { out: "拡張子は .xlsx にしてください" };
  },
  async onSubmit(v, ctx) {
    const files = v.src.walk("*.xlsx").toArray();
    ctx.log(`${files.length} ファイルが見つかりました`);
    /** @type {any[]} */
    const rows = [];
    // 中止ボタンが押されると onSubmit はここで終わる（以降の保存は行わない）
    for (const h of ctx.withProgress(files)) {
      try {
        const book = workbooks.open(h);
        const sheet = book.sheets["明細"] ?? book.sheets[0];
        for (const r of sheet.toObjects()) {
          if (v.from && r.日付 instanceof Date && r.日付 < v.from) continue;
          rows.push({ ...r, ファイル: h.relativePath });
        }
        book.close();
      } catch (e) {
        ctx.error(`${h.relativePath}: ${e.message}`);
      }
    }
    const groups = Object.groupBy(rows, (r) => r[v.key] ?? "(未設定)");
    const summary = Object.entries(groups).map(([name, list]) => ({
      [v.key]: name,
      件数: list.length,
      合計: list.reduce((s, r) => s + (Number(r.金額) || 0), 0),
    }));
    const out = workbooks.create();
    out.addSheet("集計").write(summary, { table: true });
    if (v.details) out.addSheet("明細").write(rows, { table: true });
    out.save(v.out);
    out.close();
    ctx.log(`${rows.length} 件を集計して保存しました: ${v.out.path}`);
    return v.out; // 完了後に「フォルダを開く」ボタンを表示
  },
});
if (!values) console.log("キャンセルされました");
