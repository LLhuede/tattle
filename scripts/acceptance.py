#!/usr/bin/env python3
"""Run a big test's hidden acceptance tests against an app (evaluation only).

    scripts/acceptance.py --app ~/Code/cv-sandbox/big/A1-... --suite bigtests/A-flagship \
        [--milestones m1,m2 | --upto m3] [--out result.json]

Copies the app (without node_modules, .checker, .git) to a temp folder, runs
`npm ci`/`npm install` and `npm run build --if-present`, then runs the
suite's `acceptance/<milestone>.*.test.mjs` files with `node --test` and
APP_DIR pointing at the copy. Prints and returns per-milestone results.
The app under test never sees these files. See bigtests/CONTRACT.md.
"""
import argparse
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parent.parent
TAP_LINE = re.compile(r"^(\s*)(not ok|ok) \d+ - (.*?)(?:\s+#\s*(SKIP|TODO).*)?$")


def milestone_of(path: pathlib.Path) -> str:
    return path.name.split(".", 1)[0]


def select_files(suite: pathlib.Path, milestones=None, upto=None):
    files = sorted((suite / "acceptance").glob("*.test.mjs"))
    if milestones:
        files = [f for f in files if milestone_of(f) in milestones]
    elif upto:
        limit = int(upto.lstrip("m"))
        files = [f for f in files if int(milestone_of(f).lstrip("m")) <= limit]
    return files


def run(cmd, cwd, timeout, env=None):
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, shell=True, env=env)
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        return -1, "timeout"


def parse_tap(text: str):
    """Leaf test results from `node --test --test-reporter=tap` output. A
    suite's own result line comes right after its children's plan (`1..N`),
    so those lines are skipped."""
    lines = text.splitlines()
    results = []
    for i, line in enumerate(lines):
        m = TAP_LINE.match(line)
        if not m:
            continue
        _, status, name, directive = m.groups()
        prev = lines[i - 1].strip() if i else ""
        if re.fullmatch(r"1\.\.\d+", prev):
            continue  # a describe() block, not a test
        results.append({"name": name.strip(), "passed": status == "ok", "skipped": directive is not None})
    return results


def acceptance(app: pathlib.Path, suite: pathlib.Path, milestones=None, upto=None, install_timeout=600):
    files = select_files(suite, milestones, upto)
    out = {"app": str(app), "suite": str(suite.relative_to(REPO) if suite.is_relative_to(REPO) else suite), "milestones": {}, "setup": {}}
    if not files:
        out["error"] = "no acceptance files selected"
        return out
    work = pathlib.Path(tempfile.mkdtemp(prefix="acceptance-"))
    try:
        copy = work / "app"
        shutil.copytree(app, copy, ignore=shutil.ignore_patterns("node_modules", ".checker", ".git", ".claude"))
        if not (copy / "package.json").is_file():
            out["error"] = "no package.json at the app root"
            # Nothing to test counts as failing, not as zero failures.
            for f in files:
                m = out["milestones"].setdefault(milestone_of(f), {"passed": 0, "failed": 0, "tests": [], "error": out["error"]})
                m["failed"] += 1
                m["tests"].append({"name": f"{f.name}: app could not be set up ({out['error']})", "passed": False, "file": f.name})
            return out
        install = "npm ci --no-audit --no-fund" if (copy / "package-lock.json").is_file() else "npm install --no-audit --no-fund"
        code, text = run(install, copy, install_timeout)
        out["setup"]["install"] = {"exit": code, "tail": text[-800:]}
        code, text = run("npm run build --if-present", copy, install_timeout)
        out["setup"]["build"] = {"exit": code, "tail": text[-800:]}
        env = dict(__import__("os").environ, APP_DIR=str(copy))
        for f in files:
            code, text = run(f"node --test --test-reporter=tap {f}", copy, 300, env=env)
            tests = parse_tap(text)
            m = out["milestones"].setdefault(milestone_of(f), {"passed": 0, "failed": 0, "tests": [], "files": []})
            m["files"].append({"file": f.name, "exit": code, "tail": "" if code == 0 else text[-1500:]})
            for t in tests:
                t["file"] = f.name
                m["tests"].append(t)
                m["passed" if t["passed"] else "failed"] += 1
            if not tests and code != 0:
                m["failed"] += 1
                m["tests"].append({"name": f"{f.name} (file failed to run)", "passed": False, "file": f.name})
        return out
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True)
    ap.add_argument("--suite", required=True)
    ap.add_argument("--milestones")
    ap.add_argument("--upto")
    ap.add_argument("--out")
    a = ap.parse_args()
    res = acceptance(pathlib.Path(a.app).expanduser().resolve(), pathlib.Path(a.suite).resolve(),
                     a.milestones.split(",") if a.milestones else None, a.upto)
    text = json.dumps(res, indent=2)
    if a.out:
        pathlib.Path(a.out).write_text(text + "\n")
    for k, v in sorted(res["milestones"].items()):
        print(f"{k}: {v['passed']} passed, {v['failed']} failed")
    if res.get("error"):
        print("error:", res["error"], file=sys.stderr)


if __name__ == "__main__":
    main()
