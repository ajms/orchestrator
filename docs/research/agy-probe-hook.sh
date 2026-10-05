#!/bin/bash
ev=$1; in=$(cat)
ts=$(date +%s.%N)
env | grep -E '^(ORCH|AGY|GEMINI|ANTIGRAVITY|TERM|PWD)' > /dev/null
printf '{"ts":%s,"event":"%s","cwd":"%s","env":%s,"payload":%s}\n' "$ts" "$ev" "$PWD" "$(env | grep -iE 'antigravity|gemini|agy|^PROBE' | python3 -c 'import sys,json;print(json.dumps(sys.stdin.read().splitlines()))')" "${in:-null}" >> /tmp/claude-1000/probe/log/hooks.jsonl
if [ "$ev" = PreToolUse ]; then if [ -f /tmp/claude-1000/probe/log/decision ]; then cat /tmp/claude-1000/probe/log/decision; else echo '{"decision":"ask"}'; fi; exit 0; fi
echo '{}'
