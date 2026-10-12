import json
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[2] if len(sys.argv) > 2 else None


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def scenario_and_step(messages):
    tag = None
    for m in messages:
        if m.get("role") == "user":
            found = re.search(r"\[\[scenario:([a-z0-9-]+)\]\]", text_of(m.get("content")))
            if found:
                tag = found.group(1)
                break
    step = sum(1 for m in messages if m.get("role") == "assistant")
    return tag or "basic", step


def call(cid, name, args):
    return {"id": cid, "name": name, "args": args}


def plan(tag, step, messages):
    last_tool = [m for m in messages if m.get("role") == "tool"]
    if tag == "basic":
        return {"thinking": "The user wants a greeting.", "text": "Hello from the fake model."}
    if tag == "tools":
        if step == 0:
            return {"text": "Writing the file.", "calls": [call("call_bash_1", "bash", {"command": "echo hello > hello.txt && ls"})]}
        return {"text": "Wrote hello.txt."}
    if tag == "error":
        return {"http_error": 400}
    if tag == "question-single":
        if step == 0:
            return {"calls": [call("call_q_1", "question", {"question": "Which database should the service use?", "options": ["Postgres", "SQLite"]})]}
        return {"text": "Using " + text_of(last_tool[-1].get("content")) + "."}
    if tag == "question-multi":
        if step == 0:
            return {"calls": [call("call_q_2", "question", {"question": "Which checks should run?", "options": ["lint", "test", "typecheck"], "multiSelect": True})]}
        return {"text": "Answer: " + text_of(last_tool[-1].get("content"))}
    if tag == "question-text":
        if step == 0:
            return {"calls": [call("call_q_3", "question", {"question": "What should the release be called?"})]}
        return {"text": "Release name: " + text_of(last_tool[-1].get("content"))}
    if tag == "slow-bash":
        if step == 0:
            return {"calls": [call("call_bash_slow", "bash", {"command": "sleep 30"})]}
        return {"text": "Slept."}
    if tag == "subagent-parallel":
        if step == 0:
            return {"text": "Delegating two tasks.", "calls": [
                call("call_sub_explorer", "subagent", {"agent": "explorer", "taskName": "Find readme", "task": "Locate the README file and summarise it. [[scenario:child-read]]", "model": "fake/fake-model"}),
                call("call_sub_planner", "subagent", {"agent": "planner", "taskName": "Plan greeting rename", "task": "Plan how to rename the greeting. [[scenario:child-text]]", "model": "fake/fake-model"}),
            ]}
        return {"text": "Both subagents finished."}
    if tag == "subagent-worktree":
        if step == 0:
            return {"calls": [call("call_sub_builder", "subagent", {"agent": "builder", "taskName": "Add changelog", "task": "Add a CHANGELOG.md file with one entry. [[scenario:child-write]]", "model": "fake/fake-model"})]}
        return {"text": "Builder merged its change."}
    if tag == "subagent-refused":
        if step == 0:
            return {"calls": [call("call_sub_refused", "subagent", {"agent": "builder", "taskName": "Survey code", "task": "Explore the repository layout. [[scenario:child-text]]", "model": "fake/fake-model"})]}
        return {"text": "The builder refused."}
    if tag == "subagent-question":
        if step == 0:
            return {"calls": [call("call_sub_asker", "subagent", {"agent": "planner", "taskName": "Ask scope", "task": "Plan the change, asking the user for scope. [[scenario:child-question]]", "model": "fake/fake-model"})]}
        return {"text": "The planner asked and finished."}
    if tag == "child-read":
        if step == 0:
            return {"calls": [call("call_child_read", "read", {"path": "README.md"})]}
        return {"text": "README.md: a one-line demo readme."}
    if tag == "child-text":
        return {"text": "Plan: rename greet() to welcome() in greet.py."}
    if tag == "child-write":
        if step == 0:
            return {"calls": [call("call_child_write", "write", {"path": "CHANGELOG.md", "content": "# Changelog\n\n- Added greeting.\n"})]}
        return {"text": "Added CHANGELOG.md."}
    if tag == "child-question":
        if step == 0:
            return {"calls": [call("call_child_q", "question", {"question": "Should the plan cover tests too?", "options": ["Yes", "No"]})]}
        return {"text": "Scope answer: " + text_of(last_tool[-1].get("content"))}
    return {"text": "Unknown scenario " + tag}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_GET(self):
        body = json.dumps({"object": "list", "data": [{"id": "fake-model", "object": "model"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        req = json.loads(self.rfile.read(n) or b"{}")
        messages = req.get("messages", [])
        tag, step = scenario_and_step(messages)
        p = plan(tag, step, messages)
        if LOG:
            with open(LOG, "a") as f:
                f.write(json.dumps({"tag": tag, "step": step, "plan": p, "tools": [t.get("function", {}).get("name") for t in req.get("tools", [])]}) + "\n")
        if "http_error" in p:
            body = json.dumps({"error": {"message": "fake upstream rejected the request", "type": "invalid_request_error"}}).encode()
            self.send_response(p["http_error"])
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        created = int(time.time())

        def chunk(delta, finish=None):
            d = {"id": "chatcmpl-fake", "object": "chat.completion.chunk", "created": created, "model": "fake-model",
                 "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
            self.wfile.write(b"data: " + json.dumps(d).encode() + b"\n\n")
            self.wfile.flush()

        chunk({"role": "assistant", "content": ""})
        if p.get("thinking"):
            chunk({"reasoning_content": p["thinking"]})
        if p.get("text"):
            words = p["text"].split(" ")
            for i, w in enumerate(words):
                chunk({"content": w + (" " if i < len(words) - 1 else "")})
        calls = p.get("calls", [])
        for i, c in enumerate(calls):
            args = json.dumps(c["args"])
            half = len(args) // 2
            chunk({"tool_calls": [{"index": i, "id": c["id"], "type": "function", "function": {"name": c["name"], "arguments": args[:half]}}]})
            chunk({"tool_calls": [{"index": i, "function": {"arguments": args[half:]}}]})
        chunk({}, "tool_calls" if calls else "stop")
        prompt_tokens = len(json.dumps(messages)) // 4
        completion_tokens = 7 + 11 * len(calls)
        usage = {"id": "chatcmpl-fake", "object": "chat.completion.chunk", "created": created, "model": "fake-model", "choices": [],
                 "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens,
                           "total_tokens": prompt_tokens + completion_tokens,
                           "prompt_tokens_details": {"cached_tokens": 0}}}
        self.wfile.write(b"data: " + json.dumps(usage).encode() + b"\n\n")
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
