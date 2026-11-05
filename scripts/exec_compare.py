#!/usr/bin/env python3
"""Phase 4b.2: write execution records for every recording and compare them
with the oracle (which installed with network and ran the real tests).

    scripts/exec_compare.py [--only R35/run1 ...]

For each recording: `tattle verify --write` (dependencies: own project, then the
~/Code/cv-sandbox pool), then `tattle check --json`. A *false failure* is a
session where the sandbox reports more failing tests than the oracle saw, or
couldn't run at all. Big-test runs (artifacts/bigtests/*/*/run*/recording)
are included. Output: artifacts/exec-compare.json (local).
"""
import argparse, json, pathlib, subprocess

REPO = pathlib.Path(__file__).resolve().parent.parent
POOL = pathlib.Path.home() / "Code/cv-sandbox"
CV = str(REPO / "target/release/tattle")

ap = argparse.ArgumentParser()
ap.add_argument("--only", nargs="*")
a = ap.parse_args()

jobs = []
for rec in sorted((REPO / "fixtures/recorded").glob("*/run*")):
    if (rec / "events.jsonl").is_file():
        key = f"{rec.parent.name}/{rec.name}"
        jobs.append((key, rec, POOL / f"{rec.parent.name}-{rec.name}", REPO / f"fixtures/oracle/{rec.parent.name}-{rec.name}.json"))
for rec in sorted((REPO / "artifacts/bigtests").glob("*/*/run*/recording")):
    run_dir = rec.parent
    ident, model, run = run_dir.parent.parent.name, run_dir.parent.name, run_dir.name
    jobs.append((f"{ident}/{model}/{run}", rec, POOL / "big" / f"{ident}-{model}-{run}", None))
if a.only:
    jobs = [j for j in jobs if j[0] in a.only]

rows = []
for key, rec, project, oracle in jobs:
    cmd = [CV, "verify", str(rec), "--write", "--pool", str(POOL), "--pool", str(POOL / "big"), "--timeout", "120"]
    if project.is_dir():
        cmd += ["--project", str(project)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
    try:
        record = json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        record = {"error": (r.stdout + r.stderr)[-300:]}
    fin = record.get("final", {}).get("tests", {})
    run0 = (fin.get("runs") or [{}])[0]
    app = record.get("final", {}).get("app", {}).get("outcome")
    row = {"session": key, "app": app, "baseline_app": record.get("baseline", {}).get("app", {}).get("outcome"), "outcome": fin.get("outcome"), "passed": run0.get("passed"), "failed": run0.get("failed"),
           "baseline": record.get("baseline", {}).get("tests", {}).get("outcome"), "error": record.get("error")}
    if oracle and oracle.is_file():
        o = json.loads(oracle.read_text()).get("real", {}).get("vitest", {})
        row["oracle_passed"], row["oracle_failed"] = o.get("numPassedTests"), o.get("numFailedTests")
        # Flaky results produce no findings, so they aren't false failures.
        row["false_failure"] = row["outcome"] == "couldnt-run" or (
            row["outcome"] != "flaky" and row["failed"] is not None and row["oracle_failed"] is not None and row["failed"] > row["oracle_failed"])
    c = subprocess.run([CV, "check", str(rec), "--json"], capture_output=True, text=True)
    try:
        row["exec_findings"] = [f["check"] for f in json.loads(c.stdout)["findings"] if f["check"].startswith("exec.")]
    except Exception:
        row["exec_findings"] = ["(check failed)"]
    rows.append(row)
    print(key, "app", row["app"], row["outcome"], row["passed"], row["failed"], "oracle", row.get("oracle_passed"), row.get("oracle_failed"),
          "FALSE" if row.get("false_failure") else "", row["exec_findings"], flush=True)
out = REPO / "artifacts/exec-compare.json"
out.write_text(json.dumps(rows, indent=1) + "\n")
ff = [r["session"] for r in rows if r.get("false_failure")]
print(f"\n{len(rows)} sessions; false failures vs oracle: {len(ff)} {ff}")
print("sessions with exec findings:", sum(1 for r in rows if r["exec_findings"]))
print("app outcomes:", __import__("collections").Counter(r.get("app") for r in rows))
print("app_does_not_start:", [r["session"] for r in rows if "exec.app_does_not_start" in r["exec_findings"]])
