# サンプル

ファイル名の先頭が、そのサンプルで主に見せている機能です。どれも `xljs <ファイル名>` で実行できます。

| サンプル | 機能 | 内容 |
|---|---|---|
| `dialog-aggregate.js` | ダイアログ・ピッカー | フォルダを選び、Excel ファイルを集計して保存する（`showDirectoryPicker`・`ui.withProgress`・`showSaveFilePicker`・`alert`） |
| `progress-cancel.js` | 進捗表示 | 進捗ダイアログとキャンセル（`ui.progress`） |
| `form-aggregate.js` | 入力フォーム（`ui.form`） | HTML なしのフォームで条件を入力し、開いたままのウィンドウで進捗・ログを表示しながら集計する（`onSubmit`・`remember`） |
| `form-tabs-table.js` | 入力フォーム（`ui.form`） | タブで項目を切り替え、表から対象を選び、結果を表に表示する（`tabs`・`type: "table"`・`ctx.table`） |
| `table-file-list.js` | 表表示（`ui.table`） | ファイル一覧を表で表示し、選んだファイルを処理する（`selectable`・`onDoubleClick`） |
| `app-hello/` | アプリ（HTML の画面） | 最小のアプリ。フォームの値を export した関数に渡し、結果を表示する（`@mode app`・`data-action`・`data-bind`） |
| `app-aggregate/` | アプリ（HTML の画面） | フォルダ選択 → 集計 → 進捗 → 結果の表 → 保存。ドロップ・中止・画面の関数呼び出しも使う（`win.state`・`win.invoke`・`xljs.invoke`） |
| `csv-to-xlsx.js` | CSV → Excel | CSV（Shift_JIS / UTF-8 自動判定）を Excel ブックに変換する |
| `xlsx-to-json.js` | Excel → JSON | Excel の先頭シートを JSON / JSON Lines に書き出す |
| `xlsx-style-report.js` | Excel の書式 | フォント・罫線・塗りつぶし・表示形式・セル結合・ウィンドウ枠の固定などを使った帳票 |
| `pack-drop-xlsx-to-csv.js` | exe 化（`xljs pack`） | exe にドロップした Excel を CSV に変換する（`xljs.args`） |
