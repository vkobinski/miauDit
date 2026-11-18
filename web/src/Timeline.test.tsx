import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { AuditView } from "./pages/AuditPage";
import { RepoPage } from "./pages/RepoPage";
import { Timeline } from "./Timeline";
import { detail } from "./fixtures";
import type { TimelineEvent, TimelineGroup } from "./types";

let seq = 0;
const T0 = Date.UTC(2026, 0, 1, 10, 0, 0);
const iso = (sec: number) => new Date(T0 + sec * 1000).toISOString().replace(".000", "");
const ev = (kind: string, sec: number, data: Record<string, unknown> = {}, extra: Partial<TimelineEvent> = {}): TimelineEvent => {
  seq += 1;
  return { id: `evt_${seq}`, seq, at: iso(sec), kind, tool: null, session_id: "s1", data, ...extra };
};
const prompt = (sec: number, text?: string, bytes = 20) => ev("user_prompt", sec, { prompt_bytes: bytes }, text === undefined ? {} : { prompt_text: text });
const cmd = (sec: number, program: string, exit = 0) => ev("command", sec, { program, exit_code: exit });
const reply = (sec: number) => ev("assistant_message", sec, { message_bytes: 12 });
const group = (events: TimelineEvent[], over: Partial<TimelineGroup> = {}): TimelineGroup => ({ source: "checkpoint", id: `chk_${"a".repeat(64)}`, created_at: iso(0), summary: null, state: "present", events, truncated: false, ...over });
const audit = (events: TimelineEvent[]) => group(events, { source: "audit", id: `evd_${"b".repeat(64)}` });

const show = (groups: TimelineGroup[], changedPaths: string[] = []) =>
  render(
    <MemoryRouter>
      <Timeline groups={groups} changedPaths={changedPaths} />
    </MemoryRouter>,
  );

const twoTurns = () => [audit([prompt(0, "Who owns the keys?"), cmd(1, "bao"), cmd(2, "kubectl", 1), reply(3), prompt(60, "And the rollout?"), cmd(61, "kubectl"), reply(62)])];

beforeEach(() => {
  seq = 0;
});

