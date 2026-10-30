import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import orch from "./index.ts";
import type { Spawn } from "./forward.ts";

type Handler = (event: object, ctx: object) => unknown;

interface Call {
	bin: string;
	args: string[];
	payload: Record<string, unknown>;
}

class FakePi {
	handlers = new Map<string, Handler[]>();
	flags = new Map<string, string>();
	name: string | undefined;
	sessionId = "conv-1";
	parent: string | undefined;
	entries: object[] = [];
	model: { id: string; name: string } | undefined = { id: "claude-opus-5-5", name: "Opus 5.5" };
	context: object | undefined = { tokens: 1200, contextWindow: 200_000, percent: 0.6 };
	idle = false;
	calls: Call[] = [];
	pending: (() => void)[] = [];
	failing = new Set<string>();
	agentDir = mkdtempSync(join(tmpdir(), "orch-pi-profile-"));

	constructor(flags: Record<string, string> = { "orch-bin": "/opt/orch", "orch-session": "s-1" }) {
		for (const [name, value] of Object.entries(flags)) this.flags.set(name, value);
		orch(this.api(), this.spawn, this.agentDir);
	}

	api(): ExtensionAPI {
		const api = {
			registerFlag: (name: string, options: { default?: string }) => {
				if (!this.flags.has(name) && options.default !== undefined) this.flags.set(name, options.default);
			},
			getFlag: (name: string) => this.flags.get(name),
			getSessionName: () => this.name,
			on: (event: string, handler: Handler) => {
				this.handlers.set(event, [...(this.handlers.get(event) ?? []), handler]);
				return () => {};
			},
		};
		return api as unknown as ExtensionAPI;
	}

	spawn: Spawn = (bin, args, payload) => {
		const call = { bin, args, payload: JSON.parse(payload) };
		this.calls.push(call);
		if (this.failing.has(call.payload.event)) return Promise.reject(new Error("no orch"));
		return new Promise((resolve) => this.pending.push(resolve));
	};

	emit(type: string, event: object = {}): unknown[] {
		const ctx = {
			sessionManager: {
				getSessionId: () => this.sessionId,
				getHeader: () => ({
					type: "session",
					id: this.sessionId,
					timestamp: "2026-10-30T09:00:00.000Z",
					cwd: "/w",
					...(this.parent ? { parentSession: this.parent } : {}),
				}),
				getEntries: () => this.entries,
			},
			model: this.model,
			getContextUsage: () => this.context,
			isIdle: () => this.idle,
		};
		return (this.handlers.get(type) ?? []).map((handler) => handler({ type, ...event }, ctx));
	}

	async settle(): Promise<void> {
		for (let round = 0; round < 50; round++) {
			await new Promise((resolve) => setImmediate(resolve));
			this.pending.splice(0).forEach((resolve) => resolve());
		}
	}

	async payloads(): Promise<Record<string, unknown>[]> {
		await this.settle();
		return this.calls.filter((call) => call.args[0] === "hook").map((call) => call.payload);
	}
}

test("nothing is forwarded before the first session_start", async () => {
	const pi = new FakePi();
	pi.emit("input", { text: "hi", source: "interactive" });
	assert.deepEqual(await pi.payloads(), []);
});

test("nothing is forwarded without an orch Session", async () => {
	const pi = new FakePi({});
	pi.emit("session_start", { reason: "startup" });
	pi.emit("agent_start");
	assert.deepEqual(await pi.payloads(), []);
});

test("each payload goes to orch hook or tap for pi and the Session", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	await pi.settle();
	assert.deepEqual(
		pi.calls.map(({ bin, args }) => ({ bin, args })),
		[
			{ bin: "/opt/orch", args: ["hook", "--agent", "pi", "--session", "s-1"] },
			{ bin: "/opt/orch", args: ["tap", "--agent", "pi", "--session", "s-1"] },
		],
	);
});

test("session_start names the Conversation and forwards a name only when there is one", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.name = "fix login";
	pi.sessionId = "conv-2";
	pi.emit("session_start", { reason: "new", previousSessionFile: "/x.jsonl" });
	pi.name = "";
	pi.emit("session_start", { reason: "reload" });
	assert.deepEqual(await pi.payloads(), [
		{ event: "session_start", reason: "startup", sessionId: "conv-1" },
		{ event: "session_start", reason: "new", sessionId: "conv-2", name: "fix login" },
		{ event: "session_start", reason: "reload", sessionId: "conv-2" },
	]);
});

