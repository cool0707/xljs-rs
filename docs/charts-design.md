# 設計: グラフ（AG Charts）とガントチャート（Frappe Gantt）

フォームやウィンドウにグラフを出す機能の検討メモ。描画には 2 つのライブラリを使うが、スクリプトから見た使い方は 1 つにそろえる。
Excel のブックにあるグラフをそのまま表示できるように、変換の経路も用意する。

## 0. 前提と方針

- **ライブラリ**
  - グラフ: AG Charts Community（MIT）。棒・折れ線・面・散布図・バブル・円・ドーナツ・ヒストグラムと、それらの組み合わせ。
    Enterprise 専用の機能（レーダー、ウォーターフォール、範囲棒、ヒートマップ、ズーム／ナビゲーターなど）は**同梱しない**。
    ライセンスキーがないと透かしが出るので、Community 版のビルドだけを入れる。
  - ガントチャート: Frappe Gantt（MIT、v1 系）。SVG で描画。タスクのドラッグで日付と進捗を変えられる。
- **使い方は 1 つ**: どちらも「グラフの定義（`ChartSpec`）」というただのオブジェクトで表す。`type: "gantt"` なら Frappe Gantt で、それ以外は AG Charts で描く。
  スクリプトがどちらのライブラリを使っているかを意識しなくてよいようにする。
- **データは行オブジェクトの配列**: `ui.table` や `sheet.rows()` と同じ形。列は「キー名」で指定する（`x: "月", y: ["売上", "原価"]`）。
  表とグラフで同じデータをそのまま使い回せる。
- **ワイヤ形式で表せること**: 定義は JSON＋`Date` だけで表せるようにする（関数は入れない）。デバッガ（`--debug`）では
  スクリプトとページの間がメッセージになるので、関数のフォーマッタはページに渡せない。
  イベント（`onClick` など）はスクリプト側で受け取り、ページからはメッセージで知らせる（フォームの `onChange` と同じ流れ）。
- **ライブラリの生のオプション**も受け付ける（`options`）。ただし JSON で表せるものだけ。生のオプションは不安定な抜け道として位置付け、
  型定義では `Record<string, unknown>` にとどめる。

## 1. 表示できる場所（3 か所で同じ定義を使う）

| 場所 | 書き方 | 更新 | 備考 |
|---|---|---|---|
| 単独のウィンドウ | `ui.chart(spec)` → `ChartView` | `view.update(spec)` | `ui.table` の表示のみと同じく、開いてすぐに戻る |
| フォームの項目 | `{ name, type: "chart", chart: spec }` | `ctx.chart(name, spec)` | `ctx.table` と同じ形。`onSubmit` 中に結果を描ける |
| アプリモードの画面 | 画面側で `xljs.chart(el, spec)` | 戻り値の `update(spec)` | 自前の HTML に埋め込む |

```js
// 単独のウィンドウ
const rows = book.sheets["集計"].rows();               // [{ 月: "4月", 売上: 120, 原価: 80 }, ...]
ui.chart({ type: "bar", data: rows, x: "月", y: ["売上", "原価"], title: "月別売上" });

// フォームの中（実行結果をグラフで見せる）
await ui.form({
  fields: [
    { name: "src", label: "集計フォルダ", type: "folder", required: true },
    { name: "graph", type: "chart", chart: { type: "line", data: [], x: "月", y: "合計" }, height: 12 },
  ],
  async onSubmit(v, ctx) {
    const rows = aggregate(v.src);
    ctx.chart("graph", { data: rows });                 // 差分で更新（type などは前のまま）
  },
});

// ガントチャート（使い方は同じ。type と列の指定が違うだけ）
ui.chart({
  type: "gantt",
  data: book.sheets["工程"].rows(),                     // [{ 作業: "設計", 開始: Date, 終了: Date, 進捗: 40, 前提: "" }, ...]
  name: "作業", start: "開始", end: "終了", progress: "進捗", dependencies: "前提",
  view: "week",
  editable: true,
  onChange(row, changes) { /* changes = { start, end } / { progress }。row は元のオブジェクト（書き換え済み） */ },
});
```

