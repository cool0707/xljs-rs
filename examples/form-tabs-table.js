// タブと表のあるフォーム。
// 「入力」「オプション」「結果」の 3 つのタブに項目を分け、対象の支店は表から選ぶ。
// 実行すると「結果」タブの表に集計結果を表示する（ctx.table）。
const branches = [
  { 支店: "東京", コード: "T01", 担当: "山田" },
  { 支店: "大阪", コード: "O01", 担当: "佐藤" },
  { 支店: "名古屋", コード: "N01", 担当: "鈴木" },
  { 支店: "福岡", コード: "F01", 担当: "高橋" },
];

const values = await ui.form({
  title: "支店別集計",
  description: "フォルダ内の Excel ファイルの「明細」シートを、選んだ支店について集計します。",
  // タブの上に常に表示する項目
  fields: [{ name: "src", label: "集計フォルダ", type: "folder", required: true }],
  tabs: [
    {
      label: "入力",
      fields: [
        // 行をクリックして選ぶ表。値は選んだ行（元のオブジェクト）の配列
        { name: "targets", label: "対象の支店", type: "table", rows: branches, selectable: true, required: true, default: branches },
      ],
    },
    {
      label: "オプション",
      fields: [
        { name: "key", label: "集計単位", type: "radio", options: ["顧客名", "商品名"], default: "顧客名" },
        { name: "from", label: "対象期間（開始）", type: "date", help: "空欄なら全期間" },
      ],
    },
    {
      label: "結果",
      // selectable なしの表は表示だけ（値には入らない）
      fields: [{ name: "summary", label: "集計結果", type: "table", rows: [], columns: ["支店", "件数", "合計"], height: 6 }],
    },
  ],
  submit: "集計",
  async onSubmit(v, ctx) {
    const codes = new Set(v.targets.map((b) => b.支店));
    const files = v.src.walk("*.xlsx").toArray();
    /** @type {any[]} */
    const rows = [];
    for (const h of ctx.withProgress(files)) {
      try {
        const book = workbooks.open(h);
        const sheet = book.sheets["明細"] ?? book.sheets[0];
        for (const r of sheet.toObjects()) {
          if (!codes.has(r.支店)) continue;
          if (v.from && r.日付 instanceof Date && r.日付 < v.from) continue;
          rows.push(r);
        }
        book.close();
      } catch (e) {
        ctx.error(`${h.relativePath}: ${e.message}`);
      }
    }
    const summary = v.targets.map((b) => {
      const list = rows.filter((r) => r.支店 === b.支店);
      return { 支店: b.支店, 件数: list.length, 合計: list.reduce((s, r) => s + (Number(r.金額) || 0), 0) };
    });
    ctx.table("summary", summary); // 「結果」タブの表を更新
    ctx.log(`${rows.length} 件を集計しました（結果タブに表示）`);
  },
});
if (!values) console.log("キャンセルされました");
