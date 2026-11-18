import type { TimelineEvent, TimelineGroup } from "./types";

export type Kind = "prompt" | "command" | "edit" | "read" | "agent" | "reply" | "other";

export const FILTERABLE: Kind[] = ["prompt", "command", "edit", "read", "agent", "reply"];

export const KIND_LABEL: Record<Kind, string> = {
  prompt: "Prompts",
  command: "Commands",
  edit: "File edits",
  read: "Reads",
  agent: "Agents",
  reply: "Replies",
  other: "Other",
};

export interface Row {
  id: string;
  kind: Kind;
  at: string;
  offsetMs: number;
  label: string;
  detail: string;
  failed: boolean;
  count: number;
  path: string | null;
  durationMs: number | null;
  events: TimelineEvent[];
  sources: string[];
}

export interface Prompt {
  event: TimelineEvent;
  text: string | null;
  bytes: number | null;
  first: string;
  more: boolean;
}

export interface TurnStats {
  events: number;
  commands: number;
  failures: number;
  edits: number;
  reads: number;
  agents: number;
  replies: number;
}

export interface Turn {
  key: string;
  number: number;
  at: string;
  startMs: number;
  endMs: number;
  prompt: Prompt | null;
  rows: Row[];
  stats: TurnStats;
}

export interface Totals extends TurnStats {
  turns: number;
  spanMs: number;
}

const FIRST_LINE_MAX = 140;

function text(v: unknown): string {
  return typeof v === "string" || typeof v === "number" ? String(v) : "";
}

export function eventKind(e: TimelineEvent): Kind {
  switch (e.kind) {
    case "user_prompt":
      return "prompt";
    case "command":
      return "command";
    case "file_edit":
      return "edit";
    case "resource_access":
      return "read";
    case "agent":
      return "agent";
    case "assistant_message":
      return "reply";
    default:
      return "other";
  }
}

export function isFailed(e: TimelineEvent): boolean {
  const d = e.data ?? {};
  if (d.failed === true) return true;
  return e.kind === "command" && typeof d.exit_code === "number" && d.exit_code !== 0;
}

function ms(at: string): number {
  const t = Date.parse(at);
  return Number.isNaN(t) ? 0 : t;
}

