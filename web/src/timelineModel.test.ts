import { allKinds, anchorFor, buildPrompt, buildTurns, clock, dayKey, eventKind, formatDuration, formatOffset, isFailed, isFiltering, promptMatches, totalsOf, turnVisible, visibleRows, type Filters } from "./timelineModel";
import type { TimelineEvent, TimelineGroup } from "./types";

let n = 0;
const ev = (kind: string, data: Record<string, unknown> = {}, at = "2026-01-01T10:00:00Z", extra: Partial<TimelineEvent> = {}): TimelineEvent => {
  n += 1;
  return { id: `evt_${n}`, seq: n, at, kind, tool: null, session_id: "s1", data, ...extra };
};
const group = (events: TimelineEvent[], id = "chk_a", source: "audit" | "checkpoint" = "checkpoint"): TimelineGroup => ({ source, id, created_at: null, summary: null, state: "present", events, truncated: false });
const at = (s: number) => new Date(Date.UTC(2026, 0, 1, 10, 0, s)).toISOString().replace(".000", "");
const open = (): Filters => ({ kinds: allKinds(), failuresOnly: false, query: "" });

beforeEach(() => {
  n = 0;
});

describe("turns", () => {
  it("start at each prompt and keep everything until the next one underneath", () => {
    const g = group([ev("user_prompt", { prompt_bytes: 5 }, at(0)), ev("command", { program: "ls" }, at(1)), ev("assistant_message", { message_bytes: 9 }, at(2)), ev("user_prompt", {}, at(10)), ev("command", { program: "pwd" }, at(11))]);
    const turns = buildTurns([g]);
    expect(turns.map((t) => [t.number, t.rows.length])).toEqual([
      [1, 2],
      [2, 1],
    ]);
    expect(turns[0]!.rows.map((r) => r.label)).toEqual(["ls", "reply"]);
  });

  it("puts events before the first prompt in a turn of their own with no prompt", () => {
    const turns = buildTurns([group([ev("command", { program: "ls" }, at(0)), ev("user_prompt", {}, at(5))])]);
    expect(turns).toHaveLength(2);
    expect(turns[0]!.prompt).toBeNull();
    expect(turns[0]!.rows).toHaveLength(1);
    expect(turns[1]!.prompt).not.toBeNull();
  });

  it("orders events by sequence across groups and remembers which group each came from", () => {
    const first = group([ev("user_prompt", {}, at(0)), ev("command", { program: "a" }, at(1))], "evd_audit", "audit");
    const second = group([ev("command", { program: "b" }, at(2))], "chk_later");
    const turns = buildTurns([second, first]);
    expect(turns).toHaveLength(1);
    expect(turns[0]!.rows.map((r) => r.label)).toEqual(["a", "b"]);
    expect(turns[0]!.rows.map((r) => r.sources[0])).toEqual(["evd_audit", "chk_later"]);
  });

  it("measures time from the start of the turn and its duration", () => {
    const [t] = buildTurns([group([ev("user_prompt", {}, at(0)), ev("command", { program: "ls" }, at(7)), ev("assistant_message", {}, at(75))])]);
    expect(t!.rows.map((r) => r.offsetMs)).toEqual([7000, 75000]);
    expect(t!.endMs - t!.startMs).toBe(75000);
  });
});

