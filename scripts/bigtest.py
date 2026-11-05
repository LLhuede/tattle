#!/usr/bin/env python3
"""Big-test runner: from-scratch builds at scale (see bigtests/README.md).

    scripts/bigtest.py validate [--with-reference]
    scripts/bigtest.py dry-run <suite.json | backlog.json | plan.json> [--model M]
    scripts/bigtest.py run <suite.json> --model M --run N [--resume] [--max-usd X]
    scripts/bigtest.py simuser <backlog.json> --model M --run N [--resume] [--max-usd X] [--llm-user]
    scripts/bigtest.py plan <plan.json> [--resume]

Agents run headless (`claude -p`) in ~/Code/cv-sandbox/big/, recorded by
cv's hooks. After every turn the hidden acceptance tests run against a copy
of the code (scripts/acceptance.py). Everything a run produces goes to
artifacts/ (gitignored): replies, costs, acceptance results, the tattle
recording, and state for --resume. Runs stop at their budget or at a usage
limit (exit code 3) and can be resumed.
"""
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from acceptance import acceptance  # noqa: E402
from record_headless import ALLOWED  # noqa: E402

REPO = pathlib.Path(__file__).resolve().parent.parent
CV = REPO / "target/release/tattle"
SANDBOXES = pathlib.Path.home() / "Code/cv-sandbox/big"
ARTIFACTS = REPO / "artifacts/bigtests"
USAGE_LIMIT = re.compile(r"(?i)hit your (?:session|usage|weekly) limit|usage limit|rate limit")
# Rough cost per agent turn (one prompt with its tool calls), from the
# recall study's small sessions scaled up for bigger builds. Estimates only.
COST_PER_TURN = {"claude-haiku-4-5": 0.35, "haiku": 0.25, "claude-sonnet-4-5": 1.2, "sonnet": 1.2, "opus": 2.5, "claude-opus-5-5": 2.5}
LIMIT_EXIT = 3
# A stand-in for `claude` (scripts/fake_claude.py) tests the runner without
# spending anything.
CLAUDE = os.environ.get("BIGTEST_CLAUDE", "claude")


# ---------- loading and validation ----------

def load(path):
    p = pathlib.Path(path)
    return json.loads(p.read_text()), p.resolve().parent


def spec_text(d, folder):
    return (folder / d["spec"]).read_text() if d.get("spec") else ""


def fill(prompt, spec):
    return prompt.replace("{{spec}}", spec)


def all_definitions():
    root = REPO / "bigtests"
    return sorted(root.glob("A-flagship/suite.json")) + sorted(root.glob("B-breadth/*/suite.json")), \
        sorted(root.glob("C-simuser/*.json")), sorted(root.glob("D-hybrid/*.json"))


def validate_suite(path, problems, with_reference):
    d, folder = load(path)
    tag = str(path.relative_to(REPO))
    for k in ("schema", "id", "title", "kind", "spec", "milestones", "budget", "turns"):
        if k not in d:
            problems.append(f"{tag}: missing {k}")
    if problems and problems[-1].startswith(tag):
        return
    if not (folder / d["spec"]).is_file():
        problems.append(f"{tag}: spec {d['spec']} not found")
    if d.get("base") and not (REPO / d["base"]).is_dir():
        problems.append(f"{tag}: base {d['base']} not found")
    ms = [m["id"] for m in d["milestones"]]
    if len(set(ms)) != len(ms):
        problems.append(f"{tag}: duplicate milestone ids")
    files = sorted((folder / "acceptance").glob("*.test.mjs"))
    file_ms = {f.name.split(".", 1)[0] for f in files}
    for m in ms:
        if m not in file_ms:
            problems.append(f"{tag}: milestone {m} has no acceptance file")
    for m in file_ms - set(ms):
        problems.append(f"{tag}: acceptance files for undeclared milestone {m}")
    due = set()
    for i, t in enumerate(d["turns"], 1):
        if t.get("n") != i:
            problems.append(f"{tag}: turn {i} has n={t.get('n')}")
        if not t.get("prompt"):
            problems.append(f"{tag}: turn {i} has no prompt")
        for m in t.get("milestones_due", []):
            if m not in ms:
                problems.append(f"{tag}: turn {i} names unknown milestone {m}")
            due.add(m)
        for tool in t.get("deny_tools", []):
            if tool not in ALLOWED:
                problems.append(f"{tag}: turn {i} denies {tool}, which isn't in the allow-list")
        trap = t.get("trap")
        if trap and not all(trap.get(k) for k in ("kind", "honest_if", "hallucination_if")):
            problems.append(f"{tag}: turn {i} trap needs kind, honest_if, hallucination_if")
    for m in set(ms) - due:
        problems.append(f"{tag}: milestone {m} is never due")
    if "{{spec}}" not in d["turns"][0]["prompt"] and not d.get("base"):
        problems.append(f"{tag}: first turn of a from-scratch build should include {{{{spec}}}}")
    for f in files:
        r = subprocess.run(["node", "--check", str(f)], capture_output=True, text=True)
        if r.returncode:
            problems.append(f"{tag}: {f.name} doesn't parse: {r.stderr.strip()[-200:]}")
    if with_reference:
        ref = folder / "reference"
        if not ref.is_dir():
            problems.append(f"{tag}: no reference/ implementation")
        else:
            res = acceptance(ref, folder)
            for m, v in sorted(res["milestones"].items()):
                if v["failed"]:
                    bad = [t["name"] for t in v["tests"] if not t["passed"]][:5]
                    problems.append(f"{tag}: reference fails {v['failed']} test(s) in {m}: {bad}")
            print(f"  {tag}: reference " + ", ".join(f"{m} {v['passed']}/{v['passed'] + v['failed']}" for m, v in sorted(res["milestones"].items())))
    return d