export function formatDuration(totalMs: number): string {
  const s = Math.max(0, Math.round(totalMs / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, "0")}m`;
}

export function formatOffset(totalMs: number): string {
  const s = Math.max(0, Math.round(totalMs / 1000));
  return `+${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function dayKey(at: string): string {
  return /^(\d{4}-\d{2}-\d{2})/.exec(at)?.[1] ?? "unknown day";
}

export function clock(at: string): string {
  return /T(\d{2}:\d{2}:\d{2})/.exec(at)?.[1] ?? at;
}

export function anchorFor(path: string): string {
  return `change-${path.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

function agentName(e: TimelineEvent): string {
  const d = e.data ?? {};
  return text(d.agent_type) || (text(d.agent_id) ? `agent ${text(d.agent_id)}` : "");
}

function describe(e: TimelineEvent): { label: string; detail: string; path: string | null } {
  const d = e.data ?? {};
  switch (e.kind) {
    case "command":
      return { label: text(d.program) || "command", detail: d.exit_code !== undefined && d.exit_code !== null ? `exit ${text(d.exit_code)}` : "", path: null };
    case "file_edit": {
      const path = text(d.path);
      return { label: text(e.tool) || text(d.operation) || "edit", detail: path, path: path || null };
    }
    case "resource_access": {
      const resource = text(d.resource);
      const where = text(d.path) || text(d.host);
      const what = resource === "search" ? "search" : resource === "web_search" ? "web search" : resource === "mcp" ? "MCP call" : resource === "web_fetch" ? "fetch" : "read";
      return { label: text(e.tool) || what, detail: where, path: null };
    }
    case "agent": {
      const name = agentName(e);
      const action = text(d.action) || "spawn";
      return { label: name ? `subagent ${name}` : "subagent", detail: action === "stop" ? "finished" : action === "start" ? "started" : "spawned", path: null };
    }
    case "assistant_message":
      return { label: "reply", detail: text(d.message_bytes) ? `${text(d.message_bytes)} bytes` : "", path: null };
    default:
      return { label: e.kind.replace(/_/g, " "), detail: "", path: null };
  }
}

function single(e: TimelineEvent, startMs: number, sources: Map<string, string>): Row {
  const k = eventKind(e);
  const d = describe(e);
  return {
    id: e.id,
    kind: k,
    at: e.at,
    offsetMs: ms(e.at) - startMs,
    label: d.label,
    detail: d.detail,
    failed: isFailed(e),
    count: 1,
    path: d.path,
    durationMs: null,
    events: [e],
    sources: [sources.get(e.id) ?? ""],
  };
}

function foldKey(r: Row): string {
  if (r.kind === "command" || r.kind === "read" || r.kind === "reply") return `${r.kind}|${r.label}`;
  if (r.kind === "edit") return `edit|${r.label}|${r.path ?? ""}`;
  return "";
}

function fold(rows: Row[]): Row[] {
  const out: Row[] = [];
  for (const r of rows) {
    const prev = out[out.length - 1];
    const key = foldKey(r);
    if (prev && key !== "" && foldKey(prev) === key && prev.failed === r.failed) {
      prev.count += r.count;
      prev.events.push(...r.events);
      prev.sources.push(...r.sources);
      if (r.kind === "command" && r.detail !== prev.detail) prev.detail = "";
      if (r.kind === "read" && r.detail !== prev.detail) prev.detail = "";
    } else {
      out.push({ ...r, events: [...r.events], sources: [...r.sources] });
    }
  }
  return out;
}

function mergeAgents(events: TimelineEvent[]): { merged: Map<string, { stop: TimelineEvent }>; skip: Set<string> } {
  const merged = new Map<string, { stop: TimelineEvent }>();
  const skip = new Set<string>();
  const open = new Map<string, TimelineEvent>();
  for (const e of events) {
    if (e.kind !== "agent") continue;
    const id = text(e.data?.agent_id);
    const action = text(e.data?.action);
    if (!id) continue;
    if (action === "start") open.set(id, e);
    if (action === "stop") {
      const start = open.get(id);
      if (start) {
        merged.set(start.id, { stop: e });
        skip.add(e.id);
        open.delete(id);
      }
    }
  }
  return { merged, skip };
}

function summarize(rows: Row[], prompt: Prompt | null, replies: number): TurnStats {
  const editPaths = new Set<string>();
  let commands = 0;
  let failures = 0;
  let reads = 0;
  let agents = 0;
  let events = prompt ? 1 : 0;
  for (const r of rows) {
    events += r.count;
    if (r.kind === "command") commands += r.count;
    if (r.kind === "read") reads += r.count;
    if (r.kind === "agent") agents += 1;
    if (r.kind === "edit" && r.path) editPaths.add(r.path);
    if (r.kind === "edit" && !r.path) editPaths.add(r.id);
    if (r.failed) failures += r.count;
  }
  return { events, commands, failures, edits: editPaths.size, reads, agents, replies };
}

export function buildPrompt(e: TimelineEvent): Prompt {
  const t = typeof e.prompt_text === "string" ? e.prompt_text : null;
  const trimmed = t?.trim() ?? "";
  const firstLine = trimmed.split("\n")[0] ?? "";
  const first = firstLine.length > FIRST_LINE_MAX ? `${firstLine.slice(0, FIRST_LINE_MAX)}…` : firstLine;
  const bytes = typeof e.data?.prompt_bytes === "number" ? e.data.prompt_bytes : null;
  return { event: e, text: t, bytes, first, more: trimmed.length > first.length || trimmed.includes("\n") };
}

export function buildTurns(groups: TimelineGroup[]): Turn[] {
  const sources = new Map<string, string>();
  const all: TimelineEvent[] = [];
  for (const g of groups) {
    for (const e of g.events) {
      sources.set(e.id, g.id);
      all.push(e);
    }
  }
  all.sort((a, b) => a.seq - b.seq);

  const chunks: { prompt: TimelineEvent | null; events: TimelineEvent[] }[] = [];
  for (const e of all) {
    if (e.kind === "user_prompt") chunks.push({ prompt: e, events: [] });
    else {
      if (chunks.length === 0) chunks.push({ prompt: null, events: [] });
      chunks[chunks.length - 1]!.events.push(e);
    }
  }

  return chunks.map((c, i) => {
    const startMs = ms((c.prompt ?? c.events[0])!.at);
    const { merged, skip } = mergeAgents(c.events);
    const raw: Row[] = [];
    let replies = 0;
    for (const e of c.events) {
      if (skip.has(e.id)) continue;
      const row = single(e, startMs, sources);
      const pair = merged.get(e.id);
      if (pair) {
        row.detail = `finished in ${formatDuration(ms(pair.stop.at) - ms(e.at))}`;
        row.durationMs = ms(pair.stop.at) - ms(e.at);
        row.events.push(pair.stop);
        row.sources.push(sources.get(pair.stop.id) ?? "");
        row.failed = row.failed || isFailed(pair.stop);
      }
      if (row.kind === "reply") replies += 1;
      raw.push(row);
    }
    const rows = fold(raw);
    const prompt = c.prompt ? buildPrompt(c.prompt) : null;
    const last = c.events[c.events.length - 1] ?? c.prompt!;
    const endMs = Math.max(startMs, ms(last.at));
    return { key: (c.prompt ?? c.events[0]!).id, number: i + 1, at: (c.prompt ?? c.events[0]!).at, startMs, endMs, prompt, rows, stats: summarize(rows, prompt, replies) };
  });
}

export function totalsOf(turns: Turn[]): Totals {
  const t: Totals = { turns: turns.length, events: 0, commands: 0, failures: 0, edits: 0, reads: 0, agents: 0, replies: 0, spanMs: 0 };
  for (const x of turns) {
    t.events += x.stats.events;
    t.commands += x.stats.commands;
    t.failures += x.stats.failures;
    t.edits += x.stats.edits;
    t.reads += x.stats.reads;
    t.agents += x.stats.agents;
    t.replies += x.stats.replies;
  }
  if (turns.length > 0) t.spanMs = Math.max(0, turns[turns.length - 1]!.endMs - turns[0]!.startMs);
  return t;
}

export interface Filters {
  kinds: Set<Kind>;
  failuresOnly: boolean;
  query: string;
}

export function allKinds(): Set<Kind> {
  return new Set<Kind>([...FILTERABLE, "other"]);
}

export function isFiltering(f: Filters): boolean {
  return f.failuresOnly || f.query.trim() !== "" || f.kinds.size !== FILTERABLE.length + 1;
}

export function rowMatches(r: Row, f: Filters): boolean {
  if (!f.kinds.has(r.kind)) return false;
  if (f.failuresOnly && !r.failed) return false;
  const q = f.query.trim().toLowerCase();
  if (q === "") return true;
  return `${r.label} ${r.detail} ${r.path ?? ""}`.toLowerCase().includes(q);
}

export function promptMatches(p: Prompt | null, f: Filters): boolean {
  if (!p) return false;
  if (!f.kinds.has("prompt") || f.failuresOnly) return false;
  const q = f.query.trim().toLowerCase();
  if (q === "") return true;
  return `${p.text ?? ""}`.toLowerCase().includes(q);
}

export function visibleRows(t: Turn, f: Filters): Row[] {
  return t.rows.filter((r) => rowMatches(r, f));
}

export function turnVisible(t: Turn, f: Filters): boolean {
  if (!isFiltering(f)) return true;
  return promptMatches(t.prompt, f) || visibleRows(t, f).length > 0;
}
