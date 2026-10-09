#!/bin/sh
# Runs the dynamic desktops KWin script against a small fake workspace (needs mujs). From the repo root: sh tests/run-dynamic-desktops-tests.sh
set -e
js=data/dynamic-desktops/contents/code/main.js
tmp=$(mktemp -d)
for t in dynamic_desktops_test dynamic_desktops_manual_test; do
  awk -v script="$js" '/\/\/SCRIPT\/\// { while ((getline line < script) > 0) print line; next } { print }' tests/$t.js > $tmp/$t.js
  echo "== $t"; mujs $tmp/$t.js
done
rm -rf $tmp
