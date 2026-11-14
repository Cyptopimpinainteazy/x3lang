/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// `npm run dev` proxies the API to a Forge host started with `python3 -m forge.server`.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { proxy: { "/api": "http://127.0.0.1:8787", "/health": "http://127.0.0.1:8787" } },
  build: { outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 1500 },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
