import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { runCli, tmpDir } from "../../../common/harness.mjs";

export const PEOPLE = "name,age,city,amount\nAlice,30,Paris,10.5\nBob,25,London,20\nCarol,35,Paris,5\nDave,25,Rome,\n";

export function run(args, input) {
  return runCli(args, { input: input ?? "" });
}
export function file(content, name = "data.csv") {
  const dir = tmpDir("csvtool-");
  const p = join(dir, name);
  writeFileSync(p, content);
  return p;
}
export function ok(r) {
  if (r.code !== 0) throw new Error(`expected exit 0, got ${r.code}; stderr: ${r.stderr}`);
  return r;
}
export function lines(text) {
  return text.split("\n");
}
