import { defineConfig } from "vitest/config";
import path from "node:path";

export default defineConfig({
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  test: {
    environment: "jsdom",
    globals: true,
    environmentOptions: {
      // jsdom defaults to `about:blank`, which has an opaque origin and so no
      // localStorage. The app runs on a real origin, so the tests should too.
      jsdom: { url: "http://localhost" },
    },
  },
});
