// The development mode's editor: completion and hover texts from the type
// declarations (types/xljs.d.ts, sent by the session in its "init" message).
//
// Not a type checker: a small reader of the d.ts (interfaces, classes, type
// aliases, globals) and a resolver that follows the code's syntax tree
// (CodeMirror's Lezer tree) far enough for the usual script:
//   ui.form(...)   wb.sheets[0].   for (const s of wb.sheets) s.
//   const wb = workbooks.open(...); wb.   await fs.readText(...).
//   ui.form({ ti|  onSubmit: async (v, ctx) => { ctx.| } })   rows.map((r) => r.|)
// Unknown types give no completion (CodeMirror's own completion of the local
// names and keywords stays).
(function () {
  "use strict";

  // Minimal declarations of the JS built-ins the scripts use most.
  const BUILTINS = `
  interface Array<T> {
    length: number;
    /** 末尾に追加し、新しい長さを返す */
    push(...items: T[]): number;
    pop(): T | undefined;
    shift(): T | undefined;
    unshift(...items: T[]): number;
    /** 各要素を変換した新しい配列 */
    map<U>(callback: (value: T, index: number, array: T[]) => U): U[];
    /** 条件に合う要素だけの新しい配列 */
    filter(predicate: (value: T, index: number, array: T[]) => unknown): T[];
    /** 条件に合う最初の要素（なければ undefined） */
    find(predicate: (value: T, index: number, array: T[]) => unknown): T | undefined;
    findIndex(predicate: (value: T, index: number, array: T[]) => unknown): number;
    findLast(predicate: (value: T, index: number, array: T[]) => unknown): T | undefined;
    forEach(callback: (value: T, index: number, array: T[]) => void): void;
    some(predicate: (value: T, index: number, array: T[]) => unknown): boolean;
    every(predicate: (value: T, index: number, array: T[]) => unknown): boolean;
    reduce<U>(callback: (previous: U, value: T, index: number, array: T[]) => U, initial: U): U;
    includes(value: T): boolean;
    indexOf(value: T): number;
    join(separator?: string): string;
    slice(start?: number, end?: number): T[];
    splice(start: number, deleteCount?: number, ...items: T[]): T[];
    sort(compare?: (a: T, b: T) => number): this;
    reverse(): this;
    concat(...items: (T | T[])[]): T[];
    flat(depth?: number): any[];
    flatMap<U>(callback: (value: T, index: number, array: T[]) => U | U[]): U[];
    at(index: number): T | undefined;
    fill(value: T, start?: number, end?: number): this;
    entries(): IterableIterator<[number, T]>;
    keys(): IterableIterator<number>;
    values(): IterableIterator<T>;
  }
  interface ArrayConstructor {
    isArray(value: any): boolean;
    from<T>(iterable: Iterable<T>): T[];
    of<T>(...items: T[]): T[];
  }
  interface String {
    length: number;
    trim(): string;
    trimStart(): string;
    trimEnd(): string;
    split(separator: string | RegExp, limit?: number): string[];
    replace(pattern: string | RegExp, replacement: string | ((match: string, ...args: any[]) => string)): string;
    replaceAll(pattern: string | RegExp, replacement: string | ((match: string, ...args: any[]) => string)): string;
    includes(search: string, position?: number): boolean;
    startsWith(search: string, position?: number): boolean;
    endsWith(search: string, length?: number): boolean;
    indexOf(search: string, position?: number): number;
    lastIndexOf(search: string, position?: number): number;
    slice(start?: number, end?: number): string;
    substring(start: number, end?: number): string;
    toUpperCase(): string;
    toLowerCase(): string;
    padStart(length: number, fill?: string): string;
    padEnd(length: number, fill?: string): string;
    repeat(count: number): string;
    match(pattern: RegExp): string[] | null;
    matchAll(pattern: RegExp): IterableIterator<string[]>;
    at(index: number): string | undefined;
    charAt(index: number): string;
    charCodeAt(index: number): number;
    normalize(form?: string): string;
    localeCompare(other: string): number;
  }
  interface Number {
    /** 小数点以下の桁数を指定して文字列にする */
    toFixed(digits?: number): string;
    /** 桁区切りなど、地域の書式の文字列にする */
    toLocaleString(locales?: string, options?: object): string;
    toString(radix?: number): string;
  }
  interface NumberConstructor {
    isInteger(value: unknown): boolean;
    isFinite(value: unknown): boolean;
    isNaN(value: unknown): boolean;
    parseFloat(text: string): number;
    parseInt(text: string, radix?: number): number;
  }
  interface Date {
    getFullYear(): number;
    /** 月（0 = 1 月） */
    getMonth(): number;
    getDate(): number;
    /** 曜日（0 = 日曜） */
    getDay(): number;
    getHours(): number;
    getMinutes(): number;
    getSeconds(): number;
    getMilliseconds(): number;
    getTime(): number;
    setFullYear(year: number, month?: number, date?: number): number;
    setMonth(month: number, date?: number): number;
    setDate(date: number): number;
    setHours(hours: number, minutes?: number, seconds?: number): number;
    setMinutes(minutes: number, seconds?: number): number;
    setSeconds(seconds: number): number;
    setTime(time: number): number;
    toISOString(): string;
    toLocaleDateString(locales?: string, options?: object): string;
    toLocaleTimeString(locales?: string, options?: object): string;
    toLocaleString(locales?: string, options?: object): string;
  }
  interface DateConstructor {
    now(): number;
    parse(text: string): number;
  }
  interface Map<K, V> {
    readonly size: number;
    get(key: K): V | undefined;
    set(key: K, value: V): this;
    has(key: K): boolean;
    delete(key: K): boolean;
    clear(): void;
    forEach(callback: (value: V, key: K) => void): void;
    keys(): IterableIterator<K>;
    values(): IterableIterator<V>;
    entries(): IterableIterator<[K, V]>;
  }
  interface Set<T> {
    readonly size: number;
    add(value: T): this;
    has(value: T): boolean;
    delete(value: T): boolean;
    clear(): void;
    forEach(callback: (value: T) => void): void;
    values(): IterableIterator<T>;
  }
  interface Promise<T> {
    then<U>(onFulfilled?: (value: T) => U): Promise<U>;
    catch(onRejected?: (reason: any) => any): Promise<T>;
    finally(onFinally?: () => void): Promise<T>;
  }
  interface PromiseConstructor {
    all(values: any[]): Promise<any[]>;
    allSettled(values: any[]): Promise<any[]>;
    race(values: any[]): Promise<any>;
    resolve<T>(value?: T): Promise<T>;
    reject(reason?: any): Promise<never>;
  }
  interface Math {
    abs(x: number): number;
    ceil(x: number): number;
    floor(x: number): number;
    round(x: number): number;
    trunc(x: number): number;
    max(...values: number[]): number;
    min(...values: number[]): number;
    pow(x: number, y: number): number;
    sqrt(x: number): number;
    random(): number;
    sign(x: number): number;
    log(x: number): number;
    log10(x: number): number;
    readonly PI: number;
  }
  interface JSON {
    parse(text: string, reviver?: (key: string, value: any) => any): any;
    stringify(value: any, replacer?: any, space?: string | number): string;
  }
  interface ObjectConstructor {
    keys(o: object): string[];
    values(o: object): any[];
    entries(o: object): [string, any][];
    assign<T>(target: T, ...sources: any[]): T;
    fromEntries(entries: Iterable<[string, any]>): any;
    freeze<T>(o: T): T;
  }
  interface RegExp {
    test(text: string): boolean;
    exec(text: string): string[] | null;
    readonly source: string;
    readonly flags: string;
    lastIndex: number;
  }
  const Math: Math;
  const JSON: JSON;
  const Object: ObjectConstructor;
  const Array: ArrayConstructor;
  const Number: NumberConstructor;
  const Date: DateConstructor;
  const Promise: PromiseConstructor;
  function parseInt(text: string, radix?: number): number;
  function parseFloat(text: string): number;
  function isNaN(value: unknown): boolean;
  function String(value?: any): string;
  function Boolean(value?: any): boolean;
  class Error { constructor(message?: string); message: string; name: string; stack?: string; }
  class TypeError extends Error {}
  class RangeError extends Error {}
`;

  // Built-in classes whose `new X()` gives the interface X.
  const CTORS = { Date: "Date", Map: "Map<any, any>", Set: "Set<any>", RegExp: "RegExp", Array: "any[]", Error: "Error" };

  // ------------------------------------------------------------ d.ts reader

  const ID = /^[A-Za-z_$][\w$]*/;

  /** Splits text at the top-level separators (outside (), [], {}, <> and strings). */
  function splitTop(text, seps) {
    const out = [];
    let depth = 0, start = 0;
    for (let i = 0; i < text.length; i++) {
      const c = text[i];
      if (c === '"' || c === "'" || c === "`") { i = skipString(text, i); continue; }
      if (c === "(" || c === "[" || c === "{" || c === "<") depth++;
      else if (c === ")" || c === "]" || c === "}") depth--;
      else if (c === ">" && text[i - 1] !== "=") depth--;
      else if (depth === 0 && seps.includes(c)) {
        // `=>` is not a separator; `|` / `&` at the start are not either.
        out.push(text.slice(start, i));
        start = i + 1;
      }
    }
    out.push(text.slice(start));
    return out.map((s) => s.trim()).filter((s, i, a) => s !== "" || (i > 0 && i < a.length - 1));
  }

  function skipString(text, i) {
    const q = text[i];
    for (i++; i < text.length && text[i] !== q; i++) if (text[i] === "\\") i++;
    return i;
  }

  /** The index just after the bracket that closes the one at i. */
  function closeOf(text, i) {
    let depth = 0;
    for (; i < text.length; i++) {
      const c = text[i];
      if (c === '"' || c === "'" || c === "`") { i = skipString(text, i); continue; }
      if (c === "(" || c === "[" || c === "{" || c === "<") depth++;
      else if (c === ")" || c === "]" || c === "}" || (c === ">" && text[i - 1] !== "=")) {
        if (--depth === 0) return i + 1;
      }
    }
    return text.length;
  }

  function cleanDoc(doc) {
    return doc
      .replace(/^\/\*\*|\*\/$/g, "")
      .split("\n")
      .map((l) => l.replace(/^\s*\* ?/, "").trimEnd())
      .join("\n")
      .trim();
  }

  /**
   * Splits declarations into {text, doc}: a statement ends at a top-level `;`
   * or `,`, or after a top-level block (`interface X { ... }`).
   */
  function statements(text) {
    const out = [];
    let doc = "", start = -1, depth = 0;
    const end = (i) => {
      const s = text.slice(start, i).trim();
      if (s) out.push({ text: s, doc });
      doc = "";
      start = -1;
    };
    for (let i = 0; i < text.length; i++) {
      const c = text[i];
      if (c === "/" && text[i + 1] === "/") { i = text.indexOf("\n", i); if (i < 0) break; continue; }
      if (c === "/" && text[i + 1] === "*") {
        const j = text.indexOf("*/", i + 2);
        const e = j < 0 ? text.length : j + 2;
        if (start < 0 && text[i + 2] === "*") doc = cleanDoc(text.slice(i, e));
        i = e - 1;
        continue;
      }
      if (start < 0) {
        if (/\s/.test(c) || c === ";" || c === ",") continue;
        start = i;
      }
      if (c === '"' || c === "'" || c === "`") { i = skipString(text, i); continue; }
      if (c === "(" || c === "[" || c === "{" || c === "<") depth++;
      else if (c === ")" || c === "]" || (c === ">" && text[i - 1] !== "=")) depth--;
      else if (c === "}") {
        depth--;
        // A block ends the statement unless the type continues (`} | X`, `}[]`, `};`).
        if (depth === 0 && /^(interface|class|declare|namespace|module|enum)\b/.test(text.slice(start, i))) {
          const rest = text.slice(i + 1).match(/^\s*(.)/);
          if (!rest || !"|&[;,".includes(rest[1])) end(i + 1);
        }
      } else if (depth === 0 && (c === ";" || (c === "," && !/^(?:(?:export|declare)\s+)*(interface|class)\b/.test(text.slice(start, i))))) end(i);
    }
    if (start >= 0) end(text.length);
    return out;
  }

  /** "T extends C = D, U" → [{name, constraint, def}] */
  function typeParams(text) {
    if (!text) return [];
    return splitTop(text.replace(/^<|>$/g, ""), ",").map((p) => {
      const m = p.match(/^([A-Za-z_$][\w$]*)\s*(?:extends\s+((?:(?!=[^>]).)+?))?\s*(?:=\s*(.+))?$/s);
      return m ? { name: m[1], constraint: m[2] && m[2].trim(), def: m[3] && m[3].trim() } : { name: p };
    });
  }

  /** "(a: X, b?: Y, ...c: Z[])" → [{name, type, optional, rest}] */
  function params(text) {
    const inner = text.replace(/^\(|\)$/g, "").trim();
    if (!inner) return [];
    return splitTop(inner, ",").map((p) => {
      const rest = p.startsWith("...");
      if (rest) p = p.slice(3);
      const m = p.match(/^([^:?]+?)\s*(\?)?\s*(?::\s*([\s\S]+?))?\s*$/);
      return m ? { name: m[1].trim(), optional: !!m[2], type: m[3] || "any", rest } : { name: p, type: "any", rest };
    });
  }

  /** Reads "name<T>(params): ret" (after the name) into a function signature. */
  function signature(rest) {
    let tp = [];
    rest = rest.trim();
    if (rest.startsWith("<")) {
      const e = closeOf(rest, 0);
      tp = typeParams(rest.slice(0, e));
      rest = rest.slice(e).trim();
    }
    if (!rest.startsWith("(")) return null;
    const e = closeOf(rest, 0);
    const ps = params(rest.slice(0, e));
    const after = rest.slice(e).trim();
    const ret = after.startsWith(":") ? after.slice(1).trim() : after.startsWith("=>") ? after.slice(2).trim() : "void";
    return { typeParams: tp, params: ps, ret };
  }

  /** A member of an interface, class or object type, or null (index signatures...). */
  function member(text, doc) {
    let t = text.replace(/^(?:(?:readonly|static|public|declare|abstract)\s+)+/, "");
    if (/^(get|set)\s+[A-Za-z_$]/.test(t)) t = t.replace(/^(get|set)\s+/, "");
    if (t.startsWith("(") || t.startsWith("<")) {
      // A call signature: the interface is a function too (dayjs()).
      const fn = signature(t);
      return fn ? { name: "", kind: "call", fn, doc, text } : null;
    }
    if (/^(\[|new\b|constructor\b)/.test(t)) return null;
    const m = t.match(/^([A-Za-z_$][\w$]*|"[^"]*"|'[^']*')\s*(\?)?/);
    if (!m) return null;
    const name = m[1].replace(/^["']|["']$/g, "");
    const rest = t.slice(m[0].length).trim();
    if (rest.startsWith("(") || rest.startsWith("<")) {
      const fn = signature(rest);
      if (!fn) return null;
      return { name, kind: "method", optional: !!m[2], fn, doc, text };
    }
    const type = rest.startsWith(":") ? rest.slice(1).trim() : "any";
    return { name, kind: "property", optional: !!m[2], readonly: /^readonly\s/.test(text), type, doc, text };
  }

  function members(body) {
    const out = [];
    for (const s of statements(body)) {
      const m = member(s.text, s.doc);
      if (m) out.push(m);
    }
    return out;
  }

  /** Reads the declarations into db.types (interfaces, classes, aliases) and db.values (globals). */
  function declare(db, text) {
    for (const s of statements(text)) {
      let t = s.text.replace(/^(?:(?:export|declare)\s+)+/, "");
      let m;
      if (/^global\s*\{/.test(t) || /^(namespace|module)\s+[\w.$"']+\s*\{/.test(t)) {
        if (/^global/.test(t)) declare(db, t.slice(t.indexOf("{") + 1, t.lastIndexOf("}")));
        continue;
      }
      if ((m = t.match(/^(interface|class)\s+([A-Za-z_$][\w$]*)/))) {
        const open = t.indexOf("{");
        if (open < 0) continue;
        let head = t.slice(m[0].length, open).trim();
        let tp = [];
        if (head.startsWith("<")) {
          const e = closeOf(head, 0);
          tp = typeParams(head.slice(0, e));
          head = head.slice(e).trim();
        }
        const ext = [];
        const em = head.match(/^extends\s+([\s\S]+?)(?:\s+implements\s+([\s\S]+))?$/) || head.match(/^implements\s+()([\s\S]+)$/);
        if (em) {
          if (em[1]) ext.push(...splitTop(em[1], ","));
          if (em[2]) ext.push(...splitTop(em[2], ","));
        }
        const body = t.slice(open + 1, t.lastIndexOf("}"));
        const decl = { kind: m[1], name: m[2], typeParams: tp, ext, members: members(body), doc: s.doc };
        push(db.types, m[2], decl);
        if (m[1] === "class") {
          const ctor = body.match(/constructor\s*(\([^)]*\))/);
          push(db.values, m[2], { kind: "class", name: m[2], type: m[2], ctor: ctor ? params(ctor[1]) : [], doc: s.doc });
        }
      } else if ((m = t.match(/^type\s+([A-Za-z_$][\w$]*)\s*(<[^=]*>)?\s*=\s*([\s\S]+)$/))) {
        push(db.types, m[1], { kind: "alias", name: m[1], typeParams: typeParams(m[2]), text: m[3].trim(), doc: s.doc });
      } else if ((m = t.match(/^(const|let|var)\s+([A-Za-z_$][\w$]*)\s*:\s*([\s\S]+)$/))) {
        push(db.values, m[2], { kind: "variable", name: m[2], type: m[3].trim(), doc: s.doc, text: t });
      } else if ((m = t.match(/^function\s+([A-Za-z_$][\w$]*)/))) {
        const fn = signature(t.slice(m[0].length));
        if (fn) push(db.values, m[1], { kind: "function", name: m[1], fn, doc: s.doc, text: t });
      }
    }
  }

  function push(map, k, v) {
    const a = map.get(k);
    if (a) a.push(v); else map.set(k, [v]);
  }

  // ------------------------------------------------------------ types

  // A type is {text, ctx} (ctx: type parameter name → type) or a function
  // {fn, ctx}. A shape (what a type has) is a Map name → [member, ctx].

  const db = { types: new Map(), values: new Map() };
  declare(db, BUILTINS);
  const builtinNames = new Set(db.values.keys());
  let loaded = false;

  function load(dts) {
    for (const k of [...db.types.keys()]) db.types.delete(k);
    for (const k of [...db.values.keys()]) db.values.delete(k);
    declare(db, BUILTINS);
    if (dts) declare(db, dts);
    shapeCache.clear();
    loaded = !!dts;
  }

  const PRIMITIVE = { string: "String", number: "Number", bigint: "Number" };
  const shapeCache = new Map();

  function lookupParam(name, ctx) {
    for (let c = ctx; c; c = c.parent) if (c.vars && Object.prototype.hasOwnProperty.call(c.vars, name)) return c.vars[name];
    return undefined;
  }

  /** {vars, parent}: the type parameters of decl applied to args (texts in ctx). */
  function bind(tp, args, ctx, self) {
    const vars = {};
    tp.forEach((p, i) => {
      if (args && args[i] !== undefined) vars[p.name] = { text: args[i], ctx };
      else if (p.def || p.constraint) vars[p.name] = { text: p.def || p.constraint, ctx: { vars, parent: ctx } };
      else vars[p.name] = null;
    });
    if (self) vars.this = self;
    return { vars, parent: ctx };
  }

  /** Normalizes a type text: removes `readonly`, outer parentheses. */
  function norm(text) {
    let t = (text || "").trim().replace(/^readonly\s+/, "");
    while (t.startsWith("(") && closeOf(t, 0) === t.length && !/^\([\s\S]*\)\s*=>/.test(t)) t = t.slice(1, -1).trim();
    return t;
  }

  /** The parts of a union, without null / undefined / void / never. */
  function unionParts(text) {
    const parts = splitTop(text, "|").filter((p) => p !== "");
    return parts.filter((p) => !/^(null|undefined|void|never)$/.test(p));
  }

  /** Resolves a type to a function signature (for calls), or null. */
  function asFn(type, depth = 0) {
    if (!type || depth > 8) return null;
    if (type.fn) return type;
    let t = norm(type.text);
    const parts = unionParts(t);
    if (parts.length !== 1) {
      for (const p of parts) { const f = asFn({ text: p, ctx: type.ctx }, depth + 1); if (f) return f; }
      return null;
    }
    t = parts[0];
    if (t.startsWith("(") || t.startsWith("<")) {
      const fn = signature(t);
      if (fn) return { fn, ctx: type.ctx };
    }
    const tm = t.match(/^typeof\s+([A-Za-z_$][\w$]*)$/);
    if (tm) {
      const v = (db.values.get(tm[1]) || [])[0];
      if (!v) return null;
      return v.fn ? { fn: v.fn, ctx: null, decl: v } : asFn({ text: v.type, ctx: null }, depth + 1);
    }
    const nm = t.match(/^([A-Za-z_$][\w$.]*)\s*(<[\s\S]*>)?$/);
    if (nm) {
      const p = lookupParam(nm[1], type.ctx);
      if (p !== undefined) return p && asFn(p, depth + 1);
      const args = nm[2] ? splitTop(nm[2].slice(1, -1), ",") : null;
      for (const d of db.types.get(nm[1]) || []) {
        const ctx = bind(d.typeParams, args, type.ctx);
        if (d.kind === "alias") return asFn({ text: d.text, ctx }, depth + 1);
        const call = d.members.find((m) => m.kind === "call");
        if (call) return { fn: call.fn, ctx };
      }
    }
    return null;
  }

  /** The return type of calling a function type. */
  function callResult(f) {
    if (!f) return null;
    return { text: f.fn.ret, ctx: bind(f.fn.typeParams || [], null, f.ctx) };
  }

  /** Resolves a type to its shape (Map name → [{m, ctx}]), or null. */
  function shape(type, depth = 0) {
    if (!type || depth > 12) return null;
    if (type.fn) return shape({ text: "Function", ctx: null }, depth + 1);
    const t = norm(type.text);
    if (!t) return null;
    const parts = unionParts(t);
    if (parts.length > 1) return merge(parts.map((p) => shape({ text: p, ctx: type.ctx }, depth + 1)));
    if (parts.length === 0) return null;
    const inter = splitTop(parts[0], "&").filter((p) => p !== "");
    if (inter.length > 1) return merge(inter.map((p) => shape({ text: p, ctx: type.ctx }, depth + 1)));
    let s = norm(inter[0]);
    if (/^["'`]/.test(s)) return shape({ text: "String", ctx: null }, depth + 1);
    if (/^-?\d/.test(s)) return shape({ text: "Number", ctx: null }, depth + 1);
    if (s.endsWith("[]") && closeOf(s, s.length - 2) === s.length) {
      return shape({ text: `Array<${s.slice(0, -2)}>`, ctx: type.ctx }, depth + 1);
    }
    if (s.startsWith("[")) return shape({ text: "Array<any>", ctx: null }, depth + 1);
    if (s.startsWith("{")) {
      const ms = members(s.slice(1, closeOf(s, 0) - 1));
      return merge([ms.length ? new Map(ms.map((m) => [m.name, [{ m, ctx: type.ctx }]])) : null]);
    }
    const tm = s.match(/^typeof\s+([A-Za-z_$][\w$]*)$/);
    if (tm) {
      const v = (db.values.get(tm[1]) || [])[0];
      return v && v.type ? shape({ text: v.type, ctx: null }, depth + 1) : null;
    }
    const nm = s.match(/^([A-Za-z_$][\w$]*)\s*(<[\s\S]*>)?$/);
    if (!nm) return null;
    const name = nm[1];
    if (PRIMITIVE[name]) return shape({ text: PRIMITIVE[name], ctx: null }, depth + 1);
    const p = lookupParam(name, type.ctx);
    if (p !== undefined) return p && shape(p, depth + 1);
    const decls = db.types.get(name);
    if (!decls) return null;
    const args = nm[2] ? splitTop(nm[2].slice(1, -1), ",") : null;
    const key = !args && !type.ctx ? name : null;
    if (key && shapeCache.has(key)) return shapeCache.get(key);
    const out = [];
    for (const d of decls) {
      const ctx = bind(d.typeParams, args, type.ctx, { text: s, ctx: type.ctx });
      if (d.kind === "alias") { out.push(shape({ text: d.text, ctx }, depth + 1)); continue; }
      const own = new Map();
      for (const m of d.members) {
        if (m.kind === "call") continue;
        if (!own.has(m.name)) own.set(m.name, []);
        own.get(m.name).push({ m, ctx });
      }
      out.push(own);
      for (const e of d.ext) out.push(shape({ text: e, ctx }, depth + 1));
    }
    const r = merge(out);
    if (key) shapeCache.set(key, r);
    return r;
  }

  /** Merges shapes; earlier ones win for the same name. */
  function merge(shapes) {
    let out = null;
    for (const s of shapes) {
      if (!s) continue;
      if (!out) out = new Map();
      for (const [k, v] of s) if (!out.has(k)) out.set(k, v);
    }
    return out;
  }

  /** The type of a member as a value: a function for methods. */
  function memberType(entry) {
    const { m, ctx } = entry;
    if (m.kind === "method") return { fn: m.fn, ctx, member: m };
    return { text: m.type, ctx, member: m };
  }

  /** The element type of an iterable / array type. */
  function elementOf(type, depth = 0) {
    if (!type || type.fn || depth > 10) return null;
    const t = norm(type.text);
    for (const p of unionParts(t)) {
      for (const q of splitTop(p, "&")) {
        const s = norm(q);
        if (s.endsWith("[]") && closeOf(s, s.length - 2) === s.length) return { text: s.slice(0, -2), ctx: type.ctx };
        const nm = s.match(/^([A-Za-z_$][\w$]*)\s*(<[\s\S]*>)?$/);
        if (!nm) continue;
        const pv = lookupParam(nm[1], type.ctx);
        if (pv !== undefined) { const r = pv && elementOf(pv, depth + 1); if (r) return r; continue; }
        const args = nm[2] ? splitTop(nm[2].slice(1, -1), ",") : null;
        if (/^(Array|Set|ReadonlyArray)$/.test(nm[1]) && args) return { text: args[0], ctx: type.ctx };
        if (/^(Iterable|AsyncIterable|IterableIterator|AsyncIterableIterator|Generator|AsyncGenerator)$/.test(nm[1]) && args) return { text: args[0], ctx: type.ctx };
        if (nm[1] === "Map" && args) return { text: `[${args[0]}, ${args[1]}]`, ctx: type.ctx };
        for (const d of db.types.get(nm[1]) || []) {
          const ctx = bind(d.typeParams, args, type.ctx);
          if (d.kind === "alias") { const r = elementOf({ text: d.text, ctx }, depth + 1); if (r) return r; continue; }
          for (const e of d.ext) { const r = elementOf({ text: e, ctx }, depth + 1); if (r) return r; }
        }
      }
    }
    return null;
  }

  /** Promise<X> → X (also in unions). */
  function awaited(type) {
    if (!type || type.fn) return type;
    const parts = unionParts(norm(type.text)).map((p) => {
      const m = p.match(/^(?:Promise|PromiseLike)\s*<([\s\S]*)>$/);
      return m ? m[1] : p;
    });
    return { text: parts.join(" | "), ctx: type.ctx };
  }

  /** The type of index access x[i]. */
  function indexOf(type) {
    const e = elementOf(type);
    if (e) return e;
    // Records: { [k: string]: X }
    return null;
  }

  // ------------------------------------------------------------ the code

  const SCOPES = new Set(["Script", "Block", "ArrowFunction", "FunctionDeclaration", "FunctionExpression", "ForStatement", "ClassBody", "MethodDeclaration", "CatchClause"]);

  function text(state, n) { return n ? state.sliceDoc(n.from, n.to) : ""; }

  /** Children of a node (Lezer SyntaxNode). */
  function kids(n) {
    const out = [];
    for (let c = n.firstChild; c; c = c.nextSibling) out.push(c);
    return out;
  }

  /** Lezer gives new node objects on each walk: compare positions. */
  function same(a, b) { return !!a && !!b && a.from === b.from && a.to === b.to && a.name === b.name; }
  function indexIn(list, n) { return list.findIndex((c) => same(c, n)); }

  function isExpr(n) {
    return !["(", ")", ",", "[", "]", "{", "}", ":", ";", "⚠", "LineComment", "BlockComment", "Spread"].includes(n.name);
  }

  /** The declaration of a variable name visible at pos: its VariableDefinition node. */
  function definition(state, tree, name, pos) {
    let best = null;
    tree.iterate({
      enter(n) {
        if (n.name !== "VariableDefinition" || state.sliceDoc(n.from, n.to) !== name) return;
        const node = n.node;
        // The scope that holds the name must contain pos.
        let s = node.parent;
        if (s && (s.name === "VariableDeclaration" || s.name === "ForOfSpec" || s.name === "ForInSpec")) {
          // `const` in a for head belongs to the for; others to the block around.
          s = s.name === "VariableDeclaration" ? s.parent : s.parent;
        } else if (s && s.name === "ParamList") s = s.parent;
        while (s && !SCOPES.has(s.name)) s = s.parent;
        if (s && (pos < s.from || pos > s.to)) return;
        if (!best || (node.from <= pos && (best.from > pos || node.from > best.from))) best = node;
      },
    });
    return best;
  }

  /** The type of a VariableDefinition node. */
  function defType(state, tree, def, depth) {
    const p = def.parent;
    if (!p) return null;
    let n = def.nextSibling;
    if (n && n.name === "TypeAnnotation") return { text: text(state, n).replace(/^:\s*/, ""), ctx: null };
    if (p.name === "VariableDeclaration") {
      if (n && n.name === "Equals" && n.nextSibling) return exprType(state, tree, n.nextSibling, depth + 1);
      return null;
    }
    if (p.name === "ForOfSpec") {
      for (let c = def.nextSibling; c; c = c.nextSibling) {
        if (c.name === "of") {
          const e = c.nextSibling && exprType(state, tree, c.nextSibling, depth + 1);
          return elementOf(e && awaited(e));
        }
      }
      return null;
    }
    if (p.name === "ParamList") {
      const fn = p.parent;
      if (!fn) return null;
      const list = kids(p).filter((c) => c.name === "VariableDefinition" || c.name.endsWith("Pattern"));
      const idx = indexIn(list, def);
      const f = asFn(expected(state, tree, fn, depth + 1));
      if (!f || idx < 0) return null;
      const prm = f.fn.params[idx];
      return prm ? { text: prm.type, ctx: bind(f.fn.typeParams || [], null, f.ctx) } : null;
    }
    if (p.name === "ArrowFunction") {
      // x => ... (no parentheses)
      const f = asFn(expected(state, tree, p, depth + 1));
      const prm = f && f.fn.params[0];
      return prm ? { text: prm.type, ctx: bind(f.fn.typeParams || [], null, f.ctx) } : null;
    }
    return null;
  }

  /** The type the code around expects for the expression node (contextual type). */
  function expected(state, tree, node, depth = 0) {
    if (depth > 20) return null;
    const p = node.parent;
    if (!p) return null;
    if (p.name === "ParenthesizedExpression") return expected(state, tree, p, depth + 1);
    if (p.name === "Property") {
      const key = p.firstChild;
      if (!key || same(key, node)) return null;
      const obj = p.parent;
      const s = obj && shape(expected(state, tree, obj, depth + 1));
      const e = s && s.get(text(state, key).replace(/^["']|["']$/g, ""));
      return e ? memberType(e[0]) : null;
    }
    if (p.name === "ArrayExpression") return elementOf(expected(state, tree, p, depth + 1));
    if (p.name === "ArgList") {
      const call = p.parent;
      if (!call || call.name !== "CallExpression" && call.name !== "NewExpression") return null;
      const args = kids(p).filter(isExpr);
      const idx = indexIn(args, node);
      const callee = call.firstChild;
      let f;
      if (call.name === "NewExpression") {
        const cls = callee && callee.nextSibling;
        const v = cls && (db.values.get(text(state, cls)) || [])[0];
        if (v && v.ctor) return idx < v.ctor.length ? { text: v.ctor[idx].type, ctx: null } : null;
        return null;
      }
      f = asFn(exprType(state, tree, callee, depth + 1));
      if (!f || idx < 0) return null;
      let prm = f.fn.params[idx];
      if (!prm && f.fn.params.length && f.fn.params[f.fn.params.length - 1].rest) prm = f.fn.params[f.fn.params.length - 1];
      if (!prm) return null;
      const ctx = bind(f.fn.typeParams || [], null, f.ctx);
      if (prm.rest) return elementOf({ text: prm.type, ctx }) || { text: prm.type, ctx };
      return { text: prm.type, ctx };
    }
    if (p.name === "VariableDeclaration") {
      const ann = node.prevSibling && node.prevSibling.prevSibling;
      if (ann && ann.name === "TypeAnnotation") return { text: text(state, ann).replace(/^:\s*/, ""), ctx: null };
      return null;
    }
    if (p.name === "ReturnStatement" || p.name === "AwaitExpression") return null;
    return null;
  }

  /** The type of an expression node, or null. */
  function exprType(state, tree, node, depth = 0) {
    if (!node || depth > 20) return null;
    switch (node.name) {
      case "VariableName": {
        const name = text(state, node);
        const def = definition(state, tree, name, node.from);
        if (def) return defType(state, tree, def, depth + 1);
        const v = db.values.get(name);
        if (!v) return null;
        const d = v[0];
        if (d.fn) return { fn: d.fn, ctx: null, member: d };
        if (d.kind === "class") return { text: "typeof " + name, ctx: null, member: d, cls: name };
        return { text: d.type, ctx: null, member: d };
      }
      case "MemberExpression": {
        const obj = node.firstChild;
        const op = obj && obj.nextSibling;
        if (op && op.name === "[") return indexOf(exprType(state, tree, obj, depth + 1));
        const prop = node.lastChild;
        if (!prop || prop.name !== "PropertyName") return null;
        const s = shape(exprType(state, tree, obj, depth + 1));
        const e = s && s.get(text(state, prop));
        return e ? memberType(e[0]) : null;
      }
      case "CallExpression": {
        const f = asFn(exprType(state, tree, node.firstChild, depth + 1));
        return callResult(f);
      }
      case "NewExpression": {
        const cls = node.firstChild && node.firstChild.nextSibling;
        const name = text(state, cls);
        if (CTORS[name]) return { text: CTORS[name], ctx: null };
        if (db.types.get(name)) return { text: name, ctx: null };
        return null;
      }
      case "AwaitExpression": return awaited(exprType(state, tree, node.lastChild, depth + 1));
      case "ParenthesizedExpression": {
        const inner = kids(node).find(isExpr);
        return exprType(state, tree, inner, depth + 1);
      }
      case "String": case "TemplateString": return { text: "string", ctx: null };
      case "Number": return { text: "number", ctx: null };
      case "ArrayExpression": return { text: "any[]", ctx: null };
      case "RegExp": return { text: "RegExp", ctx: null };
      case "this": return null;
      default: return null;
    }
  }

  // ------------------------------------------------------------ texts

  function short(t, n = 60) {
    t = (t || "").replace(/\s+/g, " ");
    return t.length > n ? t.slice(0, n - 1) + "…" : t;
  }

  function sigText(name, fn) {
    const ps = fn.params.map((p) => (p.rest ? "..." : "") + p.name + (p.optional ? "?" : "") + ": " + p.type).join(", ");
    return `${name}(${ps}): ${fn.ret}`;
  }

  /** The signature lines (all overloads) and the doc of members. */
  function describe(entries) {
    const lines = [], docs = [];
    for (const { m } of entries) {
      lines.push(m.kind === "method" ? sigText(m.name, m.fn) : (m.readonly ? "readonly " : "") + m.name + (m.optional ? "?" : "") + ": " + m.type);
      if (m.doc && !docs.includes(m.doc)) docs.push(m.doc);
    }
    return { code: lines.join("\n"), doc: docs.join("\n\n") };
  }

  function describeValue(d) {
    const code = d.fn ? sigText(d.name, d.fn) : d.kind === "class" ? "class " + d.name : (d.kind === "variable" ? "const " : "") + d.name + ": " + d.type;
    return { code, doc: d.doc || "" };
  }

  function infoNode(desc) {
    return () => {
      const box = document.createElement("div");
      box.className = "cm-xljs-info";
      const pre = document.createElement("pre");
      pre.textContent = desc.code;
      box.append(pre);
      if (desc.doc) {
        const p = document.createElement("div");
        p.className = "doc";
        p.textContent = desc.doc;
        box.append(p);
      }
      return box;
    };
  }

  function memberOption(name, entries, boost) {
    const m = entries[0].m;
    const detail = m.kind === "method"
      ? short("(" + m.fn.params.map((p) => p.name + (p.optional ? "?" : "")).join(", ") + ")", 40)
      : short(m.type, 40);
    return { label: name, type: m.kind === "method" ? "method" : "property", detail, info: infoNode(describe(entries)), boost };
  }

  // ------------------------------------------------------------ CodeMirror sources

  function treeOf(state) { return CM.syntaxTree(state); }

  /** Completion of `obj.`, `{ key }` in a typed object literal, and the globals. */
  function complete(ctx) {
    const { state, pos } = ctx;
    const tree = treeOf(state);
    const node = tree.resolveInner(pos, -1);
    if (/Comment|String|RegExp/.test(node.name)) return null;

    // obj.pro|  /  obj.|
    let mem = null, from = pos;
    if (node.name === "PropertyName" && node.parent && node.parent.name === "MemberExpression") { mem = node.parent; from = node.from; }
    else if (node.name === "." || node.name === "?.") { mem = node.parent; from = pos; }
    else if (node.name === "MemberExpression" && /\??\.\s*$/.test(state.sliceDoc(node.from, pos))) { mem = node; from = pos; }
    else if (node.name === "⚠" && node.parent && node.parent.name === "MemberExpression") { mem = node.parent; from = pos; }
    if (mem && mem.name === "MemberExpression") {
      const obj = mem.firstChild;
      const s = shape(exprType(state, tree, obj));
      if (!s) return null;
      const options = [];
      for (const [name, entries] of s) options.push(memberOption(name, entries));
      if (!options.length) return null;
      return { from, options, validFor: /^[\w$]*$/ };
    }

    // { key| } in an object whose type is known.
    let obj = null;
    if (node.name === "PropertyDefinition" && node.parent && node.parent.name === "Property" && same(node.parent.firstChild, node) && !node.nextSibling) { obj = node.parent.parent; from = node.from; }
    else if (node.name === "ObjectExpression" && /[{,]\s*$/.test(state.sliceDoc(node.from, pos))) { obj = node; from = pos; }
    else if (node.name === "VariableName" && node.parent && node.parent.name === "ObjectExpression") { obj = node.parent; from = node.from; }
    if (obj && obj.name === "ObjectExpression") {
      const s = shape(expected(state, tree, obj));
      if (!s) return null;
      const used = new Set(kids(obj).filter((c) => c.name === "Property").map((c) => text(state, c.firstChild)));
      const options = [];
      for (const [name, entries] of s) {
        if (used.has(name) && !(node.name === "PropertyDefinition" && text(state, node) === name)) continue;
        const o = memberOption(name, entries, entries[0].m.optional ? 0 : 1);
        o.apply = name + ": ";
        options.push(o);
      }
      return { from, options, validFor: /^[\w$]*$/ };
    }

    // Globals of the runtime.
    if (!loaded) return null;
    const word = ctx.matchBefore(/[\w$]+/);
    if (!word && !ctx.explicit) return null;
    if (node.name !== "VariableName" && node.name !== "Script" && node.name !== "Block" && !ctx.explicit && !(word && node.name === "⚠")) return null;
    const options = [];
    for (const [name, ds] of db.values) {
      if (builtinNames.has(name) && !["Math", "JSON", "Object", "Array", "Number", "Date", "Promise"].includes(name)) continue;
      const d = ds[0];
      options.push({
        label: name,
        type: d.kind === "function" ? "function" : d.kind === "class" ? "class" : "variable",
        detail: d.fn ? short("(" + d.fn.params.map((p) => p.name + (p.optional ? "?" : "")).join(", ") + ")", 40) : d.type ? short(d.type, 40) : "",
        info: infoNode(describeValue(d)),
        boost: builtinNames.has(name) ? -1 : 0,
      });
    }
    return { from: word ? word.from : pos, options, validFor: /^[\w$]*$/ };
  }

  /** The description of the name under pos, for the hover tooltip. */
  function hoverAt(state, pos, side) {
    const tree = treeOf(state);
    const node = tree.resolveInner(pos, side);
    if (node.name === "PropertyName" && node.parent && node.parent.name === "MemberExpression") {
      const s = shape(exprType(state, tree, node.parent.firstChild));
      const e = s && s.get(text(state, node));
      return e ? { from: node.from, to: node.to, desc: describe(e) } : null;
    }
    if (node.name === "PropertyDefinition" && node.parent && node.parent.name === "Property") {
      const s = shape(expected(state, tree, node.parent.parent));
      const e = s && s.get(text(state, node).replace(/^["']|["']$/g, ""));
      return e ? { from: node.from, to: node.to, desc: describe(e) } : null;
    }
    if (node.name === "VariableName") {
      const name = text(state, node);
      if (definition(state, tree, name, node.from)) {
        const t = exprType(state, tree, node);
        if (!t) return null;
        const code = t.fn ? sigText(name, t.fn) : name + ": " + norm(t.text);
        return { from: node.from, to: node.to, desc: { code, doc: t.member && t.member.doc || "" } };
      }
      const v = db.values.get(name);
      return v ? { from: node.from, to: node.to, desc: describeValue(v[0]) } : null;
    }
    return null;
  }

  function hover(view, pos, side) {
    const h = hoverAt(view.state, pos, side);
    if (!h) return null;
    return { pos: h.from, end: h.to, above: true, create: () => ({ dom: infoNode(h.desc)() }) };
  }

  window.devTypes = { load, complete, hover, hoverAt, exprType, shape, db, parse: (text) => { const d = { types: new Map(), values: new Map() }; declare(d, text); return d; } };
})();
