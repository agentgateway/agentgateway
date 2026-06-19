import {
  Link,
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import {
  BarChart3,
  Bot,
  Braces,
  Cable,
  Boxes,
  Coins,
  FileCode2,
  Github,
  Globe,
  Home,
  KeyRound,
  Menu,
  MessageSquarePlus,
  Network,
  Route,
  ScrollText,
  ShieldCheck,
  Shield,
  Bolt,
  Moon,
  Play,
  Server,
  Sun,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Tooltip, useDismissiblePopover } from "./Primitives";
import { useConfigDumpMode, useGatewayConfig } from "../hooks";
import logoDark from "../assets/agw-dark.svg";
import logoLight from "../assets/agw-light.svg";

type NavItemConfig = {
  to: string;
  label: string;
  icon: React.ComponentType<{ size?: number }>;
  placeholder?: boolean;
  groupStart?: boolean;
};

const projectLinks = [
  {
    label: "GitHub",
    href: "https://github.com/agentgateway/agentgateway",
    icon: Github,
  },
  {
    label: "Documentation",
    href: "https://agentgateway.dev/docs/standalone/latest/",
    icon: Globe,
  },
  {
    label: "Feedback",
    href: "https://github.com/agentgateway/agentgateway/issues/new?title=UI%20feedback%3A%20&body=Thanks%20for%20trying%20the%20agentgateway%20UI.%0A%0AWhat%20happened%3F%0A%0AWhat%20did%20you%20expect%20instead%3F%0A%0AAny%20screenshots%2C%20logs%2C%20or%20config%20that%20would%20help%3F",
    icon: MessageSquarePlus,
  },
] as const;

export function Shell() {
  const router = useRouterState();
  const mode = useConfigDumpMode();
  const dumpMode = mode.data?.mode === "dump";
  const config = useGatewayConfig({
    enabled: Boolean(mode.data && mode.data.mode !== "dump"),
  });
  const [theme, setTheme] = useState(
    () => localStorage.getItem("theme") ?? "light",
  );
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const mobileNavRef = useDismissiblePopover<HTMLDivElement>(
    mobileNavOpen,
    () => setMobileNavOpen(false),
  );
  const hasLlm = dumpMode
    ? false
    : config.data
      ? Boolean(config.data.llm)
      : true;
  const hasMcp = dumpMode
    ? false
    : config.data
      ? Boolean(config.data.mcp)
      : true;
  const hasTraffic = dumpMode
    ? true
    : config.data
      ? "binds" in config.data
      : true;
  const navGroups = navigationGroups({ hasLlm, hasMcp, hasTraffic, dumpMode });
  const nav = navGroups.flatMap((group) => group.items);
  const currentNav =
    nav.find((item) =>
      item.to === "/"
        ? router.location.pathname === "/"
        : router.location.pathname.startsWith(item.to),
    ) ?? nav[0];
  const CurrentIcon = currentNav.icon;

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("theme", theme);
  }, [theme]);

  useEffect(() => {
    setMobileNavOpen(false);
  }, [router.location.pathname]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <Link to="/" className="brand" aria-label="agentgateway home">
          <img
            className="brand-logo brand-logo-light"
            src={logoLight}
            alt="agentgateway"
          />
          <img
            className="brand-logo brand-logo-dark"
            src={logoDark}
            alt="agentgateway"
          />
        </Link>
        <nav className="nav-list" aria-label="Primary">
          {navGroups.map((group) => (
            <NavSection
              key={group.title}
              title={group.title}
              items={group.items}
              currentPath={router.location.pathname}
            />
          ))}
        </nav>
        <div className="sidebar-links" aria-label="Project links">
          {projectLinks.map((link) => {
            const Icon = link.icon;
            return (
              <Tooltip content={link.label} key={link.href} side="top">
                <a
                  className="sidebar-link"
                  href={link.href}
                  target="_blank"
                  rel="noreferrer"
                  aria-label={link.label}
                >
                  <Icon size={17} />
                </a>
              </Tooltip>
            );
          })}
        </div>
      </aside>
      <div className="main-area">
        <header className="topbar">
          <div className="topbar-left">
            <div className="mobile-nav" ref={mobileNavRef}>
              <button
                className="mobile-nav-trigger"
                type="button"
                aria-haspopup="menu"
                aria-expanded={mobileNavOpen}
                onClick={() => setMobileNavOpen((open) => !open)}
              >
                <Menu size={17} />
                <CurrentIcon size={16} />
                <span>{currentNav.label}</span>
              </button>
              {mobileNavOpen ? (
                <nav
                  className="mobile-nav-menu"
                  aria-label="Primary"
                  role="menu"
                >
                  {navGroups.map((group) => (
                    <MobileNavSection
                      key={group.title}
                      title={group.title}
                      items={group.items}
                      currentPath={router.location.pathname}
                    />
                  ))}
                </nav>
              ) : null}
            </div>
            <span className="eyebrow">
              {eyebrowForPath(router.location.pathname)}
            </span>
          </div>
          <div className="topbar-controls">
            <Tooltip content="Toggle theme">
              <button
                className="icon-button"
                type="button"
                aria-label="Toggle theme"
                onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
              >
                {theme === "dark" ? <Sun size={18} /> : <Moon size={18} />}
              </button>
            </Tooltip>
          </div>
        </header>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

function navigationGroups(options: {
  hasLlm: boolean;
  hasMcp: boolean;
  hasTraffic: boolean;
  dumpMode: boolean;
}): ReadonlyArray<{ title: string; items: readonly NavItemConfig[] }> {
  const groups: Array<{ title: string; items: readonly NavItemConfig[] }> = [
    {
      title: "Gateway",
      items: [{ to: "/", label: "Home", icon: Home }],
    },
  ];
  if (!options.dumpMode) {
    groups.push({
      title: "LLM",
      items: options.hasLlm
        ? [
            { to: "/llm/models", label: "Models", icon: Bot },
            { to: "/llm/providers", label: "Providers", icon: Boxes },

            {
              to: "/llm/policies",
              label: "Policies",
              icon: Bolt,
              groupStart: true,
            },
            { to: "/llm/guardrails", label: "Guardrails", icon: Shield },
            { to: "/llm/keys", label: "Virtual API Keys", icon: KeyRound },
            { to: "/llm/costs", label: "Costs", icon: Coins },

            {
              to: "/llm/analytics",
              label: "Analytics",
              icon: BarChart3,
              groupStart: true,
            },
            { to: "/llm/logs", label: "Logs", icon: ScrollText },

            {
              to: "/llm/client-setup",
              label: "Client Setup",
              icon: Cable,
              groupStart: true,
            },
            { to: "/llm/playground", label: "Chat Playground", icon: Play },
          ]
        : [
            {
              to: "/llm/get-started",
              label: "Get started",
              icon: Bot,
              placeholder: true,
            },
          ],
    });
    groups.push({
      title: "MCP",
      items: options.hasMcp
        ? [
            { to: "/mcp/servers", label: "Servers", icon: Server },
            { to: "/mcp/policies", label: "Policies", icon: ShieldCheck },
            { to: "/mcp/playground", label: "Tool Playground", icon: Play },
          ]
        : [
            {
              to: "/mcp/get-started",
              label: "Get started",
              icon: Server,
              placeholder: true,
            },
          ],
    });
  }
  groups.push({
    title: "Traffic",
    items: options.dumpMode
      ? [
          { to: "/traffic/listeners", label: "Listeners", icon: Network },
          { to: "/traffic/routes", label: "Routes", icon: Route },
          { to: "/traffic/policies", label: "Policies", icon: ShieldCheck },
        ]
      : options.hasTraffic
        ? [
            { to: "/traffic/listeners", label: "Listeners", icon: Network },
            { to: "/traffic/routes", label: "Routes", icon: Route },
          ]
        : [
            {
              to: "/traffic/get-started",
              label: "Get started",
              icon: Network,
              placeholder: true,
            },
          ],
  });
  groups.push({
    title: "Tools",
    items: options.dumpMode
      ? [{ to: "/cel", label: "CEL Playground", icon: Braces }]
      : [
          { to: "/cel", label: "CEL Playground", icon: Braces },
          { to: "/raw-config", label: "Raw Configuration", icon: FileCode2 },
        ],
  });
  return groups;
}

function NavSection(props: {
  title: string;
  items: readonly NavItemConfig[];
  currentPath: string;
}) {
  return (
    <>
      <div className="nav-section">{props.title}</div>
      {props.items.map((item) => (
        <NavItem key={item.to} {...item} currentPath={props.currentPath} />
      ))}
    </>
  );
}

function MobileNavSection(props: {
  title: string;
  items: readonly NavItemConfig[];
  currentPath: string;
}) {
  return (
    <>
      <div className="mobile-nav-section">{props.title}</div>
      {props.items.map((item) => (
        <MobileNavItem
          key={item.to}
          {...item}
          currentPath={props.currentPath}
        />
      ))}
    </>
  );
}

function MobileNavItem(props: {
  to: string;
  label: string;
  icon: React.ComponentType<{ size?: number }>;
  currentPath: string;
  placeholder?: boolean;
  groupStart?: boolean;
}) {
  const Icon = props.icon;
  const navigate = useNavigate();
  const active = props.placeholder
    ? false
    : props.to === "/"
      ? props.label === "Home" && props.currentPath === "/"
      : props.currentPath.startsWith(props.to);
  if (props.placeholder) {
    return (
      <button
        type="button"
        className={
          props.groupStart
            ? "mobile-nav-item nav-group-start"
            : "mobile-nav-item"
        }
        role="menuitem"
        onClick={() => void navigate({ to: props.to })}
      >
        <Icon size={16} />
        <span>{props.label}</span>
      </button>
    );
  }
  return (
    <Link
      to={props.to}
      className={`${active ? "mobile-nav-item active" : "mobile-nav-item"}${props.groupStart ? " nav-group-start" : ""}`}
      role="menuitem"
    >
      <Icon size={16} />
      <span>{props.label}</span>
    </Link>
  );
}

function eyebrowForPath(path: string) {
  if (path === "/") return "Gateway overview";
  if (path.startsWith("/mcp")) return "MCP configuration";
  if (path.startsWith("/traffic")) return "Traffic configuration";
  if (path.startsWith("/cel") || path.startsWith("/raw-config"))
    return "Policy tools";
  return "LLM configuration";
}

function NavItem(props: {
  to: string;
  label: string;
  icon: React.ComponentType<{ size?: number }>;
  currentPath: string;
  placeholder?: boolean;
  groupStart?: boolean;
}) {
  const Icon = props.icon;
  const navigate = useNavigate();
  const active = props.placeholder
    ? false
    : props.to === "/"
      ? props.label === "Home" && props.currentPath === "/"
      : props.currentPath.startsWith(props.to);
  if (props.placeholder) {
    return (
      <button
        type="button"
        className={props.groupStart ? "nav-item nav-group-start" : "nav-item"}
        onClick={() => void navigate({ to: props.to })}
      >
        <Icon size={17} />
        <span>{props.label}</span>
      </button>
    );
  }
  return (
    <Link
      to={props.to}
      className={`${active ? "nav-item active" : "nav-item"}${props.groupStart ? " nav-group-start" : ""}`}
    >
      <Icon size={17} />
      <span>{props.label}</span>
    </Link>
  );
}
