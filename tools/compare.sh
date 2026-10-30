#!/usr/bin/env bash
# Compares SmartCompression with 7-Zip and WinRAR on the files in $IN. Run from the repo root (needs awk, cmp, find).
# Prints: name|archive bytes|compress s|extract s|round-trip check. Override IN, W, Z, R, SC via environment.
IN=${IN:-/tmp/bin}; W=${W:-/tmp/cmpw}; Z=${Z:-/c/Program Files/7-Zip/7z.exe}; R=${R:-/c/Program Files/WinRAR/Rar.exe}
SC=${SC:-target/release/smartcomp.exe}
now(){ date +%s.%N; }
run(){ # name ext create-template extract-template ; @A@ = archive path, @O@ = out dir
  name=$1; A=$W/a.$2; O=$W/out
  c=${3//@A@/$A}; c=${c//@O@/$O}; x=${4//@A@/$A}; x=${x//@O@/$O}
  rm -rf $W; mkdir -p $O
  t0=$(now); eval "$c" >/dev/null 2>&1; t1=$(now)
  eval "$x" >/dev/null 2>&1; t2=$(now)
  ok=OK; for f in $IN/*; do b=$(basename $f); o=$(find $O -name "$b" -type f | head -1); [ -n "$o" ] && cmp -s "$f" "$o" || ok=FAIL; done
  echo "$name|$(stat -c %s $A)|$(awk "BEGIN{printf \"%.1f\", $t1-$t0}")|$(awk "BEGIN{printf \"%.1f\", $t2-$t1}")|$ok"
}
cd "$(pwd)" ; SC=$(realpath "$SC")
for L in medium ultra highultra; do run "SmartCompression $L" sca "'$SC' -q -a @A@ $IN --level $L" "'$SC' -q -e @A@ -o @O@"; done
for L in 1 5 9; do run "7-Zip -mx$L" 7z "'$Z' a -mx$L -y @A@ $IN" "'$Z' x -y -o@O@ @A@"; done
for L in 1 3 5; do run "WinRAR -m$L" rar "'$R' a -m$L -ma5 -ep1 -r -y -idq @A@ $IN/*" "'$R' x -y -idq @A@ @O@/"; done
