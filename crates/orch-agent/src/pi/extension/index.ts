import type { ExtensionAPI, ExtensionContext, SessionEntry } from "@earendil-works/pi-coding-agent";
import { Forwarder, type Payload, type Spawn, spawnHook } from "./forward.ts";

interface Outcome {
	stopReason?: string;
	errorMessage?: string;
}

function spent(entries: SessionEntry[], since: string) {
	const totals = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 };
	for (const entry of entries) {
		if (entry.type !== "message" || entry.message.role !== "assistant") continue;
		// A fork copies its parent's entries, which were spent before it began.
		if (Date.parse(entry.timestamp) < Date.parse(since)) continue;
		const { usage } = entry.message;
		totals.input += usage.input;
		totals.output += usage.output;
		totals.cacheRead += usage.cacheRead;
		totals.cacheWrite += usage.cacheWrite;
		totals.cost += usage.cost.total;
	}
	return totals;
}

function usage(ctx: ExtensionContext): object {
	const entries = ctx.sessionManager.getEntries();
	const started = ctx.sessionManager.getHeader()?.timestamp ?? "";
	const context = ctx.getContextUsage();
	return {
		sessionId: ctx.sessionManager.getSessionId(),
		...(ctx.model ? { model: { id: ctx.model.id, name: ctx.model.name } } : {}),
		...(context ? { context } : {}),
		usage: spent(entries, started),
	};
}

const resumed = (ctx: ExtensionContext) => ctx.sessionManager.getEntries().some((entry) => entry.type === "message");

export default function orch(pi: ExtensionAPI, spawn: Spawn = spawnHook): void {
	pi.registerFlag("orch-bin", { description: "The orch binary that receives pi's hooks", type: "string", default: "orch" });
	pi.registerFlag("orch-session", { description: "The orch Session this pi runs in", type: "string" });

	let forwarder: Forwarder | undefined;
	let outcome: Outcome = {};
	const send = (payload: Payload) => forwarder?.send(payload);

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
		if (resumed(ctx)) forwarder?.tap(usage(ctx));
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
		forwarder?.tap(usage(ctx));
	});
}
