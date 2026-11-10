import assert from "node:assert/strict";
import { test } from "node:test";
import { subagentOutcome } from "./outcome.ts";

test("a SingleResult's outcome fields are kept", () => {
	const result = {
		content: [{ type: "text", text: "Found it." }],
		details: {
			agent: "scout",
			exitCode: 0,
			stopReason: "stop",
			model: "anthropic/claude",
			usage: { input: 10, output: 5, cacheRead: 1, cacheWrite: 0, cost: 0.01, contextTokens: 16, turns: 2 },
			sessionLabel: "scout: find it",
			messages: [{ role: "user" }],
			stderr: "noise",
			worktreePath: "/tmp/wt/scout",
			worktreeBranch: "subagent/scout-1",
			worktreeCleanup: { merged: true, skipped: false, message: "merged" },
		},
	};
	assert.deepEqual(subagentOutcome(result, false), {
		isError: false,
		exitCode: 0,
		stopReason: "stop",
		model: "anthropic/claude",
		usage: { input: 10, output: 5, cost: 0.01, turns: 2 },
		text: "Found it.",
		worktreePath: "/tmp/wt/scout",
		worktreeBranch: "subagent/scout-1",
		worktreeCleanup: { merged: true, skipped: false, message: "merged" },
	});
});

test("fields of the wrong type are dropped and long text is cut", () => {
	const result = {
		content: [{ type: "text", text: "x".repeat(10_000) }, { type: "image" }],
		details: { exitCode: "1", model: 7, usage: { input: "many", cost: Number.NaN }, worktreeCleanup: "merged" },
	};
	const outcome = subagentOutcome(result, true);
	assert.equal(outcome.text?.length, 4000);
	assert.deepEqual({ ...outcome, text: undefined }, { isError: true, text: undefined });
});

test("a result with no details is just its error flag", () => {
	assert.deepEqual(subagentOutcome(undefined, false), { isError: false });
	assert.deepEqual(subagentOutcome("nonsense", true), { isError: true });
});

test("a conflicted or discarded cleanup keeps a bounded list of files and its flag", () => {
	const files = [...Array.from({ length: 60 }, (_, i) => `src/${i}.rs`), 7, ""];
	const conflict = subagentOutcome(
		{ details: { worktreeCleanup: { merged: false, skipped: false, conflictFiles: files } } },
		true,
	);
	assert.equal(conflict.worktreeCleanup?.conflictFiles?.length, 50);
	assert.equal(conflict.worktreeCleanup?.conflictFiles?.[0], "src/0.rs");
	const discarded = subagentOutcome(
		{ details: { worktreeCleanup: { merged: false, skipped: true, discarded: true, conflictFiles: "a.rs" } } },
		true,
	);
	assert.deepEqual(discarded.worktreeCleanup, { merged: false, skipped: true, discarded: true });
});
