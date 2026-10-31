# PDF 出力（book.toPdf / sheet.toPdf / xljs.htmlToPdf）

Excel がなくても、ブックを「Excel で印刷したときのページ」で PDF にする機能の設計と実装のメモ。
シートのレイアウト（ページ分割まで）は Rust で HTML にし、WebView2 に描かせて PDF にする。

## 0. 方針

- **Excel の COM（`ExportAsFixedFormat`）は使わない**（Excel 未インストールでも動かす）。
- **ページ分割は自前**: Chromium の表の改ページに任せず、Excel と同じ考え方（印刷範囲・用紙から余白を引いた幅と高さ・拡大縮小・
  手動の改ページ・印刷タイトル）でページごとの行と列を決め、1 ページ = 1 つの `<section class="xljs-page">` にする。
  ページの大きさ・余白・ヘッダー/フッターは CSS の `@page`（シートごとの名前付きページ `page: sN`）で表す。
- **PDF にするのは Chromium**: WebView2 の DevTools プロトコル `Page.printToPDF`（`preferCSSPageSize: true`）。
  `ICoreWebView2_7::PrintToPdf` は用紙を 1 つしか指定できず、シートごとに向き・用紙が違うブックを 1 つの PDF にできないため使わない。
- **グラフは既存の仕組み**（`sheet.charts` → `ChartSpec` → `web/chart.js`）で描く。Rust はグラフの位置（アンカー）だけを HTML に書き、
  prelude が `ChartSpec` を作り、ページ `web/print.html` が AG Charts / Frappe Gantt で枠の中に描く。
- **見た目の再現は「だいたい同じ」まで**。自動の改ページ位置は列幅・行の高さから計算するので、フォントの計測が違う分だけずれることがある。

## 1. 使い方

```js
const book = workbooks.open("請求書.xlsx");
await book.toPdf("請求書.pdf");                                    // 非表示でないすべてのシートを 1 つの PDF に
await book.toPdf("一覧.pdf", { sheets: ["一覧"], fitToWidth: 1 });   // 横 1 ページに収める
await book.sheets["明細"].toPdf("明細.pdf", { paper: "A4", landscape: true, margin: 10 });
const html = book.toHtml();                                        // 同じページを HTML で（グラフは空の枠）
await xljs.htmlToPdf("<h1>報告書</h1>", "報告書.pdf", { paper: "A4" }); // 任意の HTML から
```

- `toPdf` は書き出したファイルの絶対パスで resolve する。WebView2 がない UI（コンソール）ではエラー。mock は書かずにログだけ出す
  （`pdf("…/a.pdf", 4 pages, 1/1 charts)`）。
- `options`（`PrintOptions`）: `sheets` `paper` `landscape` `scale` `fitToWidth` `fitToHeight` `fitToPage` `margin`（mm）`gridLines`
  `centerHorizontally` `charts: false` `title`。省略した項目はシートのページ設定に従う。
- `htmlToPdf` は `options` の用紙・向き・余白を `@page` にして文書の `<head>` の先頭に入れる（文書の `@page` があればそちらが勝つ）。
  文書のスクリプトは動かない（`DOMParser` で取り込むため）。

## 2. シートのレイアウト（`src/api/xlsx_print.rs`、native は `xlsxPrintHtml`）

### ページ設定の読み方

| 項目 | 読み方 |
|---|---|
| 用紙・向き・拡大縮小 | umya の `PageSetup`（`paperSize` のコード → mm。不明なら A4） |
| 次のページ数に合わせて印刷 | `fitToPage`（umya）＋ `fitToWidth` / `fitToHeight`（**シートの XML から直接**。umya は属性がないこと＝1 と 0＝自動を区別できない） |
| 余白・ヘッダー/フッターの位置 | `PageMargins`（ないときは Excel の「標準」0.7/0.75/0.3 インチ） |
| 印刷範囲・印刷タイトル | 定義された名前 `_xlnm.Print_Area` / `_xlnm.Print_Titles`（シートの名前、ブックの `localSheetId` 付きの名前）。複数の範囲は範囲ごとにページを分ける |
| 改ページ | `rowBreaks` / `colBreaks`（`id` は 0 始まりで、その次の行・列から新しいページ） |
| ページの順序・枠線の印刷 | `pageOrder="overThenDown"`、`printOptions gridLines`（シートの XML から直接） |
| 非表示のシート | `workbook.xml` の `state`（umya は読まない）。ブックを開いたときに `RawPrint`（`Book::print`）に保持する。シート名の変更に追従する |
| ヘッダー/フッター | `oddHeader` / `oddFooter` の `&L` `&C` `&R`。`&P` `&N` → `counter(page)` / `counter(pages)`、`&D` `&T` `&A` `&F` `&Z`、`&B` `&I` `&U` `&S`、`&"フォント,太字"`、`&nn`（サイズ）、`&K`（色）。先頭ページ・偶数ページ用と画像（`&G`）は使わない |

