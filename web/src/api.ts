import type { AppConfig, AuditDetail, AuditPage, RepoInfo, RepoSummary } from "./types";

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path, { headers: { Accept: "application/json" } });
  if (!res.ok) {
    throw new ApiError(res.status, res.status === 404 ? "Not found" : res.status === 401 ? "Authentication required" : `Request failed (${res.status})`);
  }
  return (await res.json()) as T;
}

const enc = encodeURIComponent;

export const repoBase = (slug: string) => {
  const [owner, name] = slug.split("/");
  return `/${enc(owner ?? "")}/${enc(name ?? "")}`;
};

export const api = {
  config: () => getJson<AppConfig>("/api/v1/config"),
  repos: async (q: string) => (await getJson<{ repos: RepoSummary[] }>(`/api/v1/repos${q ? `?q=${enc(q)}` : ""}`)).repos,
  repo: (slug: string) => getJson<RepoInfo>(`${repoBase(slug)}/api/repo`),
  audits: (slug: string, page: number, author: string | null) => {
    const params = new URLSearchParams({ page: String(page) });
    if (author) params.set("author", author);
    return getJson<AuditPage>(`${repoBase(slug)}/api/audits?${params.toString()}`);
  },
  audit: (slug: string, id: string) => getJson<AuditDetail>(`${repoBase(slug)}/api/audits/${enc(id)}/detail`),
  imageUrl: (slug: string, id: string, evidenceId: string) => `${repoBase(slug)}/api/audits/${enc(id)}/evidence/${enc(evidenceId)}/image`,
  rawUrl: (slug: string, id: string) => `${repoBase(slug)}/api/audits/${enc(id)}`,
};
