# xljs (Rust 版) — 開発メモ（Claude Code 向け）

JavaScript/TypeScript のスクリプトで Excel・CSV・JSON を処理する Windows 用シングルバイナリ。
Go 版 xljs（github.com/cool0707/xljs）の Rust 移植。開発はこの Rust 版で進め、Go 版は凍結している
（Go 版に合わせる必要はない。スクリプトの API・メタデータ・pack 形式の互換は保つ）。利用者は「VS Code も入れられないような」一般の業務ユーザー。
Excel 未インストール環境で動くこと、日本語環境での使い勝手（CP932、游ゴシック、日本語メッセージ）を重視する。

HTML UI・フォーム・表表示・デバッガの設計は `docs/handoff-ui.md`（Go 版で書かれたメモ。仕組みは同じ）を参照。
グラフ・ガントチャート（`ui.chart`・フォームの `chart` 項目・`xljs.chart`、AG Charts / Frappe Gantt、Excel のグラフ `sheet.charts` からの変換）は `docs/charts-design.md`。
PDF 出力（`book.toPdf`・`sheet.toPdf`・`xljs.htmlToPdf`。シートの印刷レイアウトを HTML にして WebView2 の `Page.printToPDF`）は `docs/pdf-design.md`。
PDF の編集（グローバルの `pdf`: 結合・ページの削除・挿入・回転・文字の追加。lopdf ＋ フォントのサブセット埋め込み）は `docs/pdf-edit.md`。
開発モード（`xljs dev`。AI チャットの枠＝2 つ目の WebView2、ダウンロードの差分と置き換え、`.xljs/versions` の履歴、別プロセスでの実行、エラーの場所と AI への依頼文、フォームのプレビュー）は `docs/dev-mode.md`。

## 構成

| パス | 役割 |
|---|---|
| `src/main.rs`, `src/cli` | CLI。グローバルフラグ（`--ui` `--ui-answers` `--lang` `--debug`、Go の flag と同じ書式）→ サブコマンド（pack/check/init/vba/dev/version/help。スクリプトの代わりにブックを渡すと VBA の取り出し。引数なしは `launch.rs` の開始画面＝`Ui::choose`。アイコンは `src/cli/icons/*.ico`）→ メタデータを読んで `ui::run` でスクリプト実行。`console.rs` はコンソールへの接続（GUI 版） |
| `src/engine` | `bundle.rs`: oxc で TS 変換 → module runner 変換（import を `await __vite_ssr_import__`、export を getter に）→ モジュールごとの async 関数を 1 つの式にまとめ、インラインのソースマップを付ける（CommonJS・JSON・.txt・node_modules も解決）。`mod.rs`: QuickJS（rquickjs）で実行、`Error.prepareStackTrace` でスタックを元のファイルに対応付け。`loop_.rs` はタイマーと他スレッドからの post。`debug.rs` は `--debug` のデバッグセッション（後述） |
| `src/api` | Rust 側ネイティブ関数（`N::set("name", |host, args| ...)` で natives オブジェクトに登録）と `prelude.js`（Go 版から移したもの。公開 API をすべて JS で組み立てる）。Rust→JS は `native.xxx` 経由のみ。`Host` がスクリプト実行中の状態（Env、ループ、ブック、保持中の JS 値）。`xlsx.rs`/`xlsx_style.rs` は umya-spreadsheet、`xlsx_calc.rs` は `cell.calc()` の簡易計算、`xlsx_chart.rs` はブック内の Excel のグラフの読み取り、`xlsx_print.rs` は印刷レイアウト（ページ設定・改ページ → HTML）、`xlsx_shape.rs` は印刷用の図形・テキストボックス・画像（描画の XML → HTML/SVG）、`numfmt.rs` は表示書式。`gopath.rs` は Go の filepath と同じ意味のパス関数。`remote.rs` はデバッガ用のワイヤ形式 |
| `src/ui` | `Ui` トレイトと実装: `native`（Windows のみ。Win32 ダイアログ、IFileDialog、IProgressDialog、WebView2 ウィンドウ。`loader.rs` は WebView2Loader.dll なしでランタイムを読み込む）、`Console`、`Mock`（応答 JSON＋ログ。テスト用）。`thread.rs` の `ui::run` がスクリプトを専用スレッドで動かし、UI 呼び出しを呼び出し元（main スレッド）へ転送する。`web/` は組み込みページ（Go 版から移したもの）（`build.rs` が埋め込みリストを生成）。`web/chart.js` はグラフの共通アダプタ、`web/print.html` は PDF 出力のページ（`Ui::print_pdf` が見えないウィンドウで開く）、`web/vendor/` は同梱ライブラリ（AG Charts Community・Frappe Gantt。版は `VERSIONS.md`） |
| `src/dev` | 開発モード（`xljs dev` / `--dev` / 開始画面）。`mod.rs` がセッション（ページ `web/dev.html`・`dev.js` との JSON のやりとり）、`project.rs` がファイル一覧とダウンロードの置き換え先、`versions.rs` が履歴、`report.rs` がエラーの場所と AI への依頼文、`runner.rs` が別プロセスでの実行。AI チャットの枠は `src/ui/native/browser.rs`（`WindowOptions::browser` / `Window::browser`） |
| `src/ad` | Active Directory（グローバルの `ad`。natives は `src/api/ad.rs`）。`Directory` トレイトの裏が Windows では wldap32（`ldap.rs`。サインイン中のユーザーで Negotiate バインド、ページング検索）、テストでは `testdata/ad/directory.json` の偽ディレクトリ（`fake.rs`。`filter.rs` の評価器でフィルタを解く）。`mod.rs` がオプション → LDAP フィルタ（Search-ADAccount 相当の条件も）と結果のオブジェクト化、`value.rs` が値の変換（FILETIME・generalized time → Date、SID・GUID → 文字列） |
| `src/pdf` | PDF の編集（グローバルの `pdf`。natives は `src/api/pdf.rs`）。`mod.rs` が lopdf で読んだ文書のページの木を平らにし（継承される属性を各ページへ）、ページの取り込み（参照先のオブジェクトをたどってコピー）・削除・回転・文字の追加（表示の向きの座標 → ページの座標）。`font.rs` がフォントを名前・パスから探し（Windows の主なフォントはファイル名の表、ほかはフォントフォルダの name テーブル）、使った字だけを subsetter で埋め込む（Type0 / Identity-H ＋ ToUnicode）。テスト用の PDF は `testdata/pdf/`（`make_pdf.py`） |
| `src/meta` | エントリ先頭の `// ==xljs==` メタデータのパースと、アセットだけを見せる `AssetFs`（`DirAssets`/`MemAssets`） |
| `src/vba` | マクロブックの VBA ソースの読み取り（cfb で `vbaProject.bin` / `_VBA_PROJECT_CUR` を開き、MS-OVBA の解凍）。テスト用の固定データは `testdata/vba/`（Go 版のテスト用ビルダーで生成したもの） |
| `src/i18n` | メッセージ。`locales/<lang>.json` を `include_str!`。`en.json` が基準・フォールバック |
| `src/pack` | exe 末尾へのスクリプト埋め込み。v2 = `[zip(manifest.json, main.js, assets/...)][len u64]["XLJSPAK2"]`、v1 も読める |
| `types/xljs.d.ts` | 公開 API の型定義（`xljs init` で出力）。API を変えたら必ず更新 |
| `testdata/scripts/*.test.{js,ts}` | モック UI で実行される結合テスト（`src/engine/tests.rs`）。`<name>.answers.json` がモック応答 |
| `examples/` | サンプル。`xljs init` の d.ts と jsconfig.json で tsc の型チェックが通る状態を保つ（CI の ubuntu で検査） |
| `tools/gui/` | Windows 実機での GUI 自動操作・スクリーンショット（`XljsGui.psm1`、`smoke.ps1`、`stay-awake.ps1`）。`docs/gui-testing.md` 参照 |
| `docs/ai/` | AI チャット向けのプロンプトとリファレンス。`ai_docs` テストがコードブロックの構文と長さを検査 |

