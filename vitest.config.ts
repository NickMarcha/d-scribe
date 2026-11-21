import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/__tests__/**/*.test.{ts,tsx}"],
    setupFiles: ["src/__tests__/setup.ts"],
    css: false,
    // The first test in each file imports the whole app cold (d3 included), which can take ~10 s.
    testTimeout: 30000,
  },
});
