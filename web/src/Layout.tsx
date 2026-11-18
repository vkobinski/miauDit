import { type FormEvent, useState } from "react";
import { Link, Outlet, useNavigate, useSearchParams } from "react-router-dom";

export function Layout({ mode }: { mode: "server" | "local" }) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [q, setQ] = useState(params.get("q") ?? "");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    navigate(q.trim() ? `/?q=${encodeURIComponent(q.trim())}` : "/");
  };
  return (
    <>
      <div className="topbar">
        <div className="in">
          <Link to="/">miauDit</Link>
          <span className="brandsub">{mode === "local" ? "local read-only viewer" : "audit commits for AI-assisted changes"}</span>
          {mode === "server" && (
            <form className="search" onSubmit={submit} role="search">
              <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find a repository…" aria-label="Find a repository" />
            </form>
          )}
        </div>
      </div>
      <main>
        <Outlet />
      </main>
    </>
  );
}
