#!/usr/bin/env python3
"""Run an everyday batch through the installed tattle plugin (release gate A2).

    python3 scripts/everyday.py corpus/everyday/batch1.json --out <dir> [--lanes trust,budget] [--jobs 5]
    python3 scripts/everyday.py corpus/everyday/batch2.json --out <dir2> --seed <dir>

Needs Claude Code with the tattle plugin installed and enabled (user scope), and
<dir> OUTSIDE this repo: Claude Code reads CLAUDE.md files in parent folders,
and this repo's would tell the agent about the checker.

Each lane is one project folder (<dir>/projects/<lane>); its sessions run in
order, so later ones build on earlier work. Lanes run in parallel. Per
session it writes <dir>/runs/<id>/turn<N>.jsonl (stream-json), and
<dir>/runs/<id>/result.json with the alerts the person would have seen at
the end of each turn, the /tattle:report text, cost and duration.
Results stay local (never commit them). Label each alert true or false by
reading the session; see docs/RESULTS.md.
"""
import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import time
import uuid

REPO = pathlib.Path(__file__).resolve().parent.parent
ALLOWED = [
    "Bash(npm test:*)", "Bash(npm run:*)", "Bash(node:*)", "Bash(cargo:*)",
    "Bash(git status:*)", "Bash(git diff:*)", "Bash(git log:*)", "Bash(git show:*)", "Bash(git add:*)", "Bash(git commit:*)",
    "Bash(ls:*)", "Bash(cat:*)", "Bash(grep:*)", "Bash(rg:*)", "Bash(find:*)", "Bash(head:*)", "Bash(tail:*)",
    "Bash(wc:*)", "Bash(mkdir:*)", "Bash(sed -n:*)", "Bash(echo:*)",
    "Bash(go:*)", "Bash(python3:*)", "Bash(python:*)",
    "Bash(make:*)", "Bash(gcc:*)", "Bash(cc:*)", "Bash(ruby:*)", "Bash(./run-tests.sh:*)", "Bash(chmod +x:*)",
]
# Variables of the Claude Code session running this script; a child that
# inherits them reuses its session id.
DROP_ENV = ["CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SESSION_ATTENDED", "CLAUDECODE"]


def child_env(extra):
    env = {k: v for k, v in os.environ.items() if k not in DROP_ENV}
    env.update(extra)
    return env


def setup(lane, kind, projects, seed):
    p = projects / lane
    if p.exists():
        return p
    if kind.startswith("continue:"):
        # A project grown in an earlier batch (--seed <its --out>), without
        # its recordings, committed as the person would have left it.
        src = pathlib.Path(seed or "") / "projects" / kind.split(":", 1)[1]
        if not seed or not src.is_dir():
            raise SystemExit(f"lane {lane}: needs --seed with projects/{kind.split(':', 1)[1]}")
        shutil.copytree(src, p, ignore=shutil.ignore_patterns(".checker"), symlinks=True)
        subprocess.run(["git", "-C", str(p), "add", "-A"], check=True)
        subprocess.run(["git", "-C", str(p), "-c", "user.name=dev", "-c", "user.email=dev@localhost", "commit", "-qm", "Earlier work"], check=False)
    elif kind == "copy-of-this-repo":
        subprocess.run(["git", "clone", "-q", str(REPO), str(p)], check=True)
        # The repo records itself through its own hooks; the installed plugin
        # stands aside for such projects. Remove them so the plugin records.
        shutil.rmtree(p / ".claude", ignore_errors=True)
        subprocess.run(["git", "-C", str(p), "-c", "user.name=dev", "-c", "user.email=dev@localhost", "commit", "-qam", "Remove dev hooks"], check=False)
    else:
        p.mkdir(parents=True)
        subprocess.run(["git", "init", "-q", str(p)], check=True)
    return p


def plugin_launcher():
    """The installed plugin's launcher, from Claude Code's plugin list."""
    out = subprocess.run(["claude", "plugin", "list"], capture_output=True, text=True, env=child_env({})).stdout
    lines = out.splitlines()
    for i, line in enumerate(lines):
        if line.strip().startswith(">") and "tattle@" in line:
            for follow in lines[i + 1:i + 6]:
                if "Read from:" in follow:
                    return pathlib.Path(follow.split("Read from:", 1)[1].strip()) / "scripts" / "tattle"
    raise SystemExit("tattle plugin not installed: /plugin install tattle@<marketplace>")


class ApiRefused(Exception):
    """The API refused the turn (usage limit, overload): the session didn't
    run, so it isn't a result. The lane stops; re-run the batch later and
    finished sessions are skipped."""


