---
name: xljs-reference
description: xljs（Excel・CSV・JSON を JavaScript で処理する Windows 用の実行環境）の詳しい API リファレンスとレシピ。xljs のスクリプトを書く・直すときに、全オプション・書式・入力フォーム（ui.form）・表（ui.table）・日付（dayjs）・アプリモードを確かめるために使う。
---

# xljs API リファレンス（AI チャット向け）

`prompt.md` の要点リファレンスより詳しい版です。AI に資料（ナレッジ）として添付するか、`prompt.md` のあとに貼ります。
ここに書いていない関数・オプションは存在しないものとして扱ってください。

## 実行環境

- ES2023 の JavaScript（`async`/`await`、トップレベル `await`、`?.`、`??`、クラス、分割代入、`Object.groupBy`、`Array.prototype.toSorted` など）。TypeScript（`.ts`）もそのまま実行できる。
- 使えないもの: `require`、npm パッケージ、`process`、`Buffer`、Node.js の `fs`/`path` モジュール、`fetch`/`XMLHttpRequest`（ネットワーク）、`document`/`window`（DOM）、Excel の COM/VBA。
- `import` は同じフォルダの自作ファイル（`import { f } from "./lib.js"`）と JSON（`import master from "./master.json"`）だけ。
- 使えるグローバル: `console`、`setTimeout`/`setInterval`、`sleep(ms)`、`exit(code)`、`structuredClone`、`TextEncoder`/`TextDecoder`（Shift_JIS 対応）、`atob`/`btoa`、`Blob`/`File`、`Intl`（簡易）、数値・日付の `toLocaleString()`（日本語形式）。
- ファイル系の API は同期（`await` 不要）。`await` が必要なのはダイアログ（`showOpenFilePicker` など）、`ui.form`、`ui.table`（選択するとき）、`sleep`、`h.getFile()` など Promise を返すものだけ。
- パスを受け取る引数には、文字列のパス・ハンドル・`File` のどれでも渡せる。相対パスはカレントフォルダ基準。エクスプローラーから起動した（ダブルクリック・ドロップ）ときはスクリプトのあるフォルダ（exe ならその exe のフォルダ）がカレントフォルダになる。
- エラーが起きると、行番号付きのエラーダイアログを表示して終了する。

### 実行方法（利用者向けの説明に使う）

- `xljs.exe` にスクリプトファイルをドラッグ＆ドロップする。コマンドなら `xljsc 集計.js`（`xljsc.exe` はコンソール版）。
- `xljs pack 集計.js` で `集計.exe` ができる。その exe にファイルをドロップすると、パスが `xljs.args` に入る。
- 先頭にメタデータを書くとタイトルを変えられる:

```js
// ==xljs==
// @title 売上集計ツール
// ==/xljs==
```

## 全体の情報・終了

| API | 説明 |
|---|---|
| `xljs.args` | 引数の配列（exe にドロップしたファイルのパスなど） |
| `xljs.script` / `xljs.scriptDir` | スクリプトのパス / フォルダ |
| `xljs.env(name)` | 環境変数（無ければ `null`）。例 `xljs.env("USERPROFILE")` |
| `xljs.title` | ダイアログのタイトル（代入で変更可） |
| `exit(code?)` | スクリプトを終了する |

## ダイアログ

| API | 戻り値 |
|---|---|
| `alert(msg)` | なし（同期） |
| `confirm(msg)` | `true` / `false`（同期） |
| `prompt(msg, 既定値)` | 文字列。キャンセルで `null`（同期） |
| `ui.error(msg)` | エラーのダイアログ |
| `await showOpenFilePicker({ multiple, types, startIn, title })` | ファイルハンドルの配列。キャンセルで `[]` |
| `await showSaveFilePicker({ suggestedName, types, startIn })` | ファイルハンドル。キャンセルで `null` |
| `await showDirectoryPicker({ startIn, title })` | フォルダハンドル。キャンセルで `null` |

- `types`: `[{ description: "Excel", accept: [".xlsx", ".xlsm"] }]`
- `startIn`: `"desktop"` `"documents"` `"downloads"`、パス、ハンドル（そのファイルのフォルダから開く）
- `showSaveFilePicker` は保存先を決めるだけで、ファイルはまだ作られない。`types` を省略すると `suggestedName` の拡張子が付く（利用者が `集計` と入力しても `集計.xlsx`）。

