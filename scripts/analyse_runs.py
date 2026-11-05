#!/usr/bin/env python3
"""Measure the checker on real recordings whose prompts have expected facts.

    scripts/analyse_runs.py [ID ...]      (default: every fixtures/recorded/*/run1)

For each recording:
- facts: expected facts confirmed by the prompt rules alone ("lexicon"),
  and by prompt rules plus a verified manifest ("+manifest")
- manifest: written? requirements backed by the code? listed items that
  check out?
- verdicts: verified / violated / unknown counts, and every finding
Writes harness/results/runs-<date>.json and prints a summary.
"""
import datetime
import json
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
CV = REPO / "target/release/tattle"
STOP = {"the", "a", "an", "add", "to", "for", "of", "and", "in", "on", "it", "make", "use", "with", "dont", "not",
        "no", "new", "any", "must", "should", "be", "is", "are", "that", "this", "all", "only", "keep", "change"}


def words(s):
    return {w.rstrip("s") for w in re.findall(r"[a-z0-9]+", (s or "").lower()) if w not in STOP and len(w) > 2}


def prompt_file(pid):
    for d in ("prompts", "heldout", "heldout2", "dev"):
        p = REPO / f"corpus/{d}/{pid}.json"
        if p.is_file():
            return d, json.loads(p.read_text())
    return None, None


def fact_words(exp):
    p = exp.get("params") or {}
    out = words(exp.get("description"))
    for k in ("value", "target_hint", "old", "new"):
        v = p.get(k)
        if isinstance(v, list):
            for x in v:
                out |= words(str(x))
        elif v is not None:
            out |= words(str(v))
    for t in p.get("targets") or []:
        out |= words(t)
    return out


def analyse(rec_dir):
    pid = rec_dir.parent.name
    corpus, prompt = prompt_file(pid)
    out = json.loads(subprocess.run([str(CV), "check", "--json", str(rec_dir)], capture_output=True, text=True, check=True).stdout)
    intent = [r for r in out["intent"] if "fact" in r]
    manifest = next((r["manifest"] for r in out["intent"] if "manifest" in r), None)
    expected = [e for e in (prompt or {}).get("expected_facts", []) if not e.get("implicit")]

    def lexicon_confirms(exp):
        # an extracted fact of the same kind whose words overlap, verified by the code
        fw = fact_words(exp)
        for r in intent:
            f = r["fact"]
            if f["kind"] != exp["kind"] or f["polarity"] != exp["polarity"] or r["verdict"] != "verified":
                continue
            if r["evidence"].startswith("manifest:"):
                continue
            got = words(f["source"]["text"]) | words(json.dumps(f.get("value"))) | {w for t in f["targets"] for w in words(t["text"])}
            if fw & got or exp["kind"].startswith("constraint"):
                return True
        return False

    def manifest_confirms(exp):
        if not manifest:
            return False
        fw = fact_words(exp)
        return any(r["backed"] and fw & words(r["request"]) for r in manifest)

    lex = [e["id"] for e in expected if lexicon_confirms(e)]
    both = [e["id"] for e in expected if lexicon_confirms(e) or manifest_confirms(e)]
    items_ok = sum(len(r["supported"]) for r in (manifest or []))
    items_bad = sum(len(r["unsupported"]) for r in (manifest or []))
    events = [json.loads(l) for l in open(rec_dir / "events.jsonl")]
    return {
        "id": pid,
        "corpus": corpus,
        "expected": len(expected),
        "confirmed_lexicon": lex,
        "confirmed_with_manifest": both,
        "not_confirmed": [e["id"] for e in expected if e["id"] not in both],
        "manifest_requested": any(e.get("manifest_requested") for e in events),
        "manifest_written": manifest is not None,
        "requirements": len(manifest or []),
        "requirements_backed": sum(1 for r in (manifest or []) if r["backed"]),
        "manifest_items_ok": items_ok,
        "manifest_items_unsupported": items_bad,
        "unsupported_items": [f"{r['request'][:60]} → {u}" for r in (manifest or []) for u in r["unsupported"]],
        "verdicts": {v: sum(1 for r in intent if r["verdict"] == v) for v in ("verified", "violated", "unknown")},
        "violations": [f"{r['fact']['kind']}: \"{r['fact']['source']['text']}\" — {r['evidence']}" for r in intent if r["verdict"] == "violated"],
        "findings": [f"{f['check']} {f['file'] or ''} {f['message'][:140]}" for f in out["findings"]],
    }


def main():
    ids = sys.argv[1:]
    dirs = sorted(p for p in (REPO / "fixtures/recorded").glob("*/run1") if not ids or p.parent.name in ids)
    rows = [analyse(d) for d in dirs]
    tot = lambda k: sum(r[k] if isinstance(r[k], int) else len(r[k]) for r in rows)
    with_exp = [r for r in rows if r["expected"]]
    summary = {
        "recordings": len(rows),
        "expected_facts": sum(r["expected"] for r in with_exp),
        "confirmed_lexicon": sum(len(r["confirmed_lexicon"]) for r in with_exp),
        "confirmed_with_manifest": sum(len(r["confirmed_with_manifest"]) for r in with_exp),
        "manifests_requested": sum(r["manifest_requested"] for r in rows),
        "manifests_written": sum(r["manifest_written"] and r["manifest_requested"] for r in rows),
        "requirements": tot("requirements"),
        "requirements_backed": tot("requirements_backed"),
        "manifest_items_ok": tot("manifest_items_ok"),
        "manifest_items_unsupported": tot("manifest_items_unsupported"),
        "findings": tot("findings"),
    }
    out = REPO / f"harness/results/runs-{datetime.date.today()}.json"
    out.write_text(json.dumps({"summary": summary, "recordings": rows}, indent=2) + "\n")
    for r in rows:
        print(f"{r['id']:<4} {r['corpus'] or '-':<8} facts {len(r['confirmed_lexicon'])}/{r['expected']} lexicon, "
              f"{len(r['confirmed_with_manifest'])}/{r['expected']} +manifest | manifest "
              f"{'yes' if r['manifest_written'] else ('MISSING' if r['manifest_requested'] else '-')} "
              f"backed {r['requirements_backed']}/{r['requirements']} items {r['manifest_items_ok']} ok {r['manifest_items_unsupported']} bad | "
              f"verdicts {r['verdicts']} | findings {len(r['findings'])}")
        for v in r["violations"]:
            print("      violated:", v)
        for f in r["findings"]:
            print("      finding:", f)
        for u in r["unsupported_items"]:
            print("      unsupported claim:", u)
    print(json.dumps(summary, indent=2))
    print(f"saved {out.relative_to(REPO)}")


if __name__ == "__main__":
    main()
