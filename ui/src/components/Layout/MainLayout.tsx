import styled from "@emotion/styled";
import type { MenuProps } from "antd";
import { Layout as AntLayout, Button, Menu } from "antd";
import {
  BarChart3,
  Bot,
  Boxes,
  Braces,
  Cable,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  FileCode,
  Home,
  KeyRound,
  Moon,
  Network,
  Play,
  Route,
  Server,
  Shield,
  ShieldAlert,
  Sun
} from "lucide-react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useTheme } from "../../contexts";
import { AgentgatewayLogo } from "../AgentgatewayLogo";
import { Breadcrumbs } from "../Breadcrumbs";
import { XdsModeBanner } from "../XdsModeBanner";

const { Sider, Content, Header } = AntLayout;

const StyledLayout = styled(AntLayout)`
  display: flex;
  width: 100%;
  height: 100vh;
  overflow: hidden;
`;

const StyledSider = styled(Sider)`
  display: flex;
  flex-direction: column;
  background:
    /* Left to right gradient - subtle edge glow */
    linear-gradient(
      90deg,
      color-mix(in srgb, var(--color-sidebar) 1%, transparent) 0%,
      transparent 25%
    ),
    /* Top to bottom gradient - main gradient */
    linear-gradient(
        180deg,
        color-mix(in srgb, var(--color-sidebar) 1.5%, var(--color-bg-container))
          0%,
        color-mix(
            in srgb,
            var(--color-sidebar) 0.75%,
            var(--color-bg-container)
          )
          50%,
        var(--color-bg-container) 100%
      );
  border-right: 1px solid var(--color-border-secondary);
  overflow: hidden;
  position: relative;

  /* Additional gradient overlays for depth */
  &::before {
    content: "";
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background:
      /* Diagonal gradient for extra dimension */
      linear-gradient(
        135deg,
        color-mix(in srgb, var(--color-sidebar) 0.75%, transparent) 0%,
        transparent 50%
      ),
      /* Top gradient intensifier */
      linear-gradient(
          180deg,
          color-mix(in srgb, var(--color-sidebar) 1.25%, transparent) 0%,
          color-mix(in srgb, var(--color-sidebar) 0.5%, transparent) 40%,
          transparent 100%
        );
    pointer-events: none;
    z-index: 0;
  }

  .ant-layout-sider-children {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
    position: relative;
    z-index: 1;

    // Removes a faint border from the logo container.
    > div {
      border: none !important
    }

    // Menu takes remaining space and scrolls internally
    > ul.ant-menu {
      flex: 1;
      overflow-y: auto;
      min-height: 0;
    }
  }
`;

const StyledHeader = styled(Header)`
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--color-bg-layout);
  border-bottom: 1px solid var(--color-border-base);
  padding: 0 var(--spacing-xl);
  height: var(--header-height);
  min-height: var(--header-height);
`;

const ContentWrapper = styled(AntLayout)`
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
`;

const StyledContent = styled(Content)`
  flex: 1;
  overflow-y: auto;
  background: linear-gradient(
    135deg,
    var(--color-bg-layout) 0%,
    color-mix(in srgb, var(--color-bg-layout) 98%, white) 100%
  );
  padding: var(--spacing-xl);
  min-height: 0;
  position: relative;

  /* Subtle radial gradient overlay for depth */
  &::before {
    content: "";
    position: fixed;
    top: 0;
    left: var(--sidebar-width);
    right: 0;
    bottom: 0;
    background: radial-gradient(
      ellipse at top right,
      color-mix(in srgb, var(--color-primary) 1.5%, transparent) 0%,
      transparent 50%
    );
    pointer-events: none;
    z-index: 0;
  }

  /* Ensure content is above gradient */
  > * {
    position: relative;
    z-index: 1;
  }
`;

const Logo = styled.div<{ $collapsed?: boolean }>`
  display: flex;
  align-items: center;
  justify-content: ${({ $collapsed }) => ($collapsed ? "center" : "flex-start")};
  gap: var(--spacing-md);
  padding: ${({ $collapsed }) => ($collapsed ? "var(--spacing-xl) var(--spacing-md)" : "var(--spacing-xl) var(--spacing-lg)")};
  border-bottom: 1px solid var(--color-border-secondary);
  cursor: pointer;
  transition: opacity var(--transition-base) var(--transition-timing), padding var(--transition-base) var(--transition-timing);
  overflow: hidden;

  svg {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
  }

  span {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-base);
    white-space: nowrap;
    overflow: hidden;
  }

  &:hover {
    opacity: 0.8;
  }
`;

