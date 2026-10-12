#!/bin/bash
# usage: run.sh <name> <repo> -- pi args...
R=$TMPDIR/pi-rec
name=$1; repo=$2; shift 3
mkdir -p $R/out/$name
python3 -I $R/bin/fake_openai.py 18080 $R/out/$name/model.log & SRV=$!
for i in 1 2 3 4 5 6 7 8 9 10; do curl -s -o /dev/null http://127.0.0.1:18080/v1/models && break; sleep 0.2; done
export PI_CODING_AGENT_DIR=$R/agent PI_OFFLINE=1 PI_SKIP_VERSION_CHECK=1 PI_TELEMETRY=0 PI_REC_OUT=$R/out/$name/events.jsonl
cd $repo
if [ -n "$RPC_SCRIPT" ]; then
  python3 -I $RPC_SCRIPT "$@" > $R/out/$name/stdout.jsonl 2> $R/out/$name/stderr.txt
else
  timeout 120 pi -e $R/recorder.ts "$@" > $R/out/$name/stdout.jsonl 2> $R/out/$name/stderr.txt < /dev/null
fi
echo "exit=$?" > $R/out/$name/exit.txt
kill $SRV