test("a cleared session name is forwarded as an empty name", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("session_info_changed", { name: "fix login" });
	pi.emit("session_info_changed", { name: undefined });
	assert.deepEqual((await pi.payloads()).slice(1), [
		{ event: "session_info_changed", name: "fix login" },
		{ event: "session_info_changed", name: "" },
	]);
});

test("a prompt's coarse events are shaped to the fields orch needs", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("input", { text: "secret prompt", source: "interactive" });
	pi.emit("agent_start");
	pi.emit("tool_call", { toolCallId: "t1", toolName: "bash", input: { command: "ls" } });
	pi.emit("tool_execution_end", { toolCallId: "t1", toolName: "bash", result: { big: true }, isError: false });
	pi.emit("message_update", { message: { role: "assistant" } });
	pi.emit("message_end", { message: { role: "assistant", stopReason: "stop" } });
	pi.emit("agent_settled", { aborted: false });
	assert.deepEqual((await pi.payloads()).slice(1), [
		{ event: "input" },
		{ event: "agent_start" },
		{ event: "tool_call", toolName: "bash" },
		{ event: "tool_execution_end", toolName: "bash" },
		{ event: "agent_settled", aborted: false, stopReason: "stop" },
	]);
});

test("agent_settled carries the last assistant message's error", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("message_end", { message: { role: "assistant", stopReason: "error", errorMessage: "429 rate limit" } });
	pi.emit("message_end", { message: { role: "toolResult" } });
	pi.emit("agent_settled", { aborted: false });
	pi.emit("input", { text: "again", source: "interactive" });
	pi.emit("agent_settled", { aborted: true });
	assert.deepEqual((await pi.payloads()).slice(1), [
		{ event: "agent_settled", aborted: false, stopReason: "error", errorMessage: "429 rate limit" },
		{ event: "input" },
		{ event: "agent_settled", aborted: true },
	]);
});

test("dialogs report whether pi was idle when they closed", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("ui_prompt_start", { reason: "ui_prompt", kind: "select", title: "Pick" });
	pi.emit("ui_prompt_end", { reason: "ui_prompt", kind: "select", title: "Pick" });
	pi.idle = true;
	pi.emit("ui_prompt_end", { reason: "ui_prompt", kind: "confirm" });
	assert.deepEqual((await pi.payloads()).slice(1), [
		{ event: "ui_prompt_start" },
		{ event: "ui_prompt_end", idle: false },
		{ event: "ui_prompt_end", idle: true },
	]);
});

test("handlers never wait for orch", () => {
	const pi = new FakePi();
	const results = [
		...pi.emit("session_start", { reason: "startup" }),
		...pi.emit("tool_call", { toolCallId: "t1", toolName: "bash", input: {} }),
		...pi.emit("agent_settled", { aborted: false }),
	];
	assert.deepEqual(results, [undefined, undefined, undefined]);
});

test("a payload is spawned only after the one before it finished", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("input", { text: "hi", source: "interactive" });
	pi.emit("agent_start");
	await new Promise((resolve) => setImmediate(resolve));
	assert.equal(pi.calls.length, 1);
	pi.pending.shift()?.();
	await new Promise((resolve) => setImmediate(resolve));
	assert.equal(pi.calls.length, 2);
	assert.deepEqual(
		(await pi.payloads()).map((payload) => payload.event),
		["session_start", "input", "agent_start"],
	);
});

test("a failed spawn doesn't stop the payloads after it", async () => {
	const pi = new FakePi();
	pi.failing.add("session_start");
	pi.emit("session_start", { reason: "startup" });
	pi.emit("agent_start");
	assert.deepEqual(
		(await pi.payloads()).map((payload) => payload.event),
		["session_start", "agent_start"],
	);
});

test("a second session_start keeps one ordered stream", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("session_start", { reason: "startup" });
	pi.emit("agent_start");
	await new Promise((resolve) => setImmediate(resolve));
	assert.equal(pi.calls.length, 1);
	assert.deepEqual(
		(await pi.payloads()).map((payload) => payload.event),
		["session_start", "session_start", "agent_start"],
	);
});

test("a session_start forgets the last prompt's error", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("message_end", { message: { role: "assistant", stopReason: "error", errorMessage: "boom" } });
	pi.emit("session_start", { reason: "new" });
	pi.emit("agent_settled", { aborted: false });
	assert.deepEqual((await pi.payloads()).at(-1), { event: "agent_settled", aborted: false });
});

function assistant(id: string, input: number, output: number, cacheRead: number, cacheWrite: number, cost: number): object {
	const usage = { input, output, cacheRead, cacheWrite, totalTokens: input + output + cacheRead + cacheWrite, cost: { total: cost } };
	return { type: "message", id, timestamp: "2026-10-30T09:01:00.000Z", message: { role: "assistant", usage } };
}

