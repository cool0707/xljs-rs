"use strict";
// The development mode's window (xljs dev; the other side is src/dev/mod.rs):
// an AI chat (a browser pane that Rust puts over #chat-view) beside the
// canvas: the code, the differences of a downloaded answer, the preview,
// earlier versions and the output of runs. Messages to Rust are JSON
// {op, ...}; Rust answers with {type, ...} prefixed by \x02.
(() => {
  const wv = window.chrome && window.chrome.webview;
  const $ = (id) => document.getElementById(id);
  const el = (tag, props = {}, ...children) => {
    const e = Object.assign(document.createElement(tag), props);
    for (const c of children) if (c !== null && c !== undefined) e.append(c);
    return e;
  };
  const icon = (cls) => el("i", { className: cls });
  const fmt = (s, ...args) => String(s).replace(/\{(\d+)\}/g, (m, i) => (args[i] === undefined ? m : args[i]));
  const send = (m) => { if (wv) wv.postMessage(JSON.stringify(m)); };

  let L = {}; // texts
  const st = {
    project: null, // { path, dir, entry, app, html, title, files }
    file: "", // the file in the editor
    saved: "", // its text as saved
    dirty: false,
    chat: "", // the URL of the chat pane ("" = the start)
    tab: "code",
    running: false,
    pending: null, // a downloaded answer: { name, text, target, old }
    versionView: null, // { file, n, text } compared in the diff tab
    failure: null, // the last failed run: { prompt, location }
    layout: { mode: "both", swap: false, split: 0.42 },
  };

  // ------------------------------------------------------------ layout

  function loadLayout() {
    try { Object.assign(st.layout, JSON.parse(localStorage.getItem("xljs.dev.layout") || "{}")); } catch { /* defaults */ }
  }
  function saveLayout() {
    try { localStorage.setItem("xljs.dev.layout", JSON.stringify(st.layout)); } catch { /* not kept */ }
  }

  function applyLayout() {
    const { mode, swap, split } = st.layout;
    document.body.dataset.mode = mode;
    document.body.classList.toggle("swap", !!swap);
    $("chat").style.flexBasis = (Math.min(0.8, Math.max(0.2, split)) * 100) + "%";
    for (const m of ["both", "chat", "canvas"]) $("lay-" + m).classList.toggle("on", mode === m);
    syncBrowser();
  }

  // The native browser pane follows #chat-view; it is hidden while the
  // start of the chat pane shows or the pane is hidden.
  let syncQueued = false;
  function syncBrowser() {
    if (syncQueued) return;
    syncQueued = true;
    requestAnimationFrame(() => {
      syncQueued = false;
      const view = $("chat-view");
      const show = st.chat && st.layout.mode !== "canvas" && !document.hidden;
      $("chat-start").hidden = !!st.chat;
      view.hidden = !st.chat;
      if (!show) { send({ op: "browser", path: "hide" }); return; }
      const r = view.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) { send({ op: "browser", path: "hide" }); return; }
      send({ op: "browser", path: "show", x: r.left, y: r.top, width: r.width, height: r.height, scale: window.devicePixelRatio || 1 });
    });
  }

  function initSplitter() {
    const sp = $("splitter");
    sp.addEventListener("pointerdown", (e) => {
      sp.setPointerCapture(e.pointerId);
      document.body.classList.add("dragging");
      const box = $("panes").getBoundingClientRect();
      const move = (ev) => {
        let f = (ev.clientX - box.left) / box.width;
        if (st.layout.swap) f = 1 - f;
        st.layout.split = Math.min(0.8, Math.max(0.2, f));
        applyLayout();
      };
      const up = () => {
        sp.removeEventListener("pointermove", move);
        sp.removeEventListener("pointerup", up);
        document.body.classList.remove("dragging");
        saveLayout();
      };
      sp.addEventListener("pointermove", move);
      sp.addEventListener("pointerup", up);
    });
  }

  // ------------------------------------------------------------ status

  function status(text, cls) {
    $("status").textContent = text || "";
    $("status").className = "status " + (cls || "");
  }

  function setTab(tab) {
    st.tab = tab;
    for (const b of document.querySelectorAll("#tabs [data-tab]")) b.classList.toggle("on", b.dataset.tab === tab);
    if (tab !== "code") { st.showWelcome = false; st.wizard = false; }
    const welcome = !st.wizard && (st.showWelcome || (!st.project && (tab === "code" || tab === "history" || tab === "preview")));
    $("welcome").hidden = !welcome;
    $("wizard").hidden = !st.wizard;
    for (const p of ["code", "diff", "preview", "history", "output"]) $("panel-" + p).hidden = welcome || st.wizard || p !== tab;
    if (tab === "code" && !welcome && !st.wizard) $("code").focus();
    if (tab === "preview") showPreviewHint();
  }

  function updateButtons() {
    const has = !!st.project;
    $("btn-run").disabled = !has || st.running;
    $("btn-debug").disabled = !has || st.running;
    $("btn-stop").disabled = !st.running;
    $("btn-folder").disabled = !has;
    $("btn-save").disabled = !has;
    $("script-name").textContent = has ? st.project.entry : L.noScript || "";
    $("script-dir").textContent = has ? st.project.dir : "";
    document.title = has ? `${st.project.entry} - ${L.title}` : L.title;
  }

  // ------------------------------------------------------------ editor

  const code = () => $("code");

  function renderGutter(mark) {
    const n = code().value.split("\n").length;
    const g = $("gutter");
    g.replaceChildren();
    for (let i = 1; i <= n; i++) {
      g.append(el("span", { className: i === mark ? "mark" : "" }, String(i)), "\n");
    }
    g.scrollTop = code().scrollTop;
  }

  let markLine = 0;
  function setEditor(file, text) {
    st.file = file;
    st.saved = text;
    st.dirty = false;
    code().value = text;
    markLine = 0;
    renderGutter();
    $("file-select").value = file;
    $("save-state").textContent = "";
    $("history-title").textContent = fmt(L.historyOf, file);
    send({ op: "edit", file });
    send({ op: "versions", file });
  }

  function fillFiles(files) {
    const s = $("file-select");
    s.replaceChildren(...files.map((f) => el("option", { value: f, textContent: f })));
    s.value = st.file;
  }

  let saveTimer = 0;
  function edited() {
    st.dirty = code().value !== st.saved;
    $("save-state").textContent = st.dirty ? L.unsaved : "";
    renderGutter(markLine);
    clearTimeout(saveTimer);
    if (st.dirty) saveTimer = setTimeout(save, 1500);
  }

  function save() {
    clearTimeout(saveTimer);
    if (!st.project || !st.dirty) return;
    const text = code().value;
    send({ op: "save", file: st.file, text });
    st.saved = text;
    st.dirty = false;
    $("save-state").textContent = L.saving;
  }

  /** Puts the cursor on line (1-based) of the editor and marks it. */
  function gotoLine(line, col) {
    const lines = code().value.split("\n");
    line = Math.max(1, Math.min(line, lines.length));
    let pos = 0;
    for (let i = 0; i < line - 1; i++) pos += lines[i].length + 1;
    pos += Math.max(0, Math.min((col || 1) - 1, lines[line - 1].length));
    markLine = line;
    renderGutter(line);
    setTab("code");
    const ta = code();
    ta.focus();
    ta.setSelectionRange(pos, pos);
    const lh = parseFloat(getComputedStyle(ta).lineHeight) || 18;
    ta.scrollTop = Math.max(0, (line - 1) * lh - ta.clientHeight / 3);
    $("gutter").scrollTop = ta.scrollTop;
  }

  function initEditor() {
    const ta = code();
    ta.addEventListener("input", edited);
    ta.addEventListener("scroll", () => { $("gutter").scrollTop = ta.scrollTop; });
    ta.addEventListener("keydown", (e) => {
      if (e.key === "Tab" && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        const s = ta.selectionStart, t = ta.selectionEnd;
        if (e.shiftKey) return;
        ta.setRangeText("  ", s, t, "end");
        edited();
      }
    });
    $("file-select").addEventListener("change", () => {
      save();
      send({ op: "read", file: $("file-select").value });
      st.wantEditor = $("file-select").value;
    });
    $("btn-save").onclick = save;
  }

  // ------------------------------------------------------------ diff

  function renderDiff(oldText, newText) {
    const a = devDiff.split(oldText), b = devDiff.split(newText);
    const edits = devDiff.lines(a, b);
    let add = 0, del = 0;
    for (const e of edits) { if (e.op === "+") add++; else if (e.op === "-") del++; }
    $("diff-stats").replaceChildren(el("span", { className: "add" }, "+" + add), " ", el("span", { className: "del" }, "−" + del));
    const rows = [];
    for (const h of devDiff.hunks(edits)) {
      if (h.skip) { rows.push(el("div", { className: "skip" }, fmt(L.sameLines, h.skip))); continue; }
      const text = h.op === "+" ? b[h.b] : a[h.a];
      rows.push(el("div", { className: "line " + (h.op === "+" ? "add" : h.op === "-" ? "del" : "") },
        el("span", { className: "num" }, h.a >= 0 ? String(h.a + 1) : ""),
        el("span", { className: "num" }, h.b >= 0 ? String(h.b + 1) : ""),
        el("span", { className: "sign" }, h.op === "=" ? " " : h.op === "-" ? "−" : "+"),
        el("span", { className: "text" }, text)));
    }
    if (!add && !del) rows.push(el("div", { className: "skip" }, L.noChanges));
    $("diff").replaceChildren(...rows);
    $("diff").scrollTop = 0;
  }

  function showDiff() {
    const p = st.pending, v = st.versionView;
    $("diff-empty").hidden = !!(p || v);
    $("diff-badge").hidden = !p;
    $("diff-apply").hidden = !(p || v);
    $("diff-discard").hidden = !(p || v);
    $("diff-target").hidden = !p;
    $("diff-rename-label").hidden = true;
    if (!p && !v) {
      $("diff").replaceChildren();
      $("diff-title").textContent = "";
      $("diff-stats").replaceChildren();
      return;
    }
    if (p) {
      $("diff-title").textContent = fmt(L.diffFrom, p.name);
      const t = $("diff-target");
      t.replaceChildren(el("option", { value: "", textContent: L.chooseTarget }),
        ...p.files.map((f) => el("option", { value: f, textContent: "→ " + f })),
        el("option", { value: "\u0000new", textContent: fmt(L.asNewFile, p.name) }));
      t.value = p.target === null ? "" : p.target;
      $("diff-apply-label").textContent = L.apply;
      $("diff-apply").disabled = !p.target;
      const canRename = !!(p.rename && st.project && p.target === st.project.entry);
      $("diff-rename-label").hidden = !canRename;
      if (canRename) $("diff-rename-text").textContent = fmt(L.renameTo, p.rename);
      if (p.target) renderDiff(p.old || "", p.text);
      else $("diff").replaceChildren(el("div", { className: "skip" }, L.chooseTargetHint));
    } else {
      $("diff-title").textContent = fmt(L.diffVersion, v.file, v.n);
      $("diff-apply-label").textContent = fmt(L.restoreThis, v.n);
      $("diff-apply").disabled = false;
      // From the current text to the version: what restoring would change.
      renderDiff(v.current, v.text);
    }
  }

  function initDiff() {
    $("diff-target").addEventListener("change", () => {
      const p = st.pending;
      if (!p) return;
      const v = $("diff-target").value;
      if (v === "\u0000new") {
        p.target = p.name.replace(/[\\/:*?"<>|]/g, "_");
        p.old = "";
        showDiff();
        return;
      }
      p.target = v || null;
      p.old = null;
      if (v) send({ op: "read", file: v });
      showDiff();
    });
    $("diff-apply").onclick = () => {
      if (st.pending && st.pending.target) {
        if (st.pending.target === st.file) save();
        const rename = st.pending.rename && st.pending.target === st.project.entry && $("diff-rename").checked ? st.pending.rename : "";
        send({ op: "apply", file: st.pending.target, text: st.pending.text, note: st.pending.name, rename });
        st.pending = null;
      } else if (st.versionView) {
        const v = st.versionView;
        if (v.file === st.file) save();
        send({ op: "restore", file: v.file, n: v.n });
        st.versionView = null;
      }
      showDiff();
    };
    $("diff-discard").onclick = () => {
      st.pending = null;
      st.versionView = null;
      showDiff();
      status(L.discarded);
    };
  }

  // ------------------------------------------------------------ history

  function renderHistory(file, list) {
    if (file !== st.file) return;
    const ul = $("history");
    if (!list.length) { ul.replaceChildren(el("li", { className: "muted" }, L.noVersions)); return; }
    ul.replaceChildren(...list.slice().reverse().map((v) => el("li", {},
      el("span", { className: "n" }, "#" + v.n),
      el("span", { className: "time" }, v.time),
      el("span", { className: "note" }, v.note),
      el("button", { type: "button", className: "small", onclick: () => send({ op: "version", file, n: v.n }) }, L.compare),
      el("button", { type: "button", className: "small", onclick: () => {
        if (confirm(fmt(L.confirmRestore, file, v.n))) {
          if (file === st.file) save();
          send({ op: "restore", file, n: v.n });
        }
      } }, L.restore))));
  }

  // ------------------------------------------------------------ preview

  function showPreviewHint() {
    const p = st.project;
    if (!p) return;
    if (p.app) {
      $("preview-title").textContent = fmt(L.previewApp, p.html);
      $("preview-hint").textContent = L.previewAppHint;
      $("preview-refresh-label").textContent = L.refresh;
      if ($("preview-frame").hidden) loadAppPreview();
    } else {
      $("preview-title").textContent = L.previewForm;
      $("preview-hint").textContent = L.previewFormHint;
      $("preview-refresh-label").textContent = L.previewRun;
    }
  }

  function loadAppPreview() {
    const p = st.project;
    if (!p || !p.app || !p.html) return;
    const f = $("preview-frame");
    f.hidden = false;
    $("preview-error").hidden = true;
    f.src = "https://app.xljs/_project/" + p.html.split("/").map(encodeURIComponent).join("/") + "?t=" + Date.now();
  }

  let previewForm = null;
  function showFormPreview(form) {
    previewForm = form;
    const f = $("preview-frame");
    $("preview-error").hidden = true;
    f.hidden = false;
    f.src = "form.html?preview=" + Date.now();
  }

  // Messages of the form in the preview frame (bridge.js): "\x01" + JSON.
  window.addEventListener("message", (e) => {
    const f = $("preview-frame");
    if (e.source !== f.contentWindow || typeof e.data !== "string" || e.data[0] !== "\x01") return;
    let m;
    try { m = JSON.parse(e.data.slice(1)); } catch { return; }
    const reply = (x) => f.contentWindow.postMessage("\x02" + JSON.stringify(x), "*");
    if (m.type === "ready" && previewForm) reply(previewForm);
    else if (m.type === "submit" || m.type === "button") reply({ type: "errors", errors: {}, error: L.previewNoRun });
  });

  function initPreview() {
    $("preview-refresh").onclick = () => {
      const p = st.project;
      if (!p) return;
      save();
      if (p.app) { loadAppPreview(); return; }
      send({ op: "preview" });
      status(L.previewRunning);
    };
  }

  // ------------------------------------------------------------ runs

  function appendOutput(text, cls) {
    const out = $("output");
    const atEnd = out.scrollTop + out.clientHeight >= out.scrollHeight - 4;
    out.append(el("span", { className: cls || "" }, text));
    // Keep the output from growing without bound.
    while (out.childNodes.length > 5000) out.firstChild.remove();
    if (atEnd) out.scrollTop = out.scrollHeight;
  }

  function startRun(debug) {
    if (!st.project || st.running) return;
    save();
    $("output").replaceChildren();
    $("failure").hidden = true;
    $("output-badge").hidden = true;
    st.failure = null;
    send({ op: "run", debug });
  }

  function runStarted(m) {
    st.running = true;
    updateButtons();
    if (m.kind !== "preview") setTab("output");
    $("run-state").textContent = m.kind === "debug" ? L.debugging : m.kind === "preview" ? L.previewRunning : L.running;
    status($("run-state").textContent);
  }

  function runEnded(m) {
    st.running = false;
    updateButtons();
    if (m.kind === "preview") { status(""); return; }
    const ok = m.code === 0;
    const text = ok ? L.finished : m.stopped ? L.stoppedLoop : m.code === null ? L.killed : fmt(L.failedCode, m.code);
    $("run-state").textContent = text;
    status(text, ok ? "ok" : "error");
    if (ok || m.code === null) return;
    st.failure = { prompt: m.prompt || "", location: m.location || null };
    $("failure-title").textContent = m.stopped ? L.stoppedTitle : L.errorTitle;
    $("failure-message").textContent = m.message || "";
    const at = $("failure-at");
    if (m.location) {
      at.hidden = false;
      at.textContent = fmt(L.at, m.location.file, m.location.line);
    } else at.hidden = true;
    $("failure").hidden = false;
    $("output-badge").hidden = false;
    setTab("output");
  }

  function failurePrompt() {
    const f = st.failure;
    if (!f) return "";
    let text = f.prompt;
    if ($("failure-full").checked && st.project) {
      const file = f.location ? f.location.file : st.project.entry;
      const src = file === st.file ? code().value : null;
      if (src !== null) text += "\n\n" + fmt(L.fullCode, file) + "\n```\n" + src.replace(/\s+$/, "") + "\n```";
    }
    return text;
  }

  async function copy(text, done) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Without the clipboard API: through a selected textarea.
      const t = el("textarea", { value: text });
      document.body.append(t);
      t.select();
      document.execCommand("copy");
      t.remove();
    }
    status(done, "ok");
  }

  function initRun() {
    $("btn-run").onclick = () => startRun(false);
    $("btn-debug").onclick = () => startRun(true);
    $("btn-stop").onclick = () => send({ op: "stop" });
    $("output-clear").onclick = () => $("output").replaceChildren();
    $("failure-copy").onclick = () => copy(failurePrompt(), L.copiedPrompt);
    $("failure-at").onclick = () => {
      const loc = st.failure && st.failure.location;
      if (!loc) return;
      if (loc.file === st.file) { gotoLine(loc.line, loc.column); return; }
      save();
      st.wantEditor = loc.file;
      st.wantLine = loc;
      send({ op: "read", file: loc.file });
    };
  }

  // ------------------------------------------------------------ chat

  function openChat(url) {
    url = String(url || "").trim();
    if (!url) return;
    if (!/^https?:\/\//i.test(url)) url = "https://" + url;
    st.chat = url;
    $("chat-url").value = url;
    send({ op: "chat", url });
    syncBrowser();
  }

  function initChat(services) {
    $("services").replaceChildren(...services.map((s) => el("button", { type: "button", className: "big", onclick: () => openChat(s.url) },
      icon("fa-regular fa-comments"), el("span", {}, s.name))));
    $("chat-url-form").onsubmit = (e) => { e.preventDefault(); openChat($("chat-url-start").value); };
    $("chat-url").addEventListener("keydown", (e) => { if (e.key === "Enter") openChat($("chat-url").value); });
    $("chat-home").onclick = () => { st.chat = ""; send({ op: "chat", url: "" }); syncBrowser(); };
    $("chat-back").onclick = () => send({ op: "browser", path: "back" });
    $("chat-forward").onclick = () => send({ op: "browser", path: "forward" });
    $("chat-reload").onclick = () => send({ op: "browser", path: "reload" });
    $("chat-intro").onclick = $("chat-intro2").onclick = () => send({ op: "intro" });
  }

  // ------------------------------------------------------------ project

  function openProject(m) {
    st.showWelcome = false;
    st.wizard = false;
    st.project = { path: m.path, dir: m.dir, entry: m.entry, app: m.app, html: m.html, title: m.title, files: m.files };
    st.pending = null;
    st.versionView = null;
    st.failure = null;
    previewForm = null;
    $("preview-frame").hidden = true;
    $("preview-frame").removeAttribute("src");
    $("failure").hidden = true;
    $("output").replaceChildren();
    fillFiles(m.files);
    setEditor(m.entry, m.text);
    st.chat = m.chat || "";
    $("chat-url").value = st.chat;
    updateButtons();
    showDiff();
    setTab("code");
    syncBrowser();
    status(fmt(L.opened, m.path));
  }

  function renderRecent(list) {
    $("recent-title").hidden = !list.length;
    $("recent").replaceChildren(...list.map((r) => el("li", {},
      el("button", { type: "button", className: "link", title: r.path, onclick: () => send({ op: "open", path: r.path }) }, r.name),
      el("span", { className: "muted" }, " " + r.path))));
  }

  // ------------------------------------------------------------ wizard

  const wz = { books: [] };
  const radio = (name) => { const r = document.querySelector(`input[name="${name}"]:checked`); return r ? r.value : ""; };
  const setRadio = (name, v) => { const r = document.querySelector(`input[name="${name}"][value="${v}"]`); if (r) r.checked = true; };

  function openWizard(kind) {
    save();
    setRadio("wz-kind", kind);
    if (!$("wz-folder").value) $("wz-folder").value = st.home || "";
    $("wz-error").hidden = true;
    st.wizard = true;
    st.showWelcome = false;
    updateWizard();
    setTab("code");
    $("wizard").scrollTop = 0;
  }

  function updateWizard() {
    const w = $("wizard"), port = radio("wz-kind") === "port";
    w.dataset.kind = port ? "port" : "new";
    w.dataset.mode = radio("wz-mode");
    $("wz-overview-title").textContent = port ? L.wzOverviewPort : L.wzOverviewNew;
    $("wz-overview").placeholder = port ? L.wzOverviewPortHint : L.wzOverviewNewHint;
    $("wz-input-note").disabled = radio("wz-input") === "no";
    $("wz-output-note").disabled = radio("wz-output") === "no";
    $("wz-name").disabled = radio("wz-name-mode") !== "name";
    $("wz-rows").disabled = !$("wz-sheets").checked;
  }

  function answers() {
    const port = radio("wz-kind") === "port";
    return {
      kind: port ? "port" : "new",
      books: port ? wz.books.filter((b) => !b.error).map((b) => b.path) : [],
      includeVba: $("wz-vba").checked,
      includeSheets: $("wz-sheets").checked,
      sampleRows: Number($("wz-rows").value) || 0,
      input: radio("wz-input"), inputNote: $("wz-input-note").value,
      output: radio("wz-output"), outputNote: $("wz-output-note").value,
      form: radio("wz-form"), mode: radio("wz-mode"),
      nameMode: radio("wz-name-mode"), name: $("wz-name").value,
      overview: $("wz-overview").value,
      intro: $("wz-intro").checked,
    };
  }

  function wizardError(text) {
    $("wz-error").textContent = text || "";
    $("wz-error").hidden = !text;
    return !text;
  }

  function checkAnswers(a) {
    if (a.kind === "port" && !a.books.length) return wizardError(L.wzNeedBooks);
    if (a.kind === "new" && !a.overview.trim()) return wizardError(L.wzNeedOverview);
    if (a.nameMode === "name" && !a.name.trim()) return wizardError(L.wzNeedName);
    return wizardError("");
  }

  function wizardBooks(m) {
    wz.books = m.books || [];
    $("wz-book-list").replaceChildren(...wz.books.map((b) => el("li", { className: b.error ? "err" : "" },
      icon("fa-regular fa-file-excel"), " ", b.name, " — ",
      b.error ? b.error : b.modules ? fmt(L.wzModules, b.modules) : L.wzNoMacros)));
    const f = $("wz-folder");
    if (m.dir && (!f.value || f.value === st.home)) f.value = m.dir;
  }

  function wizardPrompt(m) {
    $("wz-result").hidden = false;
    $("wz-text").value = m.text;
    const n = m.text.length;
    $("wz-count").textContent = fmt(L.wzCount, n, m.file) + (n > 15000 ? " " + L.wzLong : "");
    $("wz-result").scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function initWizard() {
    $("wizard").addEventListener("change", updateWizard);
    $("wz-name").addEventListener("focus", () => setRadio("wz-name-mode", "name"));
    $("wz-name").parentElement.addEventListener("click", (e) => {
      if (e.target === $("wz-name") && $("wz-name").disabled) { setRadio("wz-name-mode", "name"); updateWizard(); $("wz-name").focus(); }
    });
    $("wz-books").onclick = () => send({ op: "wizardBooks" });
    $("wz-folder-pick").onclick = () => send({ op: "wizardFolder", path: $("wz-folder").value });
    $("wz-save-text").onclick = () => {
      const a = answers();
      if (!a.books.length) { wizardError(L.wzNeedBooks); return; }
      send({ op: "wizardSave", answers: a });
    };
    $("wz-make").onclick = () => {
      const a = answers();
      if (checkAnswers(a)) send({ op: "wizardPrompt", answers: a });
    };
    $("wz-create").onclick = () => {
      const a = answers();
      if (!checkAnswers(a)) return;
      if (!$("wz-folder").value.trim()) { wizardError(L.wzNeedFolder); return; }
      send({ op: "wizardCreate", answers: a, path: $("wz-folder").value.trim(), text: $("wz-text").value });
    };
    $("wz-copy").onclick = () => copy($("wz-text").value, L.wzCopiedOnly);
    $("wz-cancel").onclick = () => { st.wizard = false; setTab("code"); };
    $("w-wizard").onclick = () => openWizard("new");
    $("w-port").onclick = () => openWizard("port");
  }

  // ------------------------------------------------------------ messages

  function handle(m) {
    switch (m.type) {
      case "init":
        L = m.texts || {};
        for (const e of document.querySelectorAll("[data-t]")) e.textContent = L[e.dataset.t] || e.dataset.t;
        for (const e of document.querySelectorAll("[data-tip]")) e.title = L[e.dataset.tip] || "";
        $("chat-url").placeholder = L.urlPlaceholder;
        $("chat-url-start").placeholder = L.urlPlaceholder;
        document.documentElement.lang = m.lang || "ja";
        initChat(m.services || []);
        renderRecent(m.recent || []);
        st.home = m.home || "";
        $("wz-rows").options[0].textContent = L.wzRowsNone;
        $("wz-rows").value = "3";
        $("wz-input-note").placeholder = L.wzInputHint;
        $("wz-output-note").placeholder = L.wzOutputHint;
        $("wz-name").placeholder = L.wzNameHint;
        updateButtons();
        showDiff();
        setTab("code");
        applyLayout();
        break;
      case "project": openProject(m); break;
      case "meta":
        if (st.project) Object.assign(st.project, { app: m.app, html: m.html, title: m.title });
        break;
      case "files":
        if (st.project) { st.project.files = m.files; fillFiles(m.files); }
        break;
      case "file":
        if (st.reloadCheck && m.file === st.file) {
          st.reloadCheck = false;
          if (!st.dirty && m.text !== st.saved) { setEditor(m.file, m.text); status(fmt(L.reloaded, m.file)); }
          break;
        }
        if (st.versionView && st.wantVersionCurrent && st.versionView.file === m.file) {
          st.wantVersionCurrent = false;
          st.versionView.current = m.text;
          showDiff();
        }
        if (st.pending && st.pending.target === m.file && st.pending.old === null) {
          st.pending.old = m.text;
          showDiff();
        }
        if (st.wantEditor === m.file) {
          st.wantEditor = null;
          setEditor(m.file, m.text);
          if (st.wantLine && st.wantLine.file === m.file) { gotoLine(st.wantLine.line, st.wantLine.column); st.wantLine = null; }
        }
        break;
      case "saved":
        if (m.file === st.file) $("save-state").textContent = st.dirty ? L.unsaved : L.saved;
        if (st.project && st.project.app) loadAppPreviewIfShown();
        break;
      case "applied":
        if (m.file === st.file) {
          setEditor(m.file, m.text);
        }
        status(fmt(L.appliedMsg, m.file), "ok");
        if (st.project && st.project.app) loadAppPreviewIfShown();
        setTab("code");
        break;
      case "versions": renderHistory(m.file, m.versions || []); break;
      case "version":
        st.versionView = { file: m.file, n: m.n, text: m.text, current: m.file === st.file ? code().value : "" };
        if (m.file !== st.file) { st.wantVersionCurrent = true; send({ op: "read", file: m.file }); }
        showDiff();
        setTab("diff");
        break;
      case "download":
        st.pending = { name: m.name, text: m.text, target: m.target, old: m.old === undefined ? null : m.old, files: m.files || [], rename: m.rename || "" };
        if (st.pending.target && st.pending.old === null) st.pending.old = "";
        st.versionView = null;
        showDiff();
        setTab("diff");
        status(fmt(L.downloaded, m.name));
        break;
      case "url":
        if (m.url && !m.url.startsWith("about:")) {
          $("chat-url").value = m.url;
          if (!st.chat) { st.chat = m.url; syncBrowser(); }
        }
        break;
      case "copy":
        copy(m.text, m.what === "intro" ? L.copiedIntro : m.what === "wizard" ? L.wzCopied : L.copied);
        break;
      case "runStarted": runStarted(m); break;
      case "output": appendOutput(m.text, m.stream === "stderr" ? "err" : ""); break;
      case "runEnded": runEnded(m); break;
      case "previewForm": showFormPreview(m.form); setTab("preview"); status(""); break;
      case "previewFailed":
        $("preview-frame").hidden = true;
        $("preview-error").hidden = false;
        $("preview-error").textContent = m.message || "";
        setTab("preview");
        break;
      case "browserFailed":
        $("chat-failed").hidden = false;
        $("chat-failed").textContent = fmt(L.browserFailed, m.message);
        break;
      case "error":
        status(m.message, "error");
        break;
      case "info":
        status(m.message, "ok");
        break;
      case "wizardBooks": wizardBooks(m); break;
      case "wizardFolder": $("wz-folder").value = m.path; break;
      case "wizardPrompt": wizardPrompt(m); break;
    }
  }

  function loadAppPreviewIfShown() {
    if (st.tab === "preview" && !$("preview-frame").hidden) loadAppPreview();
  }

  // ------------------------------------------------------------ start

  loadLayout();
  initSplitter();
  initEditor();
  initDiff();
  initPreview();
  initRun();
  initWizard();
  for (const b of document.querySelectorAll("#tabs [data-tab]")) b.onclick = () => { st.wizard = false; st.showWelcome = false; setTab(b.dataset.tab); };
  $("btn-open").onclick = $("w-open").onclick = () => { save(); send({ op: "open" }); };
  $("btn-new").onclick = () => { save(); st.showWelcome = true; setTab("code"); };
  $("w-new").onclick = () => send({ op: "new", app: false });
  $("w-new-app").onclick = () => send({ op: "new", app: true });
  $("btn-folder").onclick = () => send({ op: "showFolder" });
  $("lay-both").onclick = () => { st.layout.mode = "both"; saveLayout(); applyLayout(); };
  $("lay-chat").onclick = () => { st.layout.mode = "chat"; saveLayout(); applyLayout(); };
  $("lay-canvas").onclick = () => { st.layout.mode = "canvas"; saveLayout(); applyLayout(); };
  $("lay-swap").onclick = () => { st.layout.swap = !st.layout.swap; saveLayout(); applyLayout(); };
  new ResizeObserver(syncBrowser).observe($("chat-view"));
  window.addEventListener("resize", syncBrowser);
  document.addEventListener("visibilitychange", syncBrowser);
  document.addEventListener("keydown", (e) => {
    if (e.key === "F5") { e.preventDefault(); startRun(e.shiftKey); }
    else if (e.ctrlKey && (e.key === "s" || e.key === "S")) { e.preventDefault(); save(); }
    else if (e.ctrlKey && (e.key === "r" || e.key === "R" || e.key === "p" || e.key === "P")) e.preventDefault();
  });
  window.addEventListener("focus", () => {
    // Edited elsewhere (Notepad...) while nothing is unsaved here: reload.
    if (st.project && !st.dirty && st.file && !st.wantEditor) { st.reloadCheck = true; send({ op: "read", file: st.file }); }
  });
  document.addEventListener("dragover", (e) => e.preventDefault());
  document.addEventListener("drop", (e) => e.preventDefault());
  if (wv) {
    wv.addEventListener("message", (e) => {
      if (typeof e.data !== "string" || e.data[0] !== "\x02") return;
      let m;
      try { m = JSON.parse(e.data.slice(1)); } catch { return; }
      handle(m);
    });
  }
  send({ op: "ready" });
})();
