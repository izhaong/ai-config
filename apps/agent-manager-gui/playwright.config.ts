import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  use: {
    baseURL: process.env.GUI_E2E_URL ?? "http://127.0.0.1:5173",
    viewport: { width: 1280, height: 800 },
    colorScheme: "dark",
    channel: "chrome",
  },
  reporter: [["list"]],
});
