// ==xljs==
// @mode   app
// @html   ui/index.html
// @assets ui/
// @title  売上集計
// @size   860x620
// ==/xljs==

// フォルダ内の Excel ファイル（サブフォルダ含む）の「明細」シートを顧客ごとに集計するアプリ。
// 画面（ui/index.html）から呼ぶ関数は export しておく（画面側は xljs.invoke や data-action で呼ぶ）。
// 画面の表示は win.state に値を入れると data-bind の要素に反映される。
// `xljs pack app.js` で exe にすると ui フォルダも一緒に埋め込まれる。

const win = app.main;
win.state = {
  folder: "",
  count: 0,
  running: false,
  progress: 0,
  message: "集計するフォルダを選んでください",
  done: false,
};

/** @type {FileSystemDirectoryHandle | null} */
let folder = null;
/** @type {FileSystemFileHandle[]} */
let files = [];
/** @type {any[]} */
let rows = [];
/** @type {{ 顧客名: string, 件数: number, 合計: number }[]} */
let summary = [];
let cancelled = false;

/** 「フォルダを選択」ボタン */
export async function pickFolder() {
  const dir = await showDirectoryPicker({ title: "集計するフォルダを選択", startIn: folder ?? undefined });
  if (dir) useFolder(dir);
}

/** 画面にドロップされたフォルダ（ファイルならそのフォルダ）を使う */
export function dropped(handles) {
  const first = handles.find((h) => h);
  if (!first) return;
  useFolder(first.kind === "directory" ? first : new FileSystemDirectoryHandle(path.dirname(first.path)));
}

/** @param {FileSystemDirectoryHandle} dir */
function useFolder(dir) {
  folder = dir;
  files = dir.walk("*.xlsx").toArray();
  win.state = {
    folder: dir.path,
    count: files.length,
    done: false,
    progress: 0,
    message: files.length ? `${files.length} 個の Excel ファイルがあります` : "Excel ファイルが見つかりません",
  };
}

/** 「集計」ボタン。進捗は win.state で画面に送る */
export async function aggregate() {
  if (!files.length) throw new Error("集計する Excel ファイルがありません");
  cancelled = false;
  rows = [];
  const errors = [];
  win.state = { running: true, done: false };
  try {
    for (const [i, h] of files.entries()) {
      win.state = { progress: Math.round((i / files.length) * 100), message: `${i + 1} / ${files.length}: ${h.relativePath}` };
      // 画面からの「中止」を受け取れるようにイベントループへ戻る
      await sleep(0);
      if (cancelled) {
        win.state.message = "中止しました";
        return;
      }
      try {
        const book = workbooks.open(h);
        const sheet = book.sheets["明細"] ?? book.sheets[0];
        for (const r of sheet.toObjects()) rows.push({ ...r, ファイル: h.relativePath });
        book.close();
      } catch (e) {
        errors.push(`${h.relativePath}: ${e.message}`);
      }
    }
  } finally {
    win.state.running = false;
  }

  const byCustomer = Object.groupBy(rows, (r) => r.顧客名 ?? "(未設定)");
  summary = Object.entries(byCustomer).map(([name, list]) => ({
    顧客名: name,
    件数: list.length,
    合計: list.reduce((s, r) => s + (Number(r.金額) || 0), 0),
  }));
  summary.sort((a, b) => b.合計 - a.合計);

  // 結果の表は画面側の関数（xljs.handle("render", ...)）で描く
  await win.invoke("render", summary);
  win.state = {
    progress: 100,
    done: true,
    message: `${files.length} ファイル、${rows.length} 件を集計しました` + (errors.length ? `（読めなかったファイル: ${errors.length}）` : ""),
  };
  if (errors.length) win.emit("warnings", errors);
}

/** 「中止」ボタン */
export function cancel() {
  cancelled = true;
}

/** 「明細を表示」ボタン（組み込みの表ウィンドウ） */
export function showRows() {
  ui.table(rows, { title: "明細" });
}

/** 「保存」ボタン。保存したファイル名を返す */
export async function save() {
  const dest = await showSaveFilePicker({
    suggestedName: "集計結果.xlsx",
    startIn: folder ?? undefined,
    types: [{ description: "Excel ブック", accept: [".xlsx"] }],
  });
  if (!dest) return null;
  const out = workbooks.create();
  out.addSheet("集計").write(summary).setColumnWidth("A", 24);
  out.addSheet("明細").write(rows);
  out.save(dest);
  shell.showItemInFolder(dest);
  return dest.name;
}