describe("folding", () => {
  it("folds runs of the same command, read tool or edited file into one counted row", () => {
    const [t] = buildTurns([
      group([
        ev("user_prompt", {}, at(0)),
        ev("command", { program: "kubectl", exit_code: 0 }, at(1)),
        ev("command", { program: "kubectl", exit_code: 0 }, at(2)),
        ev("command", { program: "kubectl", exit_code: 0 }, at(3)),
        ev("resource_access", { resource: "file", path: "a.rs" }, at(4), { tool: "Read" }),
        ev("resource_access", { resource: "file", path: "b.rs" }, at(5), { tool: "Read" }),
        ev("file_edit", { path: "src/x.rs" }, at(6), { tool: "Edit" }),
        ev("file_edit", { path: "src/x.rs" }, at(7), { tool: "Edit" }),
        ev("file_edit", { path: "src/y.rs" }, at(8), { tool: "Edit" }),
      ]),
    ]);
    expect(t!.rows.map((r) => [r.label, r.count])).toEqual([
      ["kubectl", 3],
      ["Read", 2],
      ["Edit", 2],
      ["Edit", 1],
    ]);
    expect(t!.rows[0]!.detail).toBe("exit 0");
    expect(t!.rows[1]!.detail).toBe("");
    expect(t!.stats).toMatchObject({ events: 9, commands: 3, reads: 2, edits: 2 });
  });

  it("does not fold different programs or a failed command into a successful run", () => {
    const [t] = buildTurns([group([ev("user_prompt", {}, at(0)), ev("command", { program: "bao", exit_code: 0 }, at(1)), ev("command", { program: "kubectl", exit_code: 0 }, at(2)), ev("command", { program: "kubectl", exit_code: 1 }, at(3)), ev("command", { program: "kubectl", exit_code: 0 }, at(4))])]);
    expect(t!.rows.map((r) => [r.label, r.count, r.failed])).toEqual([
      ["bao", 1, false],
      ["kubectl", 1, false],
      ["kubectl", 1, true],
      ["kubectl", 1, false],
    ]);
    expect(t!.stats.failures).toBe(1);
  });
});

describe("failures", () => {
  it("are a command with a non-zero exit code or anything flagged failed", () => {
    expect(isFailed(ev("command", { program: "x", exit_code: 2 }))).toBe(true);
    expect(isFailed(ev("command", { program: "x", exit_code: 0 }))).toBe(false);
    expect(isFailed(ev("command", { program: "x", exit_code: null }))).toBe(false);
    expect(isFailed(ev("file_edit", { path: "p", failed: true }))).toBe(true);
    expect(isFailed(ev("resource_access", { failed: true }))).toBe(true);
    expect(isFailed(ev("assistant_message", {}))).toBe(false);
  });
});

describe("subagents", () => {
  it("merges a start and its stop into one row with the type and the time it took", () => {
    const [t] = buildTurns([
      group([
        ev("user_prompt", {}, at(0)),
        ev("agent", { action: "start", agent_id: "ag-1", agent_type: "reviewer" }, at(1)),
        ev("command", { program: "ls" }, at(3)),
        ev("agent", { action: "stop", agent_id: "ag-1", agent_type: "reviewer" }, at(13)),
      ]),
    ]);
    expect(t!.rows.map((r) => r.kind)).toEqual(["agent", "command"]);
    expect(t!.rows[0]).toMatchObject({ label: "subagent reviewer", detail: "finished in 12s", durationMs: 12000 });
    expect(t!.rows[0]!.events).toHaveLength(2);
    expect(t!.stats.agents).toBe(1);
  });

  it("names an unpaired event by what it is, and falls back when the type is missing", () => {
    const [t] = buildTurns([group([ev("user_prompt", {}, at(0)), ev("agent", { action: "stop", agent_id: "ag-9", agent_type: "" }, at(1)), ev("agent", { action: "spawn", agent_type: "explorer" }, at(2)), ev("agent", { action: "stop" }, at(3))])]);
    expect(t!.rows.map((r) => [r.label, r.detail])).toEqual([
      ["subagent agent ag-9", "finished"],
      ["subagent explorer", "spawned"],
      ["subagent", "finished"],
    ]);
  });
});

describe("prompts", () => {
  it("keep the first line, flag when there is more, and cut very long first lines", () => {
    const one = buildPrompt(ev("user_prompt", { prompt_bytes: 5 }, at(0), { prompt_text: "short" }));
    expect(one).toMatchObject({ first: "short", more: false, bytes: 5 });
    const many = buildPrompt(ev("user_prompt", {}, at(0), { prompt_text: "first line\nsecond line" }));
    expect(many).toMatchObject({ first: "first line", more: true });
    const long = buildPrompt(ev("user_prompt", {}, at(0), { prompt_text: "x".repeat(300) }));
    expect(long.first).toHaveLength(141);
    expect(long.first.endsWith("…")).toBe(true);
    expect(long.more).toBe(true);
    const none = buildPrompt(ev("user_prompt", { prompt_bytes: 20 }));
    expect(none).toMatchObject({ text: null, first: "", more: false, bytes: 20 });
  });
});

