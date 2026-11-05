#!/usr/bin/env python3
"""Every number the README quotes, from this machine's recordings.

    cargo build --release
    python3 scripts/release_numbers.py            # prints a summary
    python3 scripts/release_numbers.py --json     # machine-readable
    python3 scripts/release_numbers.py --from score.json   # a saved `tattle score --json`

Needs the labelled recordings in fixtures/recorded/ (local only, never in
git). Without them it still reports the synthetic numbers. Nothing is
written to the repo; paste the numbers into README.md and
docs/RESULTS.md by hand, with the commit they came from.

Definitions:
  flagged        tattle reports any finding the label doesn't list as tolerated
  hallucinating  the label says the agent claimed something false
  clean          not hallucinating and no labelled defects: any flag here
                 is a false alarm
Not counted: `manifest.missing`. It only fires when the research runner
asked Claude for a summary of its work (`--manifest`); the plugin never asks,
so no user ever sees it. Sessions where it fired are listed separately, with
what each recording's manifests/ folder holds, so a missing or unreadable
summary shows up instead of hiding in the counts.
"Static" counts leave out findings from `tattle verify` records (exec.*).
Sets come from where each prompt lives in corpus/ (e.g. corpus/recall2).
Sets that were measured and then used to fix rules are optimistic; only a
batch measured before any fix is a blind number (CLAUDE.md, lessons).
"""
import json
import pathlib
import platform
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
EXE = ".exe" if platform.system() == "Windows" else ""
CV = REPO / "target" / "release" / f"tattle{EXE}"
SETS = ["prompts", "dev", "heldout", "heldout2", "recall", "recall2", "recall3"]
NOT_COUNTED = {"manifest.missing"}


def prompt_set(prompt_id):
    for s in SETS:
        if (REPO / "corpus" / s / f"{prompt_id}.json").is_file():
            return s
    return "other"


def label_for(name):
    """'R22/run1' -> corpus/labels/R22/run1.json contents."""
    path = REPO / "corpus" / "labels" / f"{name}.json"
    return json.loads(path.read_text()) if path.is_file() else None


def flagged(found, label):
    tolerated = {(t["type"], t.get("file")) for t in label.get("tolerated", [])}
    tolerated_any_file = {t for t, f in tolerated if f is None}
    return [f for f in found if f["check"] not in tolerated_any_file and (f["check"], f.get("file")) not in tolerated]


def manifests_on_disk(name):
    """What the recording's manifests/ folder holds, in a few words."""
    folder = REPO / "fixtures" / "recorded" / name / "manifests"
    if not folder.is_dir():
        return "no manifests/ folder"
    files = sorted(folder.glob("*.json"))
    if not files:
        return "manifests/ is empty"
    bad = []
    for f in files:
        raw = f.read_bytes()
        try:
            json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as e:
            bad.append(f"{f.name}: {'starts with a byte-order mark' if raw.startswith(b'\xef\xbb\xbf') else e}")
    return f"{len(files)} file(s), all readable" if not bad else f"{len(files)} file(s), unreadable: " + "; ".join(bad)


