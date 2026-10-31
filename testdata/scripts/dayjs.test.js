import { ok, equal, deepEqual } from "./_assert.js";

// Day.js is built in, with the bundled plugins and the locale of the
// message language (the tests run in Japanese), and defines no other globals.
equal(typeof dayjs, "function");
equal(typeof globalThis.dayjs_plugin_isBetween, "undefined");
equal(typeof globalThis.dayjs_locale_ja, "undefined");
equal(dayjs.locale(), "ja");

// "2026-04-01" is local midnight (new Date("2026-04-01") would be UTC).
const d = dayjs("2026-04-01");
equal(d.toDate().getTime(), new Date(2026, 3, 1).getTime());
equal(dayjs("2026/4/1 9:30").format("YYYY-MM-DD HH:mm"), "2026-04-01 09:30");
equal(d.format("YYYY年M月D日(ddd)"), "2026年4月1日(水)");
equal(dayjs("20260401", "YYYYMMDD").format("YYYY/MM/DD"), "2026/04/01");
equal(dayjs("2026/02/30", "YYYY/MM/DD", true).isValid(), false);
equal(d.endOf("month").date(), 30);
equal(d.add(1, "month").format("YYYY-MM"), "2026-05");
ok(d.isBetween("2026-03-01", "2026-04-30"));
ok(d.isSameOrAfter("2026-04-01"));
equal(d.quarter(), 2);
equal(d.isoWeek(), 14);
equal(d.dayOfYear(), 91);
equal(dayjs.max(dayjs("2026-01-01"), d).format("YYYY-MM-DD"), "2026-04-01");
equal(dayjs.duration(90, "minutes").format("H:mm"), "1:30");
equal(d.from(d.add(3, "day")), "3日前");
equal(d.diff("2026-03-01", "day"), 31);
equal(xljs.inspect(d), "dayjs(2026-04-01 00:00:00)");

// Excel serial numbers (cells without a date format).
equal(dayjs.fromExcel(45383).format("YYYY-MM-DD"), "2024-04-01");
equal(dayjs.fromExcel(45383.5).format("YYYY-MM-DD HH:mm"), "2024-04-01 12:00");
equal(dayjs.fromExcel(1).format("YYYY-MM-DD"), "1900-01-01");
equal(dayjs.fromExcel(61).format("YYYY-MM-DD"), "1900-03-01");

// Day.js objects are accepted wherever a Date is.
const book = workbooks.create();
const s = book.addSheet("x");
s.write([{ 日付: d }]);
s.cell("B1").value = d.add(1, "day");
s.setValue("C1", d);
s.range("D1:D1").values = [[d]];
ok(s.cell("A2").value instanceof Date);
equal(s.cell("A2").text, "2026/4/1");
equal(s.cell("B1").value.getDate(), 2);
ok(s.cell("C1").value instanceof Date);
ok(s.cell("D1").value instanceof Date);
equal(csv.stringify([{ d }]), "d\r\n2026/04/01\r\n");
fs.writeJSON("d.json", { d });
ok(fs.readText("d.json").includes('"2026-04-01T00:00:00'));

console.log("dayjs ok");
