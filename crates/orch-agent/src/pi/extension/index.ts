import { readFileSync } from "node:fs";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Forwarder, type Payload, type Spawn, spawnHook, type Tap, type Tokens } from "./forward.ts";

interface Outcome {
	stopReason?: string;
	errorMessage?: string;
}

const TAPPED_STARTS = new Set(["startup", "resume", "fork"]);

function idsIn(sessionFile: string | undefined): Set<string> {
	if (!sessionFile) return new Set();
	try {
		const entries = readFileSync(sessionFile, "utf8")
			.split("\n")
			.filter(Boolean)
			.map((line) => JSON.parse(line) as { type?: string; id?: string });
		return new Set(entries.flatMap((entry) => (entry.type !== "session" && entry.id ? [entry.id] : [])));
	} catch {
		return new Set();
	}
}

function tapPayload(ctx: ExtensionContext, copied: Set<string>): Tap {
	let lastResponse: Tokens = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };
	let cost = 0;
	for (const entry of ctx.sessionManager.getEntries()) {
		if (entry.type !== "message" || entry.message.role !== "assistant") continue;
		const { input, output, cacheRead, cacheWrite, cost: spent } = entry.message.usage;
		if (input + output + cacheRead + cacheWrite > 0) lastResponse = { input, output, cacheRead, cacheWrite };
		if (!copied.has(entry.id)) cost += spent.total;
	}
	const context = ctx.getContextUsage();
	return {
		sessionId: ctx.sessionManager.getSessionId(),
		...(ctx.model ? { model: { id: ctx.model.id, name: ctx.model.name } } : {}),
		...(context ? { context } : {}),
		lastResponse,
		cost,
	};
}

export default function orch(pi: ExtensionAPI, spawn: Spawn = spawnHook): void {
	pi.registerFlag("orch-bin", { description: "The orch binary that receives pi's hooks", type: "string", default: "orch" });
	pi.registerFlag("orch-session", { description: "The orch Session this pi runs in", type: "string" });

	let forwarder: Forwarder | undefined;
	let outcome: Outcome = {};
	let copied: { sessionId: string; ids: Set<string> } | undefined;
	const send = (payload: Payload) => forwarder?.send(payload);
	const tap = (ctx: ExtensionContext) => {
		const sessionId = ctx.sessionManager.getSessionId();
		if (copied?.sessionId !== sessionId) {
			// A fork copies its parent's entries with their ids; they were spent there.
			copied = { sessionId, ids: idsIn(ctx.sessionManager.getHeader()?.parentSession) };
		}
		forwarder?.tap(tapPayload(ctx, copied.ids));
	};

	pi.on("session_start", (event, ctx) => {
		const bin = pi.getFlag("orch-bin");
		const session = pi.getFlag("orch-session");
		if (typeof bin !== "string" || typeof session !== "string" || !session) return;
		if (!forwarder?.serves(bin, session)) forwarder = new Forwarder(bin, session, spawn);
		outcome = {};
		const name = pi.getSessionName();
		send({
			event: "session_start",
			reason: event.reason,
			sessionId: ctx.sessionManager.getSessionId(),
			...(name ? { name } : {}),
		});
		if (TAPPED_STARTS.has(event.reason)) tap(ctx);
	});
	pi.on("session_info_changed", (event) => {
		send({ event: "session_info_changed", name: event.name ?? "" });
	});
	pi.on("input", () => {
		outcome = {};
		send({ event: "input" });
	});
	pi.on("agent_start", () => {
		send({ event: "agent_start" });
	});
	pi.on("tool_call", (event) => {
		send({ event: "tool_call", toolName: event.toolName });
	});
	pi.on("tool_execution_end", (event) => {
		send({ event: "tool_execution_end", toolName: event.toolName });
	});
	pi.on("ui_prompt_start", () => {
		send({ event: "ui_prompt_start" });
	});
	pi.on("ui_prompt_end", (_event, ctx) => {
		send({ event: "ui_prompt_end", idle: ctx.isIdle() });
	});
	pi.on("message_end", (event) => {
		if (event.message.role !== "assistant") return;
		const { stopReason, errorMessage } = event.message;
		outcome = { stopReason, ...(errorMessage ? { errorMessage } : {}) };
	});
	pi.on("agent_settled", (event, ctx) => {
		send({ event: "agent_settled", aborted: event.aborted, ...outcome });
		tap(ctx);
	});
}
