#!/usr/bin/env python3
"""Phase 4b.1 gate: how many recorded sessions are runnable without network.

    scripts/exec_gate.py [--mode linked|offline] [--out artifacts/exec-gate-<mode>.json]

For every recording in fixtures/recorded/, runs `tattle verify` on the final
snapshot. `linked` lets dependencies come from the project's own folder or
any project under ~/Code/cv-sandbox; `offline` allows only the npm cache
(no linking, no built native packages). Runnable = the project's own test
command ran and gave a pass/fail summary, exited 0, or there are no tests.
Results are local (artifacts/, gitignored).
"""
import argparse, collections, json, pathlib, subprocess, sys

REPO = pathlib.Path(__file__).resolve().parent.parent
POOL = pathlib.Path.home() / "Code/cv-sandbox"

ap = argparse.ArgumentParser()
ap.add_argument("--mode", choices=["linked", "offline"], default="linked")
ap.add_argument("--out")
a = ap.parse_args()
out = pathlib.Path(a.out or REPO / f"artifacts/exec-gate-{a.mode}.json")
rows = []
for rec in sorted((REPO / "fixtures/recorded").glob("*/run*")):
    if not (rec / "events.jsonl").is_file():
        continue
    pid, run = rec.parent.name, rec.name
    cmd = [str(REPO / "target/release/tattle"), "verify", str(rec), "--timeout", "120"]
    if a.mode == "linked":
        own = POOL / f"{pid}-{run}"
        if own.is_dir():
            cmd += ["--project", str(own)]
        cmd += ["--pool", str(POOL)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
    try:
        d = json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        d = {"verifiable": False, "error": (r.stdout + r.stderr)[-400:]}
    t = d.get("tests", {})
    rows.append({"session": f"{pid}/{run}", "verifiable": d.get("verifiable"), "deps": d.get("dependencies", {}),
                 "passed": t.get("passed"), "failed": t.get("failed"), "exit": t.get("exit"), "timed_out": t.get("timed_out"),
                 "result": t.get("result"), "tail": (t.get("output_tail") or d.get("error") or "")[-300:]})
    print(rows[-1]["session"], rows[-1]["verifiable"], rows[-1]["deps"].get("strategy"), rows[-1]["passed"], rows[-1]["failed"], flush=True)
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(rows, indent=1) + "\n")
n = len(rows); ok = sum(1 for r in rows if r["verifiable"])
print(f"\n{a.mode}: {ok}/{n} runnable ({ok / max(n, 1):.0%})")
print("strategies:", dict(collections.Counter(r["deps"].get("strategy") for r in rows)))
print("not runnable:", [(r["session"], r["deps"].get("reason") or r["tail"][-120:]) for r in rows if not r["verifiable"]][:40])
