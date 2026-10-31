"use strict";
// Built-in chart page (ui.chart). The script sends "init" with the chart
// (a ChartSpec whose dates are { $date: ms }) and "update" to change it;
// clicks and edits of a Gantt chart go back as "click" and "change".
let view = null;
let opts = {};

/** Dates for the script: { $date: ms }. */
const wireDates = (o) => {
  const out = {};
  for (const [k, v] of Object.entries(o)) out[k] = v instanceof Date ? { $date: v.getTime() } : v;
  return out;
};

/** Draws spec; returns the error message ("" if it was drawn). */
function show(spec) {
  const box = document.getElementById("chart");
  const err = document.getElementById("error");
  try {
    const s = XljsChart.revive(spec);
    if (view) view.update(s);
    else view = XljsChart.create(box, s, opts);
    err.hidden = true;
    return "";
  } catch (e) {
    err.textContent = e.message;
    err.hidden = false;
    return e.message;
  }
}

host.on((m) => {
  switch (m.type) {
    case "init":
      opts = {
        lang: m.lang || "ja",
        texts: m.texts || {},
        onClick: m.click ? (index, info) => host.send({ type: "click", index, series: info.series ?? null }) : null,
        onChange: m.change ? (index, changes) => host.send({ type: "change", index, changes: wireDates(changes) }) : null,
      };
      {
        const error = show(m.spec);
        host.send(error ? { type: "shown", error } : { type: "shown" });
      }
      break;
    case "update":
      show(m.spec);
      break;
  }
});

host.send({ type: "ready" });
