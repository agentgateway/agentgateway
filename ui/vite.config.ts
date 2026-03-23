import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const basePath = env.BASE_PATH || "/";

  return {
    base: basePath,
    plugins: [
      react({
        jsxImportSource: "@emotion/react",
      }),
    ],
    server: {
      port: 3000,
      open: true,
    },
    build: {
      outDir: "out",
      sourcemap: true,
      rollupOptions: {
        output: {
          manualChunks(id) {
            if (["react", "react-dom", "react-router-dom"].some((p) => id.includes(`/node_modules/${p}/`))) return "react-vendor";
            if (["antd", "@emotion/react", "@emotion/styled", "framer-motion"].some((p) => id.includes(`/node_modules/${p}/`))) return "ui-vendor";
            if (id.includes("/node_modules/chart.js/")) return "chart-vendor";
          },
        },
      },
    },
  };
});