印刷範囲がなければ使用範囲（値・塗り・罫線のあるセル、値のある結合セル、画像・グラフ）。先頭の空の行・列は印刷しない。

### 大きさ

- 列幅（文字数）→ ピクセル: Excel と同じ `trunc(((256·w + trunc(128/mdw)) / 256)·mdw)`。最大の数字の幅 `mdw` は既定のフォントから
  （日本語のフォントは 11pt で 8px、それ以外は 7px）。既定の列幅は `defaultColWidth`、なければ `baseColWidth·mdw + 5` を 8 の倍数に切り上げ。
- 行の高さ: 行の `ht`、なければ既定のフォントから（游ゴシック 11pt → 18.75pt、ＭＳ 系 → 13.5pt、Calibri → 15pt）。`customHeight` のときと
  知らないフォントのときはシートの `defaultRowHeight`。非表示の行・列は 0。
- 「次のページ数に合わせて印刷」: 倍率 = min(1, 幅×横のページ数÷全体の幅, 高さ×縦のページ数÷全体の高さ)（1% 単位に切り捨て、10% 以上）。
  このとき手動の改ページは使わない（Excel と同じ）。

### ページの分け方

列と行を別々に、先頭から足していき「余白を引いた幅（高さ）÷倍率」を超える手前で切る（手動の改ページでも切る）。
印刷タイトルの行・列は、それより後ろから始まるページの先頭に入れ、その分だけ使える高さ（幅）を減らす。
ページの順序は既定で「左から右へ」ではなく「上から下へ」（列のまとまりごとに行のまとまりを並べる）。

### HTML

```html
<section class="xljs-page" style="page:s0;--w:…;--h:…;--pw:…;--ph:…;--m:…">   <!-- 1 ページ -->
  <div class="xljs-sheet" style="zoom:0.85">                                 <!-- 拡大縮小 -->
    <table>…</table>                                                          <!-- 印刷タイトル＋このページの行・列 -->
    <div class="xljs-objs" style="left;top;width;height">                    <!-- 画像・グラフ（本体の範囲で切る） -->
      <img src="data:image/png;base64,…" style="left;top;width;height">
      <div class="xljs-print-chart" data-chart="0" style="…"></div>
    </div>
  </div>
</section>
```

- セルは `<td>` の中の絶対配置の `<div>`（大きさは pt で指定）に値を入れる。行の高さは内容で伸びない。
  文字のはみ出しは Excel と同じく隣の空のセルへ（左揃え・標準は右へ、右揃えは左へ、中央は両側へ）。
- 書式はセルの CSS をまとめてクラス（`.k0` …）にする。フォント（游ゴシック → `Yu Gothic` などの別名と代わりのフォント）、太字・斜体・下線・取り消し線、
  文字の色（テーマの色は umya の `argb_with_theme`）、表示形式の色（`[Red]` `[赤]` など）、塗り（単色）、罫線（細線 = 0.75pt）、斜線（グラデーション）、
  横・縦の配置、折り返し、インデント、回転（縦書きは `writing-mode`）、リッチテキスト（ランごとの `<span>`）。
- 数式で値の保存がないセル（スクリプトで書いた数式）は `xlsx_calc` で計算する。
- 結合セルはページの中で `colspan` / `rowspan`。ページの境目をまたぐ結合はページごとに切る。
- 画像・グラフはアンカー（two-cell / one-cell / absolute）からシート上の位置を求め、重なるすべてのページに置いて本体の範囲で切る（ページをまたぐ図は分かれて出る）。
- 画面で開いたとき（`@media screen`）はページを灰色の背景の上に用紙の大きさで並べる（`toHtml` の確認用）。

