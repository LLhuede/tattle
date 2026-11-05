#!/usr/bin/env python3
"""Record one corpus run with headless Claude Code (no human at the keyboard).

    scripts/record_headless.py G1 1
    scripts/record_headless.py E2 1 --base corpus/bases/G1

Steps: `tattle sandbox init` a fresh folder in ~/Code/cv-sandbox/, send each
prompt turn with `claude -p` (later turns continue the same session), then
`tattle record export` into fixtures/recorded/<ID>/run<N>/.

Safety: the agent may edit files and run only allow-listed commands (npm,
npx, node, git and basic file commands). No blanket permission, no `rm`.

Run from the repo root after `cargo build --release`.
"""
import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

REPO = pathlib.Path(__file__).resolve().parent.parent
CV = REPO / "target/release/tattle"
SANDBOX_ROOT = pathlib.Path.home() / "Code/cv-sandbox"
ALLOWED = [
    "Read", "Edit", "Write", "MultiEdit", "Glob", "Grep", "TodoWrite",
    "Bash(npm:*)", "Bash(npx:*)", "Bash(node:*)", "Bash(git:*)",
    "Bash(ls:*)", "Bash(cat:*)", "Bash(mkdir:*)", "Bash(pwd)", "Bash(echo:*)",
    "Bash(head:*)", "Bash(tail:*)", "Bash(wc:*)", "Bash(grep:*)", "Bash(find:*)",
    "Bash(sed:*)", "Bash(touch:*)", "Bash(cp:*)", "Bash(mv:*)", "Bash(cd:*)",
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("prompt_id")
    ap.add_argument("run", type=int)
    ap.add_argument("--base", help="starting app folder for edit prompts")
    ap.add_argument("--model", help="model for the headless session")
    ap.add_argument("--timeout", type=int, default=1800, help="seconds per turn")
    ap.add_argument("--manifest", action="store_true", help="ask the agent for a manifest (Phase 6b)")
    args = ap.parse_args()

    candidates = [REPO / f"corpus/{d}/{args.prompt_id}.json" for d in ("prompts", "heldout", "heldout2", "dev", "recall", "recall2", "recall3")]
    prompt_file = next((c for c in candidates if c.is_file()), None)
    if prompt_file is None:
        sys.exit(f"no prompt file for {args.prompt_id}")
    prompt = json.loads(prompt_file.read_text())
    sandbox = SANDBOX_ROOT / f"{args.prompt_id}-run{args.run}"
    logs = SANDBOX_ROOT / "_logs"
    logs.mkdir(parents=True, exist_ok=True)
    out = REPO / f"fixtures/recorded/{args.prompt_id}/run{args.run}"

    # Hooks call a frozen copy of tattle, so rebuilding target/release/tattle can't
    # affect a session that's still running.
    digest = hashlib.sha256(CV.read_bytes()).hexdigest()[:12]
    frozen = SANDBOX_ROOT / "_bin" / f"cv-{digest}"
    if not frozen.exists():
        frozen.parent.mkdir(parents=True, exist_ok=True)
        # Per-process temp name: several runs may start at once.
        tmp = frozen.parent / f".{frozen.name}.{os.getpid()}.tmp"
        shutil.copy2(CV, tmp)
        os.replace(tmp, frozen)  # atomic; another run may have won, which is fine
    init = [str(frozen), "sandbox", "init", str(sandbox)]
    if args.base:
        init += ["--base", str(REPO / args.base)]
    if args.manifest:
        init += ["--manifest"]
    subprocess.run(init, check=True, stdout=subprocess.DEVNULL)

    # A prompt can take commands away (recall study: "you can't run the tests").
    allowed = [t for t in ALLOWED if t not in set(prompt.get("deny_tools", []))]
    summary = {"prompt_id": args.prompt_id, "run": args.run, "model": args.model or "default",
               "denied_tools": prompt.get("deny_tools", []) + prompt.get("disallowed_tools", []), "turns": []}
    for i, turn in enumerate(prompt["turns"]):
        cmd = ["claude", "-p", turn, "--output-format", "json",
               "--permission-mode", "acceptEdits", "--allowedTools", *allowed]
        if i > 0:
            cmd.insert(1, "--continue")
        if args.model:
            cmd += ["--model", args.model]
        if prompt.get("disallowed_tools"):
            cmd += ["--disallowedTools", *prompt["disallowed_tools"]]
        started = time.time()
        res = subprocess.run(cmd, cwd=sandbox, capture_output=True, text=True, timeout=args.timeout)
        (logs / f"{args.prompt_id}-run{args.run}-turn{i + 1}.json").write_text(res.stdout + "\n" + res.stderr)
        try:
            result = json.loads(res.stdout)
        except json.JSONDecodeError:
            result = {"raw": res.stdout[-2000:], "stderr": res.stderr[-2000:]}
        summary["turns"].append({
            "seconds": round(time.time() - started),
            "exit": res.returncode,
            "num_turns": result.get("num_turns"),
            "cost_usd": result.get("total_cost_usd"),
            "is_error": result.get("is_error"),
            "denials": len(result.get("permission_denials") or []),
            "result_tail": (result.get("result") or "")[-400:],
        })

    exp = subprocess.run(
        [str(frozen), "record", "export", "--project", str(sandbox), "--prompt-id", args.prompt_id,
         "--run", str(args.run), "--out", str(out)],
        cwd=REPO, capture_output=True, text=True)
    summary["export"] = (exp.stdout + exp.stderr).strip()
    summary["export_ok"] = exp.returncode == 0
    (logs / f"{args.prompt_id}-run{args.run}-summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))
    return 0 if exp.returncode == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
