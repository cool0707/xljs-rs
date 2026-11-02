// Copies Prettier's standalone build and the plugins the dev mode's formatter
// uses (JS / TS via babel-ts, JSON, HTML, CSS) to src/ui/web/vendor/prettier.
const fs = require("fs");
const path = require("path");
const src = path.join(__dirname, "node_modules", "prettier");
const dst = path.join(__dirname, "..", "..", "src", "ui", "web", "vendor", "prettier");
fs.mkdirSync(path.join(dst, "plugins"), { recursive: true });
for (const f of ["standalone.js", "LICENSE", "plugins/babel.js", "plugins/estree.js", "plugins/html.js", "plugins/postcss.js"]) {
  fs.copyFileSync(path.join(src, f), path.join(dst, f));
}
