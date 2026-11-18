import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { api } from "./api";
import { Message } from "./components";
import { useAsync } from "./hooks";
import { Layout } from "./Layout";
import { AuditPage } from "./pages/AuditPage";
import { Home } from "./pages/Home";
import { RepoPage } from "./pages/RepoPage";

export function App() {
  const { data: config, error } = useAsync(() => api.config(), []);
  if (error) return <Message title="Cannot reach the server" text={error} />;
  if (!config) return <p className="muted">Loading…</p>;
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<Layout mode={config.mode} />}>
          <Route path="/" element={config.mode === "local" && config.repo ? <Navigate to={`/${config.repo}`} replace /> : <Home />} />
          <Route path="/:owner/:name" element={<RepoPage />} />
          <Route path="/:owner/:name/audit/:id" element={<AuditPage />} />
          <Route path="*" element={<Message title="Not found" text="No such page." />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