function user(id: string): object {
	return { type: "message", id, timestamp: "2026-10-30T09:01:00.000Z", message: { role: "user", content: "hi" } };
}

function sessionFile(entries: object[]): string {
	const path = join(mkdtempSync(join(tmpdir(), "orch-pi-parent-")), "parent.jsonl");
	const header = { type: "session", id: "parent", timestamp: "2026-10-30T08:00:00.000Z", cwd: "/w" };
	writeFileSync(path, [header, ...entries].map((entry) => `${JSON.stringify(entry)}\n`).join(""));
	return path;
}

const taps = (pi: FakePi) => pi.calls.filter((call) => call.args[0] === "tap");
const tapped = async (pi: FakePi) => {
	await pi.settle();
	return taps(pi).map((call) => call.payload);
};
const zero = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };

test("agent_settled taps orch with the last response's tokens and the Conversation's cost", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "reload" });
	pi.entries.push(user("u1"), assistant("a1", 10, 20, 300, 40, 0.5));
	pi.emit("agent_settled", { aborted: false });
	pi.entries.push(user("u2"), assistant("a2", 1, 2, 400, 4, 0.25));
	pi.emit("agent_settled", { aborted: false });
	await pi.settle();
	assert.deepEqual(
		taps(pi).map(({ bin, args }) => ({ bin, args })),
		[
			{ bin: "/opt/orch", args: ["tap", "--agent", "pi", "--session", "s-1"] },
			{ bin: "/opt/orch", args: ["tap", "--agent", "pi", "--session", "s-1"] },
		],
	);
	assert.deepEqual(await tapped(pi), [
		{
			sessionId: "conv-1",
			model: { id: "claude-opus-5-5", name: "Opus 5.5" },
			context: { tokens: 1200, contextWindow: 200_000, percent: 0.6 },
			lastResponse: { input: 10, output: 20, cacheRead: 300, cacheWrite: 40 },
			cost: 0.5,
		},
		{
			sessionId: "conv-1",
			model: { id: "claude-opus-5-5", name: "Opus 5.5" },
			context: { tokens: 1200, contextWindow: 200_000, percent: 0.6 },
			lastResponse: { input: 1, output: 2, cacheRead: 400, cacheWrite: 4 },
			cost: 0.75,
		},
	]);
});

test("the tap follows the hook of the same agent_settled", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "reload" });
	pi.emit("agent_settled", { aborted: false });
	pi.emit("input", { text: "again", source: "interactive" });
	await pi.settle();
	assert.deepEqual(
		pi.calls.map((call) => `${call.args[0]} ${call.payload.event ?? ""}`.trim()),
		["hook session_start", "hook agent_settled", "tap", "hook input"],
	);
});

test("a response without tokens leaves the last one's in place", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "reload" });
	pi.entries.push(assistant("a1", 10, 20, 300, 40, 0.5), assistant("a2", 0, 0, 0, 0, 0));
	pi.emit("agent_settled", { aborted: true });
	assert.deepEqual(
		(await tapped(pi)).map((tap) => [tap.lastResponse, tap.cost]),
		[[{ input: 10, output: 20, cacheRead: 300, cacheWrite: 40 }, 0.5]],
	);
});

test("a new Conversation starts its cost over", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "reload" });
	pi.entries.push(assistant("a1", 10, 20, 300, 40, 0.5));
	pi.emit("agent_settled", { aborted: false });
	pi.sessionId = "conv-2";
	pi.entries = [];
	pi.emit("session_start", { reason: "new" });
	pi.entries.push(assistant("b1", 1, 2, 3, 4, 0.125));
	pi.emit("agent_settled", { aborted: false });
	assert.deepEqual(
		(await tapped(pi)).map((tap) => [tap.sessionId, tap.cost]),
		[
			["conv-1", 0.5],
			["conv-2", 0.125],
		],
	);
});

test("a fork's cost leaves out the entries it copied from its parent", async () => {
	const copied = [user("u1"), assistant("a1", 10, 20, 300, 40, 0.5)];
	const pi = new FakePi();
	pi.sessionId = "conv-fork";
	pi.parent = sessionFile(copied);
	pi.entries.push(...copied);
	pi.emit("session_start", { reason: "fork" });
	pi.entries.push(user("u2"), assistant("f1", 1, 2, 3, 4, 0.125));
	pi.emit("agent_settled", { aborted: false });
	assert.deepEqual(
		(await tapped(pi)).map((tap) => [tap.lastResponse, tap.cost]),
		[
			[{ input: 10, output: 20, cacheRead: 300, cacheWrite: 40 }, 0],
			[{ input: 1, output: 2, cacheRead: 3, cacheWrite: 4 }, 0.125],
		],
	);
});

