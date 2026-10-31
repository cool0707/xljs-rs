# グラフ（AG Charts）とガントチャート（Frappe Gantt）

フォームやウィンドウにグラフを出す機能の設計と実装のメモ。描画には 2 つのライブラリを使うが、スクリプトから見た使い方は 1 つにそろえている。
Excel のブックにあるグラフは `sheet.charts` で読み、そのまま表示できる。

## 0. 方針

- **ライブラリ**（`src/ui/web/vendor/`。版と取得元は `vendor/VERSIONS.md`）
  - グラフ: AG Charts Community 14.2.0（MIT）。棒・折れ線・面・散布図・バブル・ヒストグラム・円・ドーナツと、それらの組み合わせ。
    Enterprise 専用の機能（レーダー、ウォーターフォール、範囲棒、ヒートマップ、ズーム／ナビゲーターなど）は**同梱しない**
    （ライセンスキーがないと透かしが出る）。日本語の表示は `ag-charts-locale` の ja-JP。
  - ガントチャート: Frappe Gantt 1.2.2（MIT）。SVG で描画し、バーのドラッグで日付と進捗を変えられる。
- **使い方は 1 つ**: どちらも「グラフの定義（`ChartSpec`）」というただのオブジェクトで表す。`type: "gantt"` なら Frappe Gantt で、それ以外は AG Charts で描く。
- **データは行オブジェクトの配列**: `ui.table` や `sheet.rows()` と同じ形。列は「キー名」で指定する（`x: "月", y: ["売上", "原価"]`）。
- **ワイヤ形式で表せること**: 定義は JSON＋`Date`（`{ $date: ms }` で送る）だけ。関数は `onClick` / `onChange` だけで、スクリプト側で動く
  （ページからは行の番号でイベントが届く）。ほかの関数（`options` に入れたフォーマッタなど）は送れないので落ちる。デバッガ（`--debug`）でも同じ経路。
- **ライブラリの生のオプション**: `options` に書いた値を AG Charts / Frappe Gantt のオプションに深くマージする（JSON で表せるものだけ）。
- **既定の色**は Excel と同じ Office のテーマの色（`#4472C4` `#ED7D31` `#A5A5A5` …）。Excel から変換したグラフは、ブックのテーマとグラフの配色（§3）から Excel と同じ色を求めて使う。
- **逆向き（ChartSpec → Excel のネイティブグラフ）は作らない**（当面不要）。

## 1. 表示できる場所（3 か所で同じ定義を使う）

| 場所 | 書き方 | 更新 | 備考 |
|---|---|---|---|
| 単独のウィンドウ | `ui.chart(spec, options?)` → `ChartView` | `view.update(spec)` | `ui.table` の表示のみと同じく、開いてすぐ戻る。`await view.closed` で閉じるまで待つ |
| フォームの項目 | `{ name, type: "chart", chart: spec, height }` | `ctx.chart(name, spec)` | `ctx.table` と同じ形。`height` は 1 行 25px（既定 12） |
| アプリモードの画面 | 画面側で `await xljs.chart(el, spec)` | 戻り値の `update(spec)` | ハンドラーは画面側で動く。テキストは `init` で受け取る |

- `update` / `ctx.chart` は**指定したキーだけ**を置き換える（`{ data: rows }` で行だけ差し替え）。`ExcelChart` を渡すと丸ごと置き換え。
- フォームの `chart` 項目は値を持たない。`editable` なガントチャートだけ、すべての行（ドラッグの変更が書き込まれた元のオブジェクト）を値にする。
- コンソール・mock ではグラフを描けないので、グラフの元の行を表（`ui.table` の表示のみと同じ出力）で出し、「表で表示します」と 1 行出す。

```js
// 単独のウィンドウ
const rows = book.sheets["集計"].rows();               // [{ 月: "4月", 売上: 120, 原価: 80 }, ...]
ui.chart({ type: "column", data: rows, x: "月", y: ["売上", "原価"], title: "月別売上" });

// フォームの中（実行結果をグラフで見せる）
await ui.form({
  fields: [
    { name: "src", label: "集計フォルダ", type: "folder", required: true },
    { name: "graph", type: "chart", chart: { type: "line", data: [], x: "月", y: "合計" } },
  ],
  async onSubmit(v, ctx) { ctx.chart("graph", { data: aggregate(v.src) }); },
});

// ガントチャート（使い方は同じ。type と列の指定が違うだけ）
ui.chart({
  type: "gantt", data: book.sheets["工程"].rows(),
  name: "作業", start: "開始", end: "終了", progress: "進捗", dependencies: "前提",
  view: "week", editable: true,
  holidays: [new Date(2026, 4, 3), { date: "2026-05-04", name: "みどりの日" }],
  onChange(row, changes) { /* changes = { start, end } / { progress }。row は元のオブジェクト（書き込み済み） */ },
});
```

