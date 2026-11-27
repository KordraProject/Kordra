import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests",
  use: { baseURL: "http://127.0.0.1:5173", browserName: "chromium", viewport: {width: 1440, height: 900} },
  webServer: { command: "npm run dev -- --host 127.0.0.1", url: "http://127.0.0.1:5173", reuseExistingServer: !process.env.CI },
  retries: 1,
  reporter: "list",
});
