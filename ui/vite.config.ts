import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig(({ command }) => ({
  base: process.env.VITE_BASE_PATH ?? (command === "build" ? "/ui/" : "./"),
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
    host: "0.0.0.0",
    port: 19100,
  },
}));
