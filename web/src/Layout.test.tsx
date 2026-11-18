import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { Layout } from "./Layout";

function show(mode: "server" | "local") {
  return render(
    <MemoryRouter>
      <Routes>
        <Route element={<Layout mode={mode} />}>
          <Route path="/" element={<p>page</p>} />
        </Route>
      </Routes>
    </MemoryRouter>,
  );
}

describe("brand", () => {
  it("shows the pixel cat next to the name and keeps the link to the home page", () => {
    const { container } = show("server");
    const link = screen.getByRole("link", { name: "miauDit" });
    expect(link).toHaveAttribute("href", "/");
    const icon = container.querySelector("img.brand-icon");
    expect(icon).not.toBeNull();
    expect(icon).toHaveAttribute("src", "/icon-64.png");
    expect(icon).toHaveAttribute("width", "32");
    expect(icon).toHaveAttribute("height", "32");
    expect(icon).toHaveAttribute("alt", "");
  });

  it("is the same in the local viewer", () => {
    const { container } = show("local");
    expect(container.querySelector("img.brand-icon")).toHaveAttribute("src", "/icon-64.png");
    expect(screen.getByText("local read-only viewer")).toBeInTheDocument();
  });

  it("is also the favicon, and the file it points to ships with the app", () => {
    const html = readFileSync(join(process.cwd(), "index.html"), "utf8");
    expect(html).toContain('<link rel="icon" type="image/png" href="/icon-64.png" />');
    const png = readFileSync(join(process.cwd(), "public", "icon-64.png"));
    expect([...png.subarray(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    expect(png.readUInt32BE(16)).toBe(64);
    expect(png.readUInt32BE(20)).toBe(64);
  });
});
