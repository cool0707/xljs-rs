import { ok, equal, deepEqual } from "./_assert.js";

// Modern syntax (optional chaining, nullish, class fields, async generators).
const o: any = { a: { b: 1 } };
equal(o?.a?.b, 1);
equal(o.x?.y ?? "def", "def");
let v: number | null = null;
v ??= 5;
equal(v, 5);

class Counter {
  #n = 0;
  static created = 0;
  inc() { return ++this.#n; }
}
const c = new Counter();
c.inc();
equal(c.inc(), 2);

async function* gen() { yield 1; yield 2; yield 3; }
const got: number[] = [];
for await (const x of gen()) got.push(x);
deepEqual(got, [1, 2, 3]);
deepEqual(await Array.fromAsync(gen(), (x: number) => x * 10), [10, 20, 30]);

// Built-ins and polyfills.
deepEqual(Object.groupBy([1, 2, 3, 4], (x: number) => (x % 2 ? "odd" : "even")), { odd: [1, 3], even: [2, 4] });
const src = { d: new Date(2026, 0, 2), m: new Map([[1, { x: 1 }]]), arr: [1, [2]] };
const cl = structuredClone(src);
ok(cl !== src && cl.d !== src.d && cl.d.getTime() === src.d.getTime());
equal(cl.m.get(1).x, 1);
equal(new TextDecoder().decode(new TextEncoder().encode("日本語")), "日本語");
const sjis = new Uint8Array([0x93, 0xfa, 0x96, 0x7b]); // 日本 in Shift_JIS
equal(new TextDecoder("shift_jis").decode(sjis), "日本");
equal(btoa("hello"), "aGVsbG8=");
equal(atob("aGVsbG8="), "hello");
equal((1234567.891).toLocaleString(), "1,234,567.891");
equal((1234.5).toLocaleString("ja-JP", { minimumFractionDigits: 2 }), "1,234.50");
equal((-0.5).toLocaleString("ja-JP", { style: "percent" }), "-50%");
equal(new Intl.NumberFormat("ja-JP").format(1000), "1,000");
equal(new Date(2026, 0, 2, 3, 4, 5).toLocaleDateString(), "2026/1/2");
equal(new Date(2026, 0, 2, 3, 4, 5).toLocaleString(), "2026/1/2 3:04:05");
deepEqual(["a10", "a2", "a1"].sort(new Intl.Collator("ja", { numeric: true }).compare), ["a1", "a2", "a10"]);

// Timers run in order; sleep resolves.
const order: string[] = [];
setTimeout(() => order.push("b"), 20);
setTimeout(() => order.push("a"), 5);
const iv = setInterval(() => { order.push("i"); clearInterval(iv); }, 1);
await sleep(40);
deepEqual(order, ["i", "a", "b"]);

// xljs object.
ok(Array.isArray(xljs.args));
equal(typeof xljs.version, "string");
equal(xljs.ui, "mock");
ok(xljs.script.endsWith("basics.test.ts"));

// inspect formatting.
equal(xljs.inspect({ a: 1, b: "x" }), '{ a: 1, b: "x" }');
equal(xljs.inspect([1, "a", null, undefined]), '[ 1, "a", null, undefined ]');
equal(xljs.inspect(new Date(2026, 3, 1, 9, 5, 0)), "2026-04-01 09:05:00");

console.log("basics ok");
