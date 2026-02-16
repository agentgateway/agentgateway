import * as React from "react";

type Theme = "dark" | "light" | "system";

interface ThemeProviderProps {
  children: React.ReactNode;
  attribute?: string;
  defaultTheme?: Theme;
  enableSystem?: boolean;
  disableTransitionOnChange?: boolean;
  storageKey?: string;
}

interface ThemeProviderState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  resolvedTheme?: "dark" | "light";
}

const ThemeProviderContext = React.createContext<ThemeProviderState | undefined>(undefined);

export function ThemeProvider({
  children,
  attribute = "class",
  defaultTheme = "system",
  enableSystem = true,
  disableTransitionOnChange = false,
  storageKey = "theme",
  ...props
}: ThemeProviderProps) {
  const [theme, setThemeState] = React.useState<Theme>(defaultTheme);
  const [resolvedTheme, setResolvedTheme] = React.useState<"dark" | "light">();

  React.useEffect(() => {
    // Load theme from localStorage
    const stored = localStorage.getItem(storageKey);
    if (stored) {
      setThemeState(stored as Theme);
    }
  }, [storageKey]);

  React.useEffect(() => {
    const root = window.document.documentElement;

    if (disableTransitionOnChange) {
      root.classList.add("no-transition");
    }

    root.classList.remove("light", "dark");

    let resolved: "dark" | "light";
    if (theme === "system" && enableSystem) {
      const systemTheme = window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
      resolved = systemTheme;
    } else {
      resolved = theme === "dark" ? "dark" : "light";
    }

    if (attribute === "class") {
      root.classList.add(resolved);
    } else {
      root.setAttribute(attribute, resolved);
    }

    setResolvedTheme(resolved);

    if (disableTransitionOnChange) {
      requestAnimationFrame(() => {
        root.classList.remove("no-transition");
      });
    }
  }, [theme, attribute, enableSystem, disableTransitionOnChange]);

  const setTheme = React.useCallback(
    (newTheme: Theme) => {
      localStorage.setItem(storageKey, newTheme);
      setThemeState(newTheme);
    },
    [storageKey]
  );

  const value = React.useMemo(
    () => ({
      theme,
      setTheme,
      resolvedTheme,
    }),
    [theme, setTheme, resolvedTheme]
  );

  return (
    <ThemeProviderContext.Provider {...props} value={value}>
      {children}
    </ThemeProviderContext.Provider>
  );
}

export const useTheme = () => {
  const context = React.useContext(ThemeProviderContext);

  if (context === undefined) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }

  // Memoize systemTheme to avoid repeated window.matchMedia calls
  const systemTheme = React.useMemo(
    () => (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"),
    []
  );

  return {
    ...context,
    systemTheme,
  };
};
