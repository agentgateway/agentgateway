import styled from "@emotion/styled";
import { Button, Card, Input, Tag } from "antd";
import type { ConnectionState, RouteInfo } from "./types";

const RouteCard = styled(Card)`
  cursor: pointer;
  transition: all 0.15s ease;
  position: relative;

  &::before {
    content: "";
    position: absolute;
    inset: 0;
    background: var(--color-primary);
    opacity: 0;
    transition: opacity 0.15s ease;
    pointer-events: none;
    border-radius: inherit;
  }

  &:hover {
    border-color: var(--color-primary);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.12);

    &::before {
      opacity: 0.03;
    }
  }

  &:active {
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.08);

    &::before {
      opacity: 0.05;
    }
  }
`;

interface RouteSelectorProps {
  routes: RouteInfo[];
  selectedRoute: RouteInfo | null;
  connectionState: ConnectionState;
  onSelectRoute: (route: RouteInfo) => void;
  onAuthTokenChange: (token: string) => void;
  onConnect: () => void;
}

export function RouteSelector({
  routes,
  selectedRoute,
  connectionState,
  onSelectRoute,
  onAuthTokenChange,
  onConnect,
}: RouteSelectorProps) {
  return (
    <>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "1rem",
          marginBottom: "1rem",
        }}
      >
        {routes.map((routeInfo, idx) => {
          const isSelected = selectedRoute === routeInfo;
          return (
            <RouteCard
              key={`${routeInfo.bindPort}-${routeInfo.routeIndex}`}
              size="small"
              style={{
                background: isSelected
                  ? "var(--color-bg-selected)"
                  : "var(--color-bg-spotlight)",
                border: isSelected
                  ? "2px solid var(--color-primary)"
                  : undefined,
              }}
              onClick={() => onSelectRoute(routeInfo)}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "0.5rem",
                }}
              >
                <span style={{ fontWeight: 500 }}>
                  {routeInfo.route.name || `Route ${idx + 1}`}
                </span>
                <Tag color="blue">Port {routeInfo.bindPort}</Tag>
                <Tag style={{ fontSize: "11px", fontFamily: "monospace" }}>
                  {routeInfo.routePath}
                </Tag>
                <Tag color="purple">MCP</Tag>
                <span
                  style={{
                    marginLeft: "auto",
                    fontSize: "12px",
                    color: "var(--color-text-secondary)",
                    fontFamily: "monospace",
                  }}
                >
                  {routeInfo.endpoint}
                </span>
              </div>
            </RouteCard>
          );
        })}
      </div>

      {selectedRoute && (
        <div
          style={{ display: "flex", gap: "1rem", alignItems: "flex-start" }}
        >
          <div style={{ flex: 1 }}>
            <label
              style={{
                display: "block",
                marginBottom: "8px",
                fontSize: "14px",
              }}
            >
              Auth Token (optional)
            </label>
            <Input
              placeholder="Bearer token for authentication"
              value={connectionState.authToken}
              onChange={(e) => onAuthTokenChange(e.target.value)}
            />
          </div>
          <div style={{ paddingTop: "30px" }}>
            <Button
              type="primary"
              onClick={onConnect}
              loading={connectionState.isConnecting}
              disabled={!selectedRoute}
            >
              {connectionState.isConnected ? "Reconnect" : "Connect to MCP"}
            </Button>
          </div>
        </div>
      )}
    </>
  );
}
