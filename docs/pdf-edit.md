# PDF の編集（グローバルの `pdf`）

既存の PDF を結合・ページの削除・挿入・回転し、文字を書き足す機能の設計と実装のメモ。
PDF の出力（`book.toPdf` など）とは別の仕組みで、WebView2 も Excel も使わない（Linux のテスト・mock・コンソールでも動く）。

## 1. 使い方

```js
const doc = pdf.open("a.pdf", { password: "..." });   // パス・ハンドル・File・Blob・ArrayBuffer
doc.append("b.pdf", { pages: "1-3" });
doc.insert(2, doc, { pages: [-1] });                   // 自分自身から = ページの複製
doc.insertBlankPage(1, { paper: "A4", landscape: true });
doc.removePages("5-6,-1");
doc.rotatePages("all", 90);
doc.addText("all", (p) => `${p.number} / ${p.count}`, { position: "bottom-center", size: 9 });
doc.addText("all", "資料番号 A-001", { position: "top-right", margin: { x: 15, y: 8 } });
doc.save("out.pdf");                                    // this を返す。省略で開いたファイルに上書き
pdf.merge(["a.pdf", "b.pdf"], "c.pdf");                 // 保存して絶対パス（dest なしは PdfDocument）
```

- ページ番号は 1 始まり、ページの指定は番号・配列・`"1-3,5"`・`"3-"`（最後まで）・`"5-3"`（逆順）・`"all"`、負の番号は後ろから
  （prelude の `pdfPageIndexes` で 0 始まりの配列にし、範囲外は Rust 側が `pdfdoc.badPage` で 1 始まりの番号を出す）。
- 長さは mm、文字の大きさは pt（Excel・印刷のオプションと同じ）。`pages` の `width` / `height` は表示したとき（回転後）の大きさ。
- 変更はメモリ上の文書に対して行い、`save` / `toArrayBuffer` / `toBlob` で PDF に書く。

## 2. 文書（`src/pdf/mod.rs`）

- 読み書きは lopdf（`default-features = false`）。暗号化された PDF は空のパスワードか `password` で復号して読む（読めなければ `pdfdoc.password` /
  `pdfdoc.badPassword`）。保存した PDF は暗号化しない。
- **ページの木は開いたときに平らにする**: 各ページに継承される属性（`Resources` `MediaBox` `CropBox` `Rotate`）を親からたどって
  ページ自身に書き、ルートの Pages から消す。以後は `PdfDoc::pages`（ページのオブジェクト番号の並び）が正で、変更のたびに
  ルートの `Kids` / `Count` と各ページの `Parent` を書き直す（`rebuild_tree`）。途中の Pages ノードは保存時の `prune_objects` で消える。
- **ページの取り込み**（`insert_pages`）: ページの辞書（継承した属性込み、`Parent` `StructParents` `B` は除く）を新しい番号で入れ、
  参照をたどって中身をコピーする（`copy_obj`。同じ取り込みの中では同じオブジェクトは 1 回だけ。キューで反復するので深い参照でもスタックを使わない）。
  取り込まないページ・Pages ノード・カタログへの参照は `null` にする（注釈のリンク先や `/P` から文書全体を引き込まないため）。
  しおり・フォーム（AcroForm）・構造ツリー・ページラベルは取り込まない。同じ文書から取り込むときは natives が文書を複製してから渡す。
- **削除**は並びから外すだけ（しおりなどから参照されていれば保存後も孤立したオブジェクトとして残る。`Parent` は消す）。ページの並びを変えたら
  保存時にカタログの `PageLabels` を消す（番号がずれるため）。
- **回転**は `Rotate` に足して 0〜270 に。白紙のページは `MediaBox` と空の `Resources` だけ。

## 3. 文字の追加（`add_text`）

- 座標は「表示したときのページ」（`CropBox`（なければ `MediaBox`）を `Rotate` だけ回したもの）の左上からの mm。
  内容の先頭に `cm` で表示の座標（左下が原点、pt）→ ページの座標の変換を置く（0°: `1 0 0 1 x0 y0`、90°: `0 1 -1 0 x1 y0`、
  180°: `-1 0 0 -1 x1 y1`、270°: `0 -1 1 0 x0 y1`）。
