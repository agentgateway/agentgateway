import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "react-hot-toast";
import App from "./App.tsx";
import { AppProvider } from "./contexts";
import "./index.css";
import "./styles/global.css";
import "./styles/theme.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppProvider>
      <App />
      <Toaster
        position="bottom-right"
        toastOptions={{
          duration: 4000,
          style: {
            background: "var(--color-bg-container)",
            color: "var(--color-text-base)",
            border: "1px solid var(--color-border-base)",
            borderRadius: "var(--border-radius-lg)",
            padding: "16px",
            fontSize: "14px",
            boxShadow: "0 4px 12px rgba(0, 0, 0, 0.15)",
          },
          success: {
            duration: 3000,
            iconTheme: {
              primary: "var(--color-success)",
              secondary: "var(--color-bg-container)",
            },
            style: {
              border: "1px solid var(--color-success)",
            },
          },
          error: {
            duration: 4000,
            iconTheme: {
              primary: "var(--color-error)",
              secondary: "var(--color-bg-container)",
            },
            style: {
              border: "1px solid var(--color-error)",
            },
          },
          loading: {
            iconTheme: {
              primary: "var(--color-primary)",
              secondary: "var(--color-bg-container)",
            },
          },
        }}
      />
    </AppProvider>
  </StrictMode>,
);
