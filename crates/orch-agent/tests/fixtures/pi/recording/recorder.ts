import * as fs from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const EVENTS = [
	"project_trust", "resources_discover", "mcp_servers_change",
	"session_start", "session_info_changed", "session_before_switch", "session_before_fork",
	"session_before_compact", "session_compact", "session_compact_failed", "session_shutdown",
	"session_before_tree", "session_tree",
	"context", "before_provider_request", "after_provider_response",
	"before_agent_start", "agent_start", "agent_end", "agent_before_settle", "agent_settled",
	"ui_prompt_start", "ui_prompt_end",
	"turn_start", "turn_end", "message_start", "message_update", "message_end",
	"tool_execution_start", "tool_execution_update", "tool_execution_end",
	"model_select", "thinking_level_select", "user_bash", "input", "tool_call", "tool_result",
];

const SKIP_PAYLOAD = new Set(["context", "before_provider_request"]);

export default function (pi: ExtensionAPI) {
	const out = process.env.PI_REC_OUT;
	if (!out) return;
	let seq = 0;
	const write = (event: string, payload: unknown, ctx: any) => {
		let meta: Record<string, unknown> = {};
		try {
			meta = {
				mode: ctx?.mode,
				hasUI: ctx?.hasUI,
				cwd: ctx?.cwd,
				sessionId: ctx?.sessionManager?.getSessionId?.(),
				sessionFile: ctx?.sessionManager?.getSessionFile?.(),
				sessionName: ctx?.sessionManager?.getSessionName?.(),
			};
		} catch {}
		const body = SKIP_PAYLOAD.has(event) ? { type: (payload as any)?.type, omitted: true } : payload;
		fs.appendFileSync(out, JSON.stringify({ seq: seq++, ts: Date.now(), event, ctx: meta, payload: body }) + "\n");
	};
	for (const name of EVENTS) {
		pi.on(name as any, async (event: any, ctx: any) => {
			write(name, event, ctx);
			if (name === "agent_start" && process.env.PI_REC_SET_NAME) {
				pi.setSessionName(process.env.PI_REC_SET_NAME);
			}
			return undefined;
		});
	}
}
