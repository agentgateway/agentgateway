import styled from "@emotion/styled";
import { Breadcrumb } from "antd";
import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";

const StyledBreadcrumb = styled(Breadcrumb)`
  .ant-breadcrumb-link {
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: color var(--transition-base) var(--transition-timing);

    &:hover {
      color: var(--color-primary);
    }
  }

  .ant-breadcrumb-separator {
    color: var(--color-text-tertiary);
  }

  /* Last item (current page) should not be clickable */
  li:last-child .ant-breadcrumb-link {
    color: var(--color-text-base);
    font-weight: var(--font-weight-semibold);
    cursor: default;

    &:hover {
      color: var(--color-text-base);
    }
  }
`;

const pathLabels: Record<string, string> = {
  dashboard: "Home",
  listeners: "Listeners",
  routes: "Routes",
  backends: "Backends",
  policies: "Policies",
  playground: "Playground",
  llm: "LLM",
  mcp: "MCP",
  traffic: "Traffic",
  models: "Models",
  logs: "Logs",
  metrics: "Metrics",
  servers: "Servers",
  routing: "Routing",
  "cel-playground": "CEL Playground",
  "raw-config": "Config Editor",
  listener: "Listener",
  route: "Route",
  tcproute: "TCP Route",
  backend: "Backend",
  policy: "Policy",
  bind: "Bind",
  create: "Create",
  edit: "Edit",
};

export function Breadcrumbs() {
  const location = useLocation();
  const navigate = useNavigate();

  const breadcrumbItems = useMemo(() => {
    const pathSegments = location.pathname
      .split("/")
      .filter((segment) => segment !== "");

    // Don't show breadcrumbs on root
    if (pathSegments.length === 0) {
      return null;
    }

    // Dashboard gets a single breadcrumb
    if (location.pathname === "/dashboard") {
      return [{ title: <span>Dashboard</span> }];
    }

    // Special handling for traffic2 routes
    // Traffic2 has structure: /traffic2/:resourceType/:action
    // We want breadcrumbs: Traffic → Create/Edit ResourceType
    if (pathSegments[0] === "traffic2" && pathSegments.length === 3) {
      const [_, resourceType, action] = pathSegments;
      const resourceLabel =
        pathLabels[resourceType] ||
        resourceType.charAt(0).toUpperCase() + resourceType.slice(1);
      const actionLabel =
        pathLabels[action] || action.charAt(0).toUpperCase() + action.slice(1);

      return [
        {
          title: <a onClick={() => navigate("/traffic2")}>Traffic</a>,
        },
        {
          title: `${actionLabel} ${resourceLabel}`,
        },
      ];
    }

    // Special handling for traffic routes
    // Traffic has structure: /traffic/bind/:port/listener/:li/route/:ri/backend/:bi
    // We want breadcrumbs: Traffic → Port X → Listener Y → Route Z → Backend N
    if (pathSegments[0] === "traffic" && pathSegments.length > 1) {
      const items = [
        {
          title: <a onClick={() => navigate("/traffic")}>Traffic</a>,
        },
      ];

      // Build breadcrumbs from path segments
      let currentPath = "/traffic";
      for (let i = 1; i < pathSegments.length; i += 2) {
        const type = pathSegments[i];
        const value = pathSegments[i + 1];

        if (!value) break;

        currentPath += `/${type}/${value}`;
        const isLast = i + 2 >= pathSegments.length;

        let label = "";
        if (type === "bind") {
          label = `Port ${value}`;
        } else if (type === "listener") {
          label = `Listener ${value}`;
        } else if (type === "route") {
          label = `Route ${value}`;
        } else if (type === "tcproute") {
          label = `TCP Route ${value}`;
        } else if (type === "backend") {
          label = `Backend ${parseInt(value) + 1}`;
        } else {
          label = pathLabels[type] || type;
        }

        items.push({
          title: isLast ? (
            <span>{label}</span>
          ) : (
            <a onClick={() => navigate(currentPath)}>{label}</a>
          ),
        });
      }

      return items;
    }

    return pathSegments.map((segment, index) => {
      const path = `/${pathSegments.slice(0, index + 1).join("/")}`;
      const isLast = index === pathSegments.length - 1;
      const label =
        pathLabels[segment] ||
        segment.charAt(0).toUpperCase() + segment.slice(1);

      return {
        title: isLast ? (
          <span>{label}</span>
        ) : (
          <a onClick={() => navigate(path)}>{label}</a>
        ),
      };
    });
  }, [location.pathname, navigate]);

  if (!breadcrumbItems) {
    return null;
  }

  return <StyledBreadcrumb items={breadcrumbItems} />;
}