## 2. ChartSpec

型は `types/xljs.d.ts` の `ChartSpec` / `ChartSeries` / `ChartAxis`。要点:

- **種類**: `column`（縦棒、既定）`bar`（横棒）`line` `area` `scatter` `bubble` `histogram` `pie` `donut` `gantt`。Excel にそろえて column = 縦棒、bar = 横棒。
- **列の既定**: `x` を省くと先頭の列、`y` を省くと `x` 以外の数値の列すべて（円・ドーナツは最初の 1 つ）。
  ガントの列は省くとその名前の列か、日本語の `名前`/`作業`・`開始`/`開始日`・`終了`/`終了日`・`進捗`・`前提` の列。
- **組み合わせ・第 2 軸**: 系列を `{ key, type: "line", axis: "y2" }` にする。積み上げは `stacked: true` / `"percent"`。
- **X の軸の種類**: すべて日付で折れ線・面・散布図なら時間軸、散布図・バブル・ヒストグラムの数値なら数値軸、それ以外は分類軸（日付は書式で文字にする）。
- **書式**: `format`（全体）、`axes.{x,y,y2}.format`、系列の `format` に Excel の書式文字列。ページ側の `src/ui/web/numfmt.js` が
  よく使う書式（区切り・小数・%・指数・`"円"` などの文字・`[$¥-411]`・日付と時刻・`aaa`）を解釈する。解釈できない書式は既定の表示。
  100% 積み上げの値の軸は割合（`0%`）で表示する。
- **ガント**:
  - 終了日は**その日を含む**（時刻のない日付。4/1〜4/3 は 3 日間）。Frappe Gantt も時刻のない終了日を同じく扱う。ドラッグ後の終了日も同じ形で返す。
  - 進捗は 0〜100。すべて 0〜1 なら割合とみなして表示し、変更も割合で書き戻す。
  - 前提（`dependencies`）は `id` 列の値か作業名を「,」「、」区切りで（配列も可）。
  - 休日は `holidays`（`Date`・`"2026-05-03"`・`{ date, name }`）でスクリプトから渡す。土日は既定で色を付ける（`weekends: false` で消す）。
  - `view`（`day` `week` `month` `year`）と、切り替えの選択（`viewSelect: false` で消す）。見出しは `Intl` でページの言語に合わせる。
  - ドラッグ中の変更はページ側で 250ms まとめてから送る（依存する作業が一緒に動いた分も、その作業の変更として届く）。

### ライブラリへの対応（`src/ui/web/chart.js`）

| ChartSpec | AG Charts | Frappe Gantt |
|---|---|---|
| `type` | `series[].type`（`column`→`bar`+`direction:"vertical"`、`bar`→`horizontal`） | `gantt` のみ |
| `data` | 行を `{ __i, x, s0, s1, … }` に詰め替え（キー名に何が来てもよいように） | `tasks`（`id` は行番号、日付は `YYYY-MM-DD[ HH:mm]`） |
| `x` / `y` | `xKey` / `yKey`、系列名は `yName`、`size` は `sizeKey` | `name` / `start` / `end` / `progress` / `dependencies` |
| `stacked` | `stacked: true` / `normalizedTo: 100` | — |
| `axes` | `axes: { x, y, y2 }`（キー付き）、第 2 軸の系列は `yKeyAxis: "y2"` | — |
| `format` | 全体の `formatter`（軸の目盛り・ツールチップ・ラベル） | — |
| `editable` / `holidays` | — | `readonly_dates` / `readonly_progress`、`holidays`（土日は関数で） |
| `onClick` | `listeners.seriesNodeClick` → 行番号を送る | `on_click`（重複して届くので 400ms 以内の同じ作業は無視） |
| `onChange` | — | `on_date_change` / `on_progress_change` |
| 言語 | `locale.localeText`（ja-JP） | `language` と、見出し・ポップアップ・「今日」・表示の選択の文字（`chart.*` のキー） |
| 見た目 | テーマのフォントを游ゴシック UI に、背景は透明（ページの色）、ダークモードは `ag-default-dark` | `chart.css` で CSS 変数を上書き（ダークモードも） |

