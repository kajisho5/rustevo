#!/usr/bin/env bash
# Everything that must hold, in one run. Exits non-zero on the first failure.
set -euo pipefail
cd "$(dirname "$0")/.."
say() { printf '\n== %s\n' "$1"; }
fail() { echo "NG: $1"; exit 1; }

say "unit tests"
cargo test --quiet -p evo-core

say "build the tool"
cargo build --quiet --release -p evo-cli
E="$PWD/target/release/evo"

say "the ledger verifies from the stored files"
"$E" verify apps/pomodoro

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

say "one changed character in a journal is caught"
cp -r apps/pomodoro "$T/a"
J=$(ls "$T"/a/generations/0003/journals/*.jsonl | head -1)
sed -i '0,/"remaining":1497/s//"remaining":1498/' "$J"
"$E" verify "$T/a" >/dev/null 2>"$T/a.err" && fail "a changed journal passed"
grep -q "0003" "$T/a.err" || fail "the changed generation was not named"
echo "caught: $(head -1 "$T/a.err")"

say "a verdict flipped from rejected to sealed is caught"
cp -r apps/pomodoro "$T/b"
sed -i 's/"status": "rejected"/"status": "sealed"/' "$T/b/generations/0004/verdict.json"
"$E" verify "$T/b" >/dev/null 2>"$T/b.err" && fail "a flipped verdict passed"
echo "caught: $(head -1 "$T/b.err")"

say "a ledger line removed is caught"
cp -r apps/pomodoro "$T/c"
sed -i '4d' "$T/c/ledger.jsonl"
"$E" verify "$T/c" >/dev/null 2>"$T/c.err" && fail "a removed ledger line passed"
echo "caught: $(head -1 "$T/c.err")"

say "a swapped logic.wasm is caught"
cp -r apps/pomodoro "$T/d"
cp "$T/d/generations/0004/logic.wasm" "$T/d/generations/0005/logic.wasm"
"$E" verify "$T/d" >/dev/null 2>"$T/d.err" && fail "a swapped logic passed"
echo "caught: $(head -1 "$T/d.err")"

say "logic that reads the wall clock is stopped, and nothing is recorded"
cp -r apps/badclock "$T/e"
rm -rf "$T/e/pending" "$T/e/build"
mkdir -p "$T/e/build"
"$E" build apps/badclock >/dev/null
cp apps/badclock/build/plan.json apps/badclock/build/logic.wasm "$T/e/build/"
"$E" run "$T/e" s >/dev/null
"$E" tap "$T/e" s now >/dev/null 2>"$T/e.err" && fail "the clock read went through"
[ "$(wc -l < "$T/e/pending/s.jsonl")" = "1" ] || fail "a failed event was recorded"
echo "stopped: $(head -1 "$T/e.err")"

say "a journal with saved results replays with no store present"
cp -r apps/pomodoro "$T/f"
mkdir -p "$T/f/build"
cp "$T/f/generations/0012/plan.json" "$T/f/generations/0012/logic.wasm" "$T/f/build/"
grep -q '"result"' "$T/f/generations/0012/journals/saved.jsonl" || fail "no recorded result in the journal"
"$E" replay "$T/f" "$T/f/generations/0012/journals/saved.jsonl"

say "the logic rebuilt from each generation's stored source is the stored wasm"
if ! "$E" verify apps/pomodoro --rebuild > "$T/r.out" 2>&1; then
  cat "$T/r.out"
  K=apps/pomodoro/build/rebuild/0001.wasm
  if [ -f "$K" ]; then
    echo "-- generation 0001: sizes stored / rebuilt: $(wc -c < apps/pomodoro/generations/0001/logic.wasm) / $(wc -c < "$K")"
    echo "-- paths inside (< stored, > rebuilt):"
    diff <(strings -n 6 apps/pomodoro/generations/0001/logic.wasm | grep -E '\.rs' | sort -u) <(strings -n 6 "$K" | grep -E '\.rs' | sort -u) || true
  fi
  fail "a rebuild from stored source did not match"
fi
tail -1 "$T/r.out"

say "a rebuild under different build settings is caught"
mkdir "$T/g"
cp -r Cargo.toml Cargo.lock rust-toolchain.toml crates apps "$T/g/"
rm -rf "$T/g/apps/pomodoro/build" "$T/g/crates/target"
sed -i 's/^opt-level = "s"/opt-level = "z"/' "$T/g/Cargo.toml"
grep -q '^opt-level = "z"' "$T/g/Cargo.toml" || fail "the test could not change the build settings"
"$E" verify "$T/g/apps/pomodoro" --rebuild >/dev/null 2>"$T/g.err" && fail "a rebuild with other settings passed"
grep -q "0001" "$T/g.err" || fail "the mismatching generation was not named"
echo "caught: $(grep -m1 '^NG' "$T/g.err")"

say "all checks passed"
