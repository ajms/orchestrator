import json
import os
import select
import subprocess
import sys
import time

prompt = sys.argv[1]
answers = json.loads(sys.argv[2])
abort_after = float(sys.argv[3])
pi_args = sys.argv[4:]
rec = os.environ["PI_REC_OUT"].replace("events.jsonl", "rpc_stdin.jsonl")

proc = subprocess.Popen(["pi", "-e", os.environ["TMPDIR"] + "/pi-rec/recorder.ts", "--mode", "rpc", *pi_args],
                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=sys.stderr)


def send(obj):
    line = json.dumps(obj)
    with open(rec, "a") as f:
        f.write(line + "\n")
    proc.stdin.write((line + "\n").encode())
    proc.stdin.flush()


send({"id": "req-1", "type": "prompt", "message": prompt})
buf = b""
last = time.time()
aborted = False
deadline = time.time() + 90
while time.time() < deadline:
    r, _, _ = select.select([proc.stdout], [], [], 0.2)
    if r:
        chunk = os.read(proc.stdout.fileno(), 65536)
        if not chunk:
            break
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            sys.stdout.write(line.decode() + "\n")
            sys.stdout.flush()
            last = time.time()
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if ev.get("type") == "extension_ui_request" and ev.get("method") in ("select", "input", "confirm", "editor"):
                ans = answers.pop(0) if answers else "CANCEL"
                if ans == "CANCEL":
                    send({"type": "extension_ui_response", "id": ev["id"], "cancelled": True})
                elif ev["method"] == "confirm":
                    send({"type": "extension_ui_response", "id": ev["id"], "confirmed": bool(ans)})
                else:
                    send({"type": "extension_ui_response", "id": ev["id"], "value": ans})
            if ev.get("type") == "agent_settled":
                deadline = 0
    elif abort_after > 0 and not aborted and time.time() - last > abort_after:
        aborted = True
        send({"id": "req-abort", "type": "abort"})
time.sleep(0.5)
proc.stdin.close()
try:
    proc.wait(timeout=10)
except subprocess.TimeoutExpired:
    proc.kill()