def validate_backlog(path, problems):
    d, folder = load(path)
    tag = str(path.relative_to(REPO))
    for k in ("schema", "id", "suite", "opening", "items", "policy", "budget"):
        if k not in d:
            problems.append(f"{tag}: missing {k}")
            return
    suite, sfolder = load(folder / d["suite"] / "suite.json")
    ms = {m["id"] for m in suite["milestones"]}
    ids = [i["id"] for i in d["items"]]
    for it in d["items"]:
        for m in it.get("milestones", []):
            if m not in ms:
                problems.append(f"{tag}: item {it['id']} names unknown milestone {m}")
        if not it.get("prompt"):
            problems.append(f"{tag}: item {it['id']} has no prompt")
    for e in d.get("events", []):
        if e.get("after_item") not in ids:
            problems.append(f"{tag}: event after unknown item {e.get('after_item')}")
    try:
        re.compile(d["policy"]["done_claim"])
    except (re.error, KeyError) as e:
        problems.append(f"{tag}: policy.done_claim: {e}")
    if "{symptom}" not in d["policy"].get("pushback", ""):
        problems.append(f"{tag}: policy.pushback needs {{symptom}}")


def validate_plan(path, problems):
    d, _ = load(path)
    tag = str(path.relative_to(REPO))
    for r in d.get("runs", []):
        if not (REPO / r["suite"]).is_file():
            problems.append(f"{tag}: {r['suite']} not found")
        if r.get("mode", "run") not in ("run", "simuser"):
            problems.append(f"{tag}: bad mode {r.get('mode')}")
    if "max_usd" not in d.get("budget", {}):
        problems.append(f"{tag}: budget.max_usd missing")


def cmd_validate(a):
    suites, backlogs, plans = all_definitions()
    problems = []
    for s in suites:
        validate_suite(s, problems, a.with_reference)
    for b in backlogs:
        validate_backlog(b, problems)
    for p in plans:
        validate_plan(p, problems)
    print(f"{len(suites)} suites, {len(backlogs)} simulated-user backlogs, {len(plans)} plans")
    for p in problems:
        print("PROBLEM:", p)
    return 1 if problems else 0


# ---------- cost estimates ----------

def estimate(path, model, cap=None):
    """(turns, low, high) dollars for one run. Low: every turn at the
    per-turn estimate. High: twice that. Both stop at the run's cap."""
    d, folder = load(path)
    per = COST_PER_TURN.get(model, 1.0)
    cap = cap or d["budget"]["max_usd"]
    if "turns" in d:
        n = len(d["turns"])
        low = n * per
    elif "items" in d:
        n = d["policy"]["max_turns"]
        low = (len(d["items"]) + len(d.get("events", [])) + 2) * per
    else:
        raise SystemExit(f"{path}: not a suite or backlog")
    return n, min(low, cap), min(max(low * 2, per * n), cap)


def cmd_dry_run(a):
    d, _ = load(a.path)
    if "runs" in d:
        total_low = total_high = 0
        for r in d["runs"]:
            n, low, high = estimate(REPO / r["suite"], r["model"], r.get("max_usd"))
            k = r.get("runs", 1)
            total_low += low * k
            total_high += high * k
            print(f"{r['suite']} [{r.get('mode', 'run')}] {r['model']} x{k}: ~{n} turns each, ${low * k:.0f}–{high * k:.0f}")
        print(f"plan total: ${total_low:.0f}–{total_high:.0f} (cap ${d['budget']['max_usd']})")
    else:
        n, low, high = estimate(a.path, a.model)
        print(f"{a.path} with {a.model}: ~{n} turns, ${low:.0f}–{high:.0f}")
    return 0


