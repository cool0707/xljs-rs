// ==xljs==
// @mode   app
// @html   ui/index.html
// @assets ui/
// @title  はじめてのアプリ
// @size   640x420
// ==/xljs==

// @mode app では、スクリプトの実行前にメインウィンドウ（ui/index.html）が開きます。
// ウィンドウを閉じるとスクリプトが終了します。
// export した関数は画面から呼べます（data-action="greet" のボタン、または xljs.invoke("greet", ...)）。

/** フォームの値（{ name }）を受け取り、戻り値は data-result="greeting" で state に入る */
export function greet({ name }) {
  return `こんにちは、${name || "名無し"}さん！`;
}

// win.state に入れた値は data-bind の要素に表示される
app.main.state.started = new Date();

await app.main.closed;
console.log("ウィンドウが閉じられました");