- フォームの `chart` 項目は既定で値を持たない（表示専用）。`selectable` を付けるとクリックした点（行）を値にする（`table` 項目の `selectable` と同じ）。
  `editable` なガントチャートは、表の `editable` と同じく書き換えた行の配列を値にする。
- `height` は行数単位（フォームの他の項目と高さをそろえる）。単独ウィンドウでは `width` / `height`（px）。

## 2. ChartSpec（共通の定義）

```ts
interface ChartSpec {
  type: "bar" | "column" | "line" | "area" | "scatter" | "bubble" | "pie" | "donut" | "histogram" | "gantt";
  data: readonly unknown[];        // 行オブジェクト（配列の行なら x: 0 のように位置で指定）
  title?: string;
  subtitle?: string;
  // ---- 軸系（bar/column/line/area/scatter/bubble/histogram）
  x?: string | number;             // 分類または X の値のキー
  y?: string | number | (string | number | SeriesSpec)[];   // 系列。文字列はキー＝系列名
  stacked?: boolean | "percent";   // 積み上げ／100% 積み上げ
  axes?: { x?: AxisSpec; y?: AxisSpec; y2?: AxisSpec };
  // ---- 円系（pie/donut）
  label?: string | number;         // 扇の名前のキー（既定は x）
  value?: string | number;         // 大きさのキー（既定は y）
  // ---- gantt
  name?: string; start?: string; end?: string; progress?: string; dependencies?: string; id?: string;
  view?: "day" | "week" | "month" | "year";
  editable?: boolean | { dates?: boolean; progress?: boolean };
  // ---- 共通
  legend?: boolean | "top" | "bottom" | "left" | "right";
  colors?: string[];               // 系列の色（既定はフォームの配色に合わせたパレット）
  format?: string;                 // 値の表示書式（Excel の書式文字列。"#,##0" "0.0%" "yyyy/m/d" など）
  selectable?: boolean | "single" | "multiple";
  onClick?(row: any, info: { series?: string; index: number }): unknown;
  onChange?(row: any, changes: Record<string, unknown>): unknown;   // gantt の editable
  options?: Record<string, unknown>;  // ライブラリ固有のオプション（AgChartOptions / Frappe Gantt の options）にマージ
}

interface SeriesSpec {
  key: string | number;            // 値のキー
  label?: string;                  // 凡例の名前（既定は key）
  type?: "bar" | "line" | "area" | "scatter";  // 組み合わせグラフ（既定は ChartSpec.type）
  axis?: "y" | "y2";               // 第 2 軸
  color?: string;
  format?: string;
  size?: string | number;          // bubble の大きさのキー
}

interface AxisSpec { title?: string; min?: number | Date; max?: number | Date; format?: string; log?: boolean; }
```

- **`bar` と `column`**: Excel にそろえて `column` = 縦棒、`bar` = 横棒。AG Charts では両方 `type: "bar"` で、`direction` を切り替える。
- **キーの既定値**: `x` を省くと先頭の列、`y` を省くと `x` 以外の数値の列すべて。ガントは `name`/`start`/`end`/`progress`/`dependencies`/`id`
  を省くとその名前の列（なければ日本語の「名前／開始／終了／進捗」も見る）。`ui.table` の「既定はオブジェクトのキー」と同じ考え方。
- **書式**: `format` は Excel の書式文字列。セルの表示（`numfmt.rs`）と同じ書き方で、軸の目盛り・ツールチップ・ラベルに使う。
  ページ側は目盛りごとに Rust へ問い合わせられないので、よく使う書式（桁区切り、小数桁、%、通貨記号・単位の文字、日付）だけを
  解釈する小さな JS 版を `web/` に置く。解釈できない書式は既定の表示にする。
- **値**: 日付は `Date`（ローカルの壁時計時刻。xlsx の値と同じ）。`null` は欠損として線を切る。
- 2 つ目以降の `update(spec)` は差分（指定したキーだけ変える）。`data` を渡すと丸ごと差し替え。

### ライブラリへの対応