describe("filters", () => {
  const turns = () =>
    buildTurns([
      group([
        ev("user_prompt", {}, at(0), { prompt_text: "rotate the keys" }),
        ev("command", { program: "bao", exit_code: 0 }, at(1)),
        ev("command", { program: "kubectl", exit_code: 1 }, at(2)),
        ev("file_edit", { path: "src/keys.rs" }, at(3), { tool: "Write" }),
        ev("user_prompt", {}, at(20), { prompt_text: "and the rollout?" }),
        ev("assistant_message", { message_bytes: 4 }, at(21)),
      ]),
    ]);

  it("are off by default so nothing is hidden", () => {
    expect(isFiltering(open())).toBe(false);
    expect(turns().every((t) => turnVisible(t, open()))).toBe(true);
  });

  it("hide kinds that are switched off", () => {
    const f = open();
    f.kinds.delete("command");
    expect(isFiltering(f)).toBe(true);
    expect(visibleRows(turns()[0]!, f).map((r) => r.kind)).toEqual(["edit"]);
  });

  it("show only failures when asked, and drop turns that have none", () => {
    const f = { ...open(), failuresOnly: true };
    expect(visibleRows(turns()[0]!, f).map((r) => r.label)).toEqual(["kubectl"]);
    expect(turnVisible(turns()[0]!, f)).toBe(true);
    expect(turnVisible(turns()[1]!, f)).toBe(false);
  });

  it("search labels, details, paths and local prompt text, case-insensitively", () => {
    const [a, b] = turns();
    expect(visibleRows(a!, { ...open(), query: "KEYS.RS" }).map((r) => r.path)).toEqual(["src/keys.rs"]);
    expect(turnVisible(a!, { ...open(), query: "rotate" })).toBe(true);
    expect(turnVisible(b!, { ...open(), query: "rotate" })).toBe(false);
    expect(promptMatches(a!.prompt, { ...open(), query: "rotate" })).toBe(true);
    expect(turnVisible(a!, { ...open(), query: "exit 1" })).toBe(true);
    expect(turnVisible(a!, { ...open(), query: "nothing like this" })).toBe(false);
  });
});

describe("totals and formatting", () => {
  it("add up the turns and the time between the first and last event", () => {
    const turns = buildTurns([group([ev("user_prompt", {}, at(0)), ev("command", { program: "a", exit_code: 1 }, at(5)), ev("user_prompt", {}, at(30)), ev("assistant_message", {}, at(50))])]);
    expect(totalsOf(turns)).toMatchObject({ turns: 2, events: 4, commands: 1, failures: 1, replies: 1, spanMs: 50000 });
    expect(totalsOf([])).toMatchObject({ turns: 0, events: 0, spanMs: 0 });
  });

  it("format durations, offsets, days and clocks", () => {
    expect(formatDuration(0)).toBe("0s");
    expect(formatDuration(12_400)).toBe("12s");
    expect(formatDuration(134_000)).toBe("2m 14s");
    expect(formatDuration(3_900_000)).toBe("1h 05m");
    expect(formatOffset(7000)).toBe("+0:07");
    expect(formatOffset(75_000)).toBe("+1:15");
    expect(dayKey("2026-11-18T19:26:23Z")).toBe("2026-11-18");
    expect(dayKey("garbage")).toBe("unknown day");
    expect(clock("2026-11-18T19:26:23Z")).toBe("19:26:23");
  });

  it("classify event kinds and make safe anchors for file paths", () => {
    expect(["user_prompt", "command", "file_edit", "resource_access", "agent", "assistant_message", "tool_call"].map((k) => eventKind(ev(k)))).toEqual(["prompt", "command", "edit", "read", "agent", "reply", "other"]);
    expect(anchorFor("src/a b/<x>.rs")).toBe("change-src_a_b__x__rs");
  });
});
