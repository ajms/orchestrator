import { fauxAssistantMessage, fauxText, fauxThinking, fauxToolCall } from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export default function subagent(pi: ExtensionAPI): void {
	pi.registerTool({
		name: "subagent",
		label: "Subagent",
		description: "Stands in for the user's subagent extension: replays a child's messages, with a fallback retry",
		parameters: Type.Object({ agent: Type.String(), taskName: Type.String(), task: Type.String() }),
		async execute(_id, params, _signal, onUpdate) {
			const messages: object[] = [];
			const update = async (...added: object[]) => {
				messages.push(...added);
				onUpdate?.({ content: [{ type: "text", text: "(running...)" }], details: { agent: params.agent, messages: [...messages] } });
				await sleep(150);
			};
			const task = { role: "user", content: [{ type: "text", text: `Task: ${params.task}` }], timestamp: Date.now() };
			await update(task);
			await update(fauxAssistantMessage([], { stopReason: "error", errorMessage: "529 overloaded" }));
			messages.length = 0;
			await update(task);
			await update(
				fauxAssistantMessage([
					fauxThinking("Grep for it."),
					fauxText("I'll search the sources."),
					fauxToolCall("grep", { pattern: "redirect", path: "src" }, { id: "call_grep" }),
				]),
			);
			await update({
				role: "toolResult",
				toolCallId: "call_grep",
				toolName: "grep",
				content: [{ type: "text", text: "src/login.rs:42: redirect(next)" }],
				isError: false,
				timestamp: Date.now(),
			});
			await update(fauxAssistantMessage(fauxToolCall("read", { path: "src/login.rs" }, { id: "call_read" })));
			await update({
				role: "toolResult",
				toolCallId: "call_read",
				toolName: "read",
				content: [{ type: "text", text: "ENOENT: no such file" }],
				isError: true,
				timestamp: Date.now(),
			});
			messages.push(fauxAssistantMessage("Found the redirect in src/login.rs:42."));
			return { content: [{ type: "text", text: "Found the redirect in src/login.rs:42." }], details: { agent: params.agent, messages } };
		},
	});
}
