import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { pkg, appPath, exists, importApp } from "../../../common/harness.mjs";

const sched = await importApp("src/lib/scheduler.js");
const { checkAnswer, newCardState } = sched;

const html = () => readFileSync(appPath("dist", "index.html"), "utf8");

function walk(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

test("package.json is an ES module project with vite and dev/build scripts", () => {
  const p = pkg();
  assert.equal(p.type, "module");
  assert.ok({ ...p.dependencies, ...p.devDependencies }.vite, "vite is a dependency");
  assert.ok(p.scripts?.dev, "dev script");
  assert.ok(p.scripts?.build, "build script");
});

test("npm run build produces dist/index.html with the right title and mount point", () => {
  assert.ok(exists("dist", "index.html"), "dist/index.html exists");
  const h = html();
  assert.match(h, /<title>\s*Flashcards\s*<\/title>/);
  assert.match(h, /<div\s+id=["']app["']/);
});

test("asset paths in dist/index.html are relative (none starts with /)", () => {
  const h = html();
  assert.doesNotMatch(h, /\b(?:src|href)=["']\/(?!\/)/);
});

test("every local script and stylesheet the page references exists in dist", () => {
  const h = html();
  const refs = [...h.matchAll(/\b(?:src|href)=["']([^"']+)["']/g)].map((m) => m[1]).filter((u) => !/^(?:[a-z]+:)?\/\//i.test(u) && !u.startsWith("data:") && !u.startsWith("#"));
  const assets = refs.filter((u) => /\.(?:js|mjs|css)(?:\?|$)/.test(u));
  assert.ok(assets.length >= 1, "page references at least one script");
  for (const u of assets) {
    const file = join(dirname(appPath("dist", "index.html")), u.split("?")[0]);
    assert.ok(statSync(file).isFile(), `${u} exists`);
  }
});

test("the built code stores the deck under the localStorage key flashcards.v1", () => {
  const js = walk(appPath("dist"))
    .filter((f) => /\.(?:js|mjs)$/.test(f))
    .map((f) => readFileSync(f, "utf8"))
    .join("\n");
  assert.ok(js.includes("flashcards.v1"));
});

test("scheduler.js exports the documented constants", () => {
  assert.equal(sched.DAY_MS, 86400000);
  assert.equal(sched.LEARN_STEP_MS, 600000);
});

test("newCardState starts a card as new and due now", () => {
  assert.deepEqual(newCardState(1700000000000), { ease: 2.5, interval: 0, reps: 0, lapses: 0, due: 1700000000000 });
  assert.deepEqual(newCardState(0), { ease: 2.5, interval: 0, reps: 0, lapses: 0, due: 0 });
});

test("checkAnswer ignores case, surrounding whitespace and repeated inner whitespace", () => {
  assert.equal(checkAnswer("Paris", "paris"), true);
  assert.equal(checkAnswer("  PARIS \n", "Paris"), true);
  assert.equal(checkAnswer("new   york\tcity", "New York City"), true);
  assert.equal(checkAnswer("Paris", " Paris "), true);
});

test("checkAnswer treats punctuation, accents and different words as different", () => {
  assert.equal(checkAnswer("Paris.", "Paris"), false);
  assert.equal(checkAnswer("cafe", "café"), false);
  assert.equal(checkAnswer("Lyon", "Paris"), false);
  assert.equal(checkAnswer("new york", "newyork"), false);
  assert.equal(checkAnswer("", "Paris"), false);
});

test("checkAnswer is false for non-strings and for an empty expected answer", () => {
  assert.equal(checkAnswer(42, "42"), false);
  assert.equal(checkAnswer("42", 42), false);
  assert.equal(checkAnswer(undefined, undefined), false);
  assert.equal(checkAnswer("", ""), false);
  assert.equal(checkAnswer("   ", "  "), false);
  assert.equal(checkAnswer("x", ""), false);
});

test("scheduler.js can be imported in Node without browser globals", () => {
  assert.equal(typeof globalThis.window, "undefined");
  assert.equal(typeof globalThis.localStorage, "undefined");
  for (const name of ["review", "dueCards", "summarizeDeck", "shuffle", "sessionStats"]) {
    assert.equal(typeof sched[name], "function", name);
  }
});
