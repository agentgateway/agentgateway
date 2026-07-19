import { tr } from "../i18n";
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
  Languages,
  Menu,
  MessageSquarePlus,
  Network,
  Route,
  ScrollText,
  ShieldCheck,
  Shield,
  SlidersHorizontal,
  Bolt,
  Moon,
  Play,
  Server,
  Sun,
} from "lucide-react";
import { useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { Dropdown, Tooltip, useDismissiblePopover } from "./Primitives";
import { useConfigDumpMode, useGatewayConfig } from "../hooks";
import { currentLanguage, setLanguage, type AppLanguage } from "../i18n";
import logoDark from "../assets/agw-dark.svg";
import logoLight from "../assets/agw-light.svg";

type NavItemConfig = {
  to: string;
  label: string;
  icon: React.ComponentType<{ size?: number }>;
  placeholder?: boolean;
  groupStart?: boolean;
  exact?: boolean;
};

export function Shell() {
  const { t } = useTranslation();
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
      ? Boolean(config.data.binds?.length) ||
        "gateways" in config.data ||
        "routes" in config.data
      : true;
  const hasBinds = dumpMode
    ? true
    : config.data
      ? Boolean(config.data.binds?.length)
      : false;
  const language = currentLanguage();
  const projectLinks = [
    {
      label: "GitHub",
      href: "https://github.com/agentgateway/agentgateway",
      icon: Github,
    },
    {
      label: t("shell.documentation"),
      href: "https://agentgateway.dev/docs/standalone/latest/",
      icon: Globe,
    },
    {
      label: t("shell.feedback"),
      href: "https://github.com/agentgateway/agentgateway/issues/new?title=UI%20feedback%3A%20&body=Thanks%20for%20trying%20the%20agentgateway%20UI.%0A%0AWhat%20happened%3F%0A%0AWhat%20did%20you%20expect%20instead%3F%0A%0AAny%20screenshots%2C%20logs%2C%20or%20config%20that%20would%20help%3F",
      icon: MessageSquarePlus,
    },
  ] as const;
  const navGroups = navigationGroups(t, {
    hasLlm,
    hasMcp,
    hasTraffic,
    hasBinds,
    dumpMode,
  });
  const nav = navGroups.flatMap((group) => group.items);
  const currentNav =
    nav
      .filter((item) => navItemActive(item, router.location.pathname))
      .sort((left, right) => right.to.length - left.to.length)[0] ?? nav[0];
  const CurrentIcon = currentNav.icon;

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("theme", theme);
  }, [theme]);

  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  useEffect(() => {
    setMobileNavOpen(false);
  }, [router.location.pathname]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <Link to="/" className="brand" aria-label={t("nav.home")}>
          <img
            className="brand-logo brand-logo-light"
            src={logoLight}
            alt={tr("copy.agentgateway")}
          />
          <img
            className="brand-logo brand-logo-dark"
            src={logoDark}
            alt={tr("copy.agentgateway")}
          />
        </Link>
        <nav className="nav-list" aria-label={t("shell.primaryNavigation")}>
          {navGroups.map((group) => (
            <NavSection
              key={group.title}
              title={group.title}
              items={group.items}
              currentPath={router.location.pathname}
            />
          ))}
        </nav>
        <div className="sidebar-links" aria-label={t("shell.projectLinks")}>
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
                  aria-label={t("shell.primaryNavigation")}
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
              {eyebrowForPath(router.location.pathname, t)}
            </span>
          </div>
          <div className="topbar-controls">
            <Dropdown
              className="language-select"
              ariaLabel={t("language.select")}
              value={language}
              triggerIcon={<Languages size={16} aria-hidden="true" />}
              options={[
                { value: "en", label: t("language.english") },
                {
                  value: "zh-CN",
                  label: t("language.simplifiedChinese"),
                },
              ]}
              onChange={(value) => {
                void setLanguage(value as AppLanguage);
              }}
            />
            <Tooltip content={t("shell.toggleTheme")}>
              <button
                className="icon-button"
                type="button"
                aria-label={t("shell.toggleTheme")}
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

function navigationGroups(
  t: TFunction,
  options: {
    hasBinds: boolean;
    hasLlm: boolean;
    hasMcp: boolean;
    hasTraffic: boolean;
    dumpMode: boolean;
  },
): ReadonlyArray<{ title: string; items: readonly NavItemConfig[] }> {
  const groups: Array<{ title: string; items: readonly NavItemConfig[] }> = [
    {
      title: t("nav.gateway"),
      items: [{ to: "/", label: t("nav.home"), icon: Home }],
    },
  ];
  if (!options.dumpMode) {
    groups.push({
      title: t("nav.llm"),
      items: options.hasLlm
        ? [
            { to: "/llm/models", label: t("nav.models"), icon: Bot },
            { to: "/llm/providers", label: t("nav.providers"), icon: Boxes },

            {
              to: "/llm/policies",
              label: t("nav.policies"),
              icon: Bolt,
              groupStart: true,
            },
            { to: "/llm/guardrails", label: t("nav.guardrails"), icon: Shield },
            { to: "/llm/keys", label: t("nav.keys"), icon: KeyRound },
            { to: "/llm/costs", label: t("nav.costs"), icon: Coins },

            {
              to: "/llm/analytics",
              label: t("nav.analytics"),
              icon: BarChart3,
              groupStart: true,
            },
            { to: "/llm/logs", label: t("nav.logs"), icon: ScrollText },

            {
              to: "/llm/client-setup",
              label: t("nav.clientSetup"),
              icon: Cable,
              groupStart: true,
            },
            {
              to: "/llm/playground",
              label: t("nav.chatPlayground"),
              icon: Play,
            },
          ]
        : [
            {
              to: "/llm/get-started",
              label: t("nav.getStarted"),
              icon: Bot,
              placeholder: true,
            },
          ],
    });
    groups.push({
      title: t("nav.mcp"),
      items: options.hasMcp
        ? [
            { to: "/mcp/servers", label: t("nav.servers"), icon: Server },
            {
              to: "/mcp/policies",
              label: t("nav.policies"),
              icon: ShieldCheck,
            },
            {
              to: "/mcp/playground",
              label: t("nav.toolPlayground"),
              icon: Play,
            },
          ]
        : [
            {
              to: "/mcp/get-started",
              label: t("nav.getStarted"),
              icon: Server,
              placeholder: true,
            },
          ],
    });
  }
  groups.push({
    title: t("nav.traffic"),
    items: options.dumpMode
      ? [
          {
            to: "/traffic/listeners",
            label: t("nav.listeners"),
            icon: Network,
          },
          { to: "/traffic/routes", label: t("nav.routes"), icon: Route },
          {
            to: "/traffic/policies",
            label: t("nav.policies"),
            icon: ShieldCheck,
          },
        ]
      : options.hasTraffic
        ? [
            {
              to: "/traffic/gateways",
              label: t("nav.gateways"),
              icon: Network,
            },
            ...(options.hasBinds
              ? [
                  {
                    to: "/traffic/listeners",
                    label: t("nav.listeners"),
                    icon: Network,
                  },
                ]
              : []),
            { to: "/traffic/routes", label: t("nav.routes"), icon: Route },
          ]
        : [
            {
              to: "/traffic/get-started",
              label: t("nav.getStarted"),
              icon: Network,
              placeholder: true,
            },
          ],
  });
  groups.push({
    title: t("nav.tools"),
    items: options.dumpMode
      ? [{ to: "/cel", label: t("nav.celPlayground"), icon: Braces }]
      : [
          { to: "/cel", label: t("nav.celPlayground"), icon: Braces },
          {
            to: "/raw-config",
            label: t("nav.rawConfiguration"),
            icon: FileCode2,
            exact: true,
          },
          {
            to: "/settings",
            label: t("nav.settings"),
            icon: SlidersHorizontal,
          },
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
  exact?: boolean;
}) {
  const Icon = props.icon;
  const navigate = useNavigate();
  const active = props.placeholder
    ? false
    : navItemActive(props, props.currentPath);
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

function eyebrowForPath(path: string, t: TFunction) {
  if (path === "/") return t("shell.gatewayOverview");
  if (path.startsWith("/mcp")) return t("shell.mcpConfiguration");
  if (path.startsWith("/traffic")) return t("shell.trafficConfiguration");
  if (
    path.startsWith("/cel") ||
    path.startsWith("/raw-config") ||
    path.startsWith("/settings")
  )
    return t("shell.policyTools");
  return t("shell.llmConfiguration");
}

function NavItem(props: {
  to: string;
  label: string;
  icon: React.ComponentType<{ size?: number }>;
  currentPath: string;
  placeholder?: boolean;
  groupStart?: boolean;
  exact?: boolean;
}) {
  const Icon = props.icon;
  const navigate = useNavigate();
  const active = props.placeholder
    ? false
    : navItemActive(props, props.currentPath);
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

function navItemActive(item: { to: string; exact?: boolean }, path: string) {
  if (item.to === "/") return path === "/";
  if (item.exact) return path === item.to;
  return path === item.to || path.startsWith(`${item.to}/`);
}
