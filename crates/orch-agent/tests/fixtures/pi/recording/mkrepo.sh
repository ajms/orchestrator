#!/bin/sh
set -e
rm -rf "$1"; mkdir -p "$1"; cd "$1"
git init -q -b main
git config user.name dev; git config user.email dev@example.com
printf '# demo\n\nA one-line demo readme.\n' > README.md
printf 'def greet():\n    return "hi"\n' > greet.py
git add . && git commit -q -m init
