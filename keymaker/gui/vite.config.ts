import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  // Bound to 127.0.0.1 on an unusual port, and strict: if something already
  // holds it, fail rather than quietly move to the next one. See
  // crates/keymaker-gui/DEV.md for why this matters more than it looks.
  server: { host: "127.0.0.1", port: 5187, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true },
  clearScreen: false,
});
