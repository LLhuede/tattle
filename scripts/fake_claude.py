#!/usr/bin/env python3
"""Stand-in for `claude -p` to test scripts/bigtest.py without any model.

    BIGTEST_CLAUDE=scripts/fake_claude.py FAKE_APP=<dir> scripts/bigtest.py run ...

On each call it copies FAKE_APP (e.g. a suite's reference/) into the working
directory, then answers in Claude Code's JSON output format with a fixed
cost. FAKE_LIMIT_AT=<n> makes call n report a usage limit, to test --resume.
"""
import json
import os
import pathlib
import shutil
import sys

counter = pathlib.Path(".fake_claude_calls")
n = int(counter.read_text()) + 1 if counter.exists() else 1
counter.write_text(str(n))
if os.environ.get("FAKE_LIMIT_AT") == str(n):
    print(json.dumps({"is_error": True, "result": "You've hit your session limit · resets 10pm", "total_cost_usd": 0}))
    sys.exit(0)
src = os.environ.get("FAKE_APP")
if src:
    shutil.copytree(src, ".", dirs_exist_ok=True, ignore=shutil.ignore_patterns("node_modules", "dist"))
prompt = sys.argv[sys.argv.index("-p") + 1] if "-p" in sys.argv else ""
print(json.dumps({"is_error": False, "num_turns": 3, "total_cost_usd": 0.05,
                  "result": f"Done (fake call {n}). All tests pass. Prompt began: {prompt[:60]!r}"}))