describe("structure", () => {
  it("shows turns, opens only the latest and keeps the rest one line each", () => {
    show(twoTurns());
    expect(screen.getByText("Turn 1")).toBeInTheDocument();
    expect(screen.getByText("Turn 2")).toBeInTheDocument();
    const [first, second] = screen.getAllByRole("button", { expanded: undefined }).filter((b) => b.className === "tl-head");
    expect(first).toHaveAttribute("aria-expanded", "false");
    expect(second).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Who owns the keys?")).toBeInTheDocument();
    expect(screen.getByText("And the rollout?")).toBeInTheDocument();
    expect(screen.getAllByRole("listitem").some((li) => li.className.includes("tl-row"))).toBe(true);
    expect(screen.queryByText("bao")).toBeNull();
  });

  it("opens and closes a turn on click, and expands or collapses everything", async () => {
    show(twoTurns());
    await userEvent.click(screen.getByText("Who owns the keys?"));
    expect(screen.getByText("bao")).toBeInTheDocument();
    await userEvent.click(screen.getByText("Who owns the keys?"));
    expect(screen.queryByText("bao")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Expand all" }));
    expect(screen.getByText("bao")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Collapse all" }));
    expect(screen.queryByText("bao")).toBeNull();
    expect(document.querySelectorAll(".tl-head[aria-expanded=true]")).toHaveLength(0);
  });

  it("summarizes the whole conversation and each turn", () => {
    show(twoTurns());
    const summary = document.querySelector(".tl-summary")!.textContent ?? "";
    expect(summary).toContain("2 turns");
    expect(summary).toContain("7 events");
    expect(summary).toContain("3 commands");
    expect(summary).toContain("1 failed");
    expect(screen.getAllByText("1 failed").length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText(/1m 02s/)).toBeInTheDocument();
  });

  it("folds repeated commands into one counted row that can be expanded", async () => {
    show([audit([prompt(0, "go"), cmd(1, "kubectl"), cmd(2, "kubectl"), cmd(3, "kubectl"), reply(4)])]);
    expect(screen.getByText("kubectl")).toBeInTheDocument();
    expect(screen.getByText(/×3/)).toBeInTheDocument();
    await userEvent.click(screen.getByText("show all 3"));
    expect(document.querySelectorAll(".tl-fold li")).toHaveLength(3);
  });

  it("merges a subagent's start and stop into one row with its type and duration", () => {
    show([audit([prompt(0, "go"), ev("agent", 1, { action: "start", agent_id: "ag1", agent_type: "reviewer" }), ev("agent", 13, { action: "stop", agent_id: "ag1", agent_type: "reviewer" }), reply(14)])]);
    expect(screen.getByText("subagent reviewer")).toBeInTheDocument();
    expect(screen.getByText(/finished in 12s/)).toBeInTheDocument();
    expect(screen.queryByText("stop")).toBeNull();
  });

  it("puts events before the first prompt under a heading of their own", () => {
    show([audit([cmd(0, "ls"), prompt(5, "hello"), reply(6)])]);
    expect(screen.getByText("Before the first prompt")).toBeInTheDocument();
  });
});

describe("wording", () => {
  it("does not mention checkpoints when there are none, and explains them when there are", () => {
    const { unmount } = show(twoTurns());
    expect(screen.getByText("Everything recorded for this audit commit, in order.")).toBeInTheDocument();
    expect(screen.queryByText(/0 checkpoints/)).toBeNull();
    unmount();
    show([...twoTurns(), group([prompt(120, "later"), reply(121)], { id: `chk_${"c".repeat(64)}` }), group([prompt(180, "latest")], { id: `chk_${"d".repeat(64)}` })]);
    expect(screen.getByText(/then 2 checkpoints added since/)).toBeInTheDocument();
    expect(screen.getByText(/the audit ID does not change when they are added/)).toBeInTheDocument();
  });

  it("says the digest-only fact once, not on every reply", () => {
    show(twoTurns());
    expect(screen.getAllByText(/stored as hashes/)).toHaveLength(1);
    expect(screen.queryAllByText(/digest only, no text stored/)).toHaveLength(0);
    const replyRow = document.querySelector(".tl-row[title*='digest only']");
    expect(replyRow).not.toBeNull();
  });

  it("lists where each event came from, with the observed label", () => {
    show([audit([prompt(0, "a")]), group([prompt(60, "b")])]);
    expect(screen.getByText("Committed with this audit · 1 event")).toBeInTheDocument();
    expect(screen.getByText(/Checkpoint chk_/)).toBeInTheDocument();
    expect(screen.getAllByText("observed", { selector: ".badge" })).toHaveLength(2);
  });

  it("reports unavailable details, truncation and an empty conversation", () => {
    const { unmount } = show([group([], { state: "events unavailable" }), audit([prompt(0, "x")]).source === "audit" ? group([prompt(0, "x")], { truncated: true, id: `chk_${"e".repeat(64)}` }) : group([])]);
    expect(screen.getByText(/event details not available here \(events unavailable\)/)).toBeInTheDocument();
    expect(screen.getByText(/Only the first 3000 events of a group are loaded/)).toBeInTheDocument();
    unmount();
    show([]);
    expect(screen.getByText(/No hook events were recorded for this audit commit/)).toBeInTheDocument();
  });
});

describe("prompts", () => {
  it("show the first line in the header and the full text in the body", async () => {
    show([audit([prompt(0, "first line\nsecond line with more detail"), cmd(1, "ls"), reply(2)])]);
    expect(screen.getByText("first line")).toBeInTheDocument();
    expect(screen.getByText(/second line with more detail/, { selector: ".tl-quote" })).toBeInTheDocument();
  });

  it("say the text is not available instead of showing nothing", () => {
    show([audit([prompt(0, undefined, 33), reply(1)])]);
    expect(screen.getByText(/prompt, 33 bytes \(the text stays on the developer's machine\)/)).toBeInTheDocument();
  });
});

describe("failures, filters and search", () => {
  it("make failed rows stand out and can be shown alone", async () => {
    show(twoTurns());
    await userEvent.click(screen.getByText("Who owns the keys?"));
    expect(document.querySelectorAll(".tl-fail").length).toBeGreaterThanOrEqual(1);
    await userEvent.click(screen.getByRole("button", { name: "Failures only" }));
    expect(screen.getByRole("button", { name: "Failures only" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByText("bao")).toBeNull();
    expect(screen.queryByText("And the rollout?")).toBeNull();
    expect(document.querySelectorAll(".tl-row")).toHaveLength(1);
  });

  it("hide a kind when its chip is switched off and show the counts", async () => {
    show(twoTurns());
    expect(screen.getByRole("button", { name: /Commands\s*3/ })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: /Commands/ }));
    expect(screen.getByRole("button", { name: /Commands/ })).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByText("bao")).toBeNull();
    expect(screen.queryByText("kubectl")).toBeNull();
    expect(screen.getAllByText("reply").length).toBeGreaterThan(0);
  });

  it("search the conversation and offer a way back when nothing matches", async () => {
    show(twoTurns());
    await userEvent.type(screen.getByLabelText("Search the conversation"), "bao");
    expect(screen.getByText("bao")).toBeInTheDocument();
    expect(screen.queryByText("And the rollout?")).toBeNull();
    await userEvent.clear(screen.getByLabelText("Search the conversation"));
    await userEvent.type(screen.getByLabelText("Search the conversation"), "no such thing");
    expect(screen.getByText(/No turns match the current filters/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(screen.getByText("Who owns the keys?")).toBeInTheDocument();
    expect(screen.getByLabelText("Search the conversation")).toHaveValue("");
  });

  it("find text inside local prompts", async () => {
    show(twoTurns());
    await userEvent.type(screen.getByLabelText("Search the conversation"), "rollout");
    expect(screen.getByText("And the rollout?")).toBeInTheDocument();
    expect(screen.queryByText("Who owns the keys?")).toBeNull();
  });
});

describe("file edits", () => {
  it("link to the changed file in the Snapshot card, and stay plain text otherwise", () => {
    show([audit([prompt(0, "edit"), ev("file_edit", 1, { path: "src/a.rs" }, { tool: "Edit" }), ev("file_edit", 2, { path: "notes/unchanged.md" }, { tool: "Write" })])], ["src/a.rs"]);
    const link = screen.getByRole("link", { name: "src/a.rs" });
    expect(link).toHaveAttribute("href", "#change-src_a_rs");
    expect(screen.queryByRole("link", { name: "notes/unchanged.md" })).toBeNull();
    expect(screen.getByText("notes/unchanged.md")).toBeInTheDocument();
  });
});

describe("time", () => {
  it("adds a heading when the day changes and a gap marker after a long pause", () => {
    show([audit([prompt(0, "one"), reply(1), prompt(3600 + 5, "two"), reply(3600 + 6), prompt(86_400 * 2, "three"), reply(86_400 * 2 + 1)])]);
    expect(screen.getByText("2026-01-01 (UTC)")).toBeInTheDocument();
    expect(screen.getByText("2026-01-03 (UTC)")).toBeInTheDocument();
    expect(screen.getByText(/later$/, { selector: ".tl-gap" })).toBeInTheDocument();
  });

  it("shows the offset of each row from the start of its turn", () => {
    show([audit([prompt(0, "go"), cmd(7, "ls"), reply(75)])]);
    expect(screen.getByText("+0:07")).toBeInTheDocument();
    expect(screen.getByText("+1:15")).toBeInTheDocument();
  });
});

describe("long conversations", () => {
  it("show the latest 30 turns and reveal earlier ones on request", async () => {
    const events: TimelineEvent[] = [];
    for (let i = 0; i < 35; i += 1) events.push(prompt(i * 20, `question ${i + 1}`), reply(i * 20 + 1));
    show([audit(events)]);
    expect(screen.queryByText("question 5")).toBeNull();
    expect(screen.getByText("question 6")).toBeInTheDocument();
    expect(screen.getByText("question 35")).toBeInTheDocument();
    const more = screen.getByRole("button", { name: /Show 5 earlier turns \(5 turns hidden\)/ });
    await userEvent.click(more);
    expect(screen.getByText("question 1")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /earlier turn/ })).toBeNull();
  });
});

describe("safety", () => {
  it("renders every value as text, never as markup", () => {
    const payload = "<script>alert(1)</script><img src=x onerror=alert(2)>";
    const { container } = show([audit([prompt(0, payload), ev("command", 1, { program: payload, exit_code: 0 }), ev("file_edit", 2, { path: payload }, { tool: "Write" })])]);
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll("[onerror]")).toHaveLength(0);
    expect(screen.getAllByText(payload, { exact: false }).length).toBeGreaterThanOrEqual(2);
  });
});

describe("in the audit page", () => {
  it("is the last card, after Verification", () => {
    const d = detail({ timeline: [audit([prompt(0, "go"), reply(1)])] });
    const { container } = render(
      <MemoryRouter>
        <AuditView slug="acme/web" d={d} />
      </MemoryRouter>,
    );
    const headings = [...container.querySelectorAll(".card > h2")].map((h) => h.textContent);
    expect(headings.slice(-2)).toEqual(["Verification", "Conversation"]);
    expect(headings.indexOf("Snapshot")).toBeLessThan(headings.indexOf("Conversation"));
  });

  it("gives each changed file in the Snapshot card an anchor the timeline links to", () => {
    const d = detail({ timeline: [audit([prompt(0, "edit"), ev("file_edit", 1, { path: "src/a.rs" }, { tool: "Edit" })])] });
    const { container } = render(
      <MemoryRouter>
        <AuditView slug="acme/web" d={d} />
      </MemoryRouter>,
    );
    expect(container.querySelector("tr#change-src_a_rs")).not.toBeNull();
    expect(screen.getByRole("link", { name: "src/a.rs" })).toHaveAttribute("href", "#change-src_a_rs");
  });
});

describe("repository rows", () => {
  it("show how many checkpoints each audit has", async () => {
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