def hook_timings():
    """cv's own time per recorded hook event, from the recordings."""
    ms = []
    for events in (REPO / "fixtures" / "recorded").glob("*/run*/events.jsonl"):
        for line in events.read_text(errors="replace").splitlines():
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if isinstance(e.get("cv_ms"), (int, float)) and e.get("event") not in ("SessionStart", "UserPromptSubmit", "Stop"):
                ms.append(e["cv_ms"])
    ms.sort()
    return {"events": len(ms), "median_ms": ms[len(ms) // 2] if ms else None, "p95_ms": ms[int(len(ms) * 0.95)] if ms else None}


def main():
    if not CV.is_file():
        sys.exit(f"build first: cargo build --release ({CV} not found)")
    if "--from" in sys.argv:
        score = json.loads(pathlib.Path(sys.argv[sys.argv.index("--from") + 1]).read_text(encoding="utf-8-sig"))
    else:
        out = subprocess.run([str(CV), "score", "--json"], cwd=REPO, capture_output=True, text=True)
        if out.returncode != 0:
            sys.exit(out.stderr)
        score = json.loads(out.stdout)
    from_catalog = subprocess.run(["git", "grep", "-c", 'CheckInfo { id: "', "--", "crates/checks/src/catalog.rs"], cwd=REPO, capture_output=True, text=True).stdout
    checks_total = int(from_catalog.strip().rsplit(":", 1)[-1] or 0)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO, capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "status", "--porcelain", "--untracked-files=no"], cwd=REPO, capture_output=True, text=True).stdout.strip())

    # Synthetic numbers from the synthetic fixtures alone: `tattle score`'s
    # own per-check table and "false alarms on clean" also include the real
    # labelled sessions.
    per_check = {}
    synthetic_clean_fps = 0
    for r in score["results"]:
        if r["kind"] not in ("mutation", "synthetic-clean"):
            continue
        missed = [e["check"] for e in r["missed"]]
        for e in r["expected"]:
            c = per_check.setdefault(e["check"], {"tp": 0, "fn": 0, "fp": 0})
            if e["check"] in missed:
                missed.remove(e["check"])
                c["fn"] += 1
            else:
                c["tp"] += 1
        for f in r["false_positives"]:
            per_check.setdefault(f["check"], {"tp": 0, "fn": 0, "fp": 0})["fp"] += 1
        if not r["expected"]:
            synthetic_clean_fps += len(r["false_positives"])
    synth = [c for c in per_check.values() if c["tp"] + c["fn"] > 0]
    perfect = [c for c in synth if c["fn"] == 0 and c["fp"] == 0]
    real_only_fps = score["false_alarms_on_clean"] - synthetic_clean_fps

    by_set = {}
    for r in score["results"]:
        if r["kind"] != "real-labelled":
            continue
        label = label_for(r["name"])
        if label is None:
            continue
        s = by_set.setdefault(prompt_set(label["prompt_id"]), {"sessions": 0, "hallucinating": 0, "hallucinating_flagged": 0, "hallucinating_flagged_static": 0, "clean": 0, "clean_flagged": 0, "false_alarm_sessions": [], "not_counted": []})
        s["sessions"] += 1
        all_hits = flagged(r["found"], label)
        hit = [f for f in all_hits if f["check"] not in NOT_COUNTED]
        left_out = sorted({f["check"] for f in all_hits if f["check"] in NOT_COUNTED})
        if left_out and not hit:
            s["not_counted"].append({"name": r["name"], "findings": left_out, "manifests": manifests_on_disk(r["name"]), "hallucinated": bool(label.get("hallucinated"))})
        if label.get("hallucinated"):
            s["hallucinating"] += 1
            s["hallucinating_flagged"] += bool(hit)
            s["hallucinating_flagged_static"] += any(not f["check"].startswith("exec.") for f in hit)
        elif not label.get("defects"):
            s["clean"] += 1
            if hit:
                s["clean_flagged"] += 1
                s["false_alarm_sessions"].append({"name": r["name"], "findings": [f["check"] for f in hit]})
    total = {k: sum(s[k] for s in by_set.values()) for k in ("sessions", "hallucinating", "hallucinating_flagged", "hallucinating_flagged_static", "clean", "clean_flagged")}

    binary = CV.stat().st_size
    numbers = {
        "commit": commit + ("-dirty" if dirty else ""),
        "cv_version": score.get("cv_version"),
        "system": f"{platform.system()} {platform.machine()}",
        "checks_in_catalog": checks_total,
        "synthetic": {
            "mutations": score["fixtures"]["mutations"],
            "checks_with_mutations": len(synth),
            "checks_at_1_00": len(perfect),
            "false_alarms_on_clean": synthetic_clean_fps,
            "score_line_false_alarms": score["false_alarms_on_clean"],
            "deterministic": f'{score["determinism"]["identical"]}/{score["determinism"]["total"]}',
        },
        "real": {"total": total, "by_set": by_set},
        "speed": {"check_ms_mean": score["checker_millis"]["mean"], "check_ms_max": score["checker_millis"]["max"], "hook": hook_timings(), "binary_mb": round(binary / 1e6, 1)},
    }
    if "--json" in sys.argv:
        print(json.dumps(numbers, indent=2))
        return

    n = numbers
    if "--from" in sys.argv:
        # The score came from whatever binary wrote the file, not this checkout.
        print(f"Score from {sys.argv[sys.argv.index('--from') + 1]} (tattle {n['cv_version']}; the commit it was built from isn't recorded), counted by this script at {n['commit']} on {n['system']}\n")
    else:
        print(f"tattle {n['cv_version']} at {n['commit']} on {n['system']}\n")
    print(f"Checks in the catalog: {n['checks_in_catalog']}")
    sy = n["synthetic"]
    print(f"Synthetic: {sy['mutations']} mutations; {sy['checks_at_1_00']}/{sy['checks_with_mutations']} checks at precision and recall 1.00; "
          f"{sy['false_alarms_on_clean']} false alarms on clean sessions; deterministic {sy['deterministic']} (all fixtures)")
    if sy["score_line_false_alarms"] != sy["false_alarms_on_clean"]:
        print(f"  (`tattle score` prints {sy['score_line_false_alarms']} false alarms on clean fixtures: that line also counts real labelled sessions with nothing expected, so {real_only_fps} of them come from the real sessions below.)")
    t = n["real"]["total"]
    if t["sessions"] == 0:
        print("\nReal sessions: none found (fixtures/recorded/ is empty on this machine).")
    else:
        print(f"\nReal labelled sessions: {t['sessions']}")
        print(f"  hallucinating sessions flagged: {t['hallucinating_flagged']}/{t['hallucinating']} (static checks alone: {t['hallucinating_flagged_static']})")
        print(f"  clean sessions with a false alarm: {t['clean_flagged']}/{t['clean']}")
        print("\n  set        sessions  halluc. flagged (static)  clean w/ false alarm")
        for name in SETS + ["other"]:
            s = n["real"]["by_set"].get(name)
            if s:
                flagged_txt = f"{s['hallucinating_flagged']}/{s['hallucinating']} ({s['hallucinating_flagged_static']})"
                print(f"  {name:<10} {s['sessions']:>8}  {flagged_txt:<24} {s['clean_flagged']:>3}/{s['clean']}")
        for name, s in n["real"]["by_set"].items():
            for fa in s["false_alarm_sessions"]:
                print(f"  false alarm: {fa['name']}: {', '.join(fa['findings'])}")
        left_out = [x for s in n["real"]["by_set"].values() for x in s["not_counted"]]
        if left_out:
            print(f"\n  Not counted ({len(left_out)} sessions flagged only by {', '.join(sorted(NOT_COUNTED))}: the research runner's")
            print("  request for a work summary, which the plugin never makes). What each recording holds:")
            for x in left_out:
                kind = "hallucinating" if x["hallucinated"] else "clean"
                print(f"  - {x['name']} ({kind}): {x['manifests']}")
    sp = n["speed"]
    h = sp["hook"]
    print(f"\nSpeed: check {sp['check_ms_mean']} ms mean, {sp['check_ms_max']} ms max per recording; "
          f"hook median {h['median_ms']} ms (p95 {h['p95_ms']} ms) over {h['events']} tool events; binary {sp['binary_mb']} MB")
    print("\nThe recall sets were measured, then used to fix rules: their numbers are optimistic. Quote a blind number only from a batch measured before any fix.")


if __name__ == "__main__":
    main()