# ---------- running ----------

def frozen_cv():
    digest = hashlib.sha256(CV.read_bytes()).hexdigest()[:12]
    frozen = SANDBOXES.parent / "_bin" / f"cv-{digest}"
    if not frozen.exists():
        frozen.parent.mkdir(parents=True, exist_ok=True)
        tmp = frozen.parent / f".{frozen.name}.{os.getpid()}.tmp"
        shutil.copy2(CV, tmp)
        os.replace(tmp, frozen)
    return frozen


class Run:
    """One agent run: sandbox, artifacts, state, budget."""

    def __init__(self, ident, model, run, max_usd, base=None, manifest=True, resume=False):
        safe_model = model.replace("/", "_")
        self.ident, self.model, self.run, self.max_usd = ident, model, run, max_usd
        self.sandbox = SANDBOXES / f"{ident}-{safe_model}-run{run}"
        self.out = ARTIFACTS / ident / safe_model / f"run{run}"
        self.state_path = self.out / "state.json"
        self.cv = frozen_cv()
        if resume and self.state_path.is_file():
            self.state = json.loads(self.state_path.read_text())
        else:
            if self.sandbox.exists() or self.out.exists():
                raise SystemExit(f"{self.sandbox} or {self.out} exists; use --resume or another --run")
            self.out.mkdir(parents=True)
            init = [str(self.cv), "sandbox", "init", str(self.sandbox)]
            if base:
                init += ["--base", str(REPO / base)]
            if manifest:
                init += ["--manifest"]
            subprocess.run(init, check=True, stdout=subprocess.DEVNULL)
            self.state = {"ident": ident, "model": model, "run": run, "turns_done": 0, "spent_usd": 0.0, "status": "running", "started": time.time()}
            self.save()

    def save(self):
        self.state_path.write_text(json.dumps(self.state, indent=2) + "\n")

    def send(self, prompt, deny=(), max_turns=80):
        allowed = [t for t in ALLOWED if t not in set(deny)]
        cmd = [CLAUDE, "-p", prompt, "--output-format", "json", "--permission-mode", "acceptEdits",
               "--allowedTools", *allowed, "--model", self.model, "--max-turns", str(max_turns)]
        if self.state["turns_done"] > 0:
            cmd.insert(1, "--continue")
        started = time.time()
        r = subprocess.run(cmd, cwd=self.sandbox, capture_output=True, text=True, timeout=3600)
        try:
            res = json.loads(r.stdout)
        except json.JSONDecodeError:
            res = {"result": r.stdout[-2000:], "is_error": True, "stderr": r.stderr[-2000:]}
        cost = res.get("total_cost_usd") or 0.0
        reply = res.get("result") or ""
        limited = bool(res.get("is_error")) and bool(USAGE_LIMIT.search(reply + res.get("stderr", "")))
        return {"reply": reply, "cost_usd": cost, "seconds": round(time.time() - started), "is_error": res.get("is_error"),
                "num_turns": res.get("num_turns"), "limited": limited}

    def over_budget(self):
        return self.state["spent_usd"] >= self.max_usd

    def export(self):
        dest = self.out / "recording"
        if dest.exists():
            shutil.rmtree(dest)
        r = subprocess.run([str(self.cv), "record", "export", "--project", str(self.sandbox), "--prompt-id", self.ident,
                            "--run", str(self.run), "--out", str(dest)], cwd=REPO, capture_output=True, text=True)
        self.state["export"] = (r.stdout + r.stderr).strip()[-500:]
        self.save()

    def stop(self, status):
        self.state["status"] = status
        self.save()
        self.export()
        print(f"{self.ident}: {status} after {self.state['turns_done']} turn(s), ${self.state['spent_usd']:.2f}; artifacts in {self.out}")
        return LIMIT_EXIT if status == "usage-limit" else 0


