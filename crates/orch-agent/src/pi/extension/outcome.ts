// How a `subagent` call ended, taken from the tool's final result. The user's
// subagent extension puts a `SingleResult` in `details`; all of it comes from
// inside the Sandbox, so only known fields of the expected type are kept and
// every string is cut short. orch bounds it again, tighter, on its side.

const FORWARD_TEXT_CHARS = 4000;
const FORWARD_REASON_CHARS = 1000;
const FORWARD_NAME_CHARS = 200;

export interface SubagentUsage {
	input?: number;
	output?: number;
	cost?: number;
	turns?: number;
}

export interface WorktreeCleanup {
	merged?: boolean;
	skipped?: boolean;
	message?: string;
	preservedBranch?: string;
}

export interface SubagentOutcome {
	isError: boolean;
	exitCode?: number;
	stopReason?: string;
	errorMessage?: string;
	model?: string;
	usage?: SubagentUsage;
	text?: string;
	worktreePath?: string;
	worktreeBranch?: string;
	worktreeCleanup?: WorktreeCleanup;
}

type Fields = Record<string, unknown>;

const record = (value: unknown): Fields | undefined =>
	typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Fields) : undefined;

const text = (value: unknown, max: number) =>
	typeof value === "string" && value.length > 0 ? value.slice(0, max) : undefined;

const number = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : undefined);

const flag = (value: unknown) => (typeof value === "boolean" ? value : undefined);

/** Drops the keys whose value is undefined, so the payload stays small. */
function defined<T extends object>(fields: T): T {
	return Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined)) as T;
}

function finalText(result: Fields): string | undefined {
	const content = Array.isArray(result.content) ? result.content : [];
	const parts = content.flatMap((part) => {
		const fields = record(part);
		return fields?.type === "text" && typeof fields.text === "string" ? [fields.text] : [];
	});
	return text(parts.join("\n"), FORWARD_TEXT_CHARS);
}

function usageOf(value: unknown): SubagentUsage | undefined {
	const usage = record(value);
	if (!usage) return undefined;
	const kept = defined({
		input: number(usage.input),
		output: number(usage.output),
		cost: number(usage.cost),
		turns: number(usage.turns),
	});
	return Object.keys(kept).length > 0 ? kept : undefined;
}

function cleanupOf(value: unknown): WorktreeCleanup | undefined {
	const cleanup = record(value);
	if (!cleanup) return undefined;
	return defined({
		merged: flag(cleanup.merged),
		skipped: flag(cleanup.skipped),
		message: text(cleanup.message, FORWARD_REASON_CHARS),
		preservedBranch: text(cleanup.preservedBranch, FORWARD_NAME_CHARS),
	});
}

export function subagentOutcome(result: unknown, isError: boolean): SubagentOutcome {
	const fields = record(result) ?? {};
	const details = record(fields.details) ?? {};
	return defined({
		isError: isError === true,
		exitCode: number(details.exitCode),
		stopReason: text(details.stopReason, FORWARD_NAME_CHARS),
		errorMessage: text(details.errorMessage, FORWARD_REASON_CHARS),
		model: text(details.model, FORWARD_NAME_CHARS),
		usage: usageOf(details.usage),
		text: finalText(fields),
		worktreePath: text(details.worktreePath, FORWARD_REASON_CHARS),
		worktreeBranch: text(details.worktreeBranch, FORWARD_NAME_CHARS),
		worktreeCleanup: cleanupOf(details.worktreeCleanup),
	});
}