## 規約

- **ユーザー向けメッセージはすべて `src/i18n/locales/*.json`**。Rust は `t!("key", args...)` / `err!("key", args...)` / `i18n::error(&e)`、prelude は `T(key, ...)`。新しいキーは `en.json` → `ja.json` の順に追加。プレースホルダは `{0}` `{1}`。`cargo test --lib i18n` がキーの存在・プレースホルダの一致・ソース中で使われているキーの定義を検査する。
- **ライブラリの英語エラー**（QuickJS・oxc・umya-spreadsheet、デバッガでは V8）は `ja.json` の `translations`（Rust の regex）で表示時に訳す。`en.json` の `translations` は QuickJS の文面を Go 版と同じ英語にそろえる。`catch` で受け取る `e.message` は訳さない。
- **エラー型**は `crate::Error`（`Path` は Go の fs.PathError 相当で `i18n::error` が日本語化）。natives から JS へは `a.throw(&e)` / `api::throw_path`。
- **開発モードから起動された実行**（環境変数 `XLJS_DEV` → `Options::dev_guard`）では `fs.remove` が大量の削除の前に確認する。`XLJS_DEV_PREVIEW` → `Options::preview` は最初の `ui.form` の初期メッセージをファイルに書いて終わる（フォームのプレビュー）。
- **exit()** はキャッチできない例外（`JS_SetUncatchableError`）で即座にスクリプトを止める。デバッガでは `Host::exit_hook`。
- **実行中のスクリプトの中止**（`engine/watchdog.rs`。`Options::watchdog` / `Options::stop_key`、CLI の `--watchdog` / `--stop-key`、`XLJS_WATCHDOG` / `XLJS_STOP_KEY`）: QuickJS の割り込みハンドラで、スクリプトスレッドが最後に待ってから（`watchdog::waited()`）の時間が制限を過ぎたとき、または停止キーが押されたとき（別スレッドが `ui::key_down`＝`GetAsyncKeyState` を 30ms ごとに読む。待ちが明けてから 200ms 以内の押下は待ちの側のものとして無視）に `confirm` で尋ね、中止なら `set_exit(130)`＋キャッチできない例外。待ちとして数えるのはイベントループの待機・投稿されたコールバック・`ui::run` 経由の UI 呼び出し（`thread::call`）。スクリプトを長く待たせる処理を足したら `waited()` を呼ぶ。有効・無効は `Options::watchdog` / `Options::stop_key`（None で無効）で切り替える（開発モードの実行では常に有効）。中止したときは中止した場所（スタック）を stderr に書く（`engine::finish_caught`）。テストでは `StopKey::probe` でキーの状態を差し替える。
- **JS の値を Rust に保持する**ときは `Host::keep_fn` / `keep`（`Persistent`）を使い、終了時の `Host::clear` で必ず手放す。Rust のクロージャに JS の値を捕まえさせない（GC が追えない）。他スレッドから JS を呼ぶときは `Poster`（`add_pending` してから `post`）。
- **API の流儀**: ブラウザの File System Access API に寄せる。パスを受け取る引数は文字列・ハンドル・`File` のいずれも可（prelude の `toPath`）。
- **Excel**: 値は number/string/boolean/Date/null。Date はローカルの壁時計時刻（JS の `new Date(y, m, ...)` / `getFullYear()` などで変換）。書式はマージが既定（`apply_style`）、範囲端の罫線は書式済みの隣接セルにも反映。新規ブックの既定フォントはロケールの `excel.font`（umya が書けないので保存時に styles.xml を書き換える。数式があれば `fullCalcOnLoad` も）。
- **Windows ネイティブ部分**は `#[cfg(windows)]`（`src/ui/native`）。`windows` クレートと `webview2-com` を使う。型は Send + Sync だが UI スレッド以外から触らない（状態は thread_local）。
- **スレッド**: main スレッド＝UI スレッド（Win32 メッセージループ、ダイアログ、WebView2）、スクリプト＝`ui::run` が作る専用スレッド（スタック 64MB。QuickJS のスタック上限はこのスレッドだけ 32MB、ほかは 1MB）。開いているウィンドウは pending として数え、閉じるまでイベントループが終わらない。
- **デバッガ（`--debug`）**: prelude とスクリプトをデバッグウィンドウ（`web/debug.html`）のワーカーで実行し、natives は Rust 側の QuickJS に残して `api::remote::Remote` が JSON のワイヤ形式で中継する。ワーカー→Rust はページ経由のメッセージ（`call:`＋要求、応答は `R`＋応答）。終了判定は Rust 側（idle 報告の処理済みイベント数が一致し、デバッグウィンドウ以外に pending がない）。Linux では `src/engine/debug/tests.rs` の偽ページ（別スレッドの QuickJS）がページ役をし、`scripts_in_debugger` が全結合テストをこの経路でも流す。natives を足すときは値がワイヤ形式で表せること。
- **テスト**はプロセス共通の状態（言語・カレントディレクトリ・環境変数）を触るので `testutil::lock_lang` でロックする（既定は日本語）。
- コミットメッセージは英語、ドキュメント・UI 文言は日本語（英語はロケールファイル側）。

