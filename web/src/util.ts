export function short(id: string): string {
  const i = id.indexOf("_");
  const head = i < 0 ? "" : id.slice(0, i + 1);
  const rest = i < 0 ? id : id.slice(i + 1);
  return `${head}${rest.slice(0, 10)}…`;
}

const GITHUB = /^https:\/\/github\.com\/[A-Za-z0-9/._:-]+$/;

export function safeGithubUrl(url: string | null | undefined): string | null {
  return url && GITHUB.test(url) ? url : null;
}

export function ago(ts: string, now: number = Date.now()): string {
  const t = Date.parse(ts);
  if (Number.isNaN(t)) return ts;
  const secs = Math.floor((now - t) / 1000);
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"} ago`;
  if (secs < 0) return ts.slice(0, 10);
  if (secs < 60) return "just now";
  if (secs < 3600) return plural(Math.floor(secs / 60), "minute");
  if (secs < 86400) return plural(Math.floor(secs / 3600), "hour");
  if (secs < 30 * 86400) return plural(Math.floor(secs / 86400), "day");
  return ts.slice(0, 10);
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
