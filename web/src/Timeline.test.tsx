import { render, screen, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { AuditView } from "./pages/AuditPage";
import { RepoPage } from "./pages/RepoPage";
import { Timeline, describe as describeEvent } from "./Timeline";
import { detail } from "./fixtures";
import type { TimelineEvent, TimelineGroup } from "./types";

const ev = (seq: number, kind: string, data: Record<string, unknown> = {}, extra: Partial<TimelineEvent> = {}): TimelineEvent => ({
  id: `evt_${seq}`,
  seq,
  at: `2026-01-01T10:00:0${seq}Z`,
  kind,
  tool: null,
  session_id: "s1",
  data,
  ...extra,
});

const group = (over: Partial<TimelineGroup> = {}): TimelineGroup => ({
  source: "checkpoint",
  id: `chk_${"a".repeat(64)}`,
  created_at: "2026-01-01T10:05:00Z",
  summary: { prompts: 1, assistant_messages: 1, commands: 1, resources: 0, agent_events: 0 },
  state: "present",
  events: [ev(1, "user_prompt", { prompt_bytes: 20 }, { prompt_text: "who owns the keys?" }), ev(2, "command", { program: "kubectl", exit_code: 0 }), ev(3, "assistant_message", { message_bytes: 12 })],
  truncated: false,
  ...over,
});

const show = (groups: TimelineGroup[]) =>
  render(
    <MemoryRouter>
      <Timeline groups={groups} />
    </MemoryRouter>,
  );

describe("Timeline", () => {
  it("shows the whole conversation of an audit in order, with counts per checkpoint", () => {
    show([group({ source: "audit", id: `evd_${"b".repeat(64)}`, summary: null, events: [ev(1, "user_prompt", {}, { prompt_text: "start" })] }), group(), group({ id: `chk_${"c".repeat(64)}` })]);
    expect(screen.getByText("Committed with this audit")).toBeInTheDocument();
    expect(screen.getAllByText("Checkpoint")).toHaveLength(2);
    expect(screen.getByText(/2 checkpoints added since/)).toBeInTheDocument();
    expect(screen.getAllByText(/1 prompt · 1 reply · 1 command/).length).toBe(2);
    expect(screen.getAllByText("who owns the keys?", { selector: ".tl-quote" })).toHaveLength(2);
    expect(screen.getAllByText("kubectl · exit 0").length).toBeGreaterThan(0);
    expect(screen.getAllByText("observed", { selector: ".badge" }).length).toBe(3);
  });

  it("renders prompt text and every other value as text, never as markup", () => {
    const payload = "<script>alert(1)</script><img src=x onerror=alert(2)>";
    const { container } = show([group({ events: [ev(1, "user_prompt", {}, { prompt_text: payload }), ev(2, "command", { program: payload }), ev(3, "file_edit", { path: payload }, { tool: "Write" })] })]);
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll("[onerror]").length).toBe(0);
    expect(screen.getAllByText(payload, { exact: false }).length).toBeGreaterThanOrEqual(2);
  });

  it("says plainly when a prompt's text is not available and replies are digest only", () => {
    show([group({ events: [ev(1, "user_prompt", { prompt_bytes: 20 }), ev(2, "assistant_message", { message_bytes: 12 })] })]);
    expect(screen.getByText(/the text stays on the developer's machine/)).toBeInTheDocument();
    expect(screen.getByText(/digest only, no text stored/)).toBeInTheDocument();
  });

  it("marks failed commands and describes the other event kinds", () => {
    expect(describeEvent(ev(1, "command", { program: "bao", exit_code: 1, failed: true }))).toBe("bao · exit 1 · failed");
    expect(describeEvent(ev(1, "file_edit", { path: "src/a.rs" }, { tool: "Edit" }))).toBe("Edit src/a.rs");
    expect(describeEvent(ev(1, "resource_access", { host: "docs.example.com" }, { tool: "WebFetch" }))).toBe("WebFetch docs.example.com");
    expect(describeEvent(ev(1, "agent", { action: "start", agent_type: "reviewer" }))).toBe("start reviewer");
    expect(describeEvent(ev(1, "other_hook"))).toBe("other hook");
    show([group({ events: [ev(1, "command", { program: "bao", exit_code: 1, failed: true })] })]);
    expect(screen.getByText("command", { selector: ".badge" }).className).toContain("b-bad");
  });

  it("reports missing event details and truncation, and an empty conversation", () => {
    const { unmount } = show([group({ state: "events unavailable", events: [] }), group({ id: `chk_${"d".repeat(64)}`, truncated: true })]);
    expect(screen.getByText(/not available here \(events unavailable\)/)).toBeInTheDocument();
    expect(screen.getByText(/Only the first events of this group are shown/)).toBeInTheDocument();
    unmount();
    show([]);
    expect(screen.getByText(/No hook events were recorded for this audit commit/)).toBeInTheDocument();
  });

  it("appears inside the audit page, keeping the audit id unchanged", () => {
    const d = detail({ timeline: [group()] });
    render(
      <MemoryRouter>
        <AuditView slug="acme/web" d={d} />
      </MemoryRouter>,
    );
    expect(screen.getByRole("heading", { name: "Conversation" })).toBeInTheDocument();
    expect(screen.getByText(/the audit ID does not change when they are added/)).toBeInTheDocument();
  });
});

describe("RepoPage rows", () => {
  it("shows how many checkpoints each audit has", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        const body = url.endsWith("/api/repo")
          ? { repo: { slug: "acme/web", repo_id: "repo_x", remote_url: null, last_push_at: null }, authors: [] }
          : { repo: "acme/web", total: 1, page: 1, per_page: 25, audits: [{ audit_id: `audit_${"1".repeat(64)}`, created_at: "2026-01-01T00:00:00Z", description: "one audit", author: null, changes: 1, gaps: 0, checkpoints: 3, gap_codes: [], parents: [], git: [] }] };
        return new Response(JSON.stringify(body), { status: 200 });
      }),
    );
    render(
      <MemoryRouter initialEntries={["/acme/web"]}>
        <Routes>
          <Route path="/:owner/:name" element={<RepoPage />} />
        </Routes>
      </MemoryRouter>,
    );
    const row = (await screen.findByText("one audit")).closest(".row") as HTMLElement;
    expect(within(row).getByText(/3 checkpoints/)).toBeInTheDocument();
    vi.unstubAllGlobals();
  });
});
