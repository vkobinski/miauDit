import type { TimelineEvent, TimelineGroup } from "./types";
import { Badge, ProvBadge } from "./components";
import { plural, short } from "./util";

const LABELS: Record<string, string> = {
  user_prompt: "prompt",
  assistant_message: "reply",
  command: "command",
  file_edit: "file edit",
  resource_access: "read / fetch",
  agent: "agent",
  tool_call: "tool call",
};

function text(v: unknown): string {
  return typeof v === "string" || typeof v === "number" ? String(v) : "";
}

export function describe(e: TimelineEvent): string {
  const d = e.data ?? {};
  switch (e.kind) {
    case "user_prompt":
      return e.prompt_text ?? `prompt, ${text(d.prompt_bytes) || "?"} bytes (the text stays on the developer's machine)`;
    case "assistant_message":
      return `assistant reply, ${text(d.message_bytes) || "?"} bytes (digest only, no text stored)`;
    case "command":
      return [text(d.program) || "command", d.exit_code !== undefined && d.exit_code !== null ? `exit ${text(d.exit_code)}` : "", d.failed === true ? "failed" : ""].filter(Boolean).join(" · ");
    case "file_edit":
      return [text(e.tool) || text(d.operation), text(d.path)].filter(Boolean).join(" ") || "file edit";
    case "resource_access":
      return [text(e.tool), text(d.host) || text(d.path) || text(d.server)].filter(Boolean).join(" ") || "resource access";
    case "agent":
      return [text(d.action) || "spawn", text(d.agent_type)].filter(Boolean).join(" ");
    default:
      return e.kind.replace(/_/g, " ");
  }
}

function clock(at: string): string {
  const m = /T(\d{2}:\d{2}:\d{2})/.exec(at);
  return m?.[1] ?? at;
}

function EventRow({ e }: { e: TimelineEvent }) {
  const failed = e.data?.failed === true;
  return (
    <li className="tl-ev">
      <span className="tl-time" title={e.at}>
        {clock(e.at)}
      </span>
      <span className="tl-kind">
        <Badge kind={failed ? "bad" : e.kind === "user_prompt" ? "warn" : "neu"}>{LABELS[e.kind] ?? e.kind}</Badge>
      </span>
      <span className={e.kind === "user_prompt" && e.prompt_text ? "tl-quote" : undefined}>{describe(e)}</span>
    </li>
  );
}

function summaryLine(g: TimelineGroup): string {
  const s = g.summary;
  if (!s) return plural(g.events.length, "event");
  return [plural(s.prompts, "prompt"), plural(s.assistant_messages, "reply", "replies"), plural(s.commands, "command"), plural(s.resources, "read"), plural(s.agent_events, "agent event")].filter((p) => !p.startsWith("0 ")).join(" · ");
}

export function Timeline({ groups }: { groups: TimelineGroup[] }) {
  const checkpoints = groups.filter((g) => g.source === "checkpoint").length;
  return (
    <div className="card">
      <h2>Conversation</h2>
      <p className="muted small">
        Every event recorded for this audit commit, in order: the events committed with it, then {plural(checkpoints, "checkpoint")} added since. Checkpoints are attestations: the audit ID does not change when they are added.
      </p>
      {groups.length === 0 && <p className="muted">No hook events were recorded for this audit commit.</p>}
      {groups.map((g, i) => (
        <details className="tl-group" key={g.id} open={i === 0 || i === groups.length - 1}>
          <summary>
            {g.source === "audit" ? <b>Committed with this audit</b> : <b>Checkpoint</b>} <span className="muted">· {g.created_at ? g.created_at.replace("T", " ").replace("Z", " UTC") : "time unknown"} · {summaryLine(g)}</span>
            <span className="mono muted">{short(g.id)}</span> <ProvBadge p="observed" />
          </summary>
          {g.state !== "present" ? (
            <p className="muted small">The event details for this group are not available here ({g.state}).</p>
          ) : (
            <ol className="tl-list">
              {g.events.map((e) => (
                <EventRow key={e.id} e={e} />
              ))}
            </ol>
          )}
          {g.truncated && <p className="muted small">Only the first events of this group are shown.</p>}
        </details>
      ))}
    </div>
  );
}
