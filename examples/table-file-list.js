// フォルダ内のファイル一覧を表で表示し、選んだファイルの行数を数える。
// 表は列見出しのクリックで並べ替え、上の欄で絞り込み、Ctrl+C でコピーできる。
const dir = await showDirectoryPicker({ title: "フォルダを選択" });
if (!dir) exit();

const list = dir.walk({ glob: ["*.csv", "*.txt"] }).toArray().map((h) => {
  const st = h.stat();
  return { 名前: h.relativePath, サイズ: st.size, 更新日時: new Date(st.mtimeMs), handle: h };
});

// selectable を指定すると OK/キャンセル付きになり、選んだ行が返る（キャンセルは []）
const picked = await ui.table(list, {
  title: "数えるファイルを選択",
  columns: ["名前", { key: "サイズ", label: "サイズ (バイト)" }, "更新日時"],
  selectable: "multiple",
});
if (picked.length === 0) exit();

const counts = picked.map((r) => ({ 名前: r.名前, 行数: r.handle.readText().split(/\r?\n/).filter(Boolean).length }));

// 表示のみの表はすぐに戻る。閉じるまで待つなら .closed を await する。
// 行をダブルクリックするとファイルを開き、「行数」の列なら件数を表示する
await ui.table(counts, {
  title: "行数",
  onDoubleClick(e) {
    if (e.column === "行数") alert(`${e.row.名前}: ${e.value} 行`);
    else shell.openPath(picked[e.index].handle);
  },
}).closed;
