# SQLite（`sqlite` グローバル）の設計

スクリプトから SQLite のデータベースを読み書きする。普通のローカルの使い方に加え、
**ファイルサーバーにデータベースを置いて複数人で使う**（SQLite 公式には推奨されない）使い方を、
MS Access のバックエンド共有の経験則にならって「遅くてもよいので壊れにくく、壊れても簡単に戻せる」ように扱う。

| 場所 | 役割 |
|---|---|
| `src/db/mod.rs` | `Db`（接続・トランザクション・クエリ）。ローカル / 共有の 2 つのモード |
| `src/db/lock.rs` | 共有モードのロックファイル（読み取り共有・書き込み排他） |
| `src/db/share.rs` | データベースの隣のフォルダ（ロック・最後の書き込み者・自動バックアップ・変更ログ） |
| `src/db/repair.rs` | 検査と修復 |
| `src/api/sqlite.rs` | natives（値の変換、`Host::dbs` のハンドル） |
| `src/api/prelude.js` の sqlite 節 | 公開 API（`sqlite.open`、`Database`）、壊れていたときの確認と再実行 |
| `src/cli/db.rs` | `xljs repair <file.db>`、`.db` を xljs.exe にドロップしたときの検査と修復 |

SQLite は rusqlite の `bundled`（ソースを同梱してビルド。Excel と同じく利用者の環境に何も要らない）。

## API

```js
const db = sqlite.open("受注.db", { shared, readonly, create, timeout, backup: { interval, keep } | false, onCorrupt });
db.exec(sql)                 // 複数文、パラメータなし
db.run(sql, ...params)       // { changes, lastInsertRowid }
db.all / get / values(sql, ...params)
db.transaction(fn)           // 入れ子は SAVEPOINT。fn は同期のみ（better-sqlite3 と同じ）
db.insert(table, rows, { columns, replace, ignore, create })
db.tables / hasTable / columns / check / backup / backups / repair / close
sqlite.check(p) / sqlite.repair(p) / sqlite.backups(p)
```

- パラメータは可変長・配列（位置）・オブジェクト（`:name` `@name` `$name`、キーは接頭辞あり・なしどちらでも）。
- 値の変換は Excel の流儀（`Date` はローカルの壁時計時刻）に合わせる。`Date` → `"2026-04-01"`（0 時ちょうど）/
  `"2026-04-01 09:30:00"`（ミリ秒があれば `.123`）。読み取りでは、宣言型に `DATE` か `TIME` を含む列の日時文字列を `Date` に、
  `BOOL` を含む列の 0/1 を boolean にする（Python の `PARSE_DECLTYPES` と同じ考え方）。2^53 を超える整数は BigInt。
- `insert` でテーブルを作るときは値から型を決める（すべて 0 時の Date → `DATE`、時刻あり → `DATETIME`、整数 → `INTEGER`、
  混在 → 型なし）。
- 接続ごとに `foreign_keys=ON`・`trusted_schema=OFF`。
- `transaction` はコールバックを Rust から呼ばず、prelude が `sqliteBegin` → fn → `sqliteCommit` / `sqliteRollback` を呼ぶ
  （デバッガでは JS の関数を Rust から同期的に呼べないため）。値はすべてデバッガのワイヤ形式で表せる。

## 共有モード

ネットワーク上の SQLite が壊れる主な原因は、(1) ネットワークファイルシステムのバイト範囲ロックが当てにならない、
(2) クライアント側のキャッシュ（oplock）で古いページを基に書き込む、(3) 書き込み途中の切断・クラッシュ。
Access で言われる対策（ファイルを開きっぱなしにしない、排他を確実に、定期的な最適化と修復、バックアップ）にならって次のようにする。

**どれが共有モードか**: データベースの隣に `<ファイル名>.xljs-shared` フォルダがあれば常に共有モード
（スクリプトの作者が指定し忘れても、ほかの人と同じ方式で使う）。なければ `shared` の指定、省略時は
ネットワーク上（UNC パス、または `GetDriveTypeW` が `DRIVE_REMOTE`）なら共有モードにしてフォルダを作る。

```text
顧客.db
顧客.db.xljs-shared/
  README.txt                     フォルダの説明（消さないで）
  lock                           ロックファイル
  owner.json                     最後に書き込みを始めた人（待たされた人へのメッセージ用）
  backups/snap-000012-20261031-093000.db
  backups/log-000012.jsonl       スナップショット 12 のあとにコミットされたトランザクション
  broken/                        修復で退避した壊れたファイル
```

- **接続を持ち続けない**: 呼び出し（`all`・`run`・`transaction` …）ごとにファイルを開き、終わったら閉じる。
  人を待つ間（ダイアログ・フォーム）にロックもキャッシュも残らない。`transaction` の fn が同期のみなのもこのため。
