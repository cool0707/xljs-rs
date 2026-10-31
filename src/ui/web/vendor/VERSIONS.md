# 同梱しているライブラリ

| フォルダ | ライブラリ | 版 | ライセンス | 取得元（npm） |
|---|---|---|---|---|
| `ag-charts/` | AG Charts Community | 14.2.0 | MIT | `ag-charts-community/dist/umd/ag-charts-community.min.js` |
| `ag-charts/ag-charts-locale-ja-JP.js` | AG Charts のロケール（ja-JP） | 14.2.0 | MIT | `ag-charts-locale/dist/package/ja-JP.cjs.js` の `AG_CHARTS_LOCALE_JA_JP` を `window` に置く形に書き出したもの |
| `frappe-gantt/` | Frappe Gantt | 1.2.2 | MIT | `frappe-gantt/dist/frappe-gantt.umd.js`、`frappe-gantt.css` |

- AG Charts は **Community 版だけ**を入れる（Enterprise 版はライセンスキーがないと透かしが出る）。
  Community 版の系列: bar, line, area, scatter, bubble, histogram, pie, donut。
- 更新するときは同じ版の 3 パッケージ（`ag-charts-community` と `ag-charts-locale`、`frappe-gantt`）をそろえて差し替え、
  `src/ui/web/chart.js` の変換（オプション名）を見直す。ロケールの書き出し:

```
node -e 'const m=require("./ja-JP.cjs.js");require("fs").writeFileSync("ag-charts-locale-ja-JP.js",
  "window.AG_CHARTS_LOCALE_JA_JP = "+JSON.stringify(m.AG_CHARTS_LOCALE_JA_JP)+";\n")'
```