def cmd_run(a):
    d, folder = load(a.path)
    spec = spec_text(d, folder)
    run = Run(d["id"], a.model, a.run, a.max_usd or d["budget"]["max_usd"], d.get("base"), not a.no_manifest, a.resume)
    due = []
    for t in d["turns"][: run.state["turns_done"]]:
        due += t.get("milestones_due", [])
    for t in d["turns"][run.state["turns_done"]:]:
        if run.over_budget():
            return run.stop("budget")
        r = run.send(fill(t["prompt"], spec), t.get("deny_tools", []), d["budget"].get("max_turns_per_prompt", 80))
        if r["limited"]:
            return run.stop("usage-limit")  # turn not counted: resume repeats it
        due += t.get("milestones_due", [])
        acc = acceptance(run.sandbox, folder, milestones=due) if due else {}
        run.state["turns_done"] += 1
        run.state["spent_usd"] += r["cost_usd"]
        (run.out / f"turn{t['n']}.json").write_text(json.dumps({"turn": t["n"], "prompt": t["prompt"][:4000], "trap": t.get("trap"), **r, "acceptance": acc}, indent=2) + "\n")
        run.save()
        passed = sum(v["passed"] for v in acc.get("milestones", {}).values())
        total = passed + sum(v["failed"] for v in acc.get("milestones", {}).values())
        print(f"{d['id']} turn {t['n']}: ${r['cost_usd']:.2f}, hidden tests {passed}/{total}")
    return run.stop("done")


# ---------- simulated user (C) ----------

def failing_tests(acc, milestones):
    out = []
    for m in milestones:
        for t in acc.get("milestones", {}).get(m, {}).get("tests", []):
            if not t["passed"]:
                out.append(t["name"])
    return out


def symptom_for(test_names, items, generic):
    for name in test_names:
        for it in items:
            for key, text in it.get("symptoms", {}).items():
                if key.lower() in name.lower():
                    return text
    return generic


def reword(text, model="claude-haiku-4-5"):
    """--llm-user: let a model reword the simulated user's message (harness only)."""
    r = subprocess.run([CLAUDE, "-p", "Rewrite this as a short, casual message from a busy developer to their coding assistant. "
                        "Keep every fact and request; add nothing. Reply with the message only.\n\n" + text,
                        "--model", model, "--output-format", "json", "--allowedTools", ""], capture_output=True, text=True, timeout=300)
    try:
        return json.loads(r.stdout).get("result") or text
    except json.JSONDecodeError:
        return text


def cmd_simuser(a):
    d, folder = load(a.path)
    suite, sfolder = load(folder / d["suite"] / "suite.json")
    spec = spec_text(suite, sfolder)
    pol = d["policy"]
    run = Run(d["id"], a.model, a.run, a.max_usd or d["budget"]["max_usd"], suite.get("base"), not a.no_manifest, a.resume)
    st = run.state.setdefault("sim", {"item": 0, "pushbacks": 0, "queue": [], "done_items": [], "unresolved": [], "log": []})
    items = d["items"]
    done_claim = re.compile(pol["done_claim"], re.I)

    def next_message():
        if st["queue"]:
            return st["queue"].pop(0)
        if st["item"] >= len(items):
            return None
        msg = items[st["item"]]["prompt"]
        return fill(d["opening"], spec) + "\n\n" + msg if run.state["turns_done"] == 0 else msg

    message = st.pop("pending", None) or next_message()
    while message and run.state["turns_done"] < pol["max_turns"]:
        if run.over_budget():
            st["pending"] = message
            return run.stop("budget")
        text = reword(message) if a.llm_user else message
        r = run.send(text, [], suite["budget"].get("max_turns_per_prompt", 80))
        if r["limited"]:
            st["pending"] = message
            return run.stop("usage-limit")
        run.state["turns_done"] += 1
        run.state["spent_usd"] += r["cost_usd"]
        due = [m for it in items[: st["item"] + 1] for m in it.get("milestones", [])]
        acc = acceptance(run.sandbox, sfolder, milestones=sorted(set(due))) if due else {}
        current = items[st["item"]] if st["item"] < len(items) else None
        failing = failing_tests(acc, sorted(set(due)))
        claimed = bool(done_claim.search(r["reply"]))
        decision = "next"
        if current and failing and st["pushbacks"] < pol["max_pushbacks_per_item"]:
            st["pushbacks"] += 1
            decision = "pushback"
            st["queue"].insert(0, pol["pushback"].replace("{symptom}", symptom_for(failing, items, pol["generic_symptom"])))
        elif current:
            (st["unresolved"] if failing else st["done_items"]).append(current["id"])
            for e in d.get("events", []):
                if e["after_item"] == current["id"]:
                    st["queue"].append(e["prompt"])
            st["item"] += 1
            st["pushbacks"] = 0
            if st["item"] >= len(items) and pol.get("closing"):
                st["queue"].append(pol["closing"])
        entry = {"turn": run.state["turns_done"], "sent": text[:4000], "claimed_done": claimed, "failing": failing[:20],
                 "decision": decision, **r, "acceptance": acc}
        (run.out / f"turn{run.state['turns_done']}.json").write_text(json.dumps(entry, indent=2) + "\n")
        st["log"].append({k: entry[k] for k in ("turn", "claimed_done", "decision", "cost_usd")} | {"failing": len(failing)})
        run.save()
        print(f"{d['id']} turn {run.state['turns_done']}: ${r['cost_usd']:.2f}, failing {len(failing)}, claimed done {claimed}, {decision}")
        message = next_message()
    return run.stop("done")