- 位置: 置き場所の点（アンカー）と、文字のかたまり（全行）のどこをそこに合わせるか（横: 左端・中央・右端、縦: 上端・中央・下端）で決める。
  - `position`（`"top-left"` … `"bottom-right"`・`"center-left"`・`"center"`。prelude の `pdfPosition` が `"右下"` `"bottomRight"` `"bottom"` `"middle left"` などをこの形にする）:
    四隅・辺の中央・ページの中央から `margin`（mm、既定 10、[横, 縦]）だけ内側。かたまりの合わせる側も同じ（右下ならかたまりの右下）。
  - `x`（かたまりの左端。`align` が center / right なら中央・右端）か `right`（右端）、`y`（上端）か `bottom`（下端）。指定した向きは `position` より優先。
  - どれもなければ `align` に応じてページの左端・中央・右端、上端。
  - 行の揃え（`align`。省略時はかたまりの合わせる側）はかたまりの幅（最も長い行）の中で。1 行目のベースラインはかたまりの上端から最初のフォントの ascender、
    かたまりの高さは ascender ＋ descender ＋ 行の間隔（`size × lineHeight`）×（行数 − 1）。
  - `rotate` はアンカーを中心に反時計回り（`center` ならかたまりの中心で回るので透かしに使える）。
  `opacity` は ExtGState（`ca` / `CA`）。
- 既存の内容は最初に文字を足すときだけ `q` … `Q` で囲む（内容がグラフィックスの状態を戻さずに終わっても、足した文字がずれないように）。
  `Contents` は配列にして後ろに足す。ページのリソース（`Font` `ExtGState`）は複数のページで共有されていることがあるので、ページ自身の辞書にコピーしてから足す。
- 文字の描画は `Tj` に CID（2 バイト）を 16 進で。カーニング・合字・縦書き・複雑なスクリプトの字形処理はしない。タブは空白、制御文字は捨てる。

### フォント（`src/pdf/font.rs`）

- `font` は名前かファイルのパス（.ttf .ttc .otf .otc）。配列なら文字ごとに前から探し、どれにもない字は最初のフォントの .notdef。
  既定は表示言語の Excel のフォント（日本語は游ゴシック）＋ OS ごとの候補（Windows: Yu Gothic・Meiryo・MS Gothic・Arial・Segoe UI）。
  既定の候補は字が必要になったときに読む（見つからなければ次）。
- 名前の探し方: Windows の主なフォントは名前 → ファイル名の表（`KNOWN`）から `%WINDIR%\Fonts`・`%LOCALAPPDATA%\Microsoft\Windows\Fonts`
  を見る。TTC の中は name テーブル（ファミリー名・フルネーム・優先ファミリー名、全言語）が一致する面を選ぶ（`ＭＳ Ｐゴシック` → msgothic.ttc の
  該当の面）。表にない名前はフォントフォルダ全体の name テーブルを 1 回だけ読む（ファイルの先頭と name テーブルだけをシークして読む）。
  名前の比較は全角英数を半角に、空白・`-`・`_` を除き、小文字で。見つけた結果と読み込んだフォントはプロセスの間覚えておく。
- 埋め込み: 文書ごと・フォントの面ごとに `FontUse` を持ち、使った字を `subsetter::GlyphRemapper` で新しいグリフ番号に振る。
  その番号をそのまま CID として内容に書くので、内容は文字を足したときに確定し、フォント本体だけを保存時（`finish`）にまとめて書く
  （Type0 / `Identity-H`、`CIDFontType2`＋`FontFile2`（TrueType）か `CIDFontType0`＋`FontFile3 /CIDFontType0C`（CFF。サブセットの `CFF ` テーブル）、
  `W` に幅、`ToUnicode` で検索・コピー用の文字）。保存したあとに同じフォントで書き足すと新しい `FontUse`（別のサブセット）になる。
  CFF2（可変フォント）は使えない。
- 取り込み元の文書に足した文字は、取り込む前にその文書の `finish` でフォントを書いてから取り込む。

## 4. できないこと

- 既存の文字の編集・削除、画像の追加、注釈・フォームの編集、しおりの編集、ページの切り抜き、圧縮の最適化、暗号化しての保存、
  電子署名を保ったままの保存（増分保存はしない）。
- 文字の縦書き・カーニング・合字・アラビア文字などの字形処理、太字・斜体の疑似的な表示（太字のフォントを名前で指定する）。

## 5. テスト

- `src/pdf/tests.rs`: 入れ子のページの木（継承した `MediaBox` `Resources` `Rotate`）、結合・削除・回転・白紙、暗号化、文字の追加
  （回転したページの変換・色・不透明度・フォントの共有）、`position` と `margin`・`x` の優先（`Tm` の座標で確かめる）、日本語のフォールバック、CFF のフォント、エラー。結果は lopdf の
  `extract_text`（ToUnicode 経由）で確かめる。フォントがない環境では文字のテストを飛ばす。
- `testdata/scripts/pdfedit.test.js`（mock、デバッガ経由でも）: JS の API・ページの指定・エラーの文・保存と読み直し・merge・パスワード。
- 見た目は `pdftoppm` で画像にして確認した（回転したページの文字の向き・透かし・中央揃え）。
