import assert from "node:assert/strict";
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
	idle = false;
	calls: Call[] = [];
	pending: (() => void)[] = [];
	failing = new Set<string>();

	constructor(flags: Record<string, string> = { "orch-bin": "/opt/orch", "orch-session": "s-1" }) {
		for (const [name, value] of Object.entries(flags)) this.flags.set(name, value);
		orch(this.api(), this.spawn);
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
			sessionManager: { getSessionId: () => this.sessionId },
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
		return this.calls.map((call) => call.payload);
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

test("each payload goes to orch hook for pi and the Session", async () => {
	const pi = new FakePi();
	pi.emit("session_start", { reason: "startup" });
	await pi.settle();
	assert.deepEqual(
		pi.calls.map(({ bin, args }) => ({ bin, args })),
		[{ bin: "/opt/orch", args: ["hook", "--agent", "pi", "--session", "s-1"] }],
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
		{ event: "tool_execution_end", toolName: "bash", isError: false },
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
		{ event: "ui_prompt_start", kind: "select" },
		{ event: "ui_prompt_end", kind: "select", idle: false },
		{ event: "ui_prompt_end", kind: "confirm", idle: true },
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
