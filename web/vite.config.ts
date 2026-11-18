import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const target = process.env.MIAUDIT_DEV_API ?? "http://127.0.0.1:7171";

export default defineConfig({
  plugins: [react()],
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
  server: {
    proxy: {
      "/api": target,
      "^/[^/]+/[^/]+/api/.*": target,
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    css: false,
  },
});
