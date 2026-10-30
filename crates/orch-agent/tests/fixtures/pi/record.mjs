#!/usr/bin/env node
// Records what orch's pi extension sends to `orch hook` and `orch tap`: node record.mjs after `npm ci` in the extension (PI="node /path/to/cli.js" for another pi).
import { execFileSync, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const extension = join(here, "../../../src/pi/extension/index.ts");
const hooks = join(here, "hooks");
const taps = join(here, "taps");
const pinned = `node ${join(here, "../../../src/pi/extension/node_modules/@earendil-works/pi-coding-agent/dist/cli.js")}`;
const [program, ...programArgs] = (process.env.PI ?? pinned).split(" ");
const work = mkdtempSync(join(tmpdir(), "orch-pi-record-"));
const capture = join(work, "capture.jsonl");
const env = {
	...process.env,
	HOME: join(work, "home"),
	PI_CODING_AGENT_DIR: join(work, "profile"),
	PI_OFFLINE: "1",
	ORCH_CAPTURE: capture,
};
mkdirSync(env.HOME);
mkdirSync(env.PI_CODING_AGENT_DIR);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

class Pi {
	#next = 0;
	#waiting = new Map();
	#listeners = [];

	constructor(conversation) {
		this.child = spawn(
			program,
			[
				...programArgs,
				"--mode", "rpc",
				"-na",
				"--session-id", conversation,
				"-e", extension,
				"-e", join(here, "scripted.ts"),
				"--orch-bin", join(here, "capture.sh"),
				"--orch-session", "recorded",
				"--provider", "orch-scripted",
				"--model", "scripted",
			],
			{ cwd: work, env, stdio: ["pipe", "pipe", "inherit"] },
		);
		createInterface({ input: this.child.stdout }).on("line", (line) => this.#receive(line));
	}

	#receive(line) {
		let message;
		try {
			message = JSON.parse(line);
		} catch {
			return;
		}
		if (message.type === "response" && this.#waiting.has(message.id)) {
			this.#waiting.get(message.id)(message);
			this.#waiting.delete(message.id);
		}
		if (message.type === "extension_ui_request" && ["select", "confirm", "input", "editor"].includes(message.method)) {
			const answer = message.method === "select" ? { value: message.options[0] } : message.method === "confirm" ? { confirmed: true } : { value: "ok" };
			this.#write({ type: "extension_ui_response", id: message.id, ...answer });
		}
		this.#listeners = this.#listeners.filter(({ type, resolve }) => (message.type === type ? (resolve(message), false) : true));
	}

	#write(command) {
		this.child.stdin.write(`${JSON.stringify(command)}\n`);
	}

	send(command) {
		const id = `rec-${this.#next++}`;
		const response = new Promise((resolve) => this.#waiting.set(id, resolve));
		this.#write({ id, ...command });
		return response;
	}

	next(type) {
		return new Promise((resolve) => this.#listeners.push({ type, resolve }));
	}

	async prompt(message) {
		const settled = this.next("agent_settled");
		await this.send({ type: "prompt", message });
		await settled;
	}

	async quit() {
		const exited = new Promise((resolve) => this.child.on("exit", resolve));
		this.child.stdin.end();
		await Promise.race([exited, sleep(5_000).then(() => this.child.kill())]);
	}
}

async function captured(action) {
	writeFileSync(capture, "");
	await action();
	let seen = "";
	for (let quiet = 0; quiet < 5; ) {
		await sleep(200);
		const now = readFileSync(capture, "utf8");
		quiet = now === seen ? quiet + 1 : 0;
		seen = now;
	}
	const lines = seen.split("\n").filter(Boolean).map((line) => JSON.parse(line));
	const sent = (command) => lines.filter((line) => line.command === command).map((line) => line.payload);
	return { hooks: sent("hook"), taps: sent("tap") };
}

const fixtures = new Map();
const tapped = new Map();

function keep(name, { hooks: payloads }, event, matches = () => true) {
	const payload = payloads.find((payload) => payload.event === event && matches(payload));
	if (!payload) throw new Error(`${name}: no ${event} in ${JSON.stringify(payloads)}`);
	fixtures.set(name, payload);
}

function keepTap(name, { taps: payloads }) {
	if (!payloads.length) throw new Error(`${name}: no tap`);
	tapped.set(name, payloads.at(-1));
}

const first = randomUUID();
let pi;
const startup = await captured(async () => {
	pi = new Pi(first);
	await pi.send({ type: "get_state" });
});
keep("session_start_startup", startup, "session_start");
keepTap("session_start_startup", startup);

const reply = await captured(() => pi.prompt("reply"));
keep("input", reply, "input");
keep("agent_start", reply, "agent_start");
keep("agent_settled", reply, "agent_settled");
keepTap("agent_settled", reply);

const tool = await captured(() => pi.prompt("tool"));
keep("tool_call_bash", tool, "tool_call");
keep("tool_execution_end_bash", tool, "tool_execution_end");
keepTap("agent_settled_second_turn", tool);

const question = await captured(() => pi.prompt("question"));
keep("tool_call_question", question, "tool_call");
keep("ui_prompt_start_select", question, "ui_prompt_start");
keep("ui_prompt_end_working", question, "ui_prompt_end");
keep("tool_execution_end_question", question, "tool_execution_end");

const dialog = await captured(() => pi.send({ type: "prompt", message: "/orch-dialog" }));
keep("ui_prompt_start_confirm", dialog, "ui_prompt_start");
keep("ui_prompt_end_idle", dialog, "ui_prompt_end");

const abort = await captured(async () => {
	const settled = pi.next("agent_settled");
	const started = pi.next("message_update");
	await pi.send({ type: "prompt", message: "slow" });
	await started;
	await pi.send({ type: "abort" });
	await settled;
});
keep("agent_settled_aborted", abort, "agent_settled");

await pi.send({ type: "set_auto_retry", enabled: false });
for (const [name, error] of [
	["rate_limited", '429 {"type":"error","error":{"type":"rate_limit_error","message":"This request would exceed your rate limit."}}'],
	["authentication", '401 {"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}'],
	["server", '529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}'],
	["context", "prompt is too long: 213000 tokens > 200000 maximum context length"],
	["other", "fetch failed"],
]) {
	const failed = await captured(() => pi.prompt(`fail ${error}`));
	keep(`agent_settled_error_${name}`, failed, "agent_settled");
}

const named = await captured(() => pi.send({ type: "set_session_name", name: "fix login" }));
keep("session_info_changed_named", named, "session_info_changed");
const unnamed = await captured(() => pi.send({ type: "prompt", message: "/orch-unname" }));
keep("session_info_changed_cleared", unnamed, "session_info_changed");
await captured(() => pi.send({ type: "set_session_name", name: "fix login" }));

const fresh = await captured(() => pi.send({ type: "new_session" }));
keep("session_start_new", fresh, "session_start");
if (fresh.taps.length) throw new Error(`a new Conversation was tapped: ${JSON.stringify(fresh.taps)}`);
keepTap("agent_settled_new", await captured(() => pi.prompt("reply")));
keepTap("session_start_fork", await captured(() => pi.send({ type: "clone" })));
keepTap("agent_settled_fork", await captured(() => pi.prompt("reply")));
await pi.quit();

const resumed = await captured(async () => {
	pi = new Pi(first);
	await pi.send({ type: "get_state" });
});
keep("session_start_resumed_named", resumed, "session_start");
keepTap("session_start_resumed", resumed);
await pi.quit();

const version = execFileSync(program, [...programArgs, "--version"], { env, encoding: "utf8" }).trim();
rmSync(work, { recursive: true, force: true });
mkdirSync(hooks, { recursive: true });
mkdirSync(taps, { recursive: true });
for (const [dir, payloads] of [[hooks, fixtures], [taps, tapped]]) {
	for (const [name, payload] of payloads) {
		writeFileSync(join(dir, `${name}.json`), `${JSON.stringify(payload, null, 2)}\n`);
	}
}
const readme = join(here, "README.md");
writeFileSync(readme, readFileSync(readme, "utf8").replace(/^Recorded with pi .*$/m, `Recorded with pi ${version}.`));
console.log(`recorded ${fixtures.size} hook and ${tapped.size} tap payloads with pi ${version}`);
