#!/bin/sh
# Refresh the Vencord bundle that gets compiled into the binary.
set -eu
cd "$(dirname "$0")/../vendor/vencord"
BASE=https://github.com/Vendicated/Vencord/releases/download/devbuild
for f in browser.js browser.css browser.js.LEGAL.txt; do
    curl -fsSL -o "$f.tmp" "$BASE/$f" && mv "$f.tmp" "$f"
done
head -1 browser.js
