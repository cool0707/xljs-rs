# 開発モード（`xljs dev`）の設計

AI チャットと並べてスクリプトを作る・直すための画面。本格的な AI 連携（API）や Git 連携はせず、AI とのやりとりは「チャット画面」「ダウンロード」「クリップボード」だけで行う。

## 構成

| 場所 | 役割 |
|---|---|
| `src/dev/mod.rs` | セッション。開発ウィンドウを開き、ページ（`dev.js`）の要求 `{op, ...}` に `{type, ...}` のイベントで答える。ブラウザ枠・実行のイベントもここに集まる（`Event`） |
| `src/dev/project.rs` | 開いているスクリプト（`Project`）。編集できるファイルの一覧、ダウンロードの置き換え先の決定（`match_download`）、テキストの読み書き（UTF-8、だめなら Shift_JIS） |
| `src/dev/versions.rs` | 簡易バージョニング。置き換える前の内容を `<フォルダ>/.xljs/versions/<相対パス>/NNNN.<拡張子>` に、メモを同じフォルダの `log.json` に |
| `src/dev/report.rs` | 失敗した実行の stderr から、エラーの部分（`last_error`）と場所（`find_location`）を取り出し、AI への依頼文（`prompt`）を作る。`intro` は `docs/ai/prompt.md`＋今のスクリプト |
| `src/dev/runner.rs` | スクリプトを別プロセス（この exe 自身）で実行し、stdout/stderr と終了をイベントで返す |
| `src/ui/web/dev.html` `dev.js` `dev.css` `dev-diff.js` | ページ。キャンバス（タブ）、差分表示（Myers の行差分）、簡易エディタ、プレビュー |
| `src/ui/native/browser.rs` | AI チャットの枠: 同じウィンドウの 2 つ目の WebView2（`WindowOptions::browser` / `Window::browser`） |

`ui::run` のスクリプトスレッドでセッションのループ（`mpsc` の受信）を回し、ダイアログ・ウィンドウは通常のスクリプトと同じく UI スレッドへ転送する。

## AI チャットの枠

- ページの WebView2 とは別に、子ウィンドウ（`xljsBrowserHost`）に 2 つ目の WebView2 を作る。外部サイトは iframe に入れられない（X-Frame-Options）ため。環境（ユーザーデータフォルダ）はページと同じなので、サインインは残る。
- 位置はページが決める。`dev.js` が `#chat-view` の矩形（CSS ピクセル）と `devicePixelRatio` を `{op:"browser", path:"show"}` で送り、Rust が子ウィンドウを `HWND_TOP` でそこに置く。親ウィンドウは `WS_CLIPCHILDREN`。ページ側のメニューやダイアログが枠に隠れないよう、ページは枠の上に重なる要素を作らない（確認は `confirm()` のネイティブダイアログ）。
- ダウンロード（`ICoreWebView2_4::DownloadStarting`）は保存先を一時フォルダ（`%TEMP%\xljs-dev\<pid>`）に変え、ダウンロードの表示を出さない（`Handled`）。完了（`StateChanged` が COMPLETED）で `BrowserEvent::Downloaded`。
- 表示中のページ（`SourceChanged`）を `BrowserEvent::Url` で知らせ、スクリプトごとに `.xljs/dev.json` の `chat` に覚える。スクリプトを開くとその URL を開く。チャットを開いていない状態でスクリプトを開いたときは、今のチャットをそのスクリプトに紐付ける。「ホーム」でチャットを選び直す（紐付けを消し、`about:blank` にして隠す）。

## ダウンロードの置き換え先（`Project::match_download`）

1. 同じ名前（拡張子の一致を優先。`.js` と `.ts` などスクリプト同士は同じ種類として扱う）。ブラウザが付ける ` (1)` は除く。
2. 既存のファイル名がダウンロードの名前の先頭に来るもの（AI が `集計_修正版.js` のように後ろに付け足すため）。いちばん長く一致するもの。
3. アプリでない場合のスクリプトは、編集中のファイル（スクリプトなら）、なければ入口のスクリプト。
4. それ以外は決めない（ページで選ぶ。新しいファイルとして保存もできる）。

対象のファイルは、入口のスクリプトと同じフォルダのテキストファイル、およびメタデータの `@assets` / `@html` / `@icon` の下（アプリの画面）。

## 実行

