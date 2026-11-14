import { defineConfig } from "@playwright/test";

// Runs the built app against a fresh Forge host. Build first: npm run build.
// Uses a locally installed Chrome (CHROME_PATH, default /usr/bin/google-chrome).
const PORT = 8799;
const DATA = process.env.FORGE_E2E_DATA ?? "/tmp/x3-forge-e2e";
export default defineConfig({
  testDir: "e2e",
  timeout: 90_000,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    launchOptions: { executablePath: process.env.CHROME_PATH ?? "/usr/bin/google-chrome", args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"] },
    viewport: { width: 1500, height: 1000 },
    acceptDownloads: true,
  },
  webServer: {
    command: `rm -rf ${DATA} && FORGE_SETUP_TOKEN=e2e-setup-token PYTHONPATH=../.. ${process.env.PYTHON ?? "python3"} -m forge.server --data ${DATA} --port ${PORT}`,
    url: `http://127.0.0.1:${PORT}/health`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
