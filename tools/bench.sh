#!/usr/bin/env bash
# Usage: tools/bench.sh <level> [input-dir]   (run from smartcomp-rust/)
# Creates an archive, extracts it elsewhere, compares every file, prints sizes and times.
set -u
LEVEL=${1:-medium}
IN=${2:-../silesia}
BIN=target/release/smartcomp.exe
WORK=${TMPDIR:-/tmp}/sc-bench-$LEVEL
rm -rf "$WORK" && mkdir -p "$WORK/out"

t0=$SECONDS
"$BIN" -a "$WORK/a.sca" "$IN" --level "$LEVEL" -v 2> "$WORK/create.log" || { echo "create FAILED"; tail -5 "$WORK/create.log"; exit 1; }
t1=$SECONDS
"$BIN" -e "$WORK/a.sca" -o "$WORK/out" > /dev/null || { echo "extract FAILED"; exit 1; }
t2=$SECONDS

bad=0
for f in "$IN"/*; do
  n=$(basename "$f")
  cmp -s "$f" "$WORK/out/$(basename "$IN")/$n" || { echo "MISMATCH: $n"; bad=1; }
done
orig=$(du -cb "$IN"/* | tail -1 | cut -f1)
size=$(stat -c %s "$WORK/a.sca")
echo "level=$LEVEL original=$orig archive=$size create=$((t1-t0))s extract=$((t2-t1))s verify=$([ $bad = 0 ] && echo OK || echo FAILED)"
cat "$WORK/create.log"