| ChartSpec | AG Charts | Frappe Gantt |
|---|---|---|
| `type` | `series[].type`（`column`→`bar`+`direction:"vertical"`、`bar`→`horizontal`、`donut`→`donut`） | `gantt` のみ |
| `data` | `data`（行の Date はそのまま） | `tasks`（`{id,name,start,end,progress,dependencies}` に詰め替え。`id` がなければ行番号） |
| `x` / `y` | `xKey` / `yKey`、系列名は `yName` | `name` / `start` / `end` |
| `stacked` | `stacked: true` / `normalizedTo: 100` | — |
| `axes.y2` | 右側の数値軸＋系列の `keys` | — |
| `view` | — | `view_mode`（`Day`/`Week`/`Month`/`Year`） |
| `editable` | — | `readonly`（の否定）、`readonly_dates` / `readonly_progress` |
| `onClick` | `listeners.seriesNodeClick` → `datum` の行番号を送る | `on_click` → タスクの行番号を送る |
| `onChange` | — | `on_date_change` / `on_progress_change` |
| 言語 | `locale.localeText`（ja-JP のロケールを同梱） | `language: "ja"` |
| フォント | テーマの `fontFamily` を base.css と同じ（游ゴシック UI） | CSS で上書き |

行は番号（`data` の位置）でやり取りし、スクリプト側で元のオブジェクトに戻す（`table` 項目の選択と同じ仕組み）。

## 3. Excel のグラフからの変換

umya-spreadsheet はシート上のグラフを `ChartSpace`（`c:chartSpace`）として読み込んでいる（`Worksheet::chart_collection()`）。
ここから系列の参照式（`c:cat` / `c:val` / `c:tx`）、種類、積み上げ、軸の題名、書式、色を取り出して `ChartSpec` に変換する。

```js
const book = xlsx.open("売上.xlsx");
const sheet = book.sheets["集計"];
sheet.charts;                         // ExcelChart[]（シート上の順）
const c = sheet.charts[0];
c.name;                               // "グラフ 1"
c.range;                              // 置かれている範囲 "E2:L18"（two-cell anchor）
c.toSpec();                           // ChartSpec（データはセルから読んで行オブジェクトにする）
ui.chart(c);                          // ExcelChart はそのまま渡せる（toSpec() が呼ばれる）

// 変換結果を手直しして使う
ui.chart({ ...c.toSpec(), title: "4月〜9月", colors: ["#2f6fb3", "#f0a030"] });

// データだけ差し替える（グラフの見た目は Excel のものを使い、行は新しく集計したもの）
ui.chart({ ...c.toSpec(), data: newRows });
```

### 変換の手順（Rust 側 `xlsx_chart.rs`、native は `xlsx_chart_spec`）

1. `plot_area` の中にある種類ごとのグラフ（`bar_chart`、`line_chart` …）を順に見る。複数あれば組み合わせグラフとして、系列ごとに `type` を付ける。
2. 各系列の参照式（`'集計'!$B$2:$B$13`）を解決してセルの値を読む。参照先のシートがない・外部ブックなどで読めないときは、
   グラフ内のキャッシュ（`numCache` / `strCache`）を使う。
3. 分類（`c:cat`）を `x` 列、各系列を系列名（`c:tx` の参照先の値。なければ「系列 1」…）の列にして、**行オブジェクトの配列**を作る。
   例: `[{ 分類: "4月", 売上: 120, 原価: 80 }, ...]`。こうしておくと `ui.table(c.toSpec().data)` でもそのまま見られる。
   日付軸（`c:dateAx`）や分類が日付書式のセルなら `Date` にする。
4. 種類・オプションを対応表（下）で写す。色は系列の `spPr` の `srgbClr` を使い、テーマ色（`schemeClr`）は Office の既定テーマで RGB に直す。
   数値の書式は `c:numFmt`（`sourceLinked` ならセルの書式）を `format` に入れる。
5. 対応できない要素（3-D の奥行き、近似曲線、誤差範囲、データテーブル、図形の装飾など）は落とし、`c.warnings` に日本語で残す。

### 種類の対応