## 3. Excel のグラフからの変換

```js
const book = workbooks.open("売上.xlsx");
const c = book.sheets["集計"].charts[0];   // ExcelChart（シート上の順）
c.name;          // "グラフ 1"
c.range;         // "F2:N17"（two-cell anchor。one-cell anchor なら左上のセルだけ）
c.type;          // 変換後の種類（表示できなければ null）
c.supported;     // 表示できるか
c.warnings;      // 省いたもの（表示できなければその理由）
c.toSpec();      // ChartSpec（呼ぶたびに新しい行）
ui.chart(c);     // ExcelChart はそのまま渡せる
ui.chart({ ...c.toSpec(), data: newRows });   // 見た目は Excel のまま、行だけ差し替え
```

### 読み方（`src/api/xlsx_chart.rs`、native は `xlsxCharts`）

- umya-spreadsheet のグラフのモデルは、知らない種類（株価・等高線・Excel 2016 以降の chartEx）を落とし、同じ種類のグループを 1 つしか持てない
  （第 2 軸の棒＋棒が消える）ので使わない。ブックを開いたときにグラフの部品（`xl/charts/*`、`xl/drawings/*`、シートとブックの rels、テーマ）
  だけを保持し、`sheet.charts` のときに XML を読む。グラフのないブックでは何も保持しない。
- シート → 描画（drawing）→ アンカー（範囲と名前）→ グラフの XML の順にたどる。グラフは `plotArea` の各グループ（`barChart` など）の系列
  （`c:ser`）の名前・色・参照式・マーカー・近似曲線などと、軸（`catAx` `dateAx` `valAx`）の題名・書式・最小／最大・対数・反転を読む。
- 系列の値は参照式（`'集計'!$B$2:$B$13`、複数の範囲も）のセルから読む（**読んだ時点のセルの値**）。シートがないなどで読めなければグラフ内のキャッシュ。
  日付の書式のセルは `Date`。分類の範囲の 1 つ上（行方向なら 1 つ左）のセルが文字なら、分類の列の名前に使う。
- **色**（`AutoColors` と `color_of`）:
  - 系列・扇に色の指定があればそれ。テーマの色（`schemeClr`）はブックのテーマ（`xl/theme/theme1.xml`）から引く。
    明るさの調整は Office と同じく `lumMod` / `lumOff` を HSL の輝度に掛け、`shade` / `tint` は暗く・明るくする。
  - 色の指定がない系列（自動の色）は、系列の番号（`c:idx`）で Excel 2013 以降と同じ順に選ぶ: グラフの配色
    （`xl/charts/colorsN.xml`。「色の変更」で選んだもの）があればその色の並びと変化（variation）、なければテーマの accent1〜6 と
    既定の変化（`lumMod` 60%、80%＋20%、80%、60%＋40%、50%、70%＋30%、70%、50%＋50%）。7 番目以降は次の変化で繰り返す。
    単色の配色（`meth="withinLinear"` など）も同じ読み方をするので、Excel とは少し違うことがある。
  - 線を消した散布図・折れ線の系列も、マーカーには自動の色を付ける。
  - 円・ドーナツの扇は `c:dPt` の色、なければ自動の色（扇の番号順）を `colors` に入れる。
  - Office 2007 より前の形式のグラフスタイル（`c:style`）だけで決まる色、グラデーションの 2 色目以降、パターン、棒ごとの色（`c:dPt`、円以外）は使わない。
- 行は分類ごとに `{ [分類の列名]: 分類, [系列名]: 値, … }`。系列名がなければ「系列1」…、重複すれば `_2` を付ける。
  散布図で系列ごとに X が違えば、点ごとの行にする（ほかの系列の値は `null`）。バブルの大きさの列は「系列名 (大きさ)」。
- Excel の横棒は最初の分類が下に来るので、軸が反転していなければ行を逆順にする（縦棒は軸が反転していれば逆順）。
- 単一系列で題名の要素があるか自動の題名なら、系列名を題名にする（Excel と同じ）。凡例の位置もそのまま。
- 軸の書式は、軸に書式があり「ソースにリンク」でなければそれ、リンクならその軸の最初の系列の値の書式。100% 積み上げの軸は書式を付けない。

### 種類の対応

