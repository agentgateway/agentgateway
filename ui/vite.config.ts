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
        babel: {
          plugins: ["@emotion/babel-plugin"],
        },
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
          manualChunks: {
            "react-vendor": ["react", "react-dom", "react-router-dom"],
            "ui-vendor": [
              "antd",
              "@emotion/react",
              "@emotion/styled",
              "framer-motion",
            ],
            "chart-vendor": ["chart.js"],
          },
        },
      },
    },
  };
});