## ファイル・フォルダ

### ハンドル

| プロパティ・メソッド | 説明 |
|---|---|
| `h.name` / `h.path` / `h.kind` | 名前 / フルパス / `"file"` か `"directory"` |
| `h.relativePath` | `walk()` の起点からの相対パス（`"2026/4月.xlsx"`） |
| `h.exists()` | 存在するか |
| `h.stat()` | `{ size, mtimeMs, isFile, isDirectory }`（無ければ `null`）。更新日時は `new Date(h.stat().mtimeMs)` |
| `h.readText(enc?)` / `h.writeText(text, enc?)` / `h.appendText(text, enc?)` | テキストの読み書き（書き込みは UTF-8、`.csv` / `.tsv` は BOM 付き） |
| `dir.walk(options)` | サブフォルダも含めて走査。`.toArray()` で配列、`for...of` でも回せる |
| `dir.list()` | 直下の項目のハンドルの配列 |
| `dir.file(name)` / `dir.dir(name)` | 直下のファイル / フォルダのハンドル（存在確認なし） |

`walk` のオプション: `{ glob: "*.xlsx" | ["*.csv", "*.txt"], exclude: ["old"], directories: false, hidden: false }`。
`/` を含まないパターンはどの階層のファイル名にも一致し、含むときは起点からの相対パスに一致する（`"2026/**/*.xlsx"`）。大文字小文字は区別しない。除外パターンに一致したフォルダは中身ごと除外。
`walk` と `list` は、隠しファイル・システムファイル、`.` で始まる名前、Excel の一時ファイル（`~$*.xlsx`）を最初から除く（含めるときは `{ hidden: true }`）。

### fs

| API | 説明 |
|---|---|
| `fs.file(p)` / `fs.dir(p)` | パスからハンドルを作る |
| `fs.exists(p)` / `fs.isFile(p)` / `fs.isDirectory(p)` / `fs.stat(p)` | 確認 |
| `fs.readText(p, enc?)` / `fs.writeText(p, text, enc?)` / `fs.appendText(p, text, enc?)` | テキスト |
| `fs.readBytes(p)` / `fs.writeBytes(p, data)` | バイナリ（`ArrayBuffer`） |
| `fs.readJSON(p, { dates })` / `fs.writeJSON(p, value, { space, encoding })` | JSON |
| `fs.readJSONL(p)` / `fs.writeJSONL(p, rows, { append })` | JSON Lines |
| `fs.list(p)` | 直下の名前（文字列）の配列（隠しファイルも含めるなら `fs.list(p, { hidden: true })`） |
| `fs.walk(p, options)` / `fs.glob(pattern, { cwd })` | 走査 |
| `fs.mkdir(p)` | フォルダ作成（途中のフォルダも作る） |
| `fs.copy(from, to)` / `fs.rename(from, to)` / `fs.move(from, to)` | コピー・名前変更・移動 |
| `fs.remove(p, { recursive })` | 削除（ごみ箱には入らない） |
| `fs.cwd()` / `fs.chdir(p)` | カレントフォルダ |

文字コード `enc`: `"auto"`（読み込みの既定。BOM → UTF-8 → Shift_JIS の順に判定）、`"utf8"`（書き込みの既定）、`"utf8bom"`、`"cp932"`（= `"shift_jis"`）、`"euc-jp"`、`"utf16le"`。

### path