| Excel | ChartSpec | 備考 |
|---|---|---|
| 縦棒（集合・積み上げ・100% 積み上げ） | `column` | `stacked: true` / `"percent"` |
| 横棒 | `bar` | 同上。ガントの判定は下記 |
| 3-D 縦棒・横棒・折れ線・面・円 | 平面の同じ種類 | 警告「3-D グラフは平面で表示します」 |
| 折れ線（マーカーあり・なし・積み上げ） | `line` | マーカーがすべてなしなら `markers: false` |
| 面 | `area` | |
| 円・補助円付き円 | `pie` | 補助円は省く（警告）。2 つ目以降の系列も省く（警告） |
| ドーナツ | `donut` | 最初の系列だけ |
| 散布図（マーカーのみ・線付き） | `scatter`（線付きの系列は `type: "line"`） | |
| バブル | `bubble` | |
| 組み合わせ（棒＋折れ線など） | 主の種類＋系列ごとの `type` | 2 つ目の値の軸の系列は `axis: "y2"` |

### Excel にあって表示できないもの

`sheet.charts` には出るが `supported: false` になり、`toSpec()` / `ui.chart(c)` はエラーになる:

| Excel のグラフ | 理由 |
|---|---|
| レーダーチャート | AG Charts では Enterprise 専用 |
| 株価チャート（高値-安値-終値、ローソク足） | AG Charts では Enterprise 専用（ローソク足・OHLC） |
| 等高線グラフ（3-D 等高線含む） | AG Charts にない |
| ウォーターフォール、ツリーマップ、サンバースト、箱ひげ図、じょうご（Excel 2016 以降の chartEx 形式） | AG Charts では Enterprise 専用（ウォーターフォール・ツリーマップ・サンバースト・箱ひげ図・じょうご） |
| ヒストグラム・パレート図（Excel 2016 以降の chartEx 形式） | chartEx は読まない。ヒストグラムは元のデータから `type: "histogram"` で描ける |
| マップ（塗り分けマップ） | AG Charts では Enterprise 専用 |

変換はできるが省くもの（`warnings` に出る）: 3-D の奥行き、補助円、近似曲線、誤差範囲、グラフの下のデータテーブル、円の 2 つ目以降の系列。
警告なしで省くもの: データラベルの細かい設定、グラフ・プロットエリアの塗りや枠、線の太さ・破線、マーカーの形、目盛りの間隔、軸の交差位置、
系列ごとの重なり・間隔、グラフのスタイル（Excel のデザイン）、図形やテキストボックス。

### ガントチャートの判定

Excel にはガントチャートの種類がなく、ふつうは「横棒の積み上げで、1 つ目の系列（開始日）を塗りなしにする」作り方をする。これを見分けて `gantt` にする。

- 条件: 横棒・積み上げ・系列 2 つ以上・グループが 1 つ、**1 つ目の系列が塗りなし**、1 つ目の値が日付（日付のセルか、値の軸・値の書式が日付）。
- 変換: 分類 → 作業名（列名は分類の見出しか「作業」）、1 つ目の系列 → 開始日（列名は系列名）、2 つ目 → 日数で「終了」= 開始日＋日数−1（その日を含む）。
  3 つ目があれば「完了日数・残り日数」の作り方とみなし、日数 = 2 つ目＋3 つ目、進捗 = 2 つ目 ÷ 日数。4 つ目以降は省く（警告）。
- 分類の軸が反転していなければ Excel では最初の作業が下なので、行を逆順にする。
- 判定を強制・抑止したいときは `c.toSpec({ gantt: true })` / `c.toSpec({ gantt: false })`。
- セルの塗りつぶし（条件付き書式）で作ったガントチャートはグラフではないので対象外。表から作るなら `ui.chart({ type: "gantt", data: sheet.rows(), ... })`。

## 4. 実装の構成

