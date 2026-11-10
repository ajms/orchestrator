import { fauxAssistantMessage, fauxText, fauxThinking, fauxToolCall } from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const usage = { input: 1200, output: 340, cacheRead: 0, cacheWrite: 0, cost: 0.0123, contextTokens: 1540, turns: 3 };
const noUsage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, contextTokens: 0, turns: 0 };

// The user's extension's `SingleResult`, as `details` of the final result.
function singleResult(agent: string, task: string, messages: object[], outcome: object) {
	return {
		agent,
		agentSource: "user",
		sessionLabel: `${agent}: ${task}`,
		task,
		exitCode: 0,
		messages,
		stderr: "",
		usage,
		model: "orch-scripted/scripted",
		stopReason: "stop",
		...outcome,
	};
}

// A refusal: the user's extension stops before the child starts, so no messages.
function refused(agent: string, task: string) {
	const errorMessage = "Refused: the task asks to push, which this agent may not do";
	return {
		content: [{ type: "text", text: `Agent failed: ${errorMessage}` }],
		details: singleResult(agent, task, [], { exitCode: 1, usage: noUsage, model: undefined, stopReason: undefined, errorMessage }),
		isError: true,
	};
}

export default function subagent(pi: ExtensionAPI): void {
	pi.registerTool({
		name: "subagent",
		label: "Subagent",
		description: "Stands in for the user's subagent extension: replays a child's messages, with a fallback retry",
		parameters: Type.Object({ agent: Type.String(), taskName: Type.String(), task: Type.String() }),
		async execute(_id, params, _signal, onUpdate) {
			if (params.task.startsWith("refuse")) return refused(params.agent, params.task);
			const messages: object[] = [];
			const update = async (...added: object[]) => {
				messages.push(...added);
				onUpdate?.({ content: [{ type: "text", text: "(running...)" }], details: { agent: params.agent, messages: [...messages] } });
				await sleep(150);
			};
			const task = { role: "user", content: [{ type: "text", text: `Task: ${params.task}` }], timestamp: Date.now() };
			if (params.task.startsWith("fail")) {
				await update(task);
				const errorMessage = "Merge conflict merging subagent/builder-1 into the parent HEAD";
				messages.push(fauxAssistantMessage("Fixed the redirect in src/login.rs."));
				return {
					content: [{ type: "text", text: `Agent failed: ${errorMessage}` }],
					details: singleResult(params.agent, params.task, messages, {
						exitCode: 1,
						errorMessage,
						worktreePath: "/tmp/pi-subagent-wt-abc123/builder",
						worktreeBranch: "subagent/builder-1",
						worktreeCleanup: {
							merged: false,
							skipped: false,
							message: "conflict in src/login.rs",
							preservedBranch: "subagent-preserved/builder-1",
						},
					}),
					isError: true,
				};
			}
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
			return {
				content: [{ type: "text", text: "Found the redirect in src/login.rs:42." }],
				details: singleResult(params.agent, params.task, messages, {}),
			};
		},
	});
}
