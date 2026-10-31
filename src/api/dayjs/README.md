# Day.js

[Day.js](https://day.js.org/) 1.11.23（MIT ライセンス、`LICENSE`）。npm パッケージ `dayjs` のファイルを変更せずに置いている。
`dayjs.go` が CommonJS のモジュールとして読み込み、prelude がグローバルの `dayjs` にする（`plugin/` のプラグインはすべて組み込み済み、日本語ロケール付き）。

更新するときは新しいパッケージの `dayjs.min.js`、`locale/ja.js`、`plugin/` の同じファイルで置き換え、この README の版を直す。