# ---------- plans (D) ----------

def gate_hit(cond, summary):
    m = re.fullmatch(r"\s*(\w+)\s*(<=|>=|<|>|==)\s*([\d.]+)\s*", cond)
    if not m:
        raise SystemExit(f"bad gate condition: {cond}")
    field, op, value = m.group(1), m.group(2), float(m.group(3))
    v = float(summary.get(field, 0))
    return {"<": v < value, ">": v > value, "<=": v <= value, ">=": v >= value, "==": v == value}[op]


def run_summary(out):
    """Final and best hidden-test pass rates of a run. `best_pass_rate` says
    whether the spec was buildable at all; the final rate can be 0 when the
    agent broke a working app (flagship A1, Haiku 4.5: 45/79 at turn 15,
    then 0 after turn 16)."""
    st = json.loads((out / "state.json").read_text())
    turns = sorted(out.glob("turn*.json"), key=lambda p: int(re.sub(r"\D", "", p.stem) or 0))
    rates = []
    for t in turns:
        acc = json.loads(t.read_text()).get("acceptance", {}).get("milestones", {})
        p = sum(v["passed"] for v in acc.values())
        f = sum(v["failed"] for v in acc.values())
        if p + f:
            rates.append(p / (p + f))
    return {"spent_usd": st["spent_usd"], "turns_done": st["turns_done"], "status": st["status"],
            "acceptance_pass_rate": rates[-1] if rates else 0.0, "best_pass_rate": max(rates, default=0.0)}


def cmd_plan(a):
    d, _ = load(a.path)
    spent = 0.0
    for r in d["runs"]:
        for k in range(1, r.get("runs", 1) + 1):
            label = f"{pathlib.Path(r['suite']).parent.name}/{r['model']}/run{k}"
            suite_def, _ = load(REPO / r["suite"])
            out = ARTIFACTS / suite_def["id"] / r["model"].replace("/", "_") / f"run{k}"
            if (out / "state.json").is_file() and json.loads((out / "state.json").read_text())["status"] == "done":
                spent += json.loads((out / "state.json").read_text())["spent_usd"]
                print(f"{label}: already done")
                continue
            if spent >= d["budget"]["max_usd"]:
                print(f"plan budget reached (${spent:.2f}); stopping before {label}")
                return 0
            ns = argparse.Namespace(path=str(REPO / r["suite"]), model=r["model"], run=k, resume=(out / "state.json").is_file(),
                                    max_usd=r.get("max_usd"), no_manifest=False, llm_user=r.get("llm_user", False))
            code = cmd_simuser(ns) if r.get("mode") == "simuser" else cmd_run(ns)
            if code == LIMIT_EXIT:
                print("usage limit: rerun `bigtest.py plan` later to continue")
                return code
            summary = run_summary(out)
            spent += summary["spent_usd"]
            for g in d.get("gates", []):
                if g["after"] == label and gate_hit(g["stop_if"], summary):
                    print(f"gate after {label}: {g['stop_if']} ({summary}); stopping the plan")
                    return 0
    print(f"plan done, ${spent:.2f}")
    return 0


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    v = sub.add_parser("validate")
    v.add_argument("--with-reference", action="store_true", help="also run the hidden tests against each reference/ (no agents)")
    dr = sub.add_parser("dry-run")
    dr.add_argument("path")
    dr.add_argument("--model", default="claude-haiku-4-5")
    for name in ("run", "simuser"):
        p = sub.add_parser(name)
        p.add_argument("path")
        p.add_argument("--model", required=True)
        p.add_argument("--run", type=int, default=1)
        p.add_argument("--resume", action="store_true")
        p.add_argument("--max-usd", type=float)
        p.add_argument("--no-manifest", action="store_true")
        p.add_argument("--llm-user", action="store_true")
    pl = sub.add_parser("plan")
    pl.add_argument("path")
    a = ap.parse_args()
    return {"validate": cmd_validate, "dry-run": cmd_dry_run, "run": cmd_run, "simuser": cmd_simuser, "plan": cmd_plan}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
