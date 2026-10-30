import {
	createFauxCore,
	type FauxResponseFactory,
	fauxAssistantMessage,
	fauxToolCall,
} from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const PROVIDER = "orch-scripted";

function lastText(messages: { role: string; content: unknown }[], role: string): string {
	const message = messages.filter((message) => message.role === role).at(-1);
	if (!message) return "";
	if (typeof message.content === "string") return message.content;
	return (message.content as { type: string; text?: string }[])
		.map((block) => (block.type === "text" ? (block.text ?? "") : ""))
		.join("");
}

const respond: FauxResponseFactory = (context) => {
	const messages = context.messages as { role: string; content: unknown }[];
	const prompt = lastText(messages, "user");
	const answered = messages.at(-1)?.role === "toolResult";
	if (prompt.startsWith("fail ")) {
		return fauxAssistantMessage([], { stopReason: "error", errorMessage: prompt.slice(5) });
	}
	if (prompt === "tool" && !answered) return fauxAssistantMessage(fauxToolCall("bash", { command: "true" }));
	if (prompt === "question" && !answered) {
		return fauxAssistantMessage(fauxToolCall("question", { question: "Which one?", options: ["this", "that"] }));
	}
	if (prompt === "slow") return fauxAssistantMessage("still thinking ".repeat(400));
	return fauxAssistantMessage("Done.");
};

export default function scripted(pi: ExtensionAPI): void {
	const core = createFauxCore({ api: PROVIDER, provider: PROVIDER, models: [{ id: "scripted" }], tokensPerSecond: 200 });
	core.setResponses(Array.from({ length: 200 }, () => respond));
	pi.registerProvider(PROVIDER, {
		baseUrl: "http://localhost:0",
		apiKey: "scripted",
		api: PROVIDER,
		streamSimple: core.streamSimple,
		models: core.models.map((model) => ({
			id: model.id,
			name: model.name,
			reasoning: false,
			input: ["text"],
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
			contextWindow: 100_000,
			maxTokens: 4_096,
		})),
	});
	pi.registerTool({
		name: "question",
		label: "Question",
		description: "Ask the user to choose",
		parameters: Type.Object({ question: Type.String(), options: Type.Array(Type.String()) }),
		async execute(_id, params, _signal, _onUpdate, ctx) {
			const answer = await ctx.ui.select(params.question, params.options);
			return { content: [{ type: "text", text: answer ?? "no answer" }], details: {} };
		},
	});
	pi.registerCommand("orch-dialog", {
		description: "Open a dialog while pi is idle",
		handler: async (_args, ctx) => {
			await ctx.ui.confirm("Proceed?", "A dialog opened from a command");
		},
	});
	pi.registerCommand("orch-unname", {
		description: "Clear the session name",
		handler: async () => {
			pi.setSessionName("");
		},
	});
}
