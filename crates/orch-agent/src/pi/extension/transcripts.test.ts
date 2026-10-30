import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { SubagentTranscripts } from "./transcripts.ts";

const said = (text: string) => ({ role: "assistant", content: [{ type: "text", text }] });
const task = { role: "user", content: [{ type: "text", text: "Task: find the redirect" }] };

function transcripts(): { writer: SubagentTranscripts; file: string; dir: string } {
	const dir = mkdtempSync(join(tmpdir(), "orch-pi-transcripts-"));
	return { writer: new SubagentTranscripts(dir), file: join(dir, "subagents", "conv-1", "call_1.jsonl"), dir };
}

const lines = (file: string) =>
	readFileSync(file, "utf8")
		.split("\n")
		.filter(Boolean)
		.map((line) => JSON.parse(line));

test("each update appends only the messages the file doesn't have yet", () => {
	const { writer, file } = transcripts();
	writer.write("conv-1", "call_1", [task]);
	const first = statSync(file).ino;
	writer.write("conv-1", "call_1", [task, said("Searching.")]);
	writer.write("conv-1", "call_1", [task, said("Searching.")]);
	writer.write("conv-1", "call_1", [task, said("Searching."), said("Found it.")]);
	assert.deepEqual(lines(file), [task, said("Searching."), said("Found it.")]);
	assert.equal(statSync(file).ino, first);
});

test("a shorter list replaces the file with a new one", () => {
	const { writer, file } = transcripts();
	writer.write("conv-1", "call_1", [task, said("Searching."), said("Failed over.")]);
	const first = statSync(file).ino;
	writer.write("conv-1", "call_1", [task]);
	assert.deepEqual(lines(file), [task]);
	assert.notEqual(statSync(file).ino, first);
});

test("the first write of a call replaces whatever file was there", () => {
	const { dir, file } = transcripts();
	new SubagentTranscripts(dir).write("conv-1", "call_1", [task, said("Old.")]);
	new SubagentTranscripts(dir).write("conv-1", "call_1", [task, said("Old."), said("New.")]);
	assert.deepEqual(lines(file), [task, said("Old."), said("New.")]);
});

test("each call of each Conversation has its own file", () => {
	const { writer, dir } = transcripts();
	writer.write("conv-1", "call_1", [task]);
	writer.write("conv-2", "call_1", [said("Other.")]);
	assert.deepEqual(lines(join(dir, "subagents", "conv-2", "call_1.jsonl")), [said("Other.")]);
	assert.deepEqual(lines(join(dir, "subagents", "conv-1", "call_1.jsonl")), [task]);
});

test("an id that isn't a plain file name writes nothing", () => {
	const { writer, dir } = transcripts();
	for (const [conversation, call] of [
		["..", "call_1"],
		["conv-1", "../escape"],
		["conv-1", "parent/1"],
		["", "call_1"],
		["conv-1", ".hidden"],
	]) {
		writer.write(conversation, call, [task]);
	}
	assert.equal(existsSync(join(dir, "subagents")), false);
	assert.equal(existsSync(join(dir, "escape.jsonl")), false);
});