def parse_turn(log):
    alerts, result, cost, errors = [], None, 0.0, []
    for line in log.read_text(errors="replace").splitlines():
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if e.get("type") == "system" and e.get("subtype") == "informational" and "tattle" in str(e.get("content", "")):
            alerts.append(e["content"])
        if e.get("type") == "system" and e.get("subtype") == "hook_response" and e.get("exit_code") not in (None, 0, "0"):
            errors.append(f"{e.get('hook_event')}: exit {e.get('exit_code')}")
        if e.get("type") == "result":
            result = e.get("result")
            cost = e.get("total_cost_usd") or 0.0
            if e.get("api_error") or e.get("terminal_reason") == "api_error":
                raise ApiRefused(f"{log.parent.name}: {e.get('api_error') or 'api error'}: {result}")
    if result is None:
        # Claude Code never ran the turn (e.g. bad arguments): not a result.
        first = log.read_text(errors="replace").strip().splitlines()[:1]
        raise ApiRefused(f"{log.parent.name}: the turn didn't run: {first}")
    return alerts, result, cost, errors


def run_session(s, project, runs, launcher, env_extra, budget):
    out = runs / s["id"]
    if (out / "result.json").exists():
        return json.loads((out / "result.json").read_text())
    # A session the API refused part-way: start it again from scratch.
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True, exist_ok=True)
    sid = str(uuid.uuid4())
    started = time.time()
    turns = []
    for i, prompt in enumerate(s["turns"]):
        # The prompt goes in on stdin: one starting with "--" would be read
        # as an option (batch 6, G63: "--top with N bigger than…").
        args = ["claude", "-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "acceptEdits",
                "--max-budget-usd", str(budget), "--allowedTools", *ALLOWED]
        if s.get("model"):
            args += ["--model", s["model"]]
        args += ["--session-id", sid] if i == 0 else ["--resume", sid]
        log = out / f"turn{i + 1}.jsonl"
        with open(log, "w") as f:
            try:
                subprocess.run(args, cwd=project, env=child_env(env_extra), input=prompt, text=True, stdout=f, stderr=subprocess.STDOUT, timeout=1500)
                timed_out = False
            except subprocess.TimeoutExpired:
                timed_out = True
        alerts, result, cost, errors = parse_turn(log)
        turns.append({"prompt": prompt, "alerts": alerts, "final_message": result, "cost_usd": cost, "hook_errors": errors, "timed_out": timed_out})
    report = subprocess.run([str(launcher), "report", "--session", sid], cwd=project, capture_output=True, text=True, env=child_env({"TATTLE_PLUGIN": "1"})).stdout
    result = {"id": s["id"], "model": s.get("model") or "default", "session_id": sid, "project": str(project), "turns": turns,
              "report": report, "seconds": round(time.time() - started), "cost_usd": round(sum(t["cost_usd"] for t in turns), 4)}
    (out / "result.json").write_text(json.dumps(result, indent=2))
    return result


def run_lane(name, lane, projects, runs, launcher, budget, seed):
    project = setup(name, lane["setup"], projects, seed)
    env_extra = {"CARGO_TARGET_DIR": str(projects / f"{name}-target")} if lane["setup"] == "copy-of-this-repo" else {}
    results = []
    for s in lane["sessions"]:
        try:
            r = run_session(s, project, runs, launcher, env_extra, budget)
        except ApiRefused as e:
            print(f"STOPPED lane {name}: {e}", flush=True)
            break
        shown = sum(len(t["alerts"]) for t in r["turns"])
        print(f"{r['id']:<4} {r['model']:<8} {r['seconds']:>5}s ${r['cost_usd']:<6} alerts shown: {shown}", flush=True)
        results.append(r)
    return results


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("batch")
    ap.add_argument("--out", required=True)
    ap.add_argument("--lanes")
    ap.add_argument("--jobs", type=int, default=5)
    ap.add_argument("--budget", type=float, default=5.0, help="max USD per turn")
    ap.add_argument("--seed", help="an earlier batch's --out, for lanes that continue its projects")
    a = ap.parse_args()
    out = pathlib.Path(a.out).resolve()
    if out == REPO or REPO in out.parents:
        raise SystemExit("--out must be outside this repo (the agent would read its CLAUDE.md)")
    batch = json.loads(pathlib.Path(a.batch).read_text())
    lanes = {k: v for k, v in batch["lanes"].items() if not a.lanes or k in a.lanes.split(",")}
    projects, runs = out / "projects", out / "runs"
    projects.mkdir(parents=True, exist_ok=True)
    runs.mkdir(parents=True, exist_ok=True)
    launcher = plugin_launcher()
    binary = next((launcher.parent.parent / "dist").glob("tattle-*"), None)
    meta = {"batch": a.batch, "cv_commit": subprocess.run(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip(),
            "plugin_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None, "started": time.strftime("%Y-%m-%d %H:%M")}
    (out / "meta.json").write_text(json.dumps(meta, indent=2))
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        futures = {ex.submit(run_lane, n, l, projects, runs, launcher, a.budget, a.seed): n for n, l in lanes.items()}
        everything = []
        for f in cf.as_completed(futures):
            everything += f.result()
    shown = [r for r in everything if any(t["alerts"] for t in r["turns"])]
    print(f"\n{len(everything)} sessions, {len(shown)} with an alert shown, ${round(sum(r['cost_usd'] for r in everything), 2)}")


if __name__ == "__main__":
    main()
