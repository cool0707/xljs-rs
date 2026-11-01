// The development mode's editor: CodeMirror 6 (vendor/codemirror, the global
// `CM`) with the highlighting of JS / TS / JSON / HTML / CSS by the file's
// extension, and the completion and hover texts of dev-types.js.
//
//   const ed = devEditor.create(parent, { onChange, onFormat });
//   ed.set(file, text)  ed.value()  ed.goto(line, col)  ed.mark(line)  ed.focus()
//   await ed.format()   (Prettier: vendor/prettier, loaded on the first use)
//   ed.phrases(L)       (the texts of CodeMirror's own UI: search panel...)
(function () {
  "use strict";

  const style = CM.HighlightStyle.define([
    { tag: [CM.tags.keyword, CM.tags.controlKeyword, CM.tags.definitionKeyword, CM.tags.moduleKeyword, CM.tags.operatorKeyword, CM.tags.self], color: "var(--hl-keyword)" },
    { tag: [CM.tags.string, CM.tags.special(CM.tags.string), CM.tags.attributeValue], color: "var(--hl-string)" },
    { tag: [CM.tags.number, CM.tags.bool, CM.tags.null, CM.tags.atom], color: "var(--hl-number)" },
    { tag: [CM.tags.comment, CM.tags.lineComment, CM.tags.blockComment, CM.tags.docComment], color: "var(--hl-comment)", fontStyle: "italic" },
    { tag: [CM.tags.function(CM.tags.variableName), CM.tags.function(CM.tags.propertyName), CM.tags.function(CM.tags.definition(CM.tags.variableName))], color: "var(--hl-function)" },
    { tag: [CM.tags.definition(CM.tags.variableName), CM.tags.definition(CM.tags.propertyName)], color: "var(--hl-definition)" },
    { tag: [CM.tags.propertyName, CM.tags.attributeName], color: "var(--hl-property)" },
    { tag: [CM.tags.typeName, CM.tags.className, CM.tags.namespace], color: "var(--hl-type)" },
    { tag: [CM.tags.tagName, CM.tags.angleBracket], color: "var(--hl-tag)" },
    { tag: [CM.tags.regexp, CM.tags.escape], color: "var(--hl-regexp)" },
    { tag: CM.tags.invalid, color: "var(--error)" },
    { tag: CM.tags.heading, fontWeight: "bold" },
  ]);

  const theme = CM.EditorView.theme({
    "&": { height: "100%", backgroundColor: "var(--input)", color: "var(--fg)" },
    "&.cm-focused": { outline: "none" },
    ".cm-scroller": { fontFamily: "var(--mono)", fontSize: "13px", lineHeight: "20px" },
    ".cm-content": { caretColor: "var(--fg)", padding: "8px 0" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--fg)" },
    "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": { backgroundColor: "var(--editor-sel)" },
    ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--muted)", borderRight: "1px solid var(--border)" },
    ".cm-lineNumbers .cm-gutterElement": { minWidth: "36px", padding: "0 8px 0 12px" },
    ".cm-activeLine": { backgroundColor: "var(--editor-line)" },
    ".cm-activeLineGutter": { backgroundColor: "var(--editor-line)", color: "var(--fg)" },
    ".cm-markLine": { backgroundColor: "var(--mark)" },
    ".cm-gutterElement.cm-markLine": { color: "var(--fg)", fontWeight: "600" },
    ".cm-matchingBracket": { backgroundColor: "var(--editor-bracket)", outline: "1px solid var(--border)" },
    ".cm-selectionMatch": { backgroundColor: "var(--editor-match)" },
    ".cm-searchMatch": { backgroundColor: "var(--editor-match)", outline: "1px solid var(--accent)" },
    ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--sel)" },
    ".cm-foldPlaceholder": { backgroundColor: "var(--hover)", border: "1px solid var(--border)", color: "var(--muted)" },
    ".cm-tooltip": { backgroundColor: "var(--panel)", color: "var(--fg)", border: "1px solid var(--border)", borderRadius: "4px", boxShadow: "0 4px 12px rgba(0,0,0,.18)" },
    ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--mono)", fontSize: "12.5px", maxHeight: "18em" },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--sel)", color: "var(--sel-fg)" },
    ".cm-completionDetail": { color: "var(--muted)", fontStyle: "normal", marginLeft: "1em" },
    ".cm-completionMatchedText": { textDecoration: "none", fontWeight: "700" },
    ".cm-completionInfo": { maxWidth: "460px", padding: "6px 8px" },
    ".cm-panels": { backgroundColor: "var(--bg)", color: "var(--fg)" },
    ".cm-panels.cm-panels-bottom": { borderTop: "1px solid var(--border)" },
    ".cm-panel.cm-search": { fontFamily: "var(--font)", fontSize: "12px" },
    ".cm-panel.cm-search input, .cm-panel.cm-search button": { fontSize: "12px" },
    ".cm-textfield": { backgroundColor: "var(--input)", color: "var(--fg)", border: "1px solid var(--border)" },
    ".cm-button": { backgroundImage: "none", backgroundColor: "var(--panel)", color: "var(--fg)", border: "1px solid var(--border)" },
  });

  // The marked line (an error's place): a line decoration and the gutter's number.
  const setMark = CM.StateEffect.define();
  const markField = CM.StateField.define({
    create: () => 0,
    update(line, tr) {
      for (const e of tr.effects) if (e.is(setMark)) return e.value;
      if (line && tr.docChanged) {
        // Follow the line while lines above are added or removed.
        const pos = tr.startState.doc.line(Math.min(line, tr.startState.doc.lines)).from;
        return tr.state.doc.lineAt(tr.changes.mapPos(pos)).number;
      }
      return line;
    },
  });
  class MarkMarker extends CM.GutterMarker { get elementClass() { return "cm-markLine"; } }
  const markMarker = new MarkMarker();
  const markGutter = CM.gutterLineClass.compute([markField], (state) => {
    const line = state.field(markField);
    const b = new CM.RangeSetBuilder();
    if (line > 0 && line <= state.doc.lines) b.add(state.doc.line(line).from, state.doc.line(line).from, markMarker);
    return b.finish();
  });

  function markDecorations() {
    return CM.EditorView.decorations.compute([markField], (state) => {
      const line = state.field(markField);
      return line > 0 && line <= state.doc.lines
        ? CM.Decoration.set([CM.Decoration.line({ class: "cm-markLine" }).range(state.doc.line(line).from)])
        : CM.Decoration.none;
    });
  }

  // ------------------------------------------------------------ formatter

  // Prettier (vendor/prettier), loaded on the first use. The parser and the
  // plugins by the file's extension (TypeScript through babel-ts).
  const FORMATS = {
    js: ["babel", ["babel", "estree"]], mjs: ["babel", ["babel", "estree"]], cjs: ["babel", ["babel", "estree"]],
    ts: ["babel-ts", ["babel", "estree"]], mts: ["babel-ts", ["babel", "estree"]], cts: ["babel-ts", ["babel", "estree"]],
    json: ["json", ["babel", "estree"]],
    html: ["html", ["html", "babel", "estree", "postcss"]], htm: ["html", ["html", "babel", "estree", "postcss"]],
    css: ["css", ["postcss"]],
  };
  const loaded = {};
  function loadScript(src) {
    if (!loaded[src]) {
      loaded[src] = new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = src;
        s.onload = resolve;
        s.onerror = () => { delete loaded[src]; reject(new Error("cannot load " + src)); };
        document.head.append(s);
      });
    }
    return loaded[src];
  }

  function formatOf(file) {
    const ext = ((String(file || "").match(/\.([^./\\]+)$/) || [])[1] || "").toLowerCase();
    return FORMATS[ext] || null;
  }

  /** Formats text: {text, cursor}. Throws Prettier's error (e.loc.start.line) on a syntax error. */
  async function formatText(file, text, cursor) {
    const f = formatOf(file);
    if (!f) return null;
    await loadScript("vendor/prettier/standalone.js");
    await Promise.all(f[1].map((p) => loadScript(`vendor/prettier/plugins/${p}.js`)));
    const plugins = f[1].map((p) => window.prettierPlugins[p]);
    const r = await window.prettier.formatWithCursor(text, {
      parser: f[0], plugins, cursorOffset: cursor,
      printWidth: 100, tabWidth: 2, endOfLine: "auto",
    });
    return { text: r.formatted, cursor: r.cursorOffset };
  }

  function language(file) {
    const ext = (String(file || "").match(/\.([^./\\]+)$/) || [])[1];
    switch ((ext || "").toLowerCase()) {
      case "ts": case "mts": case "cts": {
        const l = CM.javascript({ typescript: true });
        return [l, CM.typescriptLanguage.data.of({ autocomplete: devTypes.complete }), CM.hoverTooltip(devTypes.hover)];
      }
      case "js": case "mjs": case "cjs": {
        const l = CM.javascript();
        return [l, CM.javascriptLanguage.data.of({ autocomplete: devTypes.complete }), CM.hoverTooltip(devTypes.hover)];
      }
      case "json": return [CM.json()];
      case "html": case "htm": return [CM.html()];
      case "css": return [CM.css()];
      default: return [];
    }
  }

  // CodeMirror's English phrases → the keys of the page's texts (devPage.* in the locales).
  const PHRASES = {
    "Find": "cmFind", "Replace": "cmReplace", "next": "cmNext", "previous": "cmPrevious", "all": "cmAll",
    "match case": "cmMatchCase", "regexp": "cmRegexp", "by word": "cmByWord", "replace": "cmReplaceOne",
    "replace all": "cmReplaceAll", "close": "cmClose", "Go to line": "cmGotoLine", "go": "cmGo",
    "current match": "cmCurrentMatch", "on line": "cmOnLine", "replaced match on line $": "cmReplacedOnLine",
    "replaced $ matches": "cmReplacedCount", "Folded lines": "cmFolded", "Unfolded lines": "cmUnfolded",
    "Fold line": "cmFoldLine", "Unfold line": "cmUnfoldLine", "folded code": "cmFoldedCode", "unfold": "cmUnfold",
    "Completions": "cmCompletions", "Control character": "cmControlChar", "Selection deleted": "cmSelectionDeleted",
    "to": "cmTo",
  };

  function create(parent, opts) {
    let phrases = {};
    let silent = false;

    const extensions = (file) => [
      CM.lineNumbers(),
      CM.highlightActiveLineGutter(),
      CM.highlightSpecialChars(),
      CM.history(),
      CM.foldGutter(),
      CM.drawSelection(),
      CM.dropCursor(),
      CM.EditorState.allowMultipleSelections.of(true),
      CM.indentOnInput(),
      CM.syntaxHighlighting(style),
      CM.bracketMatching(),
      CM.closeBrackets(),
      CM.autocompletion({ activateOnTypingDelay: 80, icons: true }),
      CM.rectangularSelection(),
      CM.crosshairCursor(),
      CM.highlightActiveLine(),
      CM.highlightSelectionMatches(),
      CM.indentUnit.of("  "),
      CM.EditorState.tabSize.of(2),
      CM.EditorState.phrases.of(phrases),
      CM.keymap.of([
        ...CM.closeBracketsKeymap,
        ...CM.defaultKeymap,
        ...CM.searchKeymap,
        ...CM.historyKeymap,
        ...CM.foldKeymap,
        ...CM.completionKeymap,
        CM.indentWithTab,
        // Shift+Alt+F as in VS Code.
        { key: "Shift-Alt-f", run: () => { if (opts.onFormat) opts.onFormat(); return true; } },
      ]),
      markField,
      markDecorations(),
      markGutter,
      theme,
      language(file),
      CM.EditorView.contentAttributes.of({ spellcheck: "false", autocorrect: "off", autocapitalize: "off" }),
      CM.EditorView.updateListener.of((u) => { if (u.docChanged && !silent && opts.onChange) opts.onChange(); }),
    ];

    let file = "";
    const view = new CM.EditorView({ parent, state: CM.EditorState.create({ doc: "", extensions: extensions("") }) });

    return {
      view,
      /** Shows a file: a new state (its own undo history and language). */
      set(f, text) {
        file = f;
        silent = true;
        view.setState(CM.EditorState.create({ doc: text, extensions: extensions(f) }));
        silent = false;
      },
      value: () => view.state.doc.toString(),
      /** Whether the file's kind can be formatted. */
      canFormat: () => !!formatOf(file),
      /**
       * Formats the document (one undoable change, only the changed middle
       * replaced so the view keeps its place). false when the kind has no
       * formatter; throws on a syntax error.
       */
      async format() {
        const f = file, before = view.state.doc.toString();
        const r = await formatText(f, before, view.state.selection.main.head);
        if (!r) return false;
        // Edited or switched while formatting: leave it.
        if (f !== file || view.state.doc.toString() !== before) return true;
        if (r.text === before) return true;
        let a = 0, b = before.length, c = r.text.length;
        while (a < b && a < c && before.charCodeAt(a) === r.text.charCodeAt(a)) a++;
        while (b > a && c > a && before.charCodeAt(b - 1) === r.text.charCodeAt(c - 1)) { b--; c--; }
        const cursor = r.cursor >= 0 ? Math.min(r.cursor, r.text.length) : undefined;
        view.dispatch({
          changes: { from: a, to: b, insert: r.text.slice(a, c) },
          selection: cursor === undefined ? undefined : { anchor: cursor },
          scrollIntoView: true,
          userEvent: "format",
        });
        return true;
      },
      focus: () => view.focus(),
      /** Marks line (1-based; 0 clears). */
      mark(line) { view.dispatch({ effects: setMark.of(line || 0) }); },
      /** Puts the cursor on line, col (1-based), marks the line and shows it. */
      goto(line, col) {
        const doc = view.state.doc;
        line = Math.max(1, Math.min(line || 1, doc.lines));
        const l = doc.line(line);
        const pos = l.from + Math.max(0, Math.min((col || 1) - 1, l.length));
        view.dispatch({
          selection: { anchor: pos },
          effects: [setMark.of(line), CM.EditorView.scrollIntoView(pos, { y: "center" })],
        });
        view.focus();
      },
      /** Takes the texts of CodeMirror's UI (search panel...) from the page's texts. */
      phrases(texts) {
        phrases = {};
        for (const [en, key] of Object.entries(PHRASES)) if (texts && texts[key]) phrases[en] = texts[key];
        const text = view.state.doc.toString();
        const sel = view.state.selection;
        silent = true;
        view.setState(CM.EditorState.create({ doc: text, selection: sel, extensions: extensions(file) }));
        silent = false;
      },
    };
  }

  window.devEditor = { create };
})();
