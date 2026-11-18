import { type ReactNode, useMemo, useState } from "react";
import { Badge, ProvBadge } from "./components";
import { FILTERABLE, KIND_LABEL, type Filters, type Kind, type Row, type Turn, allKinds, anchorFor, buildTurns, clock, dayKey, formatDuration, formatOffset, isFiltering, totalsOf, turnVisible, visibleRows } from "./timelineModel";
import type { TimelineEvent, TimelineGroup } from "./types";
import { plural, short } from "./util";

const PAGE = 30;
const GAP_MS = 10 * 60 * 1000;

const KIND_SHORT: Record<Kind, string> = { prompt: "prompt", command: "command", edit: "edit", read: "read", agent: "agent", reply: "reply", other: "other" };

function foldedLine(e: TimelineEvent): string {
  const d = e.data ?? {};
  const where = typeof d.path === "string" ? d.path : typeof d.host === "string" ? d.host : "";
  const exit = typeof d.exit_code === "number" ? `exit ${d.exit_code}` : "";
  return [clock(e.at), where, exit].filter(Boolean).join(" · ");
}

function RowView({ row, changed }: { row: Row; changed: Set<string> }) {
  const linked = row.path !== null && changed.has(row.path);
  let detail: ReactNode = row.detail;
  if (linked && row.path) detail = <a href={`#${anchorFor(row.path)}`}>{row.path}</a>;
  return (
    <li className={`tl-row${row.failed ? " tl-fail" : ""}`} title={row.kind === "reply" ? "Replies are stored as a digest only; the text is not kept." : row.at}>
      <span className="tl-off">{formatOffset(row.offsetMs)}</span>
      <span className="tl-kind">
        <Badge kind={row.failed ? "bad" : "neu"}>{KIND_SHORT[row.kind]}</Badge>
      </span>
      <span className="tl-text">
        <b>{row.label}</b>
        {row.count > 1 && <span className="tl-count"> ×{row.count}</span>}
        {detail !== "" && <span className="muted"> · </span>}
        {detail}
        {row.failed && (
          <>
            {" "}
            <Badge kind="bad">failed</Badge>
          </>
        )}
        {row.count > 1 && (
          <details className="tl-fold">
            <summary>show all {row.count}</summary>
            <ol>
              {row.events.map((e) => (
                <li key={e.id} className="mono">
                  {foldedLine(e)}
                </li>
              ))}
            </ol>
          </details>
        )}
      </span>
    </li>
  );
}

function turnTitle(t: Turn): string {
  if (!t.prompt) return "Before the first prompt";
  if (t.prompt.text !== null && t.prompt.first !== "") return t.prompt.first;
  if (t.prompt.text === null) return `prompt, ${t.prompt.bytes ?? "?"} bytes (the text stays on the developer's machine)`;
  return "prompt";
}

function TurnView({ turn, open, onToggle, filters, changed }: { turn: Turn; open: boolean; onToggle: () => void; filters: Filters; changed: Set<string> }) {
  const s = turn.stats;
  const rows = open ? visibleRows(turn, filters) : [];
  const parts = [s.commands > 0 ? plural(s.commands, "command") : "", s.edits > 0 ? `${plural(s.edits, "file")} edited` : "", formatDuration(turn.endMs - turn.startMs)].filter(Boolean);
  return (
    <li className="tl-turn">
      <button type="button" className="tl-head" aria-expanded={open} onClick={onToggle}>
        <span className="tl-caret" aria-hidden="true">
          {open ? "▾" : "▸"}
        </span>
        <span className="tl-num">Turn {turn.number}</span>
        <span className="tl-title">{turnTitle(turn)}</span>
        <span className="tl-meta">
          {clock(turn.at)} · {parts.join(" · ")}
          {s.failures > 0 && (
            <>
              {" "}
              <Badge kind="bad">{s.failures} failed</Badge>
            </>
          )}
        </span>
      </button>
      {open && (
        <div className="tl-body">
          {turn.prompt?.text && turn.prompt.more && <div className="tl-quote">{turn.prompt.text}</div>}
          {rows.length === 0 ? (
            <p className="muted small">Nothing else was recorded in this turn{isFiltering(filters) ? " that matches the filters" : ""}.</p>
          ) : (
            <ol className="tl-rows">
              {rows.map((r) => (
                <RowView key={r.id} row={r} changed={changed} />
              ))}
            </ol>
          )}
        </div>
      )}
    </li>
  );
}

function Chip({ on, count, onClick, children }: { on: boolean; count?: number; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" className="tl-chip" aria-pressed={on} onClick={onClick}>
      {children}
      {count !== undefined && <span className="tl-chipn"> {count}</span>}
    </button>
  );
}

