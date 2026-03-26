import styled from "@emotion/styled";
import type { MenuProps } from "antd";
import { Layout as AntLayout, Button, Menu } from "antd";
import {
  BarChart3,
  Brain,
  FileText,
  FlaskConical,
  Home,
  Moon,
  Network,
  Route,
  Sparkles,
  Sun,
} from "lucide-react";
import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useTheme } from "../../contexts";
import { AgentgatewayLogo } from "../AgentgatewayLogo";
import { Breadcrumbs } from "../Breadcrumbs";

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
  overflow-y: auto;
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
    position: relative;
    z-index: 1;

    // Removes a faint border from the logo container.
    > div {
      border: none !important
    }
  }
`;

const StyledHeader = styled(Header)`
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--color-bg-container);
  border-bottom: 1px solid var(--color-border-secondary);
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

const Logo = styled.div`
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: var(--spacing-md);
  padding: var(--spacing-xl) var(--spacing-lg);
  border-bottom: 1px solid var(--color-border-secondary);
  cursor: pointer;
  transition: opacity var(--transition-base) var(--transition-timing);

  svg {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
  }

  span {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-base);
  }

  &:hover {
    opacity: 0.8;
  }
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
    padding: 0px 4px 0px 0px !important;
  }

`;

type MenuItem = Required<MenuProps>["items"][number];

export const MainLayout: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const navigate = useNavigate();
  const location = useLocation();
  const { theme, toggleTheme } = useTheme();

  const menuItems: MenuItem[] = [
    {
      key: "/dashboard",
      icon: <Home size={18} />,
      label: "Dashboard",
    },
    {
      key: "llm-group",
      label: "LLM",
      type: "group",
      children: [
        {
          key: "/llm",
          icon: <Brain size={18} />,
          label: "Configuration",
        },
        {
          key: "/llm/logs",
          icon: <FileText size={18} />,
          label: "Logs",
        },
        {
          key: "/llm/metrics",
          icon: <BarChart3 size={18} />,
          label: "Metrics",
        },
        {
          key: "/llm/playground",
          icon: <FlaskConical size={18} />,
          label: "Playground",
        },
      ],
    },
    {
      key: "mcp-group",
      label: "MCP",
      type: "group",
      children: [
        {
          key: "/mcp",
          icon: <Network size={18} />,
          label: "Configuration",
        },
        {
          key: "/mcp/logs",
          icon: <FileText size={18} />,
          label: "Logs",
        },
        {
          key: "/mcp/metrics",
          icon: <BarChart3 size={18} />,
          label: "Metrics",
        },
        {
          key: "/mcp/playground",
          icon: <FlaskConical size={18} />,
          label: "Playground",
        },
      ],
    },
    {
      key: "traffic-group",
      label: "Traffic",
      type: "group",
      children: [
        {
          key: "/traffic",
          icon: <Route size={18} />,
          label: "Routing",
        },
        {
          key: "/traffic/logs",
          icon: <FileText size={18} />,
          label: "Logs",
        },
        {
          key: "/traffic/metrics",
          icon: <BarChart3 size={18} />,
          label: "Metrics",
        },
      ],
    },
    {
      key: "tools-group",
      label: "Tools",
      type: "group",
      children: [
        {
          key: "/cel-playground",
          icon: <Sparkles size={18} />,
          label: "CEL Playground",
        },
      ],
    },
  ];

  const handleMenuClick: MenuProps["onClick"] = ({ key }) => {
    navigate(key);
  };

  const selectedKeys = useMemo(() => {
    const knownPaths = [
      "/traffic",
      "/traffic/logs",
      "/traffic/metrics",
      "/llm",
      "/llm/logs",
      "/llm/metrics",
      "/llm/playground",
      "/mcp",
      "/mcp/logs",
      "/mcp/metrics",
      "/mcp/playground",
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
      <StyledSider width={240} collapsed={false}>
        <Logo onClick={() => navigate("/dashboard")}>
          <AgentgatewayLogo />
          <span>agentgateway</span>
        </Logo>
        <StyledMenu
          mode="inline"
          selectedKeys={selectedKeys}
          items={menuItems}
          onClick={handleMenuClick}
        />
      </StyledSider>
      <ContentWrapper>
        <StyledHeader>
          <Breadcrumbs />
          <ThemeToggleButton
            type="text"
            icon={theme === "dark" ? <Sun size={20} /> : <Moon size={20} />}
            onClick={toggleTheme}
            title={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
          />
        </StyledHeader>
        <StyledContent>{children}</StyledContent>
      </ContentWrapper>
    </StyledLayout>
  );
};
