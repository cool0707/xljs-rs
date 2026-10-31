import { ok, equal, deepEqual, throws } from "./_assert.js";

// Runs against the fake directory of the tests (testdata/ad/directory.json).
ok(ad.available());
deepEqual(ad.domain(), {
  name: "corp.example.com",
  netbiosName: "CORP",
  distinguishedName: "DC=corp,DC=example,DC=com",
  forest: "corp.example.com",
  server: "dc01.corp.example.com",
});

// The signed-in user and this computer.
const me = ad.me();
equal(me.type, "user");
equal(me.sAMAccountName, "yamada");
equal(me.displayName, "山田 太郎");
equal(me.employeeID, "00123");
equal(me.objectSid, "S-1-5-21-1-2-3-1105");
equal(me.objectGUID, "3f2504e0-4f89-11d3-9a0c-0305e82c3301");
deepEqual(me.memberOf, ["CN=営業部,OU=Groups,DC=corp,DC=example,DC=com"]);
ok(me.whenCreated instanceof Date);
equal(me.whenCreated.toISOString(), "2024-04-01T00:00:00.000Z");
equal(me.pwdLastSet.toISOString(), "2024-04-01T00:00:00.000Z");
equal(me.accountExpires, null);
equal(me.enabled, true);
equal(me.lockedOut, false);
equal(me["msDS-User-Account-Control-Computed"], undefined);
equal(ad.me({ properties: ["tokenGroups"] }).tokenGroups.length, 2);
const pc = ad.computer();
equal(pc.type, "computer");
equal(pc.dNSHostName, "pc001.corp.example.com");

// Search: users by default; Get-ADUser -Filter, Search-ADAccount and Get-ADComputer in one.
const sams = (list) => list.map((u) => u.sAMAccountName);
deepEqual(sams(ad.search()), ["yamada", "suzuki", "tanaka", "sato"]);
deepEqual(sams(ad.search("鈴木*")), ["suzuki"]);
deepEqual(sams(ad.search({ filter: { department: "営業部" } })), ["yamada", "suzuki"]);
deepEqual(sams(ad.search({ filter: { department: ["総務部", "開発部"] } })), ["tanaka", "sato"]);
deepEqual(sams(ad.search({ filter: { manager: true } })), ["sato"]);
deepEqual(sams(ad.search({ filter: "(title=課長)" })), ["yamada"]);
deepEqual(sams(ad.search({ disabled: true })), ["tanaka"]);
deepEqual(sams(ad.search({ disabled: false, department: undefined })), ["yamada", "suzuki", "sato"]);
deepEqual(sams(ad.search({ lockedOut: true })), ["suzuki"]);
deepEqual(sams(ad.search({ expired: true })), ["tanaka"]);
deepEqual(sams(ad.search({ passwordExpired: true })), ["sato"]);
deepEqual(sams(ad.search({ passwordNeverExpires: true })), ["tanaka"]);
deepEqual(sams(ad.search({ type: "computer" })), ["PC001$", "PC002$"]);
deepEqual(sams(ad.search({ type: "computer", inactiveDays: 3650 })), ["PC002$"]);
deepEqual(sams(ad.search({ type: ["user", "computer"], disabled: true })), ["tanaka", "PC002$"]);
deepEqual(sams(ad.search({ memberOf: "全社員", recursive: true })), ["yamada", "suzuki", "sato"]);
deepEqual(sams(ad.search({ memberOf: "全社員", type: "group" })), ["営業部", "総務部"]);
deepEqual(sams(ad.search({ searchBase: "OU=営業部,DC=corp,DC=example,DC=com" })), ["yamada", "suzuki"]);
equal(ad.search({ limit: 1 }).length, 1);

const g = ad.get("営業部", { type: "group", properties: ["member"] });
equal(g.groupScope, "global");
equal(g.groupCategory, "security");
equal(g.member.length, 2);
equal(ad.get("全社員").groupScope, "universal");
equal(ad.get("PC002").enabled, false);
equal(ad.get("CORP\\suzuki").displayName, "鈴木 花子");
equal(ad.get("nobody"), null);
const all = ad.get("yamada", { properties: ["*"] });
equal(all.cn, "山田 太郎");
equal(all.sAMAccountName, "yamada");
equal(ad.get("yamada", { properties: ["msDS-User-Account-Control-Computed"] })["msDS-User-Account-Control-Computed"], 0);

// Mistakes are errors.
throws(() => ad.search({ disable: true }), /オプション "disable"/);
throws(() => ad.search({ type: "printer" }), /printer/);
throws(() => ad.search({ filter: "(cn=a" }), /検索条件/);
throws(() => ad.search({ memberOf: "ない" }), /グループが見つかりません: ない/);
throws(() => ad.domain({ server: "down" }), /ドメインコントローラー down に接続できません/);

console.log("ad ok");
