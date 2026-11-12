#!/usr/bin/env bash
# Compare two commits on the call benchmark (examples/calls.rs), fairly on a noisy machine:
# build both in their own worktrees, then alternate their runs and keep each line's best.
#
#   scripts/ab.sh <before> <after> [rounds]     e.g. scripts/ab.sh vm-redesign-base HEAD 4
#
# The benchmark file is taken from <after>, so both commits run the same programs.
set -euo pipefail

before=${1:?usage: scripts/ab.sh <before> <after> [rounds]}
after=${2:?usage: scripts/ab.sh <before> <after> [rounds]}
rounds=${3:-4}
repo=$(git rev-parse --show-toplevel)
work=$(mktemp -d "${TMPDIR:-/tmp}/eelisp-ab.XXXXXX")
trap 'git -C "$repo" worktree remove --force "$work/a" >/dev/null 2>&1 || true;
      git -C "$repo" worktree remove --force "$work/b" >/dev/null 2>&1 || true; rm -rf "$work"' EXIT

bench=$(git -C "$repo" show "$after:examples/calls.rs")
for side in a b; do
  rev=$([ $side = a ] && echo "$before" || echo "$after")
  git -C "$repo" worktree add -q --detach "$work/$side" "$rev"
  printf '%s\n' "$bench" > "$work/$side/examples/calls.rs"
  (cd "$work/$side" && CARGO_TARGET_DIR="$work/target-$side" cargo build -q --release --example calls)
done

for _ in $(seq "$rounds"); do
  "$work/target-a/release/examples/calls" >> "$work/a.txt"
  "$work/target-b/release/examples/calls" >> "$work/b.txt"
done

python3 - "$work/a.txt" "$work/b.txt" "$before" "$after" <<'PY'
import re, sys, collections
def best(path):
    d = collections.OrderedDict()
    for line in open(path):
        m = re.match(r'(.+?)\s+([\d.]+) ms', line)
        if m:
            k = m.group(1).strip()
            d[k] = min(d.get(k, float('inf')), float(m.group(2)))
    return d
a, b = best(sys.argv[1]), best(sys.argv[2])
print(f"{'':18} {sys.argv[3][:12]:>12} {sys.argv[4][:12]:>12}")
for k in a:
    print(f"{k:18} {a[k]:9.1f} ms {b[k]:9.1f} ms   {a[k] / b[k]:.3f}x")
PY
