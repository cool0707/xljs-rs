"use strict";
// Built-in page that turns an HTML document into a PDF (book.toPdf,
// xljs.htmlToPdf). The host sends "init" with the document, the charts
// that its <div class="xljs-print-chart" data-chart="i"> stand for (a
// ChartSpec, or { error } for a chart that cannot be drawn) and the chart
// texts; the page puts the document in place, draws the charts and answers
// "rendered" (the host then prints the page), or "error".

/** Waits for ms milliseconds (timers keep running in a hidden window). */
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

/** Waits for the next frames, or at most ms if frames do not come. */
const frames = (n, ms) => Promise.race([
  new Promise((r) => { const step = () => (--n <= 0 ? r() : requestAnimationFrame(step)); requestAnimationFrame(step); }),
  wait(ms),
]);

/** Puts the document html in place of this page's body (its styles too; scripts do not run). */
function place(html) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  for (const el of doc.head.querySelectorAll("style, link[rel~=stylesheet], base")) document.head.append(document.importNode(el, true));
  if (doc.title) document.title = doc.title;
  if (doc.documentElement.lang) document.documentElement.lang = doc.documentElement.lang;
  for (const s of Array.from(document.body.children)) if (s.tagName === "SCRIPT") s.remove();
  document.body.className = doc.body.className;
  document.body.replaceChildren(...Array.from(doc.body.childNodes).map((n) => document.importNode(n, true)));
  for (const s of document.body.querySelectorAll("script")) s.remove();
}

/** The number of chart boxes with something drawn (painted canvas pixels, or an SVG). */
function drawnCharts() {
  let n = 0;
  for (const box of document.querySelectorAll(".xljs-print-chart.xljs-chart")) {
    if (box.querySelector("svg")) { n++; continue; }
    const painted = Array.from(box.querySelectorAll("canvas")).some((c) => {
      try {
        if (!c.width || !c.height) return false;
        const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
        for (let i = 3; i < d.length; i += 4 * 97) if (d[i]) return true;
      } catch (e) { /* not a 2d canvas */ }
      return false;
    });
    if (painted) n++;
  }
  return n;
}

/**
 * Numbers that do not fit their cell are shown as ### filling it, as Excel
 * prints them (cut digits or dates would read as other values).
 */
function hashNumbers() {
  const range = document.createRange();
  for (const d of document.querySelectorAll(".xljs-sheet div[data-n]")) {
    const cs = getComputedStyle(d);
    const room = d.getBoundingClientRect().width - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    const fits = () => {
      range.selectNodeContents(d);
      return range.getBoundingClientRect().width <= room + 0.01;
    };
    if (fits()) continue;
    let s = "#";
    d.textContent = s + "#";
    while (s.length < 200 && fits()) {
      s += "#";
      d.textContent = s + "#";
    }
    d.textContent = s;
  }
}

/** Draws the charts into their boxes; resolves when they are drawn. */
async function drawCharts(charts, m) {
  const boxes = Array.from(document.querySelectorAll(".xljs-print-chart[data-chart]"));
  if (!boxes.length) return;
  const ctx = { lang: m.lang || "ja", texts: m.texts || {}, dark: false, print: true };
  for (const box of boxes) {
    const c = charts[Number(box.dataset.chart)];
    if (!c || c.error) {
      box.append(Object.assign(document.createElement("div"), { className: "xljs-chart-error", textContent: c ? c.error : "" }));
      continue;
    }
    box.classList.add("xljs-chart");
    try {
      const spec = XljsChart.revive(c.spec);
      // Sharp on paper, drawn at once.
      spec.options = Object.assign({}, spec.options, { animation: { enabled: false }, overrideDevicePixelRatio: 3 });
      if (spec.type === "gantt") spec.viewSelect = false;
      XljsChart.create(box, spec, ctx);
    } catch (e) {
      box.append(Object.assign(document.createElement("div"), { className: "xljs-chart-error", textContent: e.message }));
    }
  }
  // Until every chart is drawn (or 5 seconds), then a moment for the last frame.
  const want = document.querySelectorAll(".xljs-print-chart.xljs-chart").length;
  const end = Date.now() + 5000;
  do await frames(2, 300); while (drawnCharts() < want && Date.now() < end);
  await wait(150);
}

host.on(async (m) => {
  if (m.type !== "init") return;
  try {
    place(m.html);
    if (document.fonts && document.fonts.ready) await Promise.race([document.fonts.ready, wait(3000)]);
    await Promise.all(Array.from(document.images).map((img) => (img.complete ? null : new Promise((r) => { img.onload = img.onerror = r; }))));
    hashNumbers();
    await drawCharts(m.charts || [], m);
    host.send({ type: "rendered", charts: document.querySelectorAll(".xljs-print-chart.xljs-chart").length, drawn: drawnCharts() });
  } catch (e) {
    host.send({ type: "error", message: String((e && e.message) || e) });
  }
});

host.send({ type: "ready" });