const CollapseButton = styled.button<{ $left: number }>`
  position: fixed;
  left: ${({ $left }) => $left - 18}px;
  top: 50vh;
  transform: translateY(-50%);
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  min-width: 24px;
  min-height: 24px;
  padding: 0;
  aspect-ratio: 1 / 1;
  border-radius: 50%;
  border: 1px solid var(--color-border-base);
  background: var(--color-bg-container);
  color: var(--color-text-secondary);
  cursor: pointer;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.15);
  transition: left 0.2s ease,
    background var(--transition-base) var(--transition-timing),
    color var(--transition-base) var(--transition-timing),
    box-shadow var(--transition-base) var(--transition-timing);

  svg {
    flex-shrink: 0;
    display: block;
  }

  &:hover {
    background: var(--color-bg-hover);
    color: var(--color-text-base);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
  }
`;

const HeaderActions = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
`;

const ThemeToggleButton = styled(Button)`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px !important;
  height: 40px !important;
  overflow: hidden;
  border-radius: var(--border-radius-lg);
  border: 1px solid var(--color-border-base);
  background: var(--color-bg-container);
  color: var(--color-text-base);
  cursor: pointer;
  transition: all var(--transition-base) var(--transition-timing);

  span {
    display: contents !important;
  }

  &:hover {
    background: var(--color-bg-hover);
    border-color: var(--color-primary);
    color: var(--color-primary);
  }
`;

const StyledMenu = styled(Menu)`
  /* Menu item hover and selected states */
  .ant-menu-item,
  .ant-menu-submenu-title {
    transition: background-color 250ms ease !important;
    user-select: none;
    border-radius: 20px !important;
    height: 42px !important;
    background-color: transparent !important;

    &:not(.ant-menu-item-selected) {
      &:hover {
        background-color: color-mix(
          in srgb,
          var(--color-sidebar-active) 30%,
          var(--color-bg-container)
        ) !important;
        &:active {
          background-color: color-mix(
            in srgb,
            var(--color-sidebar-active) 20%,
            var(--color-bg-container)
          ) !important;
        }
      }
    }

    .ant-menu-item-icon,
    .anticon {
      color: inherit !important;
    }

    /* Selected state — pill style */
    &.ant-menu-item-selected {
      background-color: color-mix(
        in srgb,
        var(--color-sidebar-active) 80%,
        var(--color-bg-container)
      ) !important;
      box-shadow: none !important;
      color: color-mix(in srgb, var(--color-sidebar) 0%, var(--color-text-base)) !important;

      [data-theme="light"] & {
        color: var(--color-text-inverse) !important;
      }
    }

    /* Submenu selected state */
    &.ant-menu-submenu-selected > .ant-menu-submenu-title {
      background-color: color-mix(
        in srgb,
        var(--color-sidebar) 10%,
        var(--color-bg-container)
      ) !important;
    }
  }

  .ant-menu-item-group-title {
    padding: 12px 0px 12px 20px;
    user-select: none;
    opacity: .8;
    font-weight: 700;
    font-size: 90%;
    text-transform: uppercase;
  }

  li:has(.ant-menu-item){
    padding: 0px 4px !important;
  }

  &.ant-menu-inline-collapsed li:has(.ant-menu-item) {
    padding: 0 !important;
  }