- **SQLite のロックとは別のロックファイル**: Windows では共有モード（sharing mode）で開く。読み取りは
  `GENERIC_READ`＋`FILE_SHARE_READ`、書き込みは読み書き＋共有なし。共有違反（32/33）なら少し待って再試行。
  sharing mode はファイルサーバー自身が強制し、クライアントが落ちればハンドルごと解放されるので、古いロックが残らない。
  Windows 以外（テスト）は `flock`（std の `File::try_lock` / `try_lock_shared`）。待つ間は `watchdog::waited()`。
  タイムアウト（既定 30 秒）では `owner.json` の人と時刻をメッセージに出す。
- 読み取りは読み取りロック。`all`/`get` でも書き込む文（`INSERT ... RETURNING` など。`sqlite3_stmt_readonly` で判定）は
  書き込みの扱いに切り替える。
- **ジャーナル**: WAL は共有メモリが要りネットワークでは使えないので、開くときに WAL なら DELETE に戻す。
  `synchronous=EXTRA`（ジャーナル削除後にフォルダも同期）、`cell_size_check=ON`（壊れたページを読んだ時点で気づく）。
- **自動バックアップ**: 書き込みトランザクションの開始時（書き込みロックを取った後）、最新のスナップショットが
  `interval`（既定 60 分）より古ければ、`quick_check` で検査してからオンラインバックアップ API でスナップショットを取る。
  検査に失敗したら書き込まずに「壊れている」エラーにする（壊れたファイルに書き足して被害を広げない／壊れたものをバックアップしない）。
  `keep`（既定 10）個を残し、それより古いスナップショットとそのログを消す。
- **変更ログ**: コミットしたトランザクションの、何かを変えた文（`readonly` でない文）を、位置で束縛した値とともに
  最新世代の `log-N.jsonl` に 1 行の JSON で追記し `FlushFileBuffers` する（すべて書き込みロックの中なので、ログの順＝コミットの順）。
  SAVEPOINT の取り消しで捨てた文は記録しない。値は整数と実数を区別し（`1` と `1.0`）、BLOB は `{"b": base64}`。
  コミット後・追記前にクライアントが落ちたトランザクションは記録されない（修復時にだけ失われうる）。

## 修復（`repair.rs`）

書き込みロックを取り、`integrity_check` で問題がなければ何もしない（`force` で作り直す）。問題があれば、
壊れたファイルを（ジャーナルなどとともに）`broken/`（共有モードでなければ同じフォルダの `名前.broken-日時.db`）へ移し、
次の順に試して、`integrity_check` を通った最初のものを元の場所に置く。どれも失敗したら壊れたファイルを元に戻してエラー。

1. 壊れたファイルのコピーを開く（残っていたジャーナルがロールバックされる）→ `journal`。`REINDEX` → `reindex`。
   `VACUUM` → `vacuum`。ここまではデータを失わない。
2. 共有モード: `quick_check` を通る最新のスナップショットに、その世代以降の変更ログを順に再実行する → `backup`。
   失敗したトランザクションは飛ばして数える（`skipped`）。`datetime('now')` などの非決定的な SQL は値が変わりうる。
3. サルベージ: 壊れたファイルのスキーマでテーブルを作り、読める行をコピーし（rowid 順に読み、読めなくなったら
   rowid を飛ばして続きから。WITHOUT ROWID は読めるところまで）、`sqlite_sequence`、索引・トリガー・ビューを作り直す → `salvage`。

修復後、共有モードなら新しい世代のスナップショットを取る。

スクリプトからは、natives が SQLITE_CORRUPT / SQLITE_NOTADB を見ると `Db::corrupt` を立て、prelude の `_call` が
（トランザクションの外で）`onCorrupt` に従って確認 → `db.repair()` → 結果を表示 → 操作をもう一度実行する。
`transaction` の fn も再実行されうる（取り消された後なので整合性は保たれる）。
利用者は壊れたデータベースを xljs.exe にドロップしても（`cli/db.rs`）検査・修復できる。

## テスト

- `src/db/tests.rs`: ローカルの操作、共有モードのバックアップ・ログ・ロック、ページを壊してからの修復（バックアップ＋ログ、サルベージ）。
- `src/db/lock.rs`: 読み取り同士は共存、書き込みは排他。Windows の sharing mode は wine で
  `cargo test --target x86_64-pc-windows-gnu --lib db::` で確認できる。
- `testdata/scripts/sqlite.test.js`: API 全体と、壊したファイルでの「確認 → 修復 → 再実行」（モックの confirm）。