| 置き場所 | 内容 |
|---|---|
| `src/ui/web/vendor/` | AG Charts（UMD の min）・ja-JP ロケール・Frappe Gantt（UMD と CSS）・LICENSE・`VERSIONS.md` |
| `src/ui/web/chart.js` | 共通のアダプタ `XljsChart`: `resolve`（既定値の補完）、`agOptions` / `ganttOptions`（DOM に触れない変換）、`create(el, spec, ctx)`、`load(base)`（ライブラリの読み込み） |
| `src/ui/web/numfmt.js` | Excel 書式の小さな JS 版（`xljsNumfmt.format`） |
| `src/ui/web/chart.html` / `chart-page.js` / `chart.css` | `ui.chart` のページ（`init` / `update` を受け、`click` / `change` を送る。描けなければ `shown` に `error`） |
| `src/ui/web/form.js` | `type: "chart"` の項目（`chart` メッセージで更新、`chartClick` / `chartChange` を送る）。ライブラリは最初の描画で読み込む |
| `src/ui/web/xljs.js` | アプリモードの `xljs.chart(el, spec)`（`/_xljs/chart.js` を読み込む） |
| `src/api/prelude.js` | `ui.chart` / `ChartView`、`chartOf`（検査と既定値）、`chartInit`（ワイヤ形式）、フォームの `chart` 項目と `ctx.chart`、`ExcelChart` と `excelChartSpec`、`Worksheet.charts` |
| `src/api/xlsx_chart.rs` | Excel のグラフの読み取り（§3） |
| `src/i18n/locales/*.json` | `chart.*`（画面の文字・エラー・警告・Excel の種類名）と `form.noChart` |
| `types/xljs.d.ts` | `ChartSpec` `ChartSeries` `ChartAxis` `ChartWindowOptions` `ChartView` `ExcelChart`、`FormField.chart`、`FormContext.chart`、`Worksheet.charts` |

- **定義の検査はスクリプト側**（prelude の `chartOf`）: 未知の種類、存在しない列、数値の列がない、バブルの大きさがない、ガントの開始日がない、
  ハンドラーが関数でない、などを `ui.chart` を呼んだ時点で日本語のエラーにする。ページ側の `resolve` も同じ検査をする（アプリモード用）。
- **サイズ**: 埋め込みは無圧縮（`build.rs` の `include_bytes!`）。増えるのは主に AG Charts の 1.41MB（§5）。

## 5. サイズ

| 追加したファイル | バイト |
|---|---|
| `vendor/ag-charts/ag-charts-community.min.js` | 1,411,575 |
| `vendor/frappe-gantt/frappe-gantt.umd.js` + `.css` | 55,023 |
| `vendor/ag-charts/ag-charts-locale-ja-JP.js` | 10,147 |
| LICENSE・`VERSIONS.md` | 3,421 |
| 自前のページ部品（`chart.js` `numfmt.js` `chart.html` `chart-page.js` `chart.css`） | 36,395 |
| 埋め込みの合計 | 1,516,561（約 1.45 MiB） |

実行ファイルの増加量（Linux x64 のリリースビルドで比較。`opt-level = "s"`、LTO、strip）:
9,198,360 → 10,823,512 バイト（**+1,625,152 バイト、約 1.55 MiB、+17.7%**）。埋め込みの 1.52MB に、prelude と `xlsx_chart.rs` の分が加わる。
Windows 版も埋め込みの分はほぼ同じだけ増える。
圧縮して埋め込むなら AG Charts は gzip -9 で 422,102 バイト（約 30%）になるので、配信時に `Content-Encoding: gzip` を付ける形が候補。

## 6. テスト

- `testdata/xlsx/charts.xlsx`（`make_charts.py` で openpyxl から生成）: 縦棒（色・軸の題名）、組み合わせ＋第 2 軸、100% 積み上げ横棒、
  円（扇の色）、ドーナツ、積み上げ面、3-D 縦棒、レーダー（表示できない）、マーカーなしの折れ線（グラフの配色 `colors1.xml` を後から追加）、散布図、バブル、ガント型の積み上げ横棒。
  openpyxl はテーマに Office 2007 の配色（accent1 `#4F81BD`）を書くので、自動の色がテーマから引けているかの確認にもなる。
- `testdata/scripts/chart.test.js`（mock）: `ui.chart` の表出力と検査、`ExcelChart` の変換結果、フォームの `chart` 項目と `ctx.chart`。
- `src/engine/tests/pages.rs`: `chart_window`（偽のページで `init` の内容、クリック → `onClick` → `update`）と
  `form_window_chart`（ガントの変更の書き戻し、`onClick` のエラーの表示、`ctx.chart`、値）。
- `src/ui/native/tests.rs` の `builtin_pages`（Windows）: WebView2 で chart.html に縦棒とガントを描き、エラーなく `shown` が返ること。
- 見た目は Chromium（Playwright）で chart.html / form.html を開き、偽の `chrome.webview` でメッセージを送って確認した（ライト・ダーク）。

## 7. 未対応・今後

- `toImage()`（PNG に保存）、フォームの `chart` 項目の `selectable`（クリックした点を値にする）は未実装。
- 祝日のデータは同梱しない（スクリプトから `holidays` で渡す）。
- Frappe Gantt の時刻付きの作業は `view: "day"` の 1 日単位で表示される（時間単位の表示は `options.view_mode` などで）。