| Excel（`c:*Chart`） | ChartSpec | 備考 |
|---|---|---|
| `barChart` `barDir=col` | `column` | `grouping`: clustered → なし、stacked → `stacked: true`、percentStacked → `"percent"` |
| `barChart` `barDir=bar` | `bar` | 同上。**ガントの判定は下記** |
| `bar3DChart` | `column` / `bar` | 3-D を落とす（警告） |
| `lineChart` / `line3DChart` | `line` | 積み上げの折れ線は `stacked`。`marker` なしは点を出さない |
| `areaChart` / `area3DChart` | `area` | |
| `pieChart` / `pie3DChart` / `ofPieChart` | `pie` | 補助円は落とす（警告） |
| `doughnutChart` | `donut` | |
| `scatterChart` | `scatter` | `scatterStyle` が線付きなら系列の `type: "line"`（数値の X 軸） |
| `bubbleChart` | `bubble` | `bubbleSize` を `size` に |
| `radarChart` / `stockChart` / `surfaceChart` | 変換しない | Community 版にない。`toSpec()` はエラー（理由をメッセージに） |
| 第 2 軸（系列の `axId` が 2 つ目の軸） | 系列の `axis: "y2"` | |

### ガントチャートの判定

Excel にはガントチャートの種類がなく、ふつうは「横棒の積み上げで、1 つ目の系列（開始日）を塗りなしにする」作り方をする。これを見分けて `gantt` に変換する。

- 条件: `barDir=bar`、`grouping=stacked`、系列が 2 つ以上、**1 つ目の系列が塗りなし**（`a:noFill`）、1 つ目の値が日付（日付書式か、数値軸の最小値が日付のシリアル値）。
- 変換: 分類 → `name`、1 つ目の系列 → `start`（シリアル値を `Date` に）、2 つ目 → 期間（日数）で `end = start + 期間`。
  3 つ目以降があれば「完了／残り」の作り方とみなし、2 つ目と 3 つ目の比から `progress` を出す。
- 分類軸が反転（`orientation=maxMin`）していれば Excel と同じ並び（上から順）になるので、そのままの行順にする。
- 判定を強制・抑止したいときは `c.toSpec({ as: "gantt" })` / `c.toSpec({ as: "bar" })`。
- セルの塗りつぶし（条件付き書式）で作ったガントチャートはグラフではないので対象外。表から作るなら
  `ui.chart({ type: "gantt", data: sheet.rows(), ... })` と列を指定する。

## 4. 実装の構成

| 置き場所 | 内容 |
|---|---|
| `src/ui/web/vendor/ag-charts/` | `ag-charts-community.min.js`（UMD）と LICENSE。ja-JP のロケールテキスト |
| `src/ui/web/vendor/frappe-gantt/` | `frappe-gantt.umd.js`、`frappe-gantt.css`、LICENSE |
| `src/ui/web/vendor/VERSIONS.md` | 取り込んだ版と取得元。更新手順 |
| `src/ui/web/chart.js` | **共通のアダプタ**。`ChartSpec` → AG Charts / Frappe Gantt の変換、`create(el, spec)` / `update` / `destroy` / `toImage`、イベントをメッセージにする |
| `src/ui/web/numfmt.js` | Excel 書式の小さな JS 版（§2） |
| `src/ui/web/chart.html` | `ui.chart` 用の単独ページ（`table.html` と同じ骨組み） |
| `src/ui/web/form.js` | `type: "chart"` の項目。`chart.js` を使う |
| `src/ui/web/xljs.js` | アプリモード用の `xljs.chart(el, spec)` |
| `src/api/prelude.js` | `ui.chart`、フォームの `chart` 項目と `ctx.chart`、`ExcelChart`、定義の検証・既定値の補完（行の Date はコーデックで渡す） |
| `src/api/xlsx_chart.rs` | Excel のグラフ → `ChartSpec` 相当の値（§3） |
| `types/xljs.d.ts` | `ChartSpec`、`SeriesSpec`、`AxisSpec`、`ChartView`、`ExcelChart`、`FormField.chart` |

- **読み込みは必要なページだけ**: ライブラリはサイズが大きい（AG Charts は数百 KB〜1MB 台、要計測）ので、`form.html` には直接書かず、
  `chart` 項目があるときだけ `chart.js` が `<script>` を差し込む。フォームの起動を遅くしない。
- **exe のサイズ**: `build.rs` の埋め込みで exe がその分大きくなる。気になるなら `vendor/` だけビルド時に圧縮して埋め込み、配信時に
  `Content-Encoding: gzip` を付ける（WebView2 がそのまま展開する）。
