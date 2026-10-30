import { appendFileSync, mkdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const MAX_ID_BYTES = 200;

function plainName(id: string): boolean {
	return id.length > 0 && Buffer.byteLength(id) <= MAX_ID_BYTES && !id.startsWith(".") && !/[/\\\0]/.test(id);
}

const jsonl = (messages: unknown[]) => messages.map((message) => `${JSON.stringify(message)}\n`).join("");

export class SubagentTranscripts {
	#dir: string;
	#written = new Map<string, number>();

	constructor(agentDir: string) {
		this.#dir = join(agentDir, "subagents");
	}

	write(conversation: string, toolCallId: string, messages: unknown[]): void {
		if (!plainName(conversation) || !plainName(toolCallId)) return;
		const dir = join(this.#dir, conversation);
		const file = join(dir, `${toolCallId}.jsonl`);
		const key = `${conversation}/${toolCallId}`;
		const written = this.#written.get(key);
		try {
			if (written !== undefined && messages.length >= written) {
				if (messages.length > written) appendFileSync(file, jsonl(messages.slice(written)));
			} else {
				// A new file, so orch reads a retry that starts over as a reset.
				mkdirSync(dir, { recursive: true, mode: 0o700 });
				const staged = `${file}.new`;
				writeFileSync(staged, jsonl(messages), { mode: 0o600 });
				renameSync(staged, file);
			}
			this.#written.set(key, messages.length);
		} catch {
			this.#written.delete(key);
		}
	}
}
