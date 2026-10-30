import { spawn } from "node:child_process";

export type Payload = { event: string } & Record<string, unknown>;

export type Spawn = (bin: string, args: string[], payload: string) => Promise<void>;

const HOOK_TIMEOUT_MS = 10_000;

export const spawnHook: Spawn = (bin, args, payload) =>
	new Promise((resolve) => {
		const child = spawn(bin, args, { stdio: ["pipe", "ignore", "ignore"], timeout: HOOK_TIMEOUT_MS });
		child.on("error", () => resolve());
		child.on("close", () => resolve());
		child.stdin.on("error", () => {});
		child.stdin.end(payload);
	});

export class Forwarder {
	#bin: string;
	#session: string;
	#spawn: Spawn;
	#tail: Promise<void> = Promise.resolve();

	constructor(bin: string, session: string, spawn: Spawn) {
		this.#bin = bin;
		this.#session = session;
		this.#spawn = spawn;
	}

	serves(bin: string, session: string): boolean {
		return this.#bin === bin && this.#session === session;
	}

	send(payload: Payload): void {
		this.#chain("hook", payload);
	}

	tap(payload: object): void {
		this.#chain("tap", payload);
	}

	#chain(command: "hook" | "tap", payload: object): void {
		const args = [command, "--agent", "pi", "--session", this.#session];
		const line = JSON.stringify(payload);
		this.#tail = this.#tail.then(() => this.#spawn(this.#bin, args, line)).catch(() => {});
	}
}
