import styled from "@emotion/styled";
import { Breadcrumb } from "antd";
import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { capitalizeFirstLetters } from "../utils/helpers";

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

    return pathSegments.map((segment, index) => {
      const path = `/${pathSegments.slice(0, index + 1).join("/")}`;
      const isLast = index === pathSegments.length - 1;
      const label = 
        capitalizeFirstLetters(segment.replaceAll('-',' ').toLowerCase()
          // Replace property names in the config.
          .replaceAll("frontendpolicies", 'Frontend Policies')
          .replaceAll("externalauth", 'External Auth')
          .replaceAll("apikeyauth", 'API Key Auth')
          .replaceAll("apikey", 'API Key')
          .replaceAll("basicauth", 'Basic Auth')
          .replaceAll("jwtauth", 'JWT Auth')
          .replaceAll("cors", 'CORS')
          .replaceAll("ai", 'AI')
          // Replace any keywords 
          .replaceAll("cel", 'CEL')
          .replaceAll("llm", 'LLM')
          .replaceAll('mcp','MCP')
        );

      return {
        title: isLast ? (
          <span>{label}</span>
        ) : (
          // TODO: the path can be like: /traffic-configuration/bind/8080/listener/0/route/0/backend/0
          // In this case, breadcrumbs would be: Traffic Configuration/Bind/8080/Listener/0/Route/0/Backend/0
          // If a user clicks on the "Listener" breadcrumb here, they would not get to a valid route (since the listener id would not be provided).
          // Fix in App.ts, and in the Hierarchy Tree.
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
