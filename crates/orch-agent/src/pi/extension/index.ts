import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Forwarder, type Payload, type Spawn, spawnHook } from "./forward.ts";

interface Outcome {
	stopReason?: string;
	errorMessage?: string;
}

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
	pi.on("agent_settled", (event) => {
		send({ event: "agent_settled", aborted: event.aborted, ...outcome });
	});
}