- **定義の検証はスクリプト側**（prelude）: 存在しないキー、未対応の種類、Enterprise にしかない種類は `ui.chart` を呼んだ時点で
  日本語のエラーにする（キーは `en.json` → `ja.json`）。
- **画像に保存**: `await view.toImage()` → PNG の `ArrayBuffer`。AG Charts は `getImageDataURL`、ガントは SVG を canvas に描いて PNG にする。
  レポートの Excel に貼る用途（`sheet.addImage` を作るなら合わせて）。

## 5. UI の種類ごとの動き

| UI | `ui.chart` | フォームの `chart` 項目 |
|---|---|---|
| native | WebView2 のウィンドウ | 項目として描画 |
| console | グラフは出せないので `data` を `console.table` 相当で出す（ガントは名前・開始・終了の表）。「グラフはウィンドウでのみ表示」と 1 行出す | 同上（`onSubmit` 中の `ctx.chart` も表を出す） |
| mock | ログに `chart: <type> <title> (<行数> rows)` を出す。テストは定義の変換結果（`chart.js` を通した AG Charts／Gantt のオプション）をスナップショットで見る | `onClick` は応答 JSON の `"chart": [行番号]` で起こす |

- デバッガ: `ChartSpec` は JSON＋Date なのでワイヤ形式で送れる。`onClick` / `onChange` はページからのイベントとして既存の経路に乗る。

## 6. テスト

- `chart.js` の変換部分は DOM に依存しない純粋な関数にして、QuickJS で `ChartSpec` → オプションの対応を検査する（`src/engine/tests.rs` 経由）。
- `testdata/xlsx/charts.xlsx`: 縦棒（集合・積み上げ・100%）、横棒、折れ線＋第 2 軸の組み合わせ、円、ドーナツ、散布図、バブル、レーダー（エラー）、
  ガント型の積み上げ横棒を入れた固定データ。`xlsx_chart.rs` の変換をこれで検査する（LibreOffice で作れる。参照先が別シートのものも含める）。
- `testdata/scripts/chart.test.js`: mock UI で `ui.chart`・フォームの `chart` 項目・`ctx.chart`・`ExcelChart.toSpec()`。
- Windows: `src/ui/native/tests.rs` に WebView2 で開いて描画イベント（AG Charts の `seriesNodeClick` が登録されたこと）を確認するテスト。
  見た目は `tools/gui/smoke.ps1` でスクリーンショットを撮る。

## 7. 段階

1. **基盤**: ライブラリの同梱、`chart.js`（AG Charts の基本 4 種＋ガント）、`ui.chart`、d.ts、mock/console。
2. **フォーム**: `chart` 項目、`ctx.chart`、`selectable` / `onClick`、ガントの `editable` / `onChange`。
3. **Excel から**: `sheet.charts`、`toSpec()`、ガントの判定、警告。
4. **仕上げ**: `xljs.chart`（アプリモード）、`toImage`、書式の JS 版、`docs/ai/` と `examples/` の追加。

## 8. 未決事項

- **AG Charts の版とサイズ**: Community の UMD ビルドの実サイズを測り、圧縮して埋め込むか決める。ロケールテキスト（ja-JP）の入手元（`ag-charts-locale`）。
- **逆向き（ChartSpec → Excel のネイティブグラフ）**: umya-spreadsheet はグラフを書ける（`Chart::new_chart`）ので
  `sheet.addChart(spec, "E2:L18")` も作れるが、種類・書式の対応が限られる。需要を見てから。
- **ガントの休日**: 日本の祝日を色付けするか（Frappe Gantt の `holidays`）。祝日データの持ち方（同梱するか、スクリプトから渡すか）。
- **Frappe Gantt の時刻の扱い**: 1 日単位の表示が基本。時刻を含むタスクや、終了日を「その日を含む」か「翌日 0 時」かは、Excel の慣習
  （終了日を含む）に合わせて `end` に 1 日足して渡す案。
- **色**: 既定のパレットをフォームの配色と Office の既定テーマのどちらに寄せるか（Excel からの変換は Excel の色を使う）。