## 3. PDF にする（`Ui::print_pdf`、`src/ui/native/webview.rs`、`src/ui/web/print.html` / `print-page.js`）

1. prelude の `printToPdf`: `xlsxPrintHtml` でレイアウト、`sheet.charts[i]` から `ChartSpec`（描けないグラフは `{ error }`）を作り、
   `native.pdfPrint(html, charts, texts, path)`（Promise。成功なら ""、失敗ならエラーの文）。
2. `Ui::print_pdf(job, done)`: ネイティブ UI は**見えないウィンドウ**（`WindowOptions::hidden`: `WS_EX_LAYERED` の不透明度 1/255、
   `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT`、`SW_SHOWNOACTIVATE`）に `print.html` を開く。
   完全に隠したウィンドウでは `requestAnimationFrame` が止まり AG Charts が描けないおそれがあるため、表示はしたまま見えなくしている。
   ウィンドウの一覧（`WINDOWS`）には入れず、通知も出さない。
3. ページが `ready` → `Emulation.setEmulatedMedia`（`prefers-color-scheme: light`。ダークモードでもグラフを明るい色で）→ `init` を送る。
4. ページは文書を `DOMParser` で取り込み（`<style>` と `<link rel=stylesheet>` は head へ、スクリプトは捨てる）、フォントと画像を待ち、
   グラフを描く（アニメーションなし、`overrideDevicePixelRatio: 3` で印刷に耐える解像度、ガントは表示の切り替えなし）。
   描けたグラフ（キャンバスに色のある画素がある、または SVG がある）が揃うまで最大 5 秒待って `rendered`（`charts` と `drawn` の数を添える）。
5. `Page.printToPDF`（`printBackground`、`preferCSSPageSize`、余白 0 = `@page` の余白を使う）→ base64 を復号してファイルに書き、`done`、ウィンドウを閉じる。
   ページが `error` を送ったときと、途中でウィンドウが閉じたときは失敗。
6. スクリプト側は `poster.add_pending()` で待っているので、終わるまでプロセスは終了しない。スレッドの転送（`Threaded`）は待たずに UI スレッドへ投げる。

## 4. できないこと・違い

- 条件付き書式、図形・テキストボックス・ワードアート、SmartArt、コメント、ヘッダー/フッターの画像、先頭ページ・偶数ページ用のヘッダー。
- 縮小して全体を表示（shrink to fit）、`#####` の表示（数値が入りきらない）、塗りのパターン（単色以外は前景色で塗る）、グラデーションの塗り。
- 先頭ページ番号（`firstPageNumber`）、行列番号の印刷、白黒印刷・簡易印刷、印刷の品質。
- Excel 2016 以降のグラフ（chartEx）やレーダーなど `sheet.charts` で描けないもの（枠の中に理由を書く）。
- 行の高さの自動調整（折り返しのある行で `ht` がないとき、Excel が開いたときに広げる分）は再現しない。

## 5. テスト

- `src/api/xlsx_print.rs` の単体テスト: 範囲の読み方、ページの分け方（印刷タイトル・改ページ・非表示）、ヘッダー/フッター、表示形式の色、ページ設定の XML。
- `testdata/scripts/pdf.test.js`（mock）: 新しいブックの HTML（値・結合・罫線・塗り・負数の赤）、オプション、検査、複数シート、
  `charts.xlsx` のグラフの枠、`testdata/xlsx/print.xlsx`（`make_print.py` で openpyxl から生成。横向き・横 1 ページ・印刷タイトル・
  ヘッダー/フッター・改ページ・画像・グラフ・非表示の列とシート・印刷範囲・B5）、mock のログ。デバッガ経由でも同じ（`scripts_in_debugger`）。
- `src/ui/native/tests.rs`（Windows CI）: `print_page_draws_charts_hidden`（見えないウィンドウで縦棒とガントが描けること）、
  `print_pdf_writes_a_file`（`print.xlsx` を PDF にしてファイルができること）。
- 見た目は Playwright の Chromium で `print.html` に偽の `chrome.webview` を付けて `init` を送り、`page.pdf({ preferCSSPageSize: true })` →
  `pdftoppm` で画像にして確認した（`Page.printToPDF` と同じ経路）。
