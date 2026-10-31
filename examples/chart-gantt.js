// 工程表（作業・開始・終了・進捗・前提・金額の列）をガントチャートで表示し、
// バーのドラッグで変えた日付をブックに書き戻す。休日はスクリプトから渡す。
const [file] = await showOpenFilePicker({ types: [{ description: "Excel", accept: [".xlsx"] }] });
if (!file) exit();
const book = workbooks.open(file);
const sheet = book.sheets[0];
const tasks = sheet.rows(); // [{ 作業: "設計", 開始: Date, 終了: Date, 進捗: 0.4, 前提: "" }, ...]

const v = await ui.form({
  title: "工程表",
  width: 960,
  fields: [{
    name: "plan", type: "chart", label: "工程", height: 16,
    chart: {
      type: "gantt",
      data: tasks,
      // 列はキー名で指定（省略すると 名前/作業・開始/開始日・終了/終了日・進捗・前提 の列を探す）
      name: "作業", start: "開始", end: "終了", progress: "進捗", dependencies: "前提",
      view: "week",
      editable: true, // ドラッグで日付・進捗を変えると、元の行に書き込まれる
      // 終了日はその日を含む。土日と次の日に色を付ける
      holidays: [new Date(2026, 10, 3), { date: "2026-11-23", name: "勤労感謝の日" }],
      // 今日を基準日に、進捗線（イナズマ線）と出来高累計曲線（金額の列で重み付け）を重ねる
      progressLine: true,
      sCurve: { weight: "金額" },
      onChange(row, changes) {
        console.log(row.作業, changes);
      },
    },
  }],
  submit: "保存",
});
if (!v) exit();
// editable なガントチャートの値はすべての行（変更済み）
sheet.write(v.plan, { table: true });
book.save();
alert("保存しました");