`path.join(...)` `path.resolve(...)` `path.dirname(p)` `path.basename(p, 拡張子?)` `path.extname(p)` `path.relative(from, to)` `path.normalize(p)` `path.isAbsolute(p)` `path.sep`（Windows では `\`）。ハンドルも渡せる。

### shell

| API | 説明 |
|---|---|
| `shell.showItemInFolder(p)` | エクスプローラーでファイルを選択した状態で開く（配列で複数可） |
| `shell.openPath(p)` | 既定のアプリで開く（.xlsx なら Excel、フォルダならエクスプローラー） |
| `shell.openExternal(url)` | ブラウザで開く |

## Excel

対応形式は `.xlsx` / `.xlsm`。`.xls` は不可。マクロ・ピボットテーブル・グラフは保存すると失われることがある。

### ブック

| API | 説明 |
|---|---|
| `workbooks.open(p)` | 開く |
| `workbooks.create({ font? })` | 新しいブック。最初の `addSheet` が既定のシートを置き換える |
| `book.sheets[0]` / `book.sheets["名前"]` / `book.sheet(名前か番号)` | シート（無ければ `undefined` / `null`）。名前の大文字小文字・前後の空白・全角半角の違いは許す |
| `book.sheetNames` | シート名の配列 |
| `book.addSheet(name)` / `book.removeSheet(name)` | 追加・削除 |
| `book.save()` / `book.save(p)` | 上書き保存 / 別名保存 |
| `book.close()` | 閉じる（メモリ解放） |
| `workbooks.readVBA(p)` | マクロブック（.xlsm / .xlsb など）の VBA コード。`{ name, modules: [{ name, type, code, sheet }], text }`、マクロがなければ `null`。`type` は `"standard"` `"class"` `"document"`（ThisWorkbook とシート）`"form"`。`text` は全モジュールの Markdown |
| `workbooks.columnName(3)` → `"C"`、`workbooks.cellName(2, 3)` → `"C2"` | 番地の変換 |

### シートの読み取り

| API | 説明 |
|---|---|
| `sheet.toObjects({ header, columns, text, skipEmpty })` | 行をオブジェクトに。`header` は見出しの行番号（既定 1）。`header: 0, columns: ["A列名", ...]` で見出しなし |
| `sheet.toArrays({ text })` | 2 次元配列（0 始まり）。`text: true` で表示形式どおりの文字列 |
| `sheet.cell("B2")` / `sheet.cell(行, 列)` | セル（1 始まり） |
| `cell.value` | 値: number / string / boolean / Date / null |
| `cell.text` | 表示どおりの文字列 |
| `cell.formula` | `"=SUM(A1:A3)"`。無ければ `null` |
| `sheet.getValue("B2")` / `sheet.setValue("B2", v)` | セルの値の読み書き |
| `sheet.range("A1:C3").values` | 範囲の 2 次元配列（代入で書き込み） |
| `sheet.rowCount` / `sheet.dimension` | 最終行 / 使用範囲（`"A1:F120"`） |
| `sheet.name` | シート名（代入で変更） |

- 結合セルの値は左上のセルにだけ入っている。
- 数式セルの `value` は Excel が最後に保存した計算結果。スクリプトで書いた数式は Excel で開くまで計算されない（`value` は `null`。`cell.calc()` で一部の関数は計算できる）。

### シートへの書き込み

| API | 説明 |
|---|---|
| `sheet.write(rows, options)` | オブジェクトの配列（見出し＋データ）か 2 次元配列を書く |
| `sheet.append(rows, options)` | 最終行の下に追記（見出しは書かない） |
| `cell.value = v` / `cell.formula = "=..."` | 1 セル |

`write` のオプション:

| オプション | 説明 |
|---|---|
| `start` | 書き始めのセル（既定 `"A1"`） |
| `header` | 見出し行を書くか（既定 `true`） |
| `columns` | 書く列と順番（`["日付", "金額"]`） |
| `table` | 表の体裁（見出しの太字・塗り・罫線、データの罫線、列幅自動、先頭行固定、フィルター） |
| `headerStyle` / `style` / `columnStyles` | 見出し / 全体 / 列ごとの書式（`{ 金額: { format: "#,##0" } }`） |
| `autoFit` / `freeze` / `filter` | 列幅自動 / 見出しの固定 / フィルター（`table` では既定で有効） |

### 書式

```js
sheet.style("A1:D1", { bold: true, color: "#FFFFFF", background: "#305496", align: "center" });
sheet.cell("D10").style({ font: { size: 14, bold: true }, format: "¥#,##0", border: { top: "double" } });
sheet.range("A3:D10").border({ outline: "medium", inside: { style: "thin", color: "#BFBFBF" } });
sheet.merge("A1:D1");             // セル結合（unmerge で解除）
sheet.freeze("A2");               // 1 行目を固定（"B2" で 1 行目と A 列）
sheet.autoFilter("A3:D10");       // フィルター
sheet.autoFit();                  // 列幅を内容に合わせる
sheet.setColumnWidth("A:C", 16);  // 列幅（文字数）
sheet.setRowHeight(1, 30);        // 行の高さ（ポイント）
```

| 項目 | 指定 |
|---|---|
| 文字 | `bold` `italic` `underline` `strike` `color` `fontSize` `fontName`、または `font: { name, size, bold, italic, color }` |
| 塗り | `background: "#DDEBF7"`（`null` で塗りなし） |
| 配置 | `align`（`left` `center` `right`）、`valign`（`top` `center` `bottom`）、`wrap: true`（折り返し） |
| 表示形式 | `format`: `"#,##0"` `"#,##0.00"` `"0.0%"` `"¥#,##0"` `"yyyy/m/d"` `"yyyy年m月d日"` `"h:mm"` `"@"`（文字列） |
| 罫線 | `border: "thin"`（全セルの上下左右）、または `{ outline, inside, horizontal, vertical, top, bottom, left, right }`。線種 `"thin"` `"medium"` `"thick"` `"double"` `"dashed"` `"dotted"`、色付きは `{ style: "thin", color: "#999999" }` |

色は `"#RRGGBB"` か `red` などの色名。書式は既存の書式に重ねて適用される（`{ replace: true }` で置き換え）。

## CSV

| API | 説明 |
|---|---|
| `csv.read(p, options)` | 読む。既定は見出しをキーにしたオブジェクトの配列 |
| `csv.write(p, rows, options)` | 書く。既定は BOM 付き UTF-8・CRLF（Excel で文字化けしない） |
| `csv.parse(text, options)` / `csv.stringify(rows, options)` | 文字列との変換 |

- 値は既定で変換する: 数値（`"1,234"` も）、日付（`2026/4/1`、`2026-04-01 09:30` → `Date`）、`TRUE`/`FALSE`、空欄は `null`。先頭が 0 のコード（`"00123"`）、16 桁以上の数字、`"123-4567"` などは文字列のまま。見出しの前後の空白は取り除く。
- 読むオプション: `header: false`（2 次元配列）、`cast: false`（変換せずすべて文字列）、`encoding`（既定 `"auto"`）、`delimiter`（`.tsv` はタブが既定）、`columns`、`trim`、`skipEmptyLines`。
- 書くオプション: `encoding: "cp932"`（Shift_JIS。古いシステム向け）、`columns`、`header: false`、`delimiter: "\t"`、`append: true`、`quoteAll: true`。
- `Date` は `"2026/04/01"` の形式で書き出す。

## JSON

```js
const data = fs.readJSON(h);                       // BOM・Shift_JIS も自動判定
const withDates = fs.readJSON(h, { dates: true }); // ISO 形式の日時文字列を Date に
fs.writeJSON(dest, rows);                          // 2 スペースでインデント、UTF-8
fs.writeJSON(dest, rows, { space: 0 });            // 1 行で
fs.writeJSONL(dest, rows);                         // 1 行 1 レコード
```

## 進捗表示

```js
// まとめて書く（キャンセルされるとスクリプトがその場で終わる）
for (const h of ui.withProgress(files, { title: "処理中" })) { /* ... */ }

