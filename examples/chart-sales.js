// Excel ブックのグラフをそのまま表示し、別のシートの表からもグラフを作る。
// グラフの定義（ChartSpec）は ui.chart・フォームの chart 項目・画面の xljs.chart で共通。
const [file] = await showOpenFilePicker({ types: [{ description: "Excel", accept: [".xlsx", ".xlsm"] }] });
if (!file) exit();
const book = workbooks.open(file);

// シート上の Excel のグラフ。toSpec() で ChartSpec になる（値はセルから読む）
const charts = book.sheets.flatMap((s) => s.charts);
for (const c of charts) {
  if (!c.supported) {
    console.log(c.name, c.warnings[0]); // レーダーチャートなど、表示できないもの
    continue;
  }
  if (c.warnings.length) console.log(c.name, c.warnings.join(" / "));
  ui.chart(c, `${c.sheet.name} - ${c.name}`);
}

// 表からグラフを作る（行オブジェクトの配列。列はキー名で指定）
const rows = book.sheets[0].rows();
if (rows.length && Object.keys(rows[0]).length >= 2) {
  const [x, ...ys] = Object.keys(rows[0]);
  const numeric = ys.filter((k) => rows.some((r) => typeof r[k] === "number"));
  if (numeric.length) {
    const view = ui.chart({
      type: "column",
      data: rows,
      x,
      // 2 つ目以降の系列は折れ線にして第 2 軸へ（組み合わせグラフ）
      y: numeric.map((k, i) => (i === 0 ? k : { key: k, type: "line", axis: "y2" })),
      title: book.sheets[0].name,
      format: "#,##0",
      onClick(row, info) {
        alert(`${row[x]}: ${info.series} = ${row[info.series]}`);
      },
    });
    await view.closed;
  }
}