test("a fork whose parent can't be read counts every entry", async () => {
	const pi = new FakePi();
	pi.parent = "/nonexistent/parent.jsonl";
	pi.entries.push(assistant("a1", 10, 20, 300, 40, 0.5));
	pi.emit("session_start", { reason: "fork" });
	assert.deepEqual(
		(await tapped(pi)).map((tap) => tap.cost),
		[0.5],
	);
});

test("session_start taps at startup, resume and fork, but not reload or new", async () => {
	const pi = new FakePi();
	pi.entries.push(user("u1"), assistant("a1", 10, 20, 300, 40, 0.5));
	for (const reason of ["startup", "reload", "new", "resume", "fork"]) {
		pi.sessionId = reason;
		pi.emit("session_start", { reason });
	}
	assert.deepEqual(
		(await tapped(pi)).map((tap) => tap.sessionId),
		["startup", "resume", "fork"],
	);
});

test("a fresh Conversation taps zero tokens, and leaves out a model or context pi doesn't know", async () => {
	const pi = new FakePi();
	pi.model = undefined;
	pi.context = undefined;
	pi.emit("session_start", { reason: "startup" });
	assert.deepEqual(await tapped(pi), [{ sessionId: "conv-1", lastResponse: zero, cost: 0 }]);
});

const subagentCall = {
	toolCallId: "call_7",
	toolName: "subagent",
	input: { agent: "scout", taskName: "find the redirect", task: "Find where login redirects" },
};
const task = { role: "user", content: [{ type: "text", text: "Task: Find where login redirects" }] };
const said = (text: string) => ({ role: "assistant", content: [{ type: "text", text }] });
const update = (messages: object[], toolName = "subagent") => ({
	toolCallId: "call_7",
	toolName,
	args: subagentCall.input,
	partialResult: { content: [], details: { messages } },
});

function transcript(pi: FakePi, conversation = "conv-1"): object[] {
	return readFileSync(join(pi.agentDir, "subagents", conversation, "call_7.jsonl"), "utf8")
		.split("\n")
		.filter(Boolean)
		.map((line) => JSON.parse(line));
}

test("a subagent call names its Conversation, agent and task name, and its end names the call", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("tool_call", subagentCall);
	pi.emit("tool_execution_update", update([task]));
	pi.emit("tool_execution_end", { toolCallId: "call_7", toolName: "subagent", result: {}, isError: false });
	assert.deepEqual((await pi.payloads()).slice(1), [
		{
			event: "tool_call",
			toolName: "subagent",
			toolCallId: "call_7",
			sessionId: "conv-1",
			agent: "scout",
			taskName: "find the redirect",
		},
		{ event: "tool_execution_end", toolName: "subagent", toolCallId: "call_7" },
	]);
});

test("a subagent's messages are written to its transcript as they come, and its result at the end", () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("tool_call", subagentCall);
	pi.emit("tool_execution_update", update([task]));
	pi.emit("tool_execution_update", update([task, said("Searching.")]));
	assert.deepEqual(transcript(pi), [task, said("Searching.")]);
	pi.emit("tool_execution_end", {
		toolCallId: "call_7",
		toolName: "subagent",
		result: { content: [], details: { messages: [task, said("Searching."), said("Found it.")] } },
		isError: false,
	});
	assert.deepEqual(transcript(pi), [task, said("Searching."), said("Found it.")]);
});

test("a fallback retry that starts over rewrites the transcript", () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("tool_execution_update", update([task, said("Overloaded.")]));
	pi.emit("tool_execution_update", update([task]));
	assert.deepEqual(transcript(pi), [task]);
});

test("other tools' updates and updates without messages write nothing", () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("tool_execution_update", update([task], "bash"));
	pi.emit("tool_execution_update", { toolCallId: "call_7", toolName: "subagent", partialResult: { content: [] } });
	assert.equal(existsSync(join(pi.agentDir, "subagents")), false);
});

test("a subagent call without a task name forwards its task instead", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	pi.emit("tool_call", { ...subagentCall, input: { agent: "scout", task: "Find where login redirects" } });
	assert.deepEqual((await pi.payloads()).at(-1), {
		event: "tool_call",
		toolName: "subagent",
		toolCallId: "call_7",
		sessionId: "conv-1",
		agent: "scout",
		task: "Find where login redirects",
	});
});
