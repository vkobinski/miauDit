import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { Layout } from "./Layout";
import { Home } from "./pages/Home";
import { RepoPage } from "./pages/RepoPage";
import { XSS } from "./fixtures";

function mockFetch(routes: Record<string, unknown | number>) {
  const calls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      calls.push(url);
      const hit = Object.entries(routes).find(([k]) => url === k || url.startsWith(k));
      if (!hit) return new Response("nope", { status: 404 });
      if (typeof hit[1] === "number") return new Response("err", { status: hit[1] });
      return new Response(JSON.stringify(hit[1]), { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
  return calls;
}

afterEach(() => vi.unstubAllGlobals());

const repo = { slug: "acme/web", repo_id: `repo_${"1".repeat(32)}`, remote_url: "https://github.com/acme/web", audits: 3, gaps: 5, last_audit_at: "2020-01-01T00:00:00Z", last_push_at: null };

describe("Home", () => {
  it("lists repositories with counts and links", async () => {
    mockFetch({ "/api/v1/repos": { repos: [repo, { ...repo, slug: "acme/<b>x</b>", repo_id: `repo_${"2".repeat(32)}`, remote_url: null, audits: 1, gaps: 0 }] } });
    const { container } = render(
      <MemoryRouter>
        <Home />
      </MemoryRouter>,
    );
    expect(await screen.findByText(/3 audit commits/)).toBeInTheDocument();
    expect(screen.getByText("5 gaps")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /^acme\s*\/\s*web$/ })).toHaveAttribute("href", "/acme/web");
    expect(screen.getByText("2 total")).toBeInTheDocument();
    expect(container.querySelector("b")?.textContent).toBe("Repositories");
    expect(screen.getByText("no GitHub remote")).toBeInTheDocument();
  });

  it("passes the search term to the API and says when nothing matches", async () => {
    const calls = mockFetch({ "/api/v1/repos": { repos: [] } });
    render(
      <MemoryRouter initialEntries={["/?q=we%20b"]}>
        <Home />
      </MemoryRouter>,
    );
    expect(await screen.findByText(/No repository matches/)).toBeInTheDocument();
    expect(calls).toContain("/api/v1/repos?q=we%20b");
  });

  it("shows push instructions when the server is empty", async () => {
    mockFetch({ "/api/v1/repos": { repos: [] } });
    render(
      <MemoryRouter>
        <Home />
      </MemoryRouter>,
    );
    expect(await screen.findByText("No repositories yet.")).toBeInTheDocument();
    expect(screen.getByText(/miaudit push/)).toBeInTheDocument();
  });

  it("shows an error instead of an empty list when the API fails", async () => {
    mockFetch({ "/api/v1/repos": 401 });
    render(
      <MemoryRouter>
        <Home />
      </MemoryRouter>,
    );
    expect(await screen.findByText("Authentication required")).toBeInTheDocument();
  });
});

describe("Layout search", () => {
  it("navigates to the filtered list on submit", async () => {
    mockFetch({ "/api/v1/repos": { repos: [] } });
    render(
      <MemoryRouter>
        <Routes>
          <Route element={<Layout mode="server" />}>
            <Route path="/" element={<Home />} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );
    await userEvent.type(screen.getByLabelText("Find a repository"), "payments{enter}");
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalledWith("/api/v1/repos?q=payments", expect.anything()));
  });

  it("has no search box in the local viewer", () => {
    mockFetch({});
    render(
      <MemoryRouter>
        <Routes>
          <Route element={<Layout mode="local" />}>
            <Route path="/" element={<p>x</p>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );
    expect(screen.queryByLabelText("Find a repository")).toBeNull();
    expect(screen.getByText("local read-only viewer")).toBeInTheDocument();
  });
});

describe("RepoPage", () => {
  const row = (n: number) => ({ audit_id: `audit_${String(n).repeat(64)}`, created_at: "2026-01-01T00:00:00Z", description: n === 1 ? XSS : `change ${n}`, author: "octo", changes: 2, gaps: 0, gap_codes: [], parents: [], git: [] });
  const routes = {
    "/acme/web/api/repo": { repo: { slug: "acme/web", repo_id: repo.repo_id, remote_url: repo.remote_url, last_push_at: null }, authors: [{ login: "octo", count: 2 }] },
    "/acme/web/api/audits?page=1": { repo: "acme/web", total: 60, page: 1, per_page: 25, audits: [row(1), row(2)] },
  };

  function renderRepo(url: string) {
    return render(
      <MemoryRouter initialEntries={[url]}>
        <Routes>
          <Route path="/:owner/:name" element={<RepoPage />} />
        </Routes>
      </MemoryRouter>,
    );
  }

  it("lists audits as text, pages forward and filters by author", async () => {
    mockFetch(routes);
    const { container } = renderRepo("/acme/web");
    expect(await screen.findByText(XSS)).toBeInTheDocument();
    expect(container.querySelector("script")).toBeNull();
    expect(screen.getByText("Page 1 of 3")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Older →" })).toHaveAttribute("href", "/acme/web?page=2");
    expect(screen.queryByRole("link", { name: "← Newer" })).toBeNull();
    expect(screen.getByRole("link", { name: "@octo" })).toHaveAttribute("href", "/acme/web?author=octo");
    expect(screen.getAllByText("not linked to a Git commit").length).toBe(2);
  });

  it("keeps the author filter in the pager and offers to clear it", async () => {
    const calls = mockFetch({ ...routes, "/acme/web/api/audits?page=2&author=octo": { repo: "acme/web", total: 60, page: 2, per_page: 25, audits: [row(3)] } });
    renderRepo("/acme/web?page=2&author=octo");
    expect(await screen.findByText("change 3")).toBeInTheDocument();
    expect(calls).toContain("/acme/web/api/audits?page=2&author=octo");
    expect(screen.getByRole("link", { name: "← Newer" })).toHaveAttribute("href", "/acme/web?page=1&author=octo");
    expect(screen.getByRole("link", { name: "clear" })).toHaveAttribute("href", "/acme/web");
  });

  it("shows an error for an unknown repository", async () => {
    mockFetch({ "/acme/web/api/repo": 404 });
    renderRepo("/acme/web");
    expect(await screen.findByText("Not found")).toBeInTheDocument();
  });
});
