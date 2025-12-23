/**
 * Theme Configuration
 *
 * This file centralizes branding and theming configuration for easy customization.
 * Change these values to rebrand the application without touching component files.
 */

export const themeConfig = {
  branding: {
    name: "UnitOne",
    tagline: "Agent Gateway",
    fullName: "UnitOne - Agent Gateway Platform",
    description: "Enterprise-grade agentic AI gateway platform",
    logo: {
      // Path to logo image in public directory (including basePath for static export)
      image: "/ui/images/unitone-logo.png",
      width: 36,
      height: 36,
    },
    favicon: "/icon.svg",
  },

  colors: {
    // Primary brand color
    primary: "#3b82f6",

    // Primary dark (used for top nav bar)
    primaryDark: "#020618",

    // Main backgrounds
    background: "#f8fafc",
    foreground: "#0f172a",
    card: "#ffffff",
    cardForeground: "#0f172a",
    popover: "#ffffff",
    popoverForeground: "#0f172a",

    // Secondary
    secondary: "#f1f5f9",
    secondaryForeground: "#0f172a",

    // Muted
    muted: "#f1f5f9",
    mutedForeground: "#64748b",

    // Accent
    accent: "#f1f5f9",
    accentForeground: "#0f172a",

    // Destructive
    destructive: "#ef4444",
    destructiveForeground: "#ffffff",

    // Borders and inputs
    border: "#e2e8f0",
    input: "#e2e8f0",
    ring: "#3b82f6",

    // Sidebar (dark theme)
    sidebar: "#0f172a",
    sidebarForeground: "#f8fafc",
    sidebarPrimary: "#3b82f6",
    sidebarPrimaryForeground: "#ffffff",
    sidebarAccent: "#1e293b",
    sidebarAccentForeground: "#f8fafc",
    sidebarBorder: "#1e293b",
    sidebarRing: "#3b82f6",
    sidebarMuted: "#94a3b8",

    // Top nav bar
    navBg: "#020618",
    navForeground: "#f8fafc",
    navMuted: "#94a3b8",

    // Status colors
    statusPass: "#22c55e",
    statusFail: "#ef4444",
    statusWarn: "#eab308",
    statusInfo: "#3b82f6",

    // Chart colors
    chart1: "#3b82f6",
    chart2: "#22c55e",
    chart3: "#eab308",
    chart4: "#ef4444",
    chart5: "#8b5cf6",
  },

  fonts: {
    sans: "Inter",
    mono: "Geist Mono",
  },
} as const

export type ThemeConfig = typeof themeConfig