`;

type MenuItem = Required<MenuProps>["items"][number];

export const MainLayout: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const navigate = useNavigate();
  const location = useLocation();
  const { theme, toggleTheme } = useTheme();
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem("sidebar-collapsed") === "true"
  );

  const handleCollapse = () => {
    const next = !collapsed;
    setCollapsed(next);
    localStorage.setItem("sidebar-collapsed", String(next));
  };

  const menuItems: MenuItem[] = [
    {
      key: "gateway-group",
      label: "Gateway",
      type: "group",
      children: [
        { key: "/", icon: <Home size={18} />, label: "Home" },
      ],
    },
    {
      key: "llm-group",
      label: "LLM",
      type: "group",
      children: [
        // { key: "/llm-configuration", icon: <Brain size={18} />, label: "Configuration" },
        { key: "/llm-models", icon: <Bot size={18} />, label: "Models" },
        { key: "/llm-providers", icon: <Boxes size={18} />, label: "Providers" },
        { key: "/llm-policies", icon: <ShieldAlert size={18} />, label: "Policies" },
        { key: "/llm-guardrails", icon: <Shield size={18} />, label: "Guardrails" },
        { key: "/llm-monitoring", icon: <BarChart3 size={18} />, label: "Monitoring" },
        { key: "/llm-keys", icon: <KeyRound size={18} />, label: "Virtual API Keys" },
        { key: "/llm-playground", icon: <Play size={18} />, label: "Playground" },
        { key: "/llm-client-setup", icon: <Cable size={18} />, label: "Client Setup" },
      ],
    },
    {
      key: "mcp-group",
      label: "MCP",
      type: "group",
      children: [
        // { key: "/mcp-configuration", icon: <Network size={18} />, label: "Configuration" },
        { key: "/mcp-servers", icon: <Server size={18} />, label: "Servers" },
        { key: "/mcp-playground", icon: <Play size={18} />, label: "Playground" },
      ],
    },
    {
      key: "traffic-group",
      label: "Traffic",
      type: "group",
      children: [
        // { key: "/traffic-configuration", icon: <Route size={18} />, label: "Configuration" },
        { key: "/traffic-listeners", icon: <Network size={18} />, label: "Listeners" },
        { key: "/traffic-routes", icon: <Route size={18} />, label: "Routes" },
      ],
    },
    {
      key: "tools",
      label: "Tools",
      type: "group",
      children: [
        {
          key: "/cel-playground",
          icon: <Braces size={18} />,
          label: "CEL Playground",
        },
        {
          key: "/raw-config",
          icon: <FileCode size={18} />,
          label: "Raw Configuration",
        },
      ]
    },
  ];

  const handleMenuClick: MenuProps["onClick"] = ({ key }) => {
    if (key.startsWith("http")) {
      window.open(key, "_blank", "noopener,noreferrer");
      return;
    }
    navigate(key);
  };

  const selectedKeys = useMemo(() => {
    const knownPaths = [
      "/traffic-configuration",
      "/traffic-logs",
      "/traffic-metrics",
      "/traffic-listeners",
      "/traffic-routes",
      "/llm-configuration",
      "/llm-logs",
      "/llm-metrics",
      "/llm-playground",
      "/llm-models",
      "/llm-providers",
      "/llm-policies",
      "/llm-guardrails",
      "/llm-monitoring",
      "/llm-keys",
      "/llm-client-setup",
      "/mcp-configuration",
      "/mcp-logs",
      "/mcp-metrics",
      "/mcp-playground",
      "/mcp-servers",
      "/cel-playground",
      "/raw-config",
      "/dashboard",
    ];

    // Find longest matching prefix
    let longestMatch = location.pathname;
    let maxLength = 0;

    for (const path of knownPaths) {
      if (
        location.pathname === path ||
        location.pathname.startsWith(path + "/")
      ) {
        if (path.length > maxLength) {
          longestMatch = path;
          maxLength = path.length;
        }
      }
    }

    return [longestMatch];
  }, [location.pathname]);

  return (
    <StyledLayout>
      <StyledSider
        width={240}
        collapsedWidth={64}
        collapsed={collapsed}
        trigger={null}
      >
        <Logo $collapsed={collapsed} onClick={() => navigate("/")}>
          <AgentgatewayLogo />
          {!collapsed && <span>agentgateway</span>}
        </Logo>
        <StyledMenu
          mode="inline"
          inlineCollapsed={collapsed}
          selectedKeys={selectedKeys}
          items={menuItems}
          onClick={handleMenuClick}
        />
        <CollapseButton
          $left={collapsed ? 64 : 240}
          type="button"
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          onClick={handleCollapse}
        >
          {collapsed ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
        </CollapseButton>
      </StyledSider>
      <ContentWrapper>
        <StyledHeader>
          <Breadcrumbs />
          <HeaderActions>
            <Button
              type='link'
              icon={<ExternalLink size={18} />}
              onClick={() => window.open("https://agentgateway.dev/docs/", "_blank", "noopener,noreferrer")}
            >
              Docs
            </Button>
            <ThemeToggleButton
              type="text"
              icon={theme === "dark" ? <Sun size={20} /> : <Moon size={20} />}
              onClick={toggleTheme}
              title={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
            />
          </HeaderActions>
        </StyledHeader>
        <XdsModeBanner />
        <StyledContent>{children}</StyledContent>
      </ContentWrapper>
    </StyledLayout>
  );
};
