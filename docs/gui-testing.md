# 実機 Windows での GUI 確認（Claude Code 向け）

Windows の実機で `claude remote-control`（`claude rc`）を動かすと、Claude Code のコマンドはその PC の
ユーザーのデスクトップセッションで実行される。この状態で、xljs のネイティブダイアログと WebView2 の
ウィンドウを **マウス・キーボードを使わずに** 自動操作し、スクリーンショットで見た目を確認できる。
Playwright に相当するものを PowerShell（UI Automation＋ウィンドウメッセージ）で用意したのが `tools/gui/`。

最終確認: 2026-10-31、Windows 11 Pro 26200 / x64、2560x1440、WebView2 Runtime 156、Rust 1.99.1（MSVC）。

## 前提

- **自分がどこで動いているか確認する**: `hostname` が利用者の PC 名、`$PSVersionTable` が pwsh 7 なら実機。
  クラウドのコンテナ（Linux）ではこの文書の手順は使えない（CLAUDE.md の Linux 向け手順を使う）。
- ツールチェーン: rustup（`x86_64-pc-windows-msvc`）＋ Visual Studio Build Tools（C++）。
  rustup を入れた直後のシェルは PATH に `~\.cargo\bin` がないので、`$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"` を付ける。
  `make` はないので Makefile のターゲットは `cargo` を直接呼ぶ。
- **画面がロックされていないこと**。ロック中はスクリーンショットがロック画面になり、操作もできない。
  ロックは Claude からは解除できない（利用者に頼む）。動的ロック（スマホが離れるとロック）は利用者がオフにしておく。
- pwsh 7 で `Add-Type -AssemblyName UIAutomationClient` が使える（Windows PowerShell 5.1 でなくてよい）。

## 使い方

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build                                        # target/debug/xljs.exe（GUI 版）
pwsh -File tools/gui/smoke.ps1 -MonitorOff         # スモークテスト一式（下の表）
```

スクリーンショットは `%TEMP%\xljs-gui-smoke\shots\*.png`。**Read ツールで画像を開けば Claude が見た目を確認できる**。
失敗したケースは `FAIL-<ケース名>.png`（その時点の画面全体）が残る。

個別に操作するとき:

```powershell
Import-Module ./tools/gui/XljsGui.psm1
Start-XljsAwake -Minutes 30 -MonitorOff           # スリープ・ロック防止＋画面だけ消す
$p = Start-Process ./target/debug/xljs.exe -ArgumentList '--lang=ja', 'examples/form-aggregate.js' -PassThru
$w = Get-XljsWindow -ProcessId $p.Id -Name '売上集計'   # 出るまで待つ
Show-XljsUiTree $w                                 # 何が操作できるか（種別・クラス・id・名前・パターン）
Find-XljsElement -Root $w -AutomationId submit | Invoke-XljsElement
Save-XljsScreenshot "$env:TEMP\shot.png" -Window $w
Stop-XljsAwake
```

| 関数 | 内容 |
|---|---|
| `Get-XljsWindow -ProcessId -Name [-All]` | プロセスのウィンドウを待って返す（オーナー付きダイアログも含む） |
| `Show-XljsUiTree` | UI ツリーのダンプ。まずこれで対象を調べる |
| `Find-XljsElement -Root [-Name] [-AutomationId] [-ControlType] [-ClassName]` | 子孫要素を待って返す |
| `Invoke-XljsElement` | 押す（Invoke → Toggle → SelectionItem → `BM_CLICK`） |
| `Set-XljsText` / `Get-XljsText` | 値の設定・取得（ValuePattern、なければ `WM_SETTEXT`／Name） |
| `Submit-XljsFileDialog -Dialog -Path` | 開く／保存／フォルダー選択ダイアログにパスを入れて確定 |
| `Close-XljsWindow` | 閉じる（WindowPattern、なければ `WM_CLOSE`） |
| `Save-XljsScreenshot -Path [-Window]` | 画面全体またはウィンドウを PNG に |
| `Start-XljsAwake` / `Stop-XljsAwake` | `stay-awake.ps1` をバックグラウンドで起動／停止 |
| `Wait-Until { ... }` | 条件が真になるまで待つ |

## スモークテストで確認していること（tools/gui/smoke.ps1）

テスト用ブックは `--ui=mock` で xljs 自身に作らせ、結果のブックも `--ui=mock` のスクリプトで開いて中身を検査する
（期待値と違えば `exit(n)`）。

| ケース | 操作 | 確認 |
|---|---|---|
| launch-screen | 引数なしで起動 → 「キャンセル」 | 開始画面の 4 つのボタンがある、終了する |
| dialog-aggregate | フォルダー選択 → 進捗 → 保存ダイアログ → alert の OK | alert の文言、出力ブックの集計・明細、終了。`shell.showItemInFolder` で開いたエクスプローラーは閉じる |
| form-aggregate | WebView2 のフォーム: 参照…（フォルダー／保存）、ラジオ、チェックボックス → 集計 → 閉じる | フォームのログ（件数・保存先）と「完了しました」、出力ブック（明細シートなし・商品名で集計） |
| progress-cancel | IProgressDialog のキャンセル | 「キャンセルされました」の alert、終了 |

2026-10-31 時点で 4 件とも PASS（`-MonitorOff` で画面を消した状態でも同じ）。あわせて `cargo test --lib`
（68 件、ネイティブダイアログと WebView2 のテストを含む）も実機で PASS。

## できること・できないこと

できる:
- スクリーンショット（全モニター／ウィンドウ単位）。**画面（モニター）を消していても撮れる**。DPI 拡大率に関係なく物理ピクセル。
- Win32 ダイアログ（MessageBox、IFileDialog、IProgressDialog、開始画面）の文言の読み取りとボタン操作・パス入力。
- WebView2 の中身の操作。Chromium が UIA を出すので、HTML の要素が **id を AutomationId として** 見え、
  ボタン（Invoke）、テキスト（Value）、チェックボックス（Toggle）、ラジオ（SelectionItem）、ログの文言（Text の Name）が扱える。
- 画面を消したまま操作（入力を使わないので画面が点かない）。スリープ・ロックの抑止（`stay-awake.ps1`）。

できない・注意:
- **SendInput（マウス・キーボード）は使わない**。入力があると消した画面が点く。pywinauto の `click_input` なども同じ。
- **Playwright（CDP）で WebView2 に接続できない**。`src/ui/native/loader.rs` が `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
  などの環境変数を意図的に空にしているため `--remote-debugging-port` を渡せない。必要になったら、デバッグビルド限定の
  環境変数で `AdditionalBrowserArguments` を設定する変更を入れる（現状は UIA で足りている）。
