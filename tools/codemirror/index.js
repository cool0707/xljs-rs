// The CodeMirror 6 modules the development mode's editor (src/ui/web/dev-editor.js)
// uses, bundled into src/ui/web/vendor/codemirror/codemirror.js as the global `CM`.
// Build: see README.md in this folder.
export { EditorState, StateField, StateEffect, Compartment, RangeSetBuilder } from "@codemirror/state";
export {
  EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter,
  highlightSpecialChars, drawSelection, dropCursor, rectangularSelection, crosshairCursor,
  Decoration, hoverTooltip, gutterLineClass, GutterMarker,
} from "@codemirror/view";
export { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
export {
  syntaxTree, indentOnInput, bracketMatching, foldGutter, foldKeymap,
  HighlightStyle, syntaxHighlighting, indentUnit,
} from "@codemirror/language";
export { tags } from "@lezer/highlight";
export { javascript, javascriptLanguage, typescriptLanguage } from "@codemirror/lang-javascript";
export { html } from "@codemirror/lang-html";
export { css } from "@codemirror/lang-css";
export { json } from "@codemirror/lang-json";
export {
  autocompletion, completionKeymap, closeBrackets, closeBracketsKeymap,
} from "@codemirror/autocomplete";
export { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
