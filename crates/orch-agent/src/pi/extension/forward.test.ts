import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { spawnHook } from "./forward.ts";

test("the payload reaches the hook command on its stdin", async () => {
	const out = join(mkdtempSync(join(tmpdir(), "orch-pi-")), "payload");
	const copy = `require("fs").writeFileSync(${JSON.stringify(out)}, require("fs").readFileSync(0))`;
	await spawnHook(process.execPath, ["-e", copy], '{"event":"input"}');
	assert.equal(readFileSync(out, "utf8"), '{"event":"input"}');
});

test("a missing orch binary is shrugged off", async () => {
	await spawnHook("/nonexistent/orch", ["hook"], "{}");
});