- ロック中・サインアウト中・RDP 切断中は何もできない。
- 日付入力（`<input type=date>`）の操作、ドラッグ＆ドロップ、`--debug` のデバッグウィンドウ（開発者ツール）は未検証。

## はまりどころ（実際に踏んだもの）

- **xljs の Win32 ボタン・Edit は UIA で `Pane` に見え、パターンがない**（開始画面、ファイルダイアログ下部のファイル名欄・
  ボタン、MessageBox の OK）。UIA 経由では押せないので、`NativeWindowHandle` に `BM_CLICK`／`WM_SETTEXT` を送る。
  探すときは `-ControlType` ではなく `-ClassName Button` / `Edit` で。
  （スクリーンリーダーなど支援技術からも操作しにくい可能性がある。xljs 側の課題として未調査）
- `BM_CLICK` は **PostMessage** で送る。SendMessage だと押した先がモーダルループ（次のダイアログ）に入って戻らない。
- IProgressDialog のキャンセルボタンはクラスが `CCPushButton`（`*Button` で判定している）。
- **オーナー付きのダイアログ（フォームの「参照…」から開くファイルダイアログ）は、UIA ではデスクトップ直下ではなく
  オーナーウィンドウの子** に入る。`Get-XljsWindow` は子の Window も探す。
- ファイルダイアログに **`/` 区切りのパスを入れると「フォルダー名は有効ではありません」**。`Submit-XljsFileDialog` は
  `GetFullPath` で `\` に直してから入れる。フォームの値（`f0` など）も `\` 区切りで返る。
- ファイル名欄の AutomationId: 開く・保存は `1001`（クラス Edit）、フォルダー選択は `1152`。確定ボタンは `1`、キャンセルは `2`。
- `form-aggregate` は `remember: true` なので、前回の入力が次回の初期値になる（チェックボックスは状態を見てから押す）。
- コンソール版（`--features console`）は終了時に「Enter キーで閉じます...」で止まることがある。GUI の確認は GUI 版（既定のビルド）で。
- `stay-awake.ps1` は同時に 1 つだけ動く（Mutex）。元のスクリーンセーバー設定はレジストリから読む
  （以前は SPI の現在値を覚えていたため、重ねて起動すると「止めた状態」を元の値として戻してしまった）。

## スリープ・ロック・画面オフ（tools/gui/stay-awake.ps1）

- `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` でスリープ抑止（`ES_DISPLAY_REQUIRED` は付けないので画面は消える）。
- `SystemParametersInfo(SPI_SETSCREENSAVEACTIVE, 0)` でスクリーンセーバー（＝「再開時にサインイン」によるロック）を止める。
  ファイルには書かないので、戻し損ねても次回サインインで元に戻る。
- `-MonitorOff`: `PostMessage(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2)` で画面だけ消す。
- 終了: `-Minutes` 経過、`%TEMP%\xljs-stay-awake.stop` を作る（`Stop-XljsAwake`）、または Ctrl+C。
- 動いている間は PC がロックされない。人のいる場所では利用者に確認すること。
