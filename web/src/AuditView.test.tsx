import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { AuditView } from "./pages/AuditPage";
import { AUDIT_ID, EVIDENCE_ID, XSS, detail } from "./fixtures";

function show(d = detail()) {
  return render(
    <MemoryRouter>
      <AuditView slug="acme/web" d={d} />
    </MemoryRouter>,
  );
}

describe("AuditView", () => {
  it("renders untrusted text as text and never as markup", () => {
    const { container } = show();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img[onerror]")).toBeNull();
    expect(screen.getAllByText(XSS).length).toBeGreaterThanOrEqual(2);
  });

  it("only turns github.com URLs into links", () => {
    const d = detail();
    d.bindings[0]!.body.git.commit_web_url = "javascript:alert(1)";
    d.bindings[0]!.body.git.repository_web_url = "https://evil.example/acme/repo";
    d.bindings[0]!.body.pull_request!.web_url = "https://evil.example/acme/repo/pull/7";
    const { container } = show(d);
    const hrefs = [...container.querySelectorAll("a")].map((a) => a.getAttribute("href") ?? "");
    expect(hrefs.some((h) => h.startsWith("javascript:"))).toBe(false);
    expect(hrefs.some((h) => h.includes("evil.example"))).toBe(false);
  });

  it("links safe GitHub commit, PR and author URLs with rel=noopener", () => {
    const { container } = show();
    const pr = screen.getAllByRole("link", { name: "PR #7" })[0]!;
    expect(pr).toHaveAttribute("href", "https://github.com/acme/repo/pull/7");
    expect(pr).toHaveAttribute("rel", "noopener noreferrer");
    expect(container.querySelector('a[href="https://github.com/acme/repo/commit/0123456789abcdef"]')).not.toBeNull();
    expect(container.querySelector('a[href="https://github.com/test-user"]')).not.toBeNull();
  });

  it("shows verification, approval and provenance honestly", () => {
    show();
    expect(screen.getByText("integrity: pass")).toBeInTheDocument();
    expect(screen.getByText("none recorded")).toBeInTheDocument();
    expect(screen.getByText("not evaluated")).toBeInTheDocument();
    expect(screen.getAllByText("self-reported").length).toBeGreaterThan(0);
    expect(screen.getByText("matches snapshot")).toBeInTheDocument();
    expect(screen.getByText("full match", { selector: ".badge" })).toBeInTheDocument();
  });

  it("serves evidence images through the repo-scoped API route and marks missing objects", () => {
    show();
    const img = screen.getByAltText("evidence");
    expect(img).toHaveAttribute("src", `/acme/web/api/audits/${AUDIT_ID}/evidence/${EVIDENCE_ID}/image`);
    expect(screen.getByText("object unavailable")).toBeInTheDocument();
  });

  it("reports an unavailable verification instead of a pass", () => {
    show(detail({ verification: { state: "unavailable", reason: "could not run miaudit" } }));
    expect(screen.getByText("integrity: not checked")).toBeInTheDocument();
    expect(screen.getByText(/could not run miaudit/)).toBeInTheDocument();
  });

  it("marks a decision made against a different snapshot", () => {
    const d = detail();
    d.decisions[0]!.object!.body.snapshot_id = "snapshot_other";
    show(d);
    expect(screen.getByText("different snapshot")).toBeInTheDocument();
  });

  it("marks a superseded decision and links the audit that holds the newer one", () => {
    const d = detail();
    d.decisions[0]!.superseded_by = [{ decision_id: `dec_${"7".repeat(64)}`, audit_id: `audit_${"6".repeat(64)}` }];
    const { container } = show(d);
    expect(screen.getByText("superseded", { selector: ".badge" })).toBeInTheDocument();
    expect(container.querySelector(`a[href="/acme/web/audit/audit_${"6".repeat(64)}"]`)).not.toBeNull();
    expect(screen.getByText(/This record is unchanged/)).toBeInTheDocument();
  });

  it("lists what a newer decision supersedes, as text", () => {
    const d = detail();
    d.decisions[0]!.object!.body.supersedes = [`dec_${"5".repeat(64)}`, "<img src=x onerror=alert(1)>"];
    const { container } = show(d);
    expect(screen.getByText("Supersedes")).toBeInTheDocument();
    expect(screen.getByText(`dec_${"5".repeat(64)}`)).toBeInTheDocument();
    expect(container.querySelector("img[onerror]")).toBeNull();
    expect(screen.queryByText("superseded", { selector: ".badge" })).toBeNull();
  });

  it("breadcrumbs link back to the repository", () => {
    show();
    const crumb = document.querySelector(".crumb")!;
    expect(within(crumb as HTMLElement).getByRole("link", { name: /acme \/ web/ })).toHaveAttribute("href", "/acme/web");
  });
});
