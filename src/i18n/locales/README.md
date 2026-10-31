# Message files

Every user-facing message of xljs lives in this folder, one JSON file per
language. The files are embedded into the executable at build time
(`go:embed`), so adding a language only needs a new file and a rebuild.

- `en.json` — English. **The reference and fallback**: every key must exist
  here; a key missing from another language falls back to English.
- `ja.json` — Japanese.

## Adding a language

1. Copy `en.json` to `<code>.json` (e.g. `fr.json`, `zh-TW.json`).
2. Set `"lang"` to the code, `"name"` to the language name, and `"match"` to
   the language tags that should select it. Tags are compared case
   insensitively after removing the encoding (`fr_FR.UTF-8` → `fr-fr`); the
   full tag is tried first, then the primary language (`fr`).
3. Translate the values of `"messages"`. Keep the keys and the `{0}`, `{1}`
   placeholders unchanged.
4. Optionally set `"excel"."font"`: the default font for workbooks created
   by scripts (`""` keeps the library default, Calibri).
5. Optionally add `"translations"`: `[regexp, replacement]` pairs (Go
   `regexp` syntax, `$1` references) that translate English messages coming
   from the JavaScript engine, the bundler or the Excel library. See
   `ja.json` for examples.
6. Build, then run `go test ./internal/i18n/` — it checks that your file has
   no unknown keys and the same placeholders as `en.json`.

The language is chosen from the Windows display language (or `LANG` etc. on
other systems), and can be forced with `--lang=<code>` or the `XLJS_LANG`
environment variable.

## Using messages in code

- Go: `i18n.T("key", args...)` for text, `i18n.Err("key", args...)` for
  errors, `i18n.Error(err)` to describe an OS/library error.
- JavaScript prelude: `T("key", args...)`.

Add new keys to `en.json` first (the tests fail if code uses an unknown key),
then to the other languages.
