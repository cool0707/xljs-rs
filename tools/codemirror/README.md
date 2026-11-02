# CodeMirror のバンドルと Prettier

開発モードのエディタ（`src/ui/web/dev-editor.js`）が使う CodeMirror 6 のモジュールを、
1 つのファイル（グローバルの `CM`）にまとめて `src/ui/web/vendor/codemirror/codemirror.js` に書き出し、
整形に使う Prettier の standalone 版とプラグインを `src/ui/web/vendor/prettier/` にコピーする（`copy-prettier.js`）。
`npm run build` で両方、`build:codemirror` / `build:prettier` で片方だけ。

```
cd tools/codemirror
npm ci
npm run build
```

使うものを増やすときは `index.js` に export を足して作り直す。版を変えたら `src/ui/web/vendor/VERSIONS.md` も直す。
