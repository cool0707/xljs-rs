import { ok, equal, deepEqual, throws } from "./_assert.js";

// pdf.open / create / merge: joining PDFs, removing, inserting and rotating
// pages, adding text (testdata/pdf/make_pdf.py).
const three = path.join(xljs.scriptDir, "..", "pdf", "three.pdf");
const secret = path.join(xljs.scriptDir, "..", "pdf", "secret.pdf");

const doc = pdf.open(three);
ok(doc instanceof pdf.PdfDocument, "a PdfDocument");
equal(doc.path, three);
equal(doc.pageCount, 3);
deepEqual(doc.pages, [
  { number: 1, width: 210, height: 297, rotation: 0 },
  { number: 2, width: 297, height: 210, rotation: 0 },
  { number: 3, width: 210, height: 148, rotation: 90 },
], "sizes as shown (mm) and rotation");
ok(xljs.inspect(doc).includes("pageCount: 3"), "inspect");

// Join: another file (some pages), the same document, a blank page.
doc.append(three, { pages: "3-2" });
equal(doc.pageCount, 5);
deepEqual(doc.pages.map((p) => p.width), [210, 297, 210, 210, 297], "pages 3 and 2 at the end");
doc.insert(1, doc, { pages: [-1] });
deepEqual(doc.pages.map((p) => p.width), [297, 210, 297, 210, 210, 297], "the last page copied to the front");
doc.insertBlankPage(2, { paper: "A5", landscape: true });
deepEqual(doc.pages[1], { number: 2, width: 210, height: 148, rotation: 0 }, "a blank A5 landscape page");
doc.insertBlankPage();
deepEqual(doc.pages[7], { number: 8, width: 297, height: 210, rotation: 0 }, "a blank page as big as the last one");
doc.insertBlankPage(1, { width: 100, height: 50 });
equal(doc.pages[0].width, 100);

// Remove and rotate.
doc.removePages("1-2,9");
equal(doc.pageCount, 6);
doc.removePages([-1, 1]);
deepEqual(doc.pages.map((p) => p.width), [210, 297, 210, 210]);
doc.rotatePages("all", -90).rotatePages(4, 180);
deepEqual(doc.pages.map((p) => p.rotation), [270, 270, 0, 180]);
throws(() => doc.removePages(9), /9 ページはありません/);
throws(() => doc.removePages("a-b"), /ページの指定が正しくありません/);
throws(() => doc.removePages(0), /ページの指定/);
throws(() => doc.rotatePages(1, 45), /90 の倍数/);
throws(() => doc.insert(9, three), /9 ページはありません/);
throws(() => doc.insertBlankPage(1, { paper: "A9" }), /用紙のサイズ A9/);
throws(() => doc.append(123), /PdfDocument、パス/);

// Text: on every page (a function for page numbers) and a watermark.
let fonts = true;
try {
  doc.addText("all", (p) => `${p.number} / ${p.count}`, { bottom: 10, align: "center", size: 9 });
} catch (e) {
  if (!/フォントが見つかりません/.test(e.message)) throw e;
  fonts = false;
  console.log("no font: text skipped");
}
if (fonts) {
  doc.addText(1, "社外秘\nCONFIDENTIAL", { x: 105, y: 140, align: "center", size: 40, color: "#c00", opacity: 0.3, rotate: 30 });
  throws(() => doc.addText(1, "x", { color: "teal" }), /色の指定/);
  throws(() => doc.addText(1, "x", { align: "middle" }), /align/);
  throws(() => doc.addText(1, "x", { size: -1 }), /size/);
  throws(() => doc.addText(1, "x", { font: "存在しないフォント" }), /フォントが見つかりません: 存在しないフォント/);
  throws(() => doc.addText(1, 3), /文字列か関数/);
  // Positions: page numbers and document numbers in the corners and at the centers.
  for (const pos of ["top-left", "top-center", "top-right", "bottom-left", "bottom-center", "bottom-right", "center",
    "右下", "下中央", "bottomRight", "top", "bottom", "middle left", "CENTER"]) doc.addText(2, "No.1", { position: pos });
  doc.addText(2, "資料 1", { position: "top-right", margin: { x: 15, y: 8 } });
  doc.addText(2, "資料 2", { position: "bottom-left", margin: 5, size: 8 });
  throws(() => doc.addText(1, "x", { position: "top-bottom" }), /position の指定が正しくありません: top-bottom/);
  throws(() => doc.addText(1, "x", { position: "上下" }), /position/);
  throws(() => doc.addText(1, "x", { position: "bottom-right", margin: -1 }), /margin/);
}
const saved = doc.save("out.pdf");
equal(saved, doc);
equal(doc.path, path.resolve("out.pdf"));
const bytes = new Uint8Array(fs.readBytes("out.pdf"));
equal(String.fromCharCode(...bytes.slice(0, 5)), "%PDF-");
if (fonts) {
  let text = ""; for (const b of bytes) text += String.fromCharCode(b);
  ok(/\/FontFile[23]/.test(text) && text.includes("/ToUnicode"), "an embedded font with ToUnicode");
}
doc.close();
throws(() => doc.pageCount, /閉じられています/);

const again = pdf.open("out.pdf");
deepEqual(again.pages.map((p) => p.rotation), [270, 270, 0, 180], "saved");
again.close();

// merge: files and documents into one.
const merged = pdf.merge([three, pdf.open(three), fs.readBytes(three)], "merged.pdf");
equal(merged, path.resolve("merged.pdf"));
equal(pdf.open(merged).pageCount, 9);
const m2 = pdf.merge([three, new Blob([fs.readBytes(three)])]);
equal(m2.pageCount, 6);
equal(m2.path, null);
throws(() => m2.save(), /保存先のファイルがありません/);
ok(m2.toBlob().size > 1000, "toBlob");
ok(m2.toArrayBuffer() instanceof ArrayBuffer, "toArrayBuffer");

// A new document and a split.
const part = pdf.create().append(three, { pages: 2 });
equal(part.pageCount, 1);
equal(part.pages[0].width, 297);
throws(() => pdf.create().toArrayBuffer(), /ページがありません/);

// Passwords.
throws(() => pdf.open(secret), /パスワードで保護/);
throws(() => pdf.open(secret, { password: "x" }), /パスワードが違います/);
equal(pdf.open(secret, { password: "secret" }).pageCount, 1);
throws(() => pdf.open(fs.readBytes(path.join(xljs.scriptDir, "_assert.js"))), /PDF ファイルではありません/);
console.log("pdfedit ok");
