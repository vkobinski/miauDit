import { ago, plural, safeGithubUrl, short } from "./util";

describe("util", () => {
  it("accepts only github.com URLs with a safe alphabet", () => {
    expect(safeGithubUrl("https://github.com/acme/repo/pull/7")).not.toBeNull();
    for (const bad of ["javascript:alert(1)", "http://github.com/a/b", "https://github.com.evil.com/x", "https://evil.com/github.com/x", "https://github.com/a b", "https://github.com/<x>", "", null, undefined]) {
      expect(safeGithubUrl(bad as string | null | undefined)).toBeNull();
    }
  });

  it("formats relative times", () => {
    const now = Date.parse("2026-01-10T12:00:00Z");
    expect(ago("2026-01-10T11:59:50Z", now)).toBe("just now");
    expect(ago("2026-01-10T11:55:00Z", now)).toBe("5 minutes ago");
    expect(ago("2026-01-10T11:00:00Z", now)).toBe("1 hour ago");
    expect(ago("2026-01-07T12:00:00Z", now)).toBe("3 days ago");
    expect(ago("2025-11-01T12:00:00Z", now)).toBe("2025-11-01");
    expect(ago("not a date", now)).toBe("not a date");
  });

  it("shortens ids and pluralizes", () => {
    expect(short(`audit_${"a".repeat(64)}`)).toBe("audit_aaaaaaaaaa…");
    expect(plural(1, "gap")).toBe("1 gap");
    expect(plural(2, "gap")).toBe("2 gaps");
  });
});
