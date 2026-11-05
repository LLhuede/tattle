#!/bin/bash
# Run several headless recordings, N at a time.
#   scripts/run_batch.sh 4 "H27 corpus/bases/G1" "H23 -" ...
#   MODEL=haiku RUN=2 scripts/run_batch.sh 4 "R01 corpus/bases/G2"
# Each job is "<PROMPT_ID> <BASE or ->". All jobs use manifest mode.
# MODEL (optional) picks the agent's model; RUN (default 1) is the run number.
set -u
PAR=$1; shift
cd "$(dirname "$0")/.."
export MODEL="${MODEL:-}" RUN="${RUN:-1}"
printf '%s\n' "$@" | xargs -P "$PAR" -L 1 bash -c '
  id=$0; base=$1
  args=("$id" "$RUN" --manifest)
  [ "$base" != "-" ] && args+=(--base "$base")
  [ -n "$MODEL" ] && args+=(--model "$MODEL")
  python3 scripts/record_headless.py "${args[@]}" > "$HOME/Code/cv-sandbox/_logs/$id-run$RUN-stdout.txt" 2>&1
  echo "$id done (exit $?)"
'