## 開発

```
cargo test --lib                                          # Linux でも全ロジックをテスト可（ネイティブ UI 以外）
cargo check --target x86_64-pc-windows-gnu --all-targets  # Windows 専用コードのコンパイル確認
make windows-gnu                                          # Linux から x64 版をクロスビルド（mingw-w64）
make windows && make dist                                 # Windows（MSVC）で x64/arm64 と配布 zip
xljs --ui=mock --ui-answers=a.json --lang=ja script.js    # ダイアログなしで実行
xljs --debug script.ts                                    # デバッグウィンドウ＋開発者ツール（Windows）
```

- GUI 版は既定のビルド（`windows_subsystem = "windows"`）、コンソール版（xljsc.exe）は `--features console`。アイコンとマニフェストは `build.rs` が `src/cli/xljs.rc` から埋め込む。
- Linux で Windows 版のテストを流すなら: mingw-w64 と wine を入れて `LANG=C.UTF-8 CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine64 cargo test --target x86_64-pc-windows-gnu --lib`（ネイティブダイアログのテストは `xvfb-run` 付きで）。
- CI（.github/workflows/ci.yml）: ubuntu と windows で `cargo test --lib`。Windows ではダイアログを実際に開いて閉じるテスト（`src/ui/native/tests.rs`）と WebView2 のウィンドウのテストも走る。タグ `v*` で Release に zip を添付。
- **Windows 実機（`claude remote-control`）では GUI を自動操作・撮影して確認できる**: `cargo build` → `pwsh -File tools/gui/smoke.ps1 -MonitorOff`。ヘルパーは `tools/gui/XljsGui.psm1`（UI Automation＋ウィンドウメッセージ。入力は使わない）。手順・できること・はまりどころは `docs/gui-testing.md`。
- 出力の見た目確認: LibreOffice があれば `soffice --headless --convert-to pdf` → `pdftoppm` で画像化できる。