// キャンセル後も続けたいとき（ループが終わるだけ）
for (const h of ui.withProgress(files, { title: "処理中", onCancel: "break" })) { /* ... */ }
if (ui.cancelled) alert("途中で中止しました");

// 細かく制御する
const p = ui.progress({ title: "処理中", total: files.length });
for (const [i, f] of files.entries()) {
  if (p.cancelled) break;
  p.update(i, f.name);    // 完了件数と表示する文字
}
p.close();
```

## 入力フォーム（ui.form）

入力は原則このフォームで受け取る（バッチ・他のアプリから自動で呼ばれるときは画面を出さず `xljs.args` や設定ファイルから、使い捨ての処理はダイアログで）。`required` `min` `max` `pattern` と `validate` で入力チェックを必ず入れ、ファイルは中身（シート・列）まで `validate` で確かめる。

```js
const v = await ui.form({
  title: "請求書作成",
  description: "一覧の各行から請求書を作ります",
  fields: [
    { name: "list", label: "請求一覧", type: "file", accept: [".xlsx"], required: true },
    { name: "outDir", label: "出力フォルダ", type: "folder", required: true },
    { name: "date", label: "請求日", type: "date", default: new Date() },
    { name: "prefix", label: "ファイル名の先頭", type: "text", default: "請求書_", pattern: "[^\\\\/:*?\"<>|]*", patternMessage: "使えない文字があります" },
    { name: "rate", label: "税率(%)", type: "number", default: 10, min: 0, max: 100 },
  ],
  submit: "作成",
  remember: true,
  validate(v) {                                        // 追加の検証。問題があれば { 項目名: "メッセージ" } を返す
    if (v.rate !== 8 && v.rate !== 10) return { rate: "8 か 10 を入力してください" };
  },
});
if (!v) exit();
```

| type | 値 |
|---|---|
| `text` `password` `textarea` | 文字列 |
| `number` | 数値か `null`（`min` `max` `step`） |
| `date` | `Date` か `null` |
| `month` | `"2026-04"` か `null` |
| `time` | `"09:30"` か `null` |
| `select` `radio` | 選んだ文字列（`options: ["A", "B"]` または `[{ value, label }]`） |
| `checkbox` | `true` / `false` |
| `file` `saveFile` | ファイルハンドルか `null`（`accept: [".xlsx"]`） |
| `files` | ファイルハンドルの配列 |
| `folder` | フォルダハンドルか `null` |
| `table` | 表から選んだ行（`rows` `columns` `selectable: true | "single"`） |

- 項目の共通オプション: `label` `default` `required` `placeholder`（`select` では未選択の表示になり、初期値が `""`）`help`（説明文）`tooltip`（マウスを重ねたときの説明。ボタンにも付けられる）、`validate(value, values)`（項目ごとの検証。メッセージを返す。入力欄を離れたときにも検証される）。フォームの `validate(values)` は送信時だけ。
- `onChange(value, values, ctx)`: ユーザーが値を変え、その項目の検証が通ったときに呼ばれる。`ctx.setValue(name, value)` でほかの項目を埋められる（setValue で変えた項目の onChange は呼ばれない）。

```js
// 1 つ目のファイルを選ぶと、同じフォルダの対になるファイルを自動で設定する
await ui.form({
  title: "突合",
  fields: [
    { name: "sales", label: "売上", type: "file", accept: [".csv"], required: true, tooltip: "売上_YYYYMM.csv",
      onChange(h, v, ctx) {
        const m = h && /^売上_(\d{6})\.csv$/.exec(h.name);
        if (!m) return;
        const dir = path.dirname(h.path);
        for (const [name, file] of [["purchase", `仕入_${m[1]}.csv`], ["stock", `在庫_${m[1]}.csv`]]) {
          if (!v[name] && fs.exists(path.join(dir, file))) ctx.setValue(name, path.join(dir, file));
        }
      } },
    { name: "purchase", label: "仕入", type: "file", accept: [".csv"], required: true },
    { name: "stock", label: "在庫", type: "file", accept: [".csv"], required: true },
  ],
});
```
- `tabs: [{ label: "基本", fields: [...] }, { label: "詳細", fields: [...] }]` でタブ分け。
- アイコン（Font Awesome Free の名前。`"folder-open"` など）: 項目・タブ・ボタンの `icon`、送信ボタンは `submitIcon`。フォームの `icon` でページの先頭にアイコン付きのタイトルを出す。
- `table` に `editable: true` を付けるとセルをその場で編集でき、入力のたびに検証してエラーのセルを赤く表示する（エラーが残っていると送信できない）。検証を通った値は元の行オブジェクトに書き込まれる。

```js
const v = await ui.form({
  title: "エラー行の修正",
  fields: [{
    name: "rows", label: "エラーのある行", type: "table", rows: badRows, editable: true,
    columns: [
      { key: "行", editable: false },
      { key: "コード", pattern: "[A-Z]\\d{2}", patternMessage: "英大文字＋数字2桁" },
      { key: "金額", type: "number", min: 0, required: true },
      { key: "区分", type: "select", options: ["売上", "返品"] },
      { key: "日付", type: "date", validate: (d) => (dayjs(d).isAfter(dayjs(), "day") ? "未来の日付です" : "") },
    ],
    validateRow: (r) => (r.区分 === "返品" && r.金額 > 10000 ? { 金額: "返品は 1 万円まで" } : ""),
  }],
  submit: "修正を反映",
});
if (!v) exit();
// v.rows は（編集済みの）全行。badRows の各オブジェクトも書き換わっている
```
- `onSubmit(v, ctx)` を書くと、送信後もフォームが開いたまま処理し、進捗・ログ・中止ボタンを表示する。出力ファイルを `return` すると「フォルダを開く」ボタンが出る。

```js
await ui.form({
  title: "集計",
  fields: [{ name: "src", label: "フォルダ", type: "folder", required: true }],
  async onSubmit(v, ctx) {
    const files = v.src.walk("*.xlsx").toArray();
    for (const h of ctx.withProgress(files)) {  // 中止ボタンが押されると onSubmit はここで終わる
      ctx.log(h.relativePath);      // ログ欄に表示（ctx.error で赤字）
    }
    // ... 保存
    // return dest;
  },
});
```

- `buttons: [{ label, onClick(v, ctx) }]` で送信ボタンの左にボタンを追加できる。押すと onClick が onSubmit と同じように動く（ウィンドウは開いたまま）。`validate: false` で検証なし、`close: true` で正常終了後に閉じる。`submit: false` で送信ボタンを出さない。`ctx.setValue(name, value)` で項目の値を変えられる。

```js
await ui.form({
  title: "名簿",
  fields: [{ name: "file", label: "名簿", type: "file", accept: [".xlsx"], required: true }, { name: "count", label: "件数", type: "number" }],
  buttons: [
    { label: "件数を確認", onClick(v, ctx) { ctx.setValue("count", workbooks.open(v.file).sheets[0].toObjects().length); } },
  ],
  submit: "作成",
  async onSubmit(v, ctx) { /* ... */ },
});
```

## 表の表示（ui.table）

```js
ui.table(rows, { title: "結果" });                         // 表示するだけ（すぐ次へ進む）
await ui.table(rows, { title: "結果" }).closed;            // 閉じるまで待つ
const picked = await ui.table(rows, { title: "選択", selectable: "multiple" }); // 選んだ行。キャンセルは []
const [one] = await ui.table(rows, { selectable: "single" });
ui.table(rows, {
  columns: ["名前", { key: "size", label: "サイズ", width: 80 }],
  onDoubleClick(e) { shell.openPath(e.row.handle); },      // e: { row, index, column, value, close() }
});
```

表は並べ替え・絞り込み・コピー・CSV / Excel への保存が画面上でできる。

- `ui.table` は表示と選択だけで、セルは編集できない。編集させるときは `ui.form` の `type: "table"` に `editable: true`（上の「入力フォーム」）。
- `onDoubleClick` は `ui.table` だけにある（フォームの表にはない）。`e.column` はダブルクリックした列の見出し（行番号の欄なら `null`）、`e.key` は列のキー。`selectable: "single"` と一緒に指定すると、ダブルクリックでは OK にならない。

## 日付（dayjs）

[Day.js](https://day.js.org/) を組み込んである（`import` 不要。グローバルの `dayjs`）。PC の表示言語のロケール（日本語環境なら日本語。`dayjs.locale("ja")` で固定できる）と次のプラグインを読み込み済み: customParseFormat, isBetween, isSameOrAfter, isSameOrBefore, minMax, quarterOfYear, weekOfYear, isoWeek, dayOfYear, duration, relativeTime, advancedFormat, isLeapYear, isToday, isTomorrow, isYesterday, weekday, localeData。

`Date` を直接計算しない（`new Date("2026-04-01")` が UTC になる、月が 0 始まり、`setMonth` で月末がずれる、など）。Excel・CSV から読んだ `Date` も `dayjs(値)` で包んで扱う。

```js
const d = dayjs(r.日付);                         // Date・"2026/4/1"・"2026-04-01 09:30" など（文字列はローカル時刻）
dayjs("20260401", "YYYYMMDD");                   // 形式を指定して読む
dayjs("2026/02/30", "YYYY/MM/DD", true).isValid(); // false（厳密に読む）
d.format("YYYY/MM/DD");                          // "2026/04/01"
d.format("YYYY年M月D日(ddd)");                  // "2026年4月1日(水)"
d.format("YYYY-MM");                             // 月別集計のキー
dayjs().format("YYYYMMDD_HHmmss");               // ファイル名用のタイムスタンプ
d.add(1, "month"); d.subtract(7, "day");         // 新しい値を返す（d は変わらない）
d.startOf("month"); d.endOf("month");            // 月初・月末（endOf は 23:59:59.999）
d.year(); d.month() + 1; d.date(); d.day();      // 年・月（0 始まり）・日・曜日（0 = 日曜）
d.quarter(); d.isoWeek(); d.daysInMonth();       // 四半期・ISO 週番号・月の日数
dayjs(b).diff(a, "day");                         // 日数の差（b - a）。"month" "year" も
d.isBefore(x); d.isAfter(x); d.isSame(x, "month");
d.isBetween(from, to, "day", "[]");              // 期間内（両端含む）
dayjs.max(list); dayjs.min(list);                // 最新・最古（dayjs の配列）
dayjs.duration(90, "minutes").format("H:mm");    // "1:30"
dayjs.fromExcel(45383);                          // Excel のシリアル値 → 2024/4/1（xljs 拡張）
d.toDate();                                      // Date に戻す
```

- dayjs の値はそのまま `cell.value`・`sheet.write`・`csv.write`・`fs.writeJSON`・`ui.table`・フォームの `default` に渡せる（`Date` として扱う）。
- 和暦（令和など）の解釈・表示はできない。
- 数値: `(1234567).toLocaleString()` → `"1,234,567"`、`Math.round(x * 100) / 100` で小数第 2 位に丸める。

## アプリモード（HTML の画面）

自由なレイアウトの画面が必要なときだけ使う（たいていは `ui.form` と `ui.table` で足りる）。HTML ファイルを別に用意する。

```js
// ==xljs==
// @mode   app
// @html   ui/index.html
// @assets ui/
// ==/xljs==
const win = app.main;
export async function run({ month }) {    // 画面から呼べる関数
  win.state.status = "処理中";            // 画面の data-bind="status" に反映
  return 10;                              // 画面の data-result に入る
}
```

```html
<!-- ui/index.html -->
<link rel="stylesheet" href="/_xljs/fontawesome/css/all.min.css">
<form data-action="run" data-result="count">
  <input name="month" type="month" required>
  <button><i class="fa-solid fa-play"></i> 実行</button>
