// Reference build: stands in for `vite build` so the reference installs offline.
import { mkdirSync, writeFileSync } from "node:fs";

mkdirSync("dist/assets", { recursive: true });
writeFileSync(
  "dist/index.html",
  `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Flashcards</title>
    <script type="module" crossorigin src="./assets/app.js"></script>
    <link rel="stylesheet" href="./assets/app.css" />
  </head>
  <body>
    <div id="app"></div>
  </body>
</html>
`,
);
writeFileSync("dist/assets/app.js", `const KEY = "flashcards.v1";\ndocument.getElementById("app").textContent = localStorage.getItem(KEY) ?? "Flashcards";\n`);
writeFileSync("dist/assets/app.css", "body{font-family:sans-serif}\n");
