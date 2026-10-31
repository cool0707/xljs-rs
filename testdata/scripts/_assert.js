// Minimal assertion helpers shared by the script tests.
const show = (v) => xljs.inspect(v);

export function ok(cond, msg = "assertion failed") {
  if (!cond) throw new Error(msg);
}

export function equal(actual, expected, msg = "") {
  if (!Object.is(actual, expected)) {
    throw new Error(`${msg ? msg + ": " : ""}expected ${show(expected)}, got ${show(actual)}`);
  }
}

function deepEq(a, b) {
  if (Object.is(a, b)) return true;
  if (a instanceof Date && b instanceof Date) return a.getTime() === b.getTime();
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a), kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => deepEq(a[k], b[k]));
}

export function deepEqual(actual, expected, msg = "") {
  if (!deepEq(actual, expected)) {
    throw new Error(`${msg ? msg + ": " : ""}expected ${show(expected)}, got ${show(actual)}`);
  }
}

export function throws(fn, pattern, msg = "") {
  try {
    fn();
  } catch (e) {
    if (pattern && !pattern.test(String(e && e.message))) {
      throw new Error(`${msg ? msg + ": " : ""}unexpected error message: ${e && e.message}`);
    }
    return e;
  }
  throw new Error(`${msg ? msg + ": " : ""}expected an exception`);
}

export async function rejects(fn, pattern, msg = "") {
  try {
    await fn();
  } catch (e) {
    if (pattern && !pattern.test(String(e && (e.name + " " + e.message)))) {
      throw new Error(`${msg ? msg + ": " : ""}unexpected error: ${e && e.message}`);
    }
    return e;
  }
  throw new Error(`${msg ? msg + ": " : ""}expected a rejection`);
}
