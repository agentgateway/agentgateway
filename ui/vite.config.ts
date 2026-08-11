import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig(({ command, mode }) => ({
  base: mode === "e2e" ? "/" : command === "build" ? "/ui/" : "./",
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
      "path-browserify": fileURLToPath(
        new URL("./src/pathBrowserifyEsm.ts", import.meta.url),
      ),
    },
  },
  build: {
    emptyOutDir: true,
    reportCompressedSize: false,
    sourcemap: false,
    chunkSizeWarningLimit: 5000,
  },
  server: {
    host: "0.0.0.0",
    port: 19000,
  },
  preview: {
    host: mode === "e2e" ? "127.0.0.1" : "0.0.0.0",
    port: 19100,
  },
}));
