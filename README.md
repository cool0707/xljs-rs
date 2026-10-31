# <img src="assets/xljs.png" width="40" align="top"> xljs

JavaScript / TypeScript で書いたスクリプトで Excel・CSV の集計やデータ変換を行う、Windows 用のシングルバイナリです。Excel のインストールは不要です。

このリポジトリは Go 版 [xljs](https://github.com/cool0707/xljs) を Rust に移植したものです。スクリプトの API・メタデータ・pack した exe の形式・メッセージは Go 版と同じです（違いは「[Go 版との違い](#go-版との違い)」）。

```js
const dir = await showDirectoryPicker();
if (!dir) exit();

const rows = [];
for (const h of ui.withProgress(dir.walk("*.xlsx"), { title: "集計中" })) {
  const book = workbooks.open(h);
  for (const r of book.sheets["明細"]?.toObjects() ?? []) rows.push({ ...r, ファイル: h.relativePath });
}

const dest = await showSaveFilePicker({ suggestedName: "集計結果.xlsx" });
if (dest) {
  const out = workbooks.create();
  out.addSheet("明細").write(rows);
  out.save(dest);
  alert(`${rows.length} 件を集計しました`);
}
```

- **言語**: ES2023 相当（オプショナルチェーン、`??`、async/await、トップレベル await、クラスフィールドなど）と TypeScript。`import` でファイル分割可能
- **ファイル操作**: ブラウザの File System Access API 風（`showOpenFilePicker` / `FileSystemFileHandle` など）＋ フォルダ走査などの独自拡張
- **Excel**: xlsx の読み書き（[umya-spreadsheet](https://github.com/MathNya/umya-spreadsheet)）。値は数値・文字列・真偽値・`Date` で受け渡し
- **CSV / JSON**: Shift_JIS(CP932) / UTF-8 の自動判定、BOM 付き UTF-8 での書き出し、JSON Lines
- **エクスプローラー連携**: 出力したファイルを選択した状態でエクスプローラーを開く、既定のアプリで開く
- **UI**: Windows ネイティブのダイアログ（メッセージボックス、ファイル選択、進捗ダイアログ）。コンソールでも動作

## 使い方

機能ごとのサンプルは [`examples/`](examples/README.md) にあります。AI チャット（Claude・Gemini・Copilot など）にスクリプトを書いてもらうときは、[`docs/ai/`](docs/ai/README.md) のプロンプトとリファレンスを最初に渡してください。

```
xljs [オプション] <script.js|script.ts> [引数...]
xljs pack <script> [-o 出力.exe]   スクリプトを埋め込んだ exe を作成
xljs check <script>                構文と import を検査（実行しない）
xljs init [フォルダ]               型定義 (xljs.d.ts) と jsconfig.json を作成
xljs vba <book.xlsm>... [-o 出力.txt]  マクロブックの VBA コードを 1 つのテキストにまとめる
xljs version
```

| オプション | 説明 |
|---|---|
| `--ui=auto` | 既定。Windows ではネイティブダイアログ、それ以外はコンソール |
| `--ui=native` / `--ui=console` | UI を固定 |
| `--ui=mock --ui-answers=answers.json` | ダイアログを出さず、ファイルに書いた応答を返す（自動実行・テスト用） |
| `--lang=auto` / `ja` / `en` | メッセージの言語（既定はシステムの表示言語） |
| `--debug` | デバッグウィンドウでスクリプトを実行する（ブレークポイント・ステップ実行。下記「デバッグ」） |
| `--debug=window` | スクリプトは通常どおり実行し、画面（HTML ウィンドウ）の開発者ツール（F12）と右クリックメニューだけを有効にする |
| `--watchdog=<秒>` / `on` / `off` | スクリプトが応答しないまま（ダイアログ・画面・タイマーを待たずに JavaScript を実行し続けて）指定の秒数を過ぎたら、中止するか尋ねる（無限ループ対策）。既定はネイティブ UI では 15 秒、コンソール・モック UI では off（Ctrl+C で止められるため）。`0` も off |
| `--stop-key=<キー>` / `on` / `off` | 実行中にキーが押されたら中止するか尋ねる。キーは `esc`・`pause`・`f1`〜`f24`・英数字などと `ctrl+`・`shift+`・`alt+` の組み合わせ（例: `ctrl+shift+q`）。既定はネイティブ UI では `esc`、コンソール・モック UI では off。Windows のみ |

- `xljs.exe` は GUI 版（ダブルクリックしても黒い窓が出ない）、`xljsc.exe` はコンソール版です。中身は同じで、GUI 版をコマンドプロンプトから起動した場合はそのコンソールに出力します（ただしコマンドプロンプトは終了を待たないので、コマンドラインで使うときや出力をリダイレクトするときはコンソール版の `xljsc` が向いています）。GUI 版で出力先がないときのエラーはダイアログで表示し、文字列を選択したり「コピー」ボタンでクリップボードにコピーしたりできます。
- エラー・警告はシステムの表示言語に合わせて日本語または英語で表示します。`--lang=ja|en`（または環境変数 `XLJS_LANG`）で切り替えられます。JavaScript エンジンや構文チェック、Excel 読み込み、OS のエラーのうちよく出るものは日本語に訳して表示します（`catch` で受け取る `e.message` は元のままです）。
- メッセージは言語ごとのファイル `src/i18n/locales/<言語>.json` にまとまっています（`en.json` が基準）。言語を追加するときはファイルを 1 つ足してビルドするだけです（手順は同じフォルダの README.md）。
- 引数なしで起動すると、「スクリプトを実行」「スクリプトをデバッグ」「exe を作成」「VBA コードを取り出す」から選ぶ画面が開き、選んだあとで対象のファイルを選びます。exe にスクリプトをドラッグ＆ドロップしても実行できます（マクロ付きブックをドロップすると VBA コードの取り出し）。
- コンソール版をダブルクリックで起動した場合は、終了時に Enter 待ちになります。
- 進捗表示中の Ctrl+C はキャンセル要求（`cancelled` が true になる）、2 回目で強制終了です。
- 無限ループなどでスクリプトが応答しなくなったときは、`--watchdog` の秒数を過ぎると「中止しますか？」と尋ねます（尋ねている間スクリプトは止まっています）。「OK」で中止（終了コード 130。`catch` では受け取れません）、「キャンセル」で続行し、同じ時間が過ぎたらまた尋ねます。
- 実行中に `--stop-key` のキー（既定は Esc）を押しても同じように尋ねます。キーはどのウィンドウが前面にあっても効きます（ほかのアプリで押した場合も尋ねるので、誤って止まることはありません）。ダイアログ・フォーム・画面の操作を待っている間や、タイマーで待っている間に押したキーは数えません（Esc でダイアログを閉じても中止の確認は出ません）。進捗表示中は進捗ダイアログの「キャンセル」を使ってください。
- 埋め込み exe は環境変数 `XLJS_WATCHDOG` / `XLJS_STOP_KEY` で同じ指定ができます。`--debug` のデバッグウィンドウでは使いません（開発者ツールで一時停止できるため）。

### VBA マクロを取り出す（AI チャットでの移植用）

マクロ付きのブック（.xlsm / .xlsb / .xlam / .xltm）を `xljs.exe` にドラッグ＆ドロップすると、保存先を聞いたうえで、全モジュールの VBA コードを 1 つのテキストファイル（既定は `<ブック名>_VBA.txt`）にまとめます。保存後に「今すぐ開きますか？」で開けば、そのまま AI チャットに貼り付けられます。複数のブックをまとめてドロップすると 1 つのファイルに続けて書きます。引数なしで起動した画面の「VBA コードを取り出す」からも選べます。

```
xljs vba 売上集計.xlsm              → 売上集計_VBA.txt（ブックと同じフォルダ）
xljs vba a.xlsm b.xlsm -o マクロ.txt → 2 冊分を マクロ.txt に
xljsc vba 売上集計.xlsm -o -        → 標準出力へ
```

- 中身は Markdown です。モジュールごとに見出し（種類と、シートのモジュールならシート名）と `vba` のコードブロックを書きます。モジュール先頭の `Attribute VB_...` 行と、`Option Explicit` しかないモジュールは省きます。
- Excel がなくても読めます。VBA プロジェクトにパスワード（表示ロック）がかかっていても、コードは暗号化されていないので読めます。読み取りパスワードで保護されたブックは読めません。
- ユーザーフォームはコードだけで、コントロールの配置は含みません。コンパイル済みのコードだけを残してソースを消したブック（VBA stomping）からは取り出せません。
- スクリプトからは `workbooks.readVBA(パス)` で読めます（下記「Excel」）。
- 移植を頼むときは、[`docs/ai/`](docs/ai/README.md) のプロンプトと一緒に貼ってください。

### exe にする

```
xljs pack 集計.js            → 集計.exe
xljsc pack 集計.js -o 集計.exe （コンソール版をベースにする場合）
```

引数なしで起動した画面の「exe を作成」からも、スクリプトと保存先を選んで作れます。

作成した exe に渡した引数（ドロップしたファイルのパスなど）は `xljs.args` で受け取れます。スクリプトはバンドル済みの状態で埋め込まれるので、`import` しているファイルも一緒に入ります。画面用の HTML・CSS・画像などは、メタデータの `@html` と `@assets` に書いたものが一緒に埋め込まれます。

### メタデータ（スクリプト先頭のコメント）

```js
// ==xljs==
// @mode    app            // console（既定）| app
// @html    ui/index.html  // 最初に開く HTML（スクリプトからの相対パス）
// @assets  ui/            // exe に同梱するフォルダ・ファイル（複数行可）
// @title   売上集計ツール  // ダイアログ・ウィンドウのタイトル（既定はスクリプト名）
// @size    960x640        // ウィンドウの大きさ
// @icon    ui/app.ico     // ウィンドウのアイコン（.ico か .png。exe にも同梱）
// @titlebar custom        // メインウィンドウを自前のタイトルバーにする（既定は system）
// @lang    ja             // メッセージの言語（--lang / XLJS_LANG が優先）
// ==/xljs==
```

- エントリのスクリプトの先頭（最初のコード行より前）の `//` コメントだけを読みます。知らないキーは警告を出して無視します。
- `@mode app` にすると、スクリプトの実行前に `@html` のページを表示するウィンドウ（`app.main`）が開き、ウィンドウを閉じるまでプロセスは終了しません。`app.open({ html, title, width, height })` で別のウィンドウも開けます（`await win.closed` で閉じるのを待てます）。画面とのやり取りは「[アプリ（HTML の画面）](#アプリhtml-の画面)」を参照。サンプル: `examples/app-hello/`、`examples/app-aggregate/`
- ウィンドウは Microsoft Edge WebView2 で表示します（Windows 10/11 には通常インストール済み）。ランタイムがない場合はダウンロードページを案内して終了します。ページは `https://app.xljs/` から配信され、見えるのは `@html` と `@assets` に書いたファイルだけです（pack した exe と同じ）。ほかの URL へのリンクは既定のブラウザで開きます。
- `@icon` を書くと、メインウィンドウ・`app.open` のウィンドウ・`ui.form` / `ui.table` のウィンドウのタイトルバーとタスクバーのアイコンがそのファイルになります（.ico は複数サイズ入りを推奨）。exe ファイル自体のアイコン（エクスプローラーでの表示）は変わりません。
- `titleBar: "custom"`（`app.open` / `ui.form` / `ui.table` のオプション。メインウィンドウはメタデータの `@titlebar custom`）にすると、PWA のように OS のタイトルバーを使わず、ページの上端に自前のタイトルバー（アイコン・タイトル・最小化／最大化／閉じるボタン）を表示します。ドラッグで移動、ダブルクリックで最大化でき、ライト・ダークの配色に従います。`icon` に Font Awesome のアイコン（`"table"` や `"fa-regular fa-calendar"`）を指定するとタイトルの前に表示します（`titleBar` が `"system"` のときは `icon` は無視）。`width` / `height` はタイトルバー（32px）を除いた大きさです。コンソール・モックでは無視されます。

### デバッグ

```
xljs --debug 集計.ts
```

デバッグウィンドウと WebView2 の開発者ツールが開きます。開発者ツールで Ctrl+P からスクリプトのファイル（TypeScript ならそのまま）を開いてブレークポイントを置き、［実行］（F5）を押します。ステップ実行、変数・式の確認、コンソールでの評価、例外での一時停止など、ブラウザの開発者ツールの機能がそのまま使えます。「実行の直前で一時停止する」をオンにすると、スクリプトの 1 行目に入る直前で止まります（F11 でステップイン）。

- `console.log` の出力は通常の出力先と開発者ツールのコンソールの両方に出ます。
- 終了後もウィンドウは残り、結果（終了コード・エラー）を表示します。［閉じる］で xljs が終了します。実行中にウィンドウを閉じるとスクリプトを中止します。
- 仕組み: デバッグ時だけ、prelude とスクリプトを QuickJS ではなく WebView2（V8）のワーカーで動かし、ファイル・Excel・ダイアログなどの Rust 側の処理はメッセージで中継します（ワーカーは `Atomics.wait` で応答を待つので、スクリプトからは同期の呼び出しに見えます）。そのため Rust 側の処理 1 回ごとに往復が入り、セルを 1 つずつ大量に読み書きするスクリプトは通常より遅くなります。
- V8 は QuickJS より機能が多いので、デバッグでは動くのに通常実行では動かないことがあります（`fetch` などワーカーで使えるブラウザの機能が見えてしまう、など）。最後は通常実行で確認してください。`Blob`・`File`・`TextEncoder`・`TextDecoder`・`Intl` は通常実行と同じ xljs のものに置き換えています。
- ネイティブ UI（Windows・WebView2）が必要です。コンソール UI やモック UI では `--debug=window` と同じ動作になります。埋め込み exe は環境変数 `XLJS_DEBUG=1`（または `window`）で同じことができます（元のソースは埋め込まれていないので、バンドル後のコードでのデバッグになります）。

### エディタの補完

```
xljs init
```

`xljs.d.ts` と `jsconfig.json` が作られ、VS Code で補完と型チェックが効くようになります。

## API

### ダイアログ・ピッカー

| API | 戻り値 |
|---|---|
| `alert(msg)` / `confirm(msg)` / `prompt(msg, 既定値)` | 同期。`prompt` はキャンセルで `null` |
| `await showOpenFilePicker({ multiple, types, startIn, title })` | `FileSystemFileHandle[]`（キャンセルで `[]`） |
| `await showSaveFilePicker({ suggestedName, types, startIn })` | `FileSystemFileHandle \| null` |
| `await showDirectoryPicker({ startIn, title })` | `FileSystemDirectoryHandle \| null` |
| `ui.notify(msg, { title })` / `new Notification(title, { body })` | 画面右下に通知を出してすぐに戻る（コンソールでは 1 行表示） |

`types` は `[{ description: "Excel", accept: { "<MIME>": [".xlsx"] } }]` のほか、`accept: [".xlsx", ".xlsm"]` とも書けます。`showSaveFilePicker` で `types` を省略すると `suggestedName` の拡張子が種類になり、拡張子を消して入力しても付け足します（`集計` → `集計.xlsx`）。`startIn` には `"desktop"` `"documents"` `"downloads"`、パス、ハンドルを指定できます。

他のアプリで作業している間に `alert`・`confirm`・`prompt`・`ui.error`・ピッカー・フォームなどが開くと（Windows はダイアログを前面に出させないので、タスクバーが点滅するだけになる）、同じ内容の通知を画面右下に自動で出します。通知をクリックするとダイアログが前面に出ます。`ui.autoNotify = false` で止められます。通知が表示中のままスクリプトが終わると、通知が閉じるまで（最大 10 秒）プロセスの終了を待ちます。

### 進捗表示

```js
const p = ui.progress({ title: "処理中", total: files.length });
for (const [i, f] of files.entries()) {
  if (p.cancelled) break;
  p.update(i, f.name);
  // ...
}
p.close();

// まとめて書く場合。キャンセルされるとスクリプトが終了する（終了コード 1）ので、途中までの結果で先へ進まない
for (const f of ui.withProgress(files, { title: "処理中" })) { /* ... */ }
// キャンセル後も続けたいときは onCancel: "break"（ループが終わるだけ。ui.cancelled で判定）
for (const f of ui.withProgress(files, { title: "処理中", onCancel: "break" })) { /* ... */ }
for await (const h of ui.withProgress(dir.values())) { /* 総数不明ならマーキー表示 */ }
```

Windows では標準の進捗ダイアログ（IProgressDialog）、コンソールでは標準エラー出力にバーを表示します。

### 入力フォーム（`ui.form`）

HTML を書かずに、項目の定義だけで入力フォームを出せます。キャンセルは `null`。

```js
const v = await ui.form({
  title: "売上集計",
  description: "フォルダ内の Excel を集計します",
  fields: [
    { name: "src",  label: "集計フォルダ", type: "folder", required: true },
    { name: "out",  label: "出力先", type: "saveFile", accept: [".xlsx"], default: "集計.xlsx" },
    { name: "mode", label: "集計単位", type: "select", options: ["顧客別", "商品別"] },
    { name: "tax",  label: "税込で集計", type: "checkbox" },
  ],
  submit: "実行",
  remember: true,            // 前回の値を %APPDATA%\xljs\<スクリプト名>.json に保存して次回の初期値にする
});
if (!v) exit();              // v.src は FileSystemDirectoryHandle、v.out は FileSystemFileHandle
```

- 項目の型: `text` `password` `number`（`min` `max` `step`）`date`（`Date`）`month` `time` `select` `radio`（`options`）`checkbox` `textarea` `file` `files` `folder` `saveFile` `table`。ファイル系は「参照…」ボタンとドラッグ＆ドロップで入力でき、値はハンドルになります（`folder` にファイルを落とすとそのフォルダ、`saveFile` にフォルダを落とすと入力済みのファイル名のままそのフォルダへの保存先になります）。
- `type: "table"` で表を表示します（`rows` `columns` は `ui.table` と同じ。`height` は表示する行数）。`selectable: true`（複数）/ `"single"` を付けると行をクリックして選べ、値は選んだ行（元のオブジェクト）になります。`selectable` なしは表示だけです。列見出しのクリックで並べ替えできます。`onSubmit` の中で `ctx.table(name, rows)` を呼ぶと行を入れ替えられます（結果の表示など）。
- `editable: true`（または列ごとに `columns: [{ key, editable: true }]`）でセルをその場で編集できます。列に `type`（`text` `number` `date` `month` `time` `select` `checkbox`。既定は値から判断）、`required` `min` `max` `pattern` `options`、`validate(value, row, index)` を書くと入力のたびに検証し、エラーのセルを赤く表示して表の下に一覧を出します（クリックでそのセルへ）。行全体の検証は項目の `validateRow(row, index)`。エラーが残っていると送信できません。検証を通った値は元の行オブジェクトに直接書き込まれます。Enter で下のセルへ移ります。数値は全角数字や「1,000」も受け付けます。コンソールでは表のあとに編集する行の番号を尋ねます。
- `fields`（タブの `fields` でも可）に `{ group: "見出し", icon, help, fields: [...] }` を項目と並べて書くと、見出しの下に項目をまとめて表示します。値はグループに関係なく 1 つのオブジェクトにまとまります（グループの中にグループは入れられません）。コンソールでは `-- 見出し --` を出してから順に尋ねます。
- `tabs: [{ label, fields }]` を指定するとタブで項目を切り替えて表示します（`fields` はタブの上に常に表示）。値はタブに関係なく 1 つのオブジェクトにまとまります。エラーのある項目のタブには印が付き、そのタブに切り替わります。Ctrl+Tab でも切り替えられます。サンプル: `examples/form-tabs-table.js`
- ウィンドウのアイコンはメタデータの `@icon` で変えられます（`@mode app` でなくても有効。表も同じ）。
- 同梱の [Font Awesome Free](https://fontawesome.com/icons) のアイコンを、項目のラベル（`icon`）、タブ（`tabs: [{ label, icon, fields }]`）、ボタン（`buttons: [{ label, icon, ... }]`、送信ボタンは `submitIcon`）に付けられます。`"folder-open"` のように名前だけなら solid、`"fa-regular fa-calendar"` のようにクラスでも書けます。フォームの `icon` は `titleBar: "custom"` のときに自前のタイトルバーでタイトルの前に表示します（`"system"` では無視）。コンソール・モックでは無視されます。
- `accent` でフォームのアクセントカラー（送信ボタン・タブ・フォーカス枠・チェックボックス・表の選択行・タイトルバーのアイコン）を変えられます。`accent: "#c42b1c"` のような CSS の色はライト・ダークの両方に、`accent: { light: "#0f7b0f", dark: "#6ccb5f" }` なら配色ごとに使います。ボタンの文字色（黒か白）やホバー色は指定した色から決めます。コンソール・モックでは無視されます。
- 検証: `required`、`pattern`（＋`patternMessage`）、`min` / `max`、ファイル・フォルダの存在、項目ごとの `validate(value, values)`（メッセージを返す）。これらは入力欄を離れたときに検証して項目の下に表示し、エラーの表示中は入力のたびに検証し直します（直すとすぐ消えます）。フォームの `validate(values)`（`{ 項目名: "メッセージ" }` か全体へのメッセージを返す）は送信時だけ呼ばれるので、項目をまたぐ検証に使います。
- `onSubmit(values, ctx)` を書くと、実行後もウィンドウが開いたままになり、進捗（`ctx.progress(done, total, message)` / `ctx.withProgress(iterable)`。中止ボタンで `ctx.withProgress` のループから onSubmit が終わる）、ログ（`ctx.log` / `ctx.error`）、中止ボタン（`ctx.cancelled`）を表示します。出力ファイルを返すと完了後に「フォルダを開く」ボタンが出ます。サンプル: `examples/form-aggregate.js`
- `buttons: [{ label, onClick(values, ctx) }]` で送信ボタンの左にボタンを追加できます。押すと `onClick` が `onSubmit` と同じように実行されます（進捗・ログ・中止。ウィンドウは開いたまま）。`validate: false` を付けると検証せずに入力のままの値で実行し（読み込み・クリアなど）、`close: true` を付けると正常に終わったらフォームを閉じます。`submit: false` で送信ボタンを出さないこともできます。`ctx.setValue(name, value)` で項目の値を変えられます。コンソールでは項目のあとにどのボタンを押すか尋ねます。
- `remember` ではパスワードは保存しません。保存したくない項目には `remember: false` を付けます。保存先のフォルダは環境変数 `XLJS_CONFIG_DIR` で変えられます。

### 表表示（`ui.table`）

PowerShell の `Out-GridView` のように、配列を表で表示します。列見出しのクリックで並べ替え、上の欄で絞り込み（空白区切りで AND）、列の境界のドラッグで幅を変更、Ctrl+C でコピー（タブ区切り）、CSV / Excel に保存できます。数十万行でも表示できます。

```js
ui.table(rows, { title: "集計結果", columns: ["顧客名", "件数", "合計"] });   // 表示のみ。すぐに戻る
await ui.table(rows).closed;                                                   // 閉じるまで待つ
const picked = await ui.table(files, { title: "処理するファイル", selectable: "multiple" });
// 選んだ行（元のオブジェクト）の配列。キャンセルは []。1 行だけなら selectable: "single"
```

`columns` は省略するとオブジェクトのキー（配列の行は A, B, C…）。`{ key, label, width }` でも指定できます。

行やセルのダブルクリックには `onDoubleClick` で処理を登録できます（ネイティブ UI のみ）。

```js
ui.table(files, {
  title: "ファイル一覧",
  onDoubleClick(e) {            // e: { row, index, column, key, value, close() }
    if (e.column === "更新日時") alert(`${e.row.名前} の更新日時: ${e.value}`);
    else shell.openPath(e.row.handle);
  },
});
```サンプル: `examples/table-file-list.js`

フォームと表はネイティブ UI では WebView2 のウィンドウ、コンソール（`--ui=console`）では対話入力と罫線の表で表示します。モック UI（`--ui=mock`）では応答ファイルの `"form": [{ 項目名: 値 }]`（既定値との差分。表の項目は選ぶ行の番号の配列。`null` でキャンセル）と `"table": [[0, 2]]`（選ぶ行の番号）を使います。

### アプリ（HTML の画面）

`@mode app`（または `app.open`）のウィンドウでは、画面（HTML）とスクリプトが関数呼び出し・イベント・共有の値でやり取りできます。画面の `xljs` オブジェクトは自動で使えるようになります（`<script>` の読み込みは不要）。

```js
// ==xljs==
// @mode   app
// @html   ui/index.html
// @assets ui/
// ==/xljs==
const win = app.main;

// export した関数は画面から呼べる（this は呼び出し元のウィンドウ）
export async function aggregate({ month }) {
  win.state.progress = 0;                 // 共有の値: 画面の data-bind に反映される
  // …
  await win.invoke("render", summary);    // 画面の関数を呼ぶ（xljs.handle）
  win.emit("done", { count: 10 });        // 画面へイベント（xljs.on）
  return summary.length;                  // 戻り値は画面の Promise に返る
}
win.on("hello", (name) => console.log(name)); // 画面からのイベント（xljs.emit）
win.$("#status").text("準備完了").addClass("ok"); // 簡易 DOM 操作（送りっぱなし）
const title = await win.$("h1").text();          // 読み取りは await
const size = await win.evaluate(() => [innerWidth, innerHeight]); // 関数を画面で実行
app.handle("dynamic", (x) => x * 2);       // export 以外の名前で登録
```

```html
<!-- ui/index.html -->
<progress data-bind="progress" max="100"></progress>   <!-- win.state.progress を表示 -->
<span data-bind="summary.total"></span>                 <!-- ネストした値も表示できる -->
<div data-show="done">完了しました</div>                  <!-- 値が真のときだけ表示（!done で逆） -->
<form data-action="aggregate" data-result="count">       <!-- 送信でフォームの値を渡して呼ぶ -->
  <input name="month" type="month" required>
  <button>集計</button>
</form>
<input type="checkbox" data-model="tax">                 <!-- 入力と win.state.tax を双方向に同期 -->
<button data-action="cancel">中止</button>               <!-- フォーム外: 引数なし（data-args='[1,"a"]' で指定） -->
<script>
  xljs.handle("render", (rows) => { /* 表を描く */ return rows.length; });
  xljs.on("done", (r) => console.log(r.count));
  document.querySelector("#drop").addEventListener("drop", (e) => {
    e.preventDefault();
    xljs.invoke("load", e.dataTransfer.files);  // ファイルはスクリプトで実パス付きのハンドルになる
  });
</script>
```

- 画面→スクリプトの呼び出しは常に非同期（`await xljs.invoke(...)`）。スクリプト→画面は、読み取り（`invoke` `evaluate` `$().text()` など）は `await`、書き込み（`state` `emit` `$().text(v)` など）は送りっぱなしで、送った順に反映されます。ページの読み込み前に送ったものは読み込み後に届きます。
- 受け渡せる値は JSON の値と `Date`、ファイル/フォルダのハンドル、`ArrayBuffer`、`Error` です。画面で選んだ・ドロップしたファイル（`File`）はスクリプトで `FileSystemFileHandle`、ドロップしたフォルダは `FileSystemDirectoryHandle` になります（WebView2 113 以降）。
- `win.state` / `xljs.state` は最上位のキーだけ同期します（`win.state.summary = {...}` のように代入し直す）。相手側で値が変わると `on("state", (key, value) => ...)` が呼ばれます。
- `data-action` の関数がエラーになると、エラーのダイアログを表示します（スクリプトは続行）。`xljs.invoke` のエラーは Promise の reject で、`catch` しなければ同じくダイアログになります。スクリプトの `win.on` / `$().on` のハンドラのエラーも同様です。
- `win.$(sel).on(type, (e) => ...)` の `e` は DOM イベントではなく値（`type` `id` `name` `value` `checked` `dataset` `key`、submit ならフォームの値 `values`）。`{ preventDefault: true }` で既定の動作を止めます。
- `data-action` の実行中は要素が無効になり、`xljs-busy` クラスが付きます。
- `titleBar: "custom"` のウィンドウでは、タイトルバーが `<body>` の先頭に差し込まれます（高さは CSS 変数 `--xljs-titlebar-height`。`height: 100vh` のレイアウトは `calc(100vh - var(--xljs-titlebar-height))` に）。色は `--xljs-titlebar-bg` `--xljs-titlebar-fg` `--xljs-titlebar-accent` で変えられます。ページに `data-xljs-titlebar` 属性の要素があれば差し込まずにそれをタイトルバーとして使い（ドラッグで移動）、`data-xljs-window="minimize"` / `"maximize"` / `"close"` を付けたボタンでウィンドウを操作できます。

```html
<header data-xljs-titlebar class="my-bar">
  <span>売上集計</span>
  <button data-xljs-window="minimize">_</button>
  <button data-xljs-window="close">×</button>
</header>
```

- アイコンには同梱の [Font Awesome Free](https://fontawesome.com/icons)（7.3.1）が使えます。`<link rel="stylesheet" href="/_xljs/fontawesome/css/all.min.css">` を書き、`<i class="fa-solid fa-file-excel"></i>` のように使います（インターネット接続は不要）。

モック UI では応答ファイルの `"app"` にウィンドウごとの操作の列を書くと、画面の代わりに実行してログに出します（サンプル: `testdata/scripts/bridge.test.answers.json`）。

```json
{ "app": [[
  { "invoke": "aggregate", "args": [{ "month": "2026-04" }], "expect": 3 },
  { "emit": "hello", "args": ["画面"] },
  { "state": { "tax": true } },
  { "event": "click", "selector": "#run" },
  { "handle": "render", "result": 3 },
  { "close": true }
]] }
```

### ファイル・フォルダ

```js
const h = fs.file("C:\\data\\a.xlsx");        // FileSystemFileHandle（存在確認なし）
const d = fs.dir("C:\\data");                  // FileSystemDirectoryHandle
const file = await h.getFile();                // File（name, size, path, text(), arrayBuffer()）

for (const f of d.walk({ glob: "*.xlsx", exclude: ["old"] })) {
  console.log(f.relativePath);                 // サブフォルダも走査
}
```

- `walk()` のパターンは `"/"` を含まなければ任意の階層のファイル名に一致し、含めば起点からの相対パスに一致します（`**` 可、大文字小文字は区別しない）。除外パターンにフォルダが一致すると中身ごと除外します。
- `walk()` `list()` `fs.list()` `fs.glob()` `entries()` / `values()` は、既定で隠しファイル・システムファイル（`desktop.ini`、`Thumbs.db` など）、`.` で始まる名前、Office のロックファイル（`~$集計.xlsx`）を除きます（隠しフォルダは中も見ません）。含めるときは `{ hidden: true }` を渡します（`fs.list(p, { hidden: true })`、`d.list({ hidden: true })`）。
- ハンドルの独自拡張: `path`, `relativePath`, `exists()`, `readText()`, `writeText()`, `readBytes()` など
- `fs`: `exists` `stat` `readText` `writeText` `appendText` `readBytes` `writeBytes` `readJSON` `writeJSON` `list` `walk` `glob` `mkdir` `remove` `rename` `copy` `cwd` `chdir`
- `path`: `join` `resolve` `dirname` `basename` `extname` `relative` `normalize` `isAbsolute`
- パスを受け取る API には文字列・ハンドル・`File`（ディスク上のもの）のどれでも渡せます。
- 相対パスはカレントフォルダが基準です。エクスプローラーから起動した場合（ダブルクリック、exe やスクリプトへのドロップ）はスクリプト（埋め込み exe なら exe）のあるフォルダがカレントフォルダになります。コマンドプロンプトから起動した場合はそのままです。
- `writeText` / `appendText` の既定は UTF-8 で、`.csv` / `.tsv` のファイルだけは BOM 付き（Excel で文字化けしない）にします。
- 文字コード: `"auto"`（BOM → UTF-8 → CP932 の順に判定）, `"utf8"`, `"utf8bom"`, `"cp932"`（= `"shift_jis"`）, `"euc-jp"`, `"utf16le"` など。CP932 で表せない文字は `?` になります。

### CSV

```js
const rows = csv.read(h);                         // 見出し行をキーにしたオブジェクトの配列。数値・日付・真偽値は変換、空欄は null
const grid = csv.read("a.tsv", { header: false });// 2 次元配列（.tsv はタブ区切り）
const text = csv.read(h, { cast: false });        // 変換せず、すべて文字列のまま

csv.write("out.csv", rows);                       // 既定は BOM 付き UTF-8・CRLF
csv.write("out.csv", rows, { encoding: "cp932", columns: ["日付", "金額"] });
csv.parse(text) / csv.stringify(rows)
```

- 値の変換（既定）: `1234`・`-2.5`・`"1,234"` は数値、`2026/4/1`・`2026-04-01 09:30` は `Date`、`TRUE`/`FALSE` は真偽値、空欄は `null`。Excel で開いたときに近い結果ですが、`00123` のように先頭が 0 のコード、16 桁以上の数字、`123-4567` などは文字列のままです。
- 見出しの前後の空白は取り除きます（`"金額 "` も `r.金額` で読めます）。

### JSON

```js
const data = fs.readJSON(h);                          // パス・ハンドル・File・Blob。BOM・CP932 も自動判定
const withDates = fs.readJSON(h, { dates: true });    // ISO 形式の日時文字列を Date に戻す
fs.writeJSON("out.json", rows);                       // 2 スペースでインデント、UTF-8
fs.writeJSON("out.json", rows, { space: 0, encoding: "cp932", dates: "iso" });
fs.readJSONL("log.jsonl") / fs.writeJSONL("log.jsonl", rows, { append: true })
import master from "./master.json";                   // import でも読める（exe に埋め込まれる）
```

- `Date` は既定でローカル時刻＋オフセット（`2026-04-01T09:00:00+09:00`）で書き出します。`dates: "iso"` で `JSON.stringify` と同じ UTC 表記になります。
- 解析エラーはファイル名（JSON Lines は行番号も）付きで表示します。

### SQLite

```js
const db = sqlite.open("受注.db");                   // なければ作る
db.exec("CREATE TABLE IF NOT EXISTS 受注 (番号 TEXT PRIMARY KEY, 日付 DATE, 金額 INTEGER)");
db.run("INSERT INTO 受注 VALUES (?, ?, ?)", "A1", new Date(2026, 3, 1), 1000);  // { changes, lastInsertRowid }
db.all("SELECT * FROM 受注 WHERE 金額 > :min", { min: 500 });                   // オブジェクトの配列
db.get("SELECT count(*) AS n FROM 受注");                                      // 最初の行か null
db.insert("受注", rows, { replace: true });          // まとめて挿入（テーブルがなければ値から型を決めて作る）
db.transaction(() => { ... });                        // 例外なら取り消し（入れ子は SAVEPOINT）
db.check() / db.backup("copy.db") / db.close()
```

- `Date` はローカル時刻の文字列（`"2026-04-01"`・`"2026-04-01 09:30:00"`）で保存し、`DATE`・`DATETIME` と宣言した列は `Date` で読めます。`BOOLEAN` の列は真偽値、BLOB は `Uint8Array`。
- **ファイルサーバーで複数人が使う（共有モード）**: `sqlite.open(p, { shared: true })`、ネットワークドライブ・`\\server\share` 上なら自動。データベースの隣に `<ファイル名>.xljs-shared` フォルダを作り、以後は誰が開いても共有モードになります。呼び出しのたびにファイルを開いてロック（読み取りは共有、書き込みは排他）を取り、終わったらすぐ閉じます（ローカルより遅くなりますが壊れにくくなります）。書き込みの前に 1 時間ごとに検査と自動バックアップ（10 世代）を取り、以降の変更をすべて記録します。
- **壊れたとき**: 次の呼び出しで「修復しますか？」と確認し、はいなら修復して処理をやり直します。修復は 中断された書き込みの取り消し・索引の作り直し → 自動バックアップ＋その後の変更の再実行 → 読める行だけで作り直し の順に試し、壊れたファイルは `broken` フォルダに残します。データベースのファイルを xljs.exe にドロップするか `xljs repair 受注.db` でも修復できます。詳しくは `docs/sqlite-design.md`。

### エクスプローラー・既定のアプリ

```js
shell.showItemInFolder(dest);                 // 保存したファイルを選択した状態でエクスプローラーを開く
shell.showItemInFolder([a, b, c]);            // 複数選択（フォルダが違えばフォルダごとに開く）
shell.openPath("集計.xlsx");                  // 既定のアプリ（Excel など）で開く
shell.openPath(dir);                          // フォルダを開く
shell.openExternal("https://example.com/");   // ブラウザで開く
```

パス・ハンドル・`File` のどれでも渡せます。mock UI では実際には開かず、ログに出すだけです。

### Excel

```js
const book = workbooks.open(h);              // パス・ハンドル・File・Blob
const sheet = book.sheets["売上"] ?? book.sheets[0]; // 名前は大文字小文字・前後の空白・全角半角の違いも許す
const rows = sheet.toObjects();              // [{ 日付: Date, 金額: 1200, ... }]
const grid = sheet.toArrays({ text: true }); // 表示形式を適用した文字列

sheet.cell("B2").value = 100;
sheet.cell(3, 2).value = new Date();         // (行, 列)
sheet.cell("C10").formula = "=SUM(C2:C9)";
sheet.append([{ 日付: new Date(), 金額: 500 }]);
book.save();                                 // 上書き（save(パスやハンドル) で別名保存）

const out = workbooks.create();
out.addSheet("集計").write(summary).setColumnWidth("A:C", 16);
out.save("集計.xlsx");
```

- 新しいブック（`workbooks.create()`）の既定フォントは表示言語に合わせます（日本語では Excel と同じ游ゴシック）。`workbooks.create({ font: "Meiryo UI" })` で変更できます。
- 日付の書式が付いた数値セルは `Date` になります（Excel の日付はタイムゾーンを持たないので、ローカル時刻として扱います）。
- 数式は保存時に「開いたときに再計算」フラグを立てるので、Excel で開けば計算結果が表示されます。スクリプト内で値が必要なら `cell.calc()`（対応関数は一部）。
- xlsx / xlsm に対応しています（旧形式の .xls は不可）。マクロ、ピボットテーブル、グラフなどは読み書きの過程で失われることがあります。
- `workbooks.readVBA(src)` はマクロブック（.xlsm / .xlsb / .xlam など）の VBA コードを、ブックを開かずに読みます。`{ name, codePage, modules: [{ name, type, code, sheet }], text }` を返し、マクロがなければ `null` です。`type` は `"standard"`（標準モジュール）・`"class"`・`"document"`（ThisWorkbook とシート）・`"form"`、`sheet` はシートのモジュールのシート名（.xlsm などで分かる場合）、`text` は `xljs vba` と同じ Markdown です。

### 書式（フォント・罫線・塗りつぶしなど）

```js
// 表の体裁をまとめて付ける（見出しの太字・塗り・罫線、列幅自動、先頭行固定、フィルター）
sheet.write(rows, { table: true, columnStyles: { 金額: { format: "#,##0" } } });

// 範囲・セル単位（指定した項目だけが既存の書式に上書きされる）
sheet.style("A1:D1", { bold: true, color: "#FFFFFF", background: "#305496", align: "center" });
sheet.cell("D10").style({ font: { size: 14, bold: true }, format: "¥#,##0", border: { top: "double" } });
sheet.range("A3:D10").border({ outline: "medium", inside: { style: "thin", color: "#BFBFBF" } });

sheet.merge("A1:D1");          // セル結合
sheet.freeze("A2");            // 1 行目を固定（freeze({ rows: 1, columns: 1 }) も可）
sheet.autoFilter("A3:D10");    // フィルター
sheet.autoFit();               // 列幅を内容に合わせる（全角は 2 文字分）
sheet.setRowHeight(1, 30);
sheet.cell("A1").getStyle();   // 現在の書式を取得
```

| 項目 | 指定方法 |
|---|---|
| `font` | `{ name, size, bold, italic, underline, strike, color, vertAlign }` |
| `fill` | `"#RRGGBB"`（`null` で塗りなし） |
| `border` | `"thin"` など（全セルの上下左右）、または `{ all, outline, inside, horizontal, vertical, top, bottom, left, right, diagonalUp, diagonalDown }`。各辺は `"thin"` `"medium"` `"thick"` `"double"` `"dashed"` `"dotted"` `"hair"` など、または `{ style, color }` |
| `alignment` | `{ horizontal, vertical, wrap, shrink, indent, rotation }` |
| `numberFormat` | `"#,##0"` `"yyyy/m/d"` `"0.0%"` `"¥#,##0"` など（`"General"` で標準） |
| 省略形 | `bold` `italic` `underline` `strike` `color` `fontSize` `fontName` `background` `align` `valign` `wrap` `format` |

- 色は `"#RRGGBB"`・`"#RGB"`・`red` などの色名で指定します。
- 既定では既存の書式にマージします。`{ replace: true }` を渡すと書式を置き換えます。
- 範囲の端に引いた罫線は、隣の（書式が設定済みの）セルの接する辺にも反映するので、線が食い違いません。
- サンプル: `examples/xlsx-style-report.js`

### 日付（Day.js）

[Day.js](https://day.js.org/) を組み込んでいます（グローバルの `dayjs`。`import` は不要）。エラー・警告と同じ言語（システムの表示言語、`--lang` で変更可）のロケールと主なプラグイン（customParseFormat, isBetween, isSameOrAfter / isSameOrBefore, minMax, quarterOfYear, weekOfYear, isoWeek, dayOfYear, duration, relativeTime, advancedFormat, isLeapYear, isToday など）を読み込み済みです。

```js
const d = dayjs(r.日付);                       // Date・"2026/4/1"・"2026-04-01"（文字列はローカル時刻）
d.format("YYYY年M月D日(ddd)");                // "2026年4月1日(水)"
d.add(1, "month").endOf("month");              // 翌月末
dayjs(b).diff(a, "day");                       // 日数の差
dayjs("20260401", "YYYYMMDD");                 // 形式を指定して読む
dayjs.fromExcel(45383);                        // Excel の日付シリアル値 → 2024/4/1（xljs 拡張）
sheet.cell("A1").value = dayjs();              // dayjs の値はセル・CSV・JSON・表にそのまま渡せる（Date として扱う）
```

- ロケールはエラー・警告と同じ言語です（日本語以外は英語。`dayjs.locale("ja")` / `dayjs.locale("en")` で切り替え）。
- Day.js は MIT ライセンスです（`src/api/dayjs/LICENSE`）。

### その他

- `console.log/error/table/time` など、`setTimeout` / `setInterval`、`sleep(ms)`
- `fs.readJSON` などのほか、`csv.read` / `workbooks.open` もパス・ハンドル・`File`・`Blob` を受け付けます
- `xljs.args` `xljs.script` `xljs.scriptDir` `xljs.version` `xljs.title`（ダイアログのタイトル。`@title` で変更可） `xljs.env(name)` `exit(code)`
- 補っている組み込み: `Array.fromAsync`, `structuredClone`, `TextEncoder` / `TextDecoder`（Shift_JIS 対応）, `atob` / `btoa`, `Blob` / `File`, 数値・日付の `toLocaleString`（日本語形式）, 簡易 `Intl`

## ビルド

Rust（stable、1.91 以降）が必要です。JavaScript エンジンの QuickJS は C で書かれているので、C コンパイラも使います（Windows では Visual Studio Build Tools の MSVC、Linux からのクロスビルドでは MinGW-w64）。

```
# Windows 上（MSVC）: x64 の GUI 版とコンソール版
cargo build --release                                  # target\release\xljs.exe（GUI 版）
cargo build --release --features console --target-dir target\console   # コンソール版（xljsc.exe として配布）

# x64 / arm64 の両方を dist/ に（Git Bash などの bash で）
make windows
make dist           # dist/xljs-windows-x64.zip と xljs-windows-arm64.zip

# Linux から x64 版をクロスビルド（apt install mingw-w64）
make windows-gnu    # dist/x64/
```

- GUI 版（xljs.exe）は `windows_subsystem = "windows"` でビルドし、ターミナルから起動したときはそのコンソールに接続します。`console` フィーチャーを付けるとコンソール版（xljsc.exe）になります。
- exe のアイコン（`assets/xljs.ico`）とマニフェスト（`src/cli/xljs.manifest`、ビジュアルスタイル・高 DPI 対応）は `build.rs` が `src/cli/xljs.rc` をリソースとして埋め込みます。アイコンを作り直すときは `python3 tools/make_icon.py ロゴ.png`（`assets/xljs.png` と `assets/xljs.ico` を生成）。
- WebView2 は WebView2Loader.dll を使わず、インストール済みのランタイムを直接読み込みます（`src/ui/native/loader.rs`）。配布物は exe 1 つです。

## 開発

```
cargo test --lib                                   # Linux でも全ロジックをテスト可（ネイティブ UI 以外）
cargo check --target x86_64-pc-windows-gnu --all-targets   # Windows 専用コードのコンパイル確認
```

- `testdata/scripts/*.test.{js,ts}` はモック UI で実行される結合テストです。`*.answers.json` に応答を書きます。同じスクリプトをデバッガ経由（偽のデバッグページ）でも実行します。
- UI はトレイトで分離しているので、ネイティブダイアログ以外のロジックは Linux でもテストできます。Windows では `src/ui/native/tests.rs` がダイアログを実際に開いて自動で閉じます。
- スクリプトは専用のスレッドで動き、メインスレッドが UI（ダイアログ・ウィンドウのメッセージループ）を受け持ちます（`ui::run`）。重い処理の間もウィンドウが固まりません。

| パス | 内容 |
|---|---|
| `src/main.rs`, `src/cli` | CLI（引数処理、サブコマンド、開始画面、コンソール処理） |
| `src/engine` | バンドラ（oxc）、QuickJS での実行、イベントループ、エラー表示、デバッガ |
| `src/api` | Rust 側のネイティブ関数と `prelude.js`（公開 API の JS 実装） |
| `src/i18n` | メッセージの多言語化（`locales/*.json` を埋め込み） |
| `src/ui` | UI トレイトと native（Win32・WebView2）/ console / mock 実装、UI スレッド |
| `src/pack` | exe へのスクリプト・アセット埋め込み（zip） |
| `src/meta` | スクリプト先頭のメタデータの読み取りとアセットの範囲 |
| `src/vba` | マクロブックの VBA コードの読み取り |
| `types/xljs.d.ts` | 型定義（`xljs init` で出力） |

スクリプトは [oxc](https://oxc.rs/) で TypeScript を変換し、ES モジュールを async 関数に変えて（oxc の module runner 変換）1 ファイルにまとめ、[QuickJS](https://github.com/quickjs-ng/quickjs)（[rquickjs](https://github.com/DelSkayn/rquickjs)）で実行します。インラインのソースマップでエラーの行番号は元のファイルの位置になります。`--debug` のときだけはデバッグウィンドウ（WebView2）のワーカーで実行し、ネイティブ関数を Rust 側からメッセージで提供します（`src/api/remote.rs`、`src/engine/debug.rs`、`src/ui/web/debug*.js`）。

### Go 版との違い

- JavaScript エンジンが goja から QuickJS に、xlsx のライブラリが excelize から umya-spreadsheet に変わりました。エンジンの英語のエラーメッセージの文面が違います（日本語訳は `ja.json` の `translations` で付けています）。
- `cell.calc()` は xljs 内蔵の簡易計算です（四則演算・比較・文字列連結・セル参照、SUM・AVERAGE・MIN・MAX・COUNT・ROUND・IF・SUMIF・COUNTIF などの代表的な関数）。
- 組み込みの書式コードの `getStyle().numberFormat` は番号ではなく書式の文字列で返ります（`"#,##0"` など）。
- `--debug` の偽ページのテストは QuickJS で行います。実機の WebView2 での動作確認は Windows の CI（`webview_window` テスト）で行います。

## ライセンス

Apache License 2.0（[LICENSE](LICENSE)）。移植元の Go 版 xljs は MIT ライセンスです。

同梱のライブラリ: Day.js（MIT、`src/api/dayjs/LICENSE`）、Font Awesome Free（アイコン CC BY 4.0・フォント SIL OFL 1.1・CSS MIT、`src/ui/web/fontawesome/LICENSE.txt`）。