</form>
<p>状態: <span data-bind="status"></span> / 件数: <span data-bind="count"></span></p>
```

- アイコンは同梱の Font Awesome Free 7 が使える（上の `<link>` を書く。CDN は使わない）: `<i class="fa-solid fa-file-excel"></i>`、`fa-regular`、`fa-brands`。

## レシピ

### 1. 複数の CSV を 1 つの Excel に（ファイルごとにシート）

```js
const handles = await showOpenFilePicker({ multiple: true, types: [{ description: "CSV", accept: [".csv", ".tsv", ".txt"] }] });
if (handles.length === 0) exit();

const book = workbooks.create();
for (const h of handles) {
  const rows = csv.read(h);
  const name = path.basename(h.name, path.extname(h.name)).replace(/[\\\/?*\[\]:]/g, "_").slice(0, 31);
  book.addSheet(name).write(rows, { table: true });
}
const dest = await showSaveFilePicker({ suggestedName: "CSVまとめ.xlsx", startIn: handles[0], types: [{ description: "Excel", accept: [".xlsx"] }] });
if (!dest) exit();
book.save(dest);
alert(`${handles.length} ファイルを変換しました`);
shell.showItemInFolder(dest);
```

### 2. 列の値ごとに別ファイルに分ける

```js
const [h] = await showOpenFilePicker({ types: [{ description: "Excel", accept: [".xlsx"] }] });
if (!h) exit();
const book = workbooks.open(h);
const sheet = book.sheets[0];
const rows = sheet.toObjects();
const key = prompt("分ける列の名前", Object.keys(rows[0] ?? {})[0] ?? "");
if (!key) exit();

