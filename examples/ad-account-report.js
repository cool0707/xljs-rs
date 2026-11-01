// Active Directory のアカウントを点検して Excel に書き出す（ドメインに参加している Windows で実行）。
// 無効・ロックアウト中・パスワード期限切れのユーザーと、90 日ログオンのないコンピューターをシートに分ける。
if (!ad.available()) {
  alert("このパソコンはドメインに参加していないため、Active Directory を検索できません。");
  exit();
}

const me = ad.me();
const domain = ad.domain();
console.log(`${domain.name} に ${me.displayName || me.sAMAccountName} として接続しました（${domain.server}）`);

/** @param {ADObject[]} users */
const userRows = (users) =>
  users.map((u) => ({
    名前: u.displayName || u.name,
    ログオン名: u.sAMAccountName,
    部署: u.department || "",
    最終ログオン: u.lastLogonTimestamp || null,
    パスワード変更: u.pwdLastSet || null,
    場所: u.distinguishedName,
  }));

const sheets = [
  { name: "無効", rows: userRows(ad.search({ disabled: true })) },
  { name: "ロックアウト", rows: userRows(ad.search({ lockedOut: true })) },
  { name: "パスワード期限切れ", rows: userRows(ad.search({ passwordExpired: true, disabled: false })) },
  {
    name: "使われていないPC",
    rows: ad.search({ type: "computer", inactiveDays: 90, disabled: false }).map((c) => ({
      名前: c.name,
      OS: c.operatingSystem || "",
      最終ログオン: c.lastLogonTimestamp || null,
      場所: c.distinguishedName,
    })),
  },
];

const dest = await showSaveFilePicker({ suggestedName: `アカウント点検_${dayjs().format("YYYYMMDD")}.xlsx` });
if (!dest) exit();
const book = workbooks.create();
for (const s of sheets) {
  const sheet = book.addSheet(s.name);
  if (s.rows.length) sheet.write(s.rows);
  else sheet.cell("A1").value = "該当なし";
}
book.save(dest);
alert(sheets.map((s) => `${s.name}: ${s.rows.length} 件`).join("\n"));
shell.showItemInFolder(dest);
