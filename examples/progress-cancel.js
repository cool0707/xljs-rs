// 進捗表示とキャンセルのデモ
const p = ui.progress({ title: "デモ", total: 50, message: "準備中…" });
for (let i = 0; i < 50; i++) {
  if (p.cancelled) break;
  p.update(i, `${i + 1} 件目を処理中`);
  await sleep(100);
}
p.close();
alert(ui.cancelled ? "キャンセルされました" : "完了しました");