const outDir = path.join(path.dirname(h), "分割");
fs.mkdir(outDir);
const groups = Object.groupBy(rows, (r) => String(r[key] ?? "(空欄)"));
for (const [name, list] of ui.withProgress(Object.entries(groups), "分割中")) {
  const out = workbooks.create();
  out.addSheet(sheet.name).write(list, { table: true });
  out.save(path.join(outDir, `${name.replace(/[\\\/:*?"<>|]/g, "_")}.xlsx`));
}
alert(`${Object.keys(groups).length} ファイルに分けました`);
shell.openPath(outDir);
```

### 3. ひな形（テンプレート）に 1 行ずつ差し込んで、行の数だけファイルを作る

```js
const v = await ui.form({
  title: "差し込み作成",
  fields: [
    { name: "list", label: "一覧（1 行目が見出し）", type: "file", accept: [".xlsx"], required: true },
    { name: "template", label: "ひな形", type: "file", accept: [".xlsx"], required: true },
    { name: "outDir", label: "出力フォルダ", type: "folder", required: true },
  ],
  remember: true,
});
if (!v) exit();

const rows = workbooks.open(v.list).sheets[0].toObjects();
const safe = (s) => String(s ?? "").replace(/[\\\/:*?"<>|]/g, "_");  // ファイル名に使えない文字を置き換える
let count = 0;
for (const r of ui.withProgress(rows, "作成中")) {
  const book = workbooks.open(v.template);   // 毎回ひな形を開き直す
  const ws = book.sheets[0];
  ws.cell("B3").value = r.顧客名;
  ws.cell("B4").value = r.住所;
  ws.cell("E3").value = new Date();
  ws.cell("D10").value = Number(r.金額) || 0;
  book.save(path.join(v.outDir.path, `請求書_${safe(r.顧客名)}.xlsx`));  // 別名で保存（ひな形は変わらない）
  book.close();
  count++;
}
alert(`${count} 件作成しました`);
shell.openPath(v.outDir);
```

### 4. 2 つの一覧を突き合わせる（VLOOKUP の代わり）

```js
const v = await ui.form({
  title: "突き合わせ",
  fields: [
    { name: "a", label: "一覧 A", type: "file", accept: [".xlsx", ".csv"], required: true },
    { name: "b", label: "一覧 B（マスタ）", type: "file", accept: [".xlsx", ".csv"], required: true },
    { name: "key", label: "照合する列", type: "text", default: "社員番号", required: true },
  ],
});
if (!v) exit();

const load = (h) => h.name.toLowerCase().endsWith(".csv") ? csv.read(h) : workbooks.open(h).sheets[0].toObjects();
const a = load(v.a);
const master = new Map(load(v.b).map((r) => [String(r[v.key]).trim(), r]));

const result = a.map((r) => {
  const m = master.get(String(r[v.key]).trim());
  return { ...r, 照合結果: m ? "あり" : "なし", マスタ_氏名: m?.氏名 ?? null };
});
const missing = result.filter((r) => r.照合結果 === "なし");

const dest = await showSaveFilePicker({ suggestedName: "突き合わせ結果.xlsx", startIn: v.a });
if (!dest) exit();
const out = workbooks.create();
out.addSheet("結果").write(result, { table: true });
out.addSheet("見つからない").write(missing, { table: true });
out.save(dest);
alert(`${a.length} 件中 ${missing.length} 件がマスタにありませんでした`);
shell.showItemInFolder(dest);
```

### 5. 月別・担当別のクロス集計

```js
const [h] = await showOpenFilePicker({ types: [{ description: "Excel", accept: [".xlsx"] }] });
if (!h) exit();
const rows = workbooks.open(h).sheets[0].toObjects().filter((r) => r.日付 instanceof Date);

const ym = (d) => dayjs(d).format("YYYY-MM");
const months = [...new Set(rows.map((r) => ym(r.日付)))].sort();
const table = Object.entries(Object.groupBy(rows, (r) => r.担当 ?? "(未設定)")).map(([who, list]) => {
  const line = { 担当: who };
  for (const m of months) line[m] = 0;
  for (const r of list) line[ym(r.日付)] += Number(r.金額) || 0;
  line.合計 = list.reduce((s, r) => s + (Number(r.金額) || 0), 0);
  return line;
});

const dest = await showSaveFilePicker({ suggestedName: "月別集計.xlsx", startIn: h });
if (!dest) exit();
const out = workbooks.create();
const style = Object.fromEntries([...months, "合計"].map((m) => [m, { format: "#,##0" }]));
out.addSheet("月別").write(table, { table: true, columnStyles: style });
out.save(dest);
shell.openPath(dest);
```

### 6. exe にドロップしたファイルを処理する（`xljs pack` 用）

```js
// xljs.args にドロップしたファイルのパスが入る。無ければ選ばせる
let targets = xljs.args.map((p) => fs.file(p));
if (targets.length === 0) targets = await showOpenFilePicker({ multiple: true, types: [{ description: "Excel", accept: [".xlsx"] }] });
if (targets.length === 0) exit();

const created = [];
for (const h of ui.withProgress(targets, "変換中")) {
  const book = workbooks.open(h);
  for (const sheet of book.sheets) {
    const dest = path.join(path.dirname(h), `${path.basename(h.name, path.extname(h.name))}_${sheet.name}.csv`);
    csv.write(dest, sheet.toArrays({ text: true }), { encoding: "cp932" });
    created.push(dest);
  }
  book.close();
}
shell.showItemInFolder(created);
```