- `runner::Run` が `exe [--lang=..] [--debug] script` を、スクリプトのフォルダをカレントにして起動する（Windows では `CREATE_NO_WINDOW`）。標準出力・エラーはパイプ。GUI 版の子プロセスは出力先がある（パイプ）のでエラーダイアログを出さず、stderr に書く。
- 環境変数: `XLJS_DEV=1`（`Options::dev_guard`）、`XLJS_WATCHDOG=on`・`XLJS_STOP_KEY=on`（利用者が指定していなければ）、プレビューでは `XLJS_DEV_PREVIEW=<一時ファイル>`（`Options::preview`）。
- `dev_guard`: `fs.remove`（`removeEntry` も）が、中身のあるフォルダを再帰的に消すときと、その実行で 2 つ目のファイルを消すとき（1 回だけ）に `confirm` で尋ねる。キャンセルでエラー（`dev.removeDeclined`）を投げ、スクリプトは止まる。
- 無限ループで中止したとき（終了コード 130）は、エンジンが中止した例外のスタック（ソースマップで元のファイルに対応付けたもの）を「中止した場所:」に続けて stderr に書く（`engine::finish_caught`）。開発モード以外でも出る。
- 終了時、stderr の最後の「インデントのない行」から後ろ（エラー本文とスタック、構文エラー、中止の場所）をエラーとして扱い、その中の最初のスクリプトのファイル（`node_modules` を除く）を場所とする。

## フォームのプレビュー

`XLJS_DEV_PREVIEW` を付けて実行すると、prelude の `ui.form` が最初のフォームのページ向けの初期メッセージ（`formInit`。フォームのウィンドウに送るものと同じ）をファイルに書いて `exit(0)` する。開発ウィンドウはそれを `form.html?preview` の iframe に渡す（`bridge.js` は `?preview` 付きで iframe の中にいると、親ページと `window.postMessage` でやりとりする）。送信はしない（「プレビューのため実行しません」）。

アプリの画面のプレビューは、スクリプトのフォルダを `https://app.xljs/_project/` で配信して iframe で表示する（スクリプトは動かないので data-bind などは空）。

## ページとのメッセージ（主なもの）

| ページ → Rust（`op`） | 内容 |
|---|---|
| `ready` | 初期化。`init`（文言・AI の一覧・最近使ったスクリプト）を返し、起動時に指定されたスクリプトを開く |
| `open` `new` | スクリプトを開く（`path` なしはピッカー）/ 新規作成（`app` でアプリの雛形） |
| `read` `save` `edit` | ファイルの読み書き。`edit` は編集中のファイル（置き換え先の決定用） |
| `apply` `versions` `version` `restore` | 置き換え（履歴に残す）、履歴の一覧・内容、前の版に戻す |
| `run` `stop` `preview` | 実行（`debug`）・停止・フォームのプレビュー |
| `chat` `browser` | チャットの URL（空でホーム）、枠の表示・位置・戻る・進む・再読み込み |
| `intro` `showFolder` | AI への説明のコピー、フォルダを開く |

| Rust → ページ（`type`） | 内容 |
|---|---|
| `init` `project` `meta` `files` | 初期化、開いたスクリプト、メタデータの変化、ファイル一覧 |
| `file` `saved` `applied` `versions` `version` | ファイル・保存・置き換え・履歴 |
| `download` | ダウンロードしたファイル（`name` `text` `target` `old` `files`） |
| `runStarted` `output` `runEnded` | 実行。`runEnded` は失敗時に `message` `location` `prompt` |
| `previewForm` `previewFailed` | フォームのプレビュー |
| `url` `copy` `error` `browserFailed` | チャットの URL、クリップボードへのコピー、エラー |

ページの配置（両方 / チャットだけ / キャンバスだけ、左右の入れ替え、境目の位置）はページの `localStorage` に保存する。最近使ったスクリプトはユーザーの設定（`%APPDATA%\xljs\dev.json`）。

## テスト

- `src/dev/tests.rs`: 置き換え先の決定、履歴、エラーの場所と依頼文、セッションの流れ（記録するだけのウィンドウとモック UI）、別プロセスの実行（Linux の `sh`）。
- `src/engine/tests.rs`: `dev_guard`、フォームのプレビュー、無限ループで中止した場所。
- ネイティブの枠（2 つ目の WebView2、ダウンロード）は Windows 実機で確認する（`docs/gui-testing.md` の手順）。ページは `XLJS_DEV_DEBUG=1` で開発者ツールが使える。