export function Timeline({ groups, changedPaths = [] }: { groups: TimelineGroup[]; changedPaths?: string[] }) {
  const turns = useMemo(() => buildTurns(groups), [groups]);
  const totals = useMemo(() => totalsOf(turns), [turns]);
  const changed = useMemo(() => new Set(changedPaths), [changedPaths]);
  const [kinds, setKinds] = useState<Set<Kind>>(allKinds);
  const [failuresOnly, setFailuresOnly] = useState(false);
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(PAGE);
  const [openKeys, setOpenKeys] = useState<Set<string>>(() => new Set(turns.length > 0 ? [turns[turns.length - 1]!.key] : []));
  const filters: Filters = { kinds, failuresOnly, query };
  const filtering = isFiltering(filters);
  const checkpoints = groups.filter((g) => g.source === "checkpoint").length;
  const matching = turns.filter((t) => turnVisible(t, filters));
  const visible = matching.slice(-shown);
  const hidden = matching.length - visible.length;
  const counts: Record<Kind, number> = { prompt: totals.turns, command: totals.commands, edit: totals.edits, read: totals.reads, agent: totals.agents, reply: totals.replies, other: 0 };

  const toggleKind = (k: Kind) =>
    setKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  const reset = () => {
    setKinds(allKinds());
    setFailuresOnly(false);
    setQuery("");
  };
  const toggleTurn = (key: string) =>
    setOpenKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const list: ReactNode[] = [];
  let previous: Turn | null = null;
  for (const t of visible) {
    const gap = previous ? t.startMs - previous.endMs : 0;
    if (!previous || dayKey(t.at) !== dayKey(previous.at)) list.push(<li key={`day-${t.key}`} className="tl-day">{dayKey(t.at)} (UTC)</li>);
    else if (gap > GAP_MS) list.push(<li key={`gap-${t.key}`} className="tl-gap">{formatDuration(gap)} later</li>);
    list.push(<TurnView key={t.key} turn={t} open={filtering || openKeys.has(t.key)} onToggle={() => toggleTurn(t.key)} filters={filters} changed={changed} />);
    previous = t;
  }

  return (
    <div className="card">
      <h2>Conversation</h2>
      <p className="muted small">
        {checkpoints === 0
          ? "Everything recorded for this audit commit, in order."
          : `Everything recorded for this audit commit, in order: what was committed with it, then ${plural(checkpoints, "checkpoint")} added since. Checkpoints are attestations, so the audit ID does not change when they are added.`}
      </p>
      {groups.length === 0 ? (
        <p className="muted">No hook events were recorded for this audit commit.</p>
      ) : (
        <>
          <p className="tl-summary">
            <b>{plural(totals.turns, "turn")}</b> · {plural(totals.events, "event")} · {plural(totals.commands, "command")} · {plural(totals.edits, "file")} edited · {totals.failures > 0 ? <Badge kind="bad">{totals.failures} failed</Badge> : "no failures"} · over {formatDuration(totals.spanMs)}
          </p>
          <ul className="tl-sources">
            {groups.map((g) => (
              <li key={g.id}>
                <span className="tl-src">
                  {g.source === "audit" ? "Committed with this audit" : `Checkpoint ${short(g.id)}`} · {g.state === "present" ? plural(g.events.length, "event") : `event details not available here (${g.state})`}
                </span>{" "}
                <ProvBadge p="observed" />
              </li>
            ))}
          </ul>
          <div className="tl-controls">
            {FILTERABLE.map((k) => (
              <Chip key={k} on={kinds.has(k)} count={counts[k]} onClick={() => toggleKind(k)}>
                {KIND_LABEL[k]}
              </Chip>
            ))}
            <Chip on={failuresOnly} onClick={() => setFailuresOnly((v) => !v)}>
              Failures only
            </Chip>
            <input className="tl-search" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search the conversation…" aria-label="Search the conversation" />
            <button type="button" className="tl-chip" onClick={() => setOpenKeys(new Set(matching.map((t) => t.key)))}>
              Expand all
            </button>
            <button type="button" className="tl-chip" onClick={() => setOpenKeys(new Set())}>
              Collapse all
            </button>
          </div>
          <p className="muted small">Commands, reads and replies are stored as hashes, so only their kind, size and outcome are shown. Prompt text appears only in the local viewer.</p>
          {matching.length === 0 ? (
            <p className="muted">
              No turns match the current filters.{" "}
              <button type="button" className="tl-chip" onClick={reset}>
                Clear filters
              </button>
            </p>
          ) : (
            <>
              {hidden > 0 && (
                <button type="button" className="tl-chip tl-more" onClick={() => setShown((n) => n + PAGE)}>
                  Show {Math.min(PAGE, hidden)} earlier {Math.min(PAGE, hidden) === 1 ? "turn" : "turns"} ({plural(hidden, "turn")} hidden)
                </button>
              )}
              <ol className="tl-turns">{list}</ol>
            </>
          )}
          {groups.some((g) => g.truncated) && <p className="muted small">Only the first 3000 events of a group are loaded.</p>}
        </>
      )}
    </div>
  );
}
