import { DeleteOutlined, DownOutlined, PlusOutlined } from "@ant-design/icons";
import { Global, css } from "@emotion/react";
import styled from "@emotion/styled";
import type { MenuProps } from "antd";
import {
  App,
  Badge,
  Button,
  Card,
  Dropdown,
  Empty,
  Space,
  Tooltip,
  Tree,
} from "antd";
import type { DataNode } from "antd/es/tree";
import {
  Bot,
  ChevronsDownUp,
  ChevronsUpDown,
  Headphones,
  MoreVertical,
  Network,
  Pencil,
  Route,
  Server,
  Settings,
  TriangleAlert,
} from "lucide-react";
import type { Key, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import toast from "react-hot-toast";
import { useLocation, useNavigate } from "react-router-dom";
import { useConfig } from "../../api";
import * as api from "../../api/crud";
import type { LocalRouteBackend } from "../../config";
import { getResourceColor } from "../../utils/colorPalette";
import { ProtocolTag } from "../ProtocolTag";
import type {
  BackendNode,
  BindNode,
  LLMPolicyNode,
  ListenerNode,
  MCPPolicyNode,
  MCPTargetNode,
  ModelNode,
  PolicyNode,
  RouteNode,
  TrafficHierarchy,
  ValidationError,
} from "./hooks/useTrafficHierarchy";
import { getDefaultPolicyValue, getPolicyLabel, getPolicyTypesForScope } from "./policyTypes";
import { ResourceIcon } from "./ResourceIcon";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Extract a human-readable message from any thrown value (Error, ApiError, etc.) */
function getErrorMessage(e: unknown, fallback: string): string {
  if (e && typeof e === "object" && "message" in e && typeof (e as any).message === "string") {
    return (e as any).message;
  }
  return fallback;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type NodeType = "bind" | "listener" | "route" | "backend";
export type HierarchySection = "llm" | "mcp" | "binds" | "frontendPolicies";

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const TreeCard = styled(Card)`
  border: 1px solid var(--color-border);
  box-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  transition: box-shadow 0.3s ease;

  &:hover {
    box-shadow: 0 4px 12px 0 rgba(0, 0, 0, 0.08);
  }

  .ant-card-body {
    padding: var(--spacing-md);
  }

  .ant-card-head {
    border-bottom: 1px solid var(--color-border);
    padding: var(--spacing-md) var(--spacing-lg);
    background: var(--color-bg-subtle);
  }

  .ant-card-head-title {
    padding: 0;
    font-weight: 600;
    font-size: 15px;
    letter-spacing: 0.01em;
  }

  /* Enhanced tree node styling */
  .ant-tree-treenode {
    padding: 0;
    margin: 0;
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .ant-tree .ant-tree-node-content-wrapper {
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
    border-radius: 6px;
    padding: 0;
    line-height: 1.4;
    flex: 1;
  }

  .ant-tree .ant-tree-node-content-wrapper:hover {
    background: rgba(139, 92, 246, 0.08);

    [data-theme="light"] & {
      background: rgba(139, 92, 246, 0.06);
    }
  }

  .ant-tree .ant-tree-node-content-wrapper:active {
    background: rgba(139, 92, 246, 0.12);

    [data-theme="light"] & {
      background: rgba(139, 92, 246, 0.1);
    }
  }

  .ant-tree .ant-tree-node-content-wrapper.ant-tree-node-selected {
    background: linear-gradient(
      90deg,
      rgba(139, 92, 246, 0.12),
      rgba(139, 92, 246, 0.06)
    ) !important;
    border-left: 2px solid rgba(139, 92, 246, 0.6);
    padding-left: 4px;

    [data-theme="light"] & {
      background: linear-gradient(
        90deg,
        rgba(139, 92, 246, 0.1),
        rgba(139, 92, 246, 0.04)
      ) !important;
      border-left-color: rgba(139, 92, 246, 0.5);
    }

    /* Disable hover and active effects when selected */
    &:hover {
      background: linear-gradient(
        90deg,
        rgba(139, 92, 246, 0.12),
        rgba(139, 92, 246, 0.06)
      ) !important;

      [data-theme="light"] & {
        background: linear-gradient(
          90deg,
          rgba(139, 92, 246, 0.1),
          rgba(139, 92, 246, 0.04)
        ) !important;
      }
    }

    &:active {
      background: linear-gradient(
        90deg,
        rgba(139, 92, 246, 0.12),
        rgba(139, 92, 246, 0.06)
      ) !important;

      [data-theme="light"] & {
        background: linear-gradient(
          90deg,
          rgba(139, 92, 246, 0.1),
          rgba(139, 92, 246, 0.04)
        ) !important;
      }
    }
  }

  /* Tree switcher (expand/collapse icon) */
  .ant-tree-switcher {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-text-tertiary);
    transition: all 0.2s ease;

    &:hover {
      color: var(--color-primary);
    }
  }

  /* Indent lines for hierarchy */
  .ant-tree-indent {
    position: relative;
  }

  .ant-tree-indent-unit {
    position: relative;
    width: 20px;

    &::before {
      content: "";
      position: absolute;
      left: 50%;
      top: 0;
      bottom: 0;
      width: 1px;
      background: var(--color-border);
      opacity: 0.3;
    }
  }
`;

const CardHeader = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
`;

const CardTitleRow = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: var(--spacing-sm);
  border-bottom: 1px solid var(--color-border-secondary);
`;

const CardTitle = styled.h3`
  margin: 0;
  font-size: 16px;
  font-weight: 700;
  color: var(--color-text-heading);
  cursor: pointer;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  letter-spacing: -0.01em;
  background: linear-gradient(
    135deg,
    var(--color-text-heading),
    var(--color-text-base)
  );
  -webkit-background-clip: text;
  background-clip: text;
  user-select: none;

  &:hover {
    color: var(--color-primary);
    background: linear-gradient(135deg, var(--color-primary), #8b5cf6);
    -webkit-background-clip: text;
    background-clip: text;
    transform: translateX(2px);
  }
`;

const NodeRow = styled.div<{ $onHoverChange?: boolean }>`
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  flex-wrap: nowrap;
  width: 100%;
  cursor: pointer;
  border-radius: 6px;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  position: relative;

  &::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: transparent;
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
    border-radius: 2px;
  }

  /* Only show hover and active effects when parent is NOT selected */
  .ant-tree-node-content-wrapper:not(.ant-tree-node-selected) &:hover {
    background: rgba(139, 92, 246, 0.08);
    transform: translateX(2px);
    --node-label-color: var(--color-text-heading);

    [data-theme="light"] & {
      background: rgba(139, 92, 246, 0.06);
    }

    &::before {
      background: rgba(139, 92, 246, 0.6);
    }
  }

  .ant-tree-node-content-wrapper:not(.ant-tree-node-selected) &:active {
    background: rgba(139, 92, 246, 0.12);
    transform: translateX(2px);

    [data-theme="light"] & {
      background: rgba(139, 92, 246, 0.1);
    }

    &::before {
      background: rgba(139, 92, 246, 0.8);
    }
  }
`;

const NodeLabel = styled.span`
  font-weight: 500;
  font-size: 13px;
  color: var(--node-label-color, var(--color-text-base));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
  letter-spacing: 0.01em;
  transition: color 0.15s ease;
  user-select: none;
`;

const MoreButton = styled(Button)`
  opacity: 0;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  margin-left: auto;
  flex-shrink: 0;
  transform: scale(0.95);

  .ant-tree-treenode:hover & {
    opacity: 1;
    transform: scale(1);
  }

  &:hover {
    background: var(--color-bg-elevated) !important;
  }
`;

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

function urlToSelectedKey(pathname: string, basePath: string): string | null {
  // Strip basePath prefix to get the hierarchy-relative path
  const rel = basePath ? pathname.slice(basePath.length) : pathname;

  // Check for MCP target policy routes (must be before target route)
  const mcpTargetPolicyPattern = /^\/mcp\/target\/(\d+)\/policy\/(.+)/;
  const mcpTargetPolicyMatch = rel.match(mcpTargetPolicyPattern);
  if (mcpTargetPolicyMatch) return `mcp-target-${mcpTargetPolicyMatch[1]}-policy-${mcpTargetPolicyMatch[2]}`;

  // Check for model routes first (must be before general LLM route)
  const modelPattern =  /^\/llm\/model\/(\d+)/;
  const modelMatch = rel.match(modelPattern);
  if (modelMatch) return `model-${modelMatch[1]}`;

  // Check for MCP target routes (must be before general MCP route)
  const mcpTargetPattern =  /^\/mcp\/target\/(\d+)/;
  const mcpTargetMatch = rel.match(mcpTargetPattern);
  if (mcpTargetMatch) return `mcp-target-${mcpTargetMatch[1]}`;

  // Check for LLM/MCP policy routes (must be before general top-level match)
  const llmPolicyPattern =  /^\/llm\/policy\/([^/?]+)/;
  const llmPolicyMatch = rel.match(llmPolicyPattern);
  if (llmPolicyMatch) return `llm-policy-${llmPolicyMatch[1]}`;

  const mcpPolicyPattern =  /^\/mcp\/policy\/([^/?]+)/;
  const mcpPolicyMatch = rel.match(mcpPolicyPattern);
  if (mcpPolicyMatch) return `mcp-policy-${mcpPolicyMatch[1]}`;

  // Check for top-level config routes
  const topLevelMatch = rel.match(/^\/(llm|mcp|frontendPolicies)/);
  if (topLevelMatch) return topLevelMatch[1];

  // Check for bind routes
  const m = rel.match(
    /^\/bind\/(\d+)(?:\/listener\/(\d+)(?:\/(tcp)?route\/(\d+)(?:\/backend\/(\d+)|\/policy\/([^/?]+))?)?)?/,
  );
  if (!m) return null;
  const [, port, li, tcp, ri, bi, policyType] = m;
  if (policyType !== undefined)
    return `policy-${port}-${li}-${ri}-${policyType}`;
  if (bi !== undefined) return `backend-${port}-${li}-${ri}-${bi}`;
  if (ri !== undefined)
    return `route-${port}-${li}-${tcp ? "tcp" : "http"}-${ri}`;
  if (li !== undefined) return `listener-${port}-${li}`;
  if (port !== undefined) return `bind-${port}`;
  return null;
}

// ---------------------------------------------------------------------------
// Node title builders
// ---------------------------------------------------------------------------

function ValidationBadges({ errors }: { errors: ValidationError[] }) {
  if (errors.length === 0) return null;
  const errCount = errors.filter((e) => e.level === "error").length;
  const warnCount = errors.filter((e) => e.level === "warning").length;
  return (
    <Tooltip
      title={errors.map((e) => e.message).join("\n")}
      styles={{ root: { whiteSpace: "pre-wrap" } }}
    >
      {errCount > 0 && (
        <Badge
          count={errCount}
          color="var(--color-error)"
          size="small"
          style={{ marginRight: 2 }}
        />
      )}
      {warnCount > 0 && (
        <Badge count={warnCount} color="var(--color-warning)" size="small" />
      )}
    </Tooltip>
  );
}

type ConfirmDeleteFn = (
  title: string,
  description: string,
  onConfirm: () => void,
) => void;

function buildBindTitle(
  bind: BindNode,
  navigate: (path: string) => void,
  onDelete: (port: number, parentPath: string) => void,
  confirmDelete: ConfirmDeleteFn,
  onAddListener: (port: number) => void,
  basePath: string,
): ReactNode {
  const bindPath = `${basePath}/bind/${bind.bind.port}`;

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(bindPath + "?edit=true");
      },
    },
    {
      key: "addListener",
      label: "Add Listener",
      icon: <PlusOutlined />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddListener(bind.bind.port);
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete Port ${bind.bind.port}?`,
          "This will remove the bind and all its listeners.",
          () => onDelete(bind.bind.port, basePath),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(bindPath);
      }}
    >
      <ResourceIcon
        icon={<Network />}
        color={getResourceColor("bind")}
        size="small"
      />
      <NodeLabel>Port {bind.bind.port}</NodeLabel>
      {bind.bind.tunnelProtocol && (
        <ProtocolTag protocol={bind.bind.tunnelProtocol} />
      )}
      <ValidationBadges errors={bind.validationErrors} />
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildListenerTitle(
  ln: ListenerNode,
  navigate: (path: string) => void,
  onDelete: (port: number, li: number, parentPath: string) => void,
  bindPort: number,
  listenerIndex: number,
  confirmDelete: ConfirmDeleteFn,
  onAddRoute: (port: number, li: number, isTcp: boolean) => void,
  basePath: string,
): ReactNode {
  const protocol = ln.listener.protocol ?? "HTTP";
  const listenerPath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}`;
  const bindPath = `${basePath}/bind/${bindPort}`;
  const isTcp = protocol === "TCP" || protocol === "TLS";

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(listenerPath + "?edit=true");
      },
    },
    {
      key: "addRoute",
      label: isTcp ? "Add TCP Route" : "Add Route",
      icon: <PlusOutlined />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddRoute(bindPort, listenerIndex, isTcp);
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete listener "${ln.listener.name ?? "unnamed"}"?`,
          "This will remove the listener and all its routes.",
          () => onDelete(bindPort, listenerIndex, bindPath),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(listenerPath);
      }}
    >
      <ResourceIcon
        icon={<Headphones />}
        color={getResourceColor("listener")}
        size="small"
      />
      <NodeLabel>{ln.listener.name ?? "(unnamed listener)"}</NodeLabel>
      <ProtocolTag protocol={protocol} />
      <ValidationBadges errors={ln.validationErrors} />
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildPolicyTitle(
  pn: PolicyNode,
  navigate: (path: string) => void,
  onDelete: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
    policyType: string,
    parentPath: string,
  ) => void,
  bindPort: number,
  listenerIndex: number,
  routeIndex: number,
  confirmDelete: ConfirmDeleteFn,
  basePath: string,
): ReactNode {
  const routeSeg = pn.isTcpRoute ? "tcproute" : "route";
  const policyPath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}/${routeSeg}/${routeIndex}/policy/${pn.policyType}`;
  const routePath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}/${routeSeg}/${routeIndex}`;

  // Format policy type for display (camelCase to Title Case)
  const displayName = pn.policyType
    .replace(/([A-Z])/g, " $1")
    .replace(/^./, (str) => str.toUpperCase())
    .trim();

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(policyPath + "?edit=true");
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete ${displayName} policy?`,
          "This cannot be undone.",
          () =>
            onDelete(
              bindPort,
              listenerIndex,
              routeIndex,
              pn.isTcpRoute,
              pn.policyType,
              routePath,
            ),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(policyPath);
      }}
    >
      <ResourceIcon
        icon={<Settings />}
        color={getResourceColor("policy")}
        size="small"
      />
      <NodeLabel>{displayName}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildMCPTargetPolicyTitle(
  policyType: string,
  policyData: unknown,
  targetIndex: number,
  navigate: (path: string) => void,
  onDelete: (targetIndex: number, policyType: string) => void,
  confirmDelete: ConfirmDeleteFn,
  basePath: string,
): ReactNode {
  const mcpBase = basePath.endsWith("/mcp") ? basePath : `${basePath}/mcp`;
  const policyPath = `${mcpBase}/target/${targetIndex}/policy/${policyType}`;
  const targetPath = `${mcpBase}/target/${targetIndex}`;
  const displayName = getPolicyLabel(policyType);

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(policyPath + "?edit=true");
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete ${displayName} policy?`,
          "This cannot be undone.",
          () => onDelete(targetIndex, policyType),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(policyPath);
      }}
    >
      <ResourceIcon
        icon={<Settings />}
        color={getResourceColor("policy")}
        size="small"
      />
      <NodeLabel>{displayName}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function describeBackend(backend: LocalRouteBackend | unknown): {
  label: string;
  detail: string;
} {
  if (typeof backend === "string") {
    return { label: "Ref", detail: backend };
  }

  const b = backend as Record<string, unknown>;
  if ("service" in b) {
    const svc = b.service as Record<string, unknown> | undefined;
    return { label: "Service", detail: String(svc?.name ?? "") };
  }
  if ("host" in b) {
    return { label: "Host", detail: String(b.host ?? "") };
  }
  if ("mcp" in b) {
    return { label: "MCP", detail: "" };
  }
  if ("ai" in b) {
    return { label: "AI", detail: "" };
  }

  return { label: "Backend", detail: "" };
}

function buildBackendTitle(
  bn: BackendNode,
  navigate: (path: string) => void,
  onDelete: (
    port: number,
    li: number,
    ri: number,
    bi: number,
    isTcp: boolean,
    parentPath: string,
  ) => void,
  bindPort: number,
  listenerIndex: number,
  routeIndex: number,
  confirmDelete: ConfirmDeleteFn,
  basePath: string,
): ReactNode {
  const { label } = describeBackend(bn.backend);
  const routeSeg = bn.isTcpRoute ? "tcproute" : "route";
  const backendPath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}/${routeSeg}/${routeIndex}/backend/${bn.backendIndex}`;
  const routePath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}/${routeSeg}/${routeIndex}`;

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(backendPath + "?edit=true");
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete backend ${bn.backendIndex + 1}?`,
          "This cannot be undone.",
          () =>
            onDelete(
              bindPort,
              listenerIndex,
              routeIndex,
              bn.backendIndex,
              bn.isTcpRoute,
              routePath,
            ),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(backendPath);
      }}
    >
      <ResourceIcon
        icon={<Server />}
        color={getResourceColor("backend")}
        size="small"
      />
      <NodeLabel>{label}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildModelTitle(
  mn: ModelNode,
  navigate: (path: string) => void,
  onDelete: (modelIndex: number) => void,
  confirmDelete: ConfirmDeleteFn,
  basePath: string,
): ReactNode {
  const modelName = mn.model.name || `Model ${mn.modelIndex + 1}`;
  const modelPath = `${basePath}/llm/model/${mn.modelIndex}`;

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(modelPath + "?edit=true");
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete model "${modelName}"?`,
          "This cannot be undone.",
          () => onDelete(mn.modelIndex),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(modelPath);
      }}
    >
      <ResourceIcon
        icon={<Bot />}
        color={getResourceColor("model")}
        size="small"
      />
      <NodeLabel>{modelName}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildRouteTitle(
  rn: RouteNode,
  navigate: (path: string) => void,
  onDelete: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
    parentPath: string,
  ) => void,
  bindPort: number,
  listenerIndex: number,
  confirmDelete: ConfirmDeleteFn,
  onAddRouteBackend: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
  ) => void,
  onAddRoutePolicy: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
    policyType: string,
  ) => void,
  basePath: string,
): ReactNode {
  const routeSeg = rn.isTcp ? "tcproute" : "route";
  const routePath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}/${routeSeg}/${rn.categoryIndex}`;
  const listenerPath = `${basePath}/bind/${bindPort}/listener/${listenerIndex}`;


  // Check which policy types already exist
  const existingPolicyTypes = new Set(rn.policies.map((p) => p.policyType));

  const routePolicyMenuItems: MenuProps["items"] = getPolicyTypesForScope("route").map((pt) => {
    const item: NonNullable<MenuProps["items"]>[number] = {
      key: `add-policy-${pt.key}`,
      label: pt.label,
      disabled: existingPolicyTypes.has(pt.key),
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddRoutePolicy(bindPort, listenerIndex, rn.categoryIndex, rn.isTcp, pt.key);
      },
    };
    return item;
  });

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(routePath + "?edit=true");
      },
    },
    {
      key: "addBackend",
      label: "Add Backend",
      icon: <PlusOutlined />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddRouteBackend(bindPort, listenerIndex, rn.categoryIndex, rn.isTcp);
      },
    },
    {
      key: "addPolicy",
      label: "Add Policy",
      icon: <PlusOutlined />,
      children: routePolicyMenuItems,
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete route "${rn.route.name ?? "unnamed"}"?`,
          "This will remove the route and all its backends.",
          () =>
            onDelete(
              bindPort,
              listenerIndex,
              rn.categoryIndex,
              rn.isTcp,
              listenerPath,
            ),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(routePath);
      }}
    >
      <ResourceIcon icon={<Route />} color={getResourceColor("route")} />
      <NodeLabel>{rn.route.name ?? "(unnamed route)"}</NodeLabel>
      {rn.isTcp && <ProtocolTag protocol="TCP" />}
      {rn.validationErrors.length > 0 && (
        <Tooltip
          title={rn.validationErrors.map((e) => e.message).join("\n")}
          styles={{ root: { whiteSpace: "pre-wrap" } }}
        >
          <TriangleAlert
            size={14}
            style={{
              color: rn.validationErrors.some((e) => e.level === "error")
                ? "var(--color-error)"
                : "var(--color-warning)",
              flexShrink: 0,
            }}
          />
        </Tooltip>
      )}
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

// ---------------------------------------------------------------------------
// Top-level item builders
// ---------------------------------------------------------------------------

function buildTopLevelItemTitle(
  label: string,
  icon: ReactNode,
  exists: boolean,
  onEdit: () => void,
  onDelete: () => void,
  confirmDelete: ConfirmDeleteFn,
  navigate: (path: string) => void,
  path: string,
): ReactNode {
  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onEdit();
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(`Delete ${label}?`, "This cannot be undone.", onDelete);
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(path);
      }}
    >
      {icon}
      <NodeLabel>{label}</NodeLabel>
      {exists && (
        <Dropdown
          menu={{ items: menuItems }}
          trigger={["click"]}
          placement="bottomRight"
          overlayClassName="hierarchy-menu"
        >
          <MoreButton
            type="text"
            size="small"
            icon={<MoreVertical size={14} />}
            onClick={(e) => e.stopPropagation()}
          />
        </Dropdown>
      )}
    </NodeRow>
  );
}

function buildTopLevelPolicyTitle(
  pn: LLMPolicyNode | MCPPolicyNode,
  scope: "llm" | "mcp",
  navigate: (path: string) => void,
  onDelete: (policyType: string, parentPath: string) => void,
  confirmDelete: ConfirmDeleteFn,
  basePath: string,
): ReactNode{
  const label = getPolicyLabel(pn.policyType);
  const parentPath = `${basePath}/${scope}`;
  const policyPath = `${parentPath}/policy/${pn.policyType}`;

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(policyPath + "?edit=true");
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete "${label}" policy?`,
          "This cannot be undone.",
          () => onDelete(pn.policyType, parentPath),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(policyPath);
      }}
    >
      <ResourceIcon
        icon={<Settings />}
        color={getResourceColor("frontendPolicies")}
        size="small"
      />
      <NodeLabel>{label}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildLLMItemTitle(
  label: string,
  icon: ReactNode,
  onEdit: () => void,
  onDelete: () => void,
  onAddModel: () => void,
  onAddPolicy: (policyType: string) => void,
  existingPolicyTypes: Set<string>,
  confirmDelete: ConfirmDeleteFn,
  navigate: (path: string) => void,
  path: string,
): ReactNode {
  const policyMenuItems: MenuProps["items"] = getPolicyTypesForScope("llm").map((pt) => {
    const item: NonNullable<MenuProps["items"]>[number] = {
      key: pt.key,
      label: pt.label,
      disabled: existingPolicyTypes.has(pt.key),
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddPolicy(pt.key);
      },
    };
    return item;
  });

  const menuItems: MenuProps["items"] = [
    {
      key: "add-model",
      label: "Add Model",
      icon: <PlusOutlined />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddModel();
      },
    },
    {
      key: "add-policy",
      label: "Add Policy",
      icon: <PlusOutlined />,
      children: policyMenuItems,
    },
    { type: "divider" },
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onEdit();
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(`Delete ${label}?`, "This cannot be undone.", onDelete);
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(path);
      }}
    >
      {icon}
      <NodeLabel>{label}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildMCPTargetTitle(
  tn: MCPTargetNode,
  navigate: (path: string) => void,
  onDelete: (targetIndex: number) => void,
  onAddPolicy: (targetIndex: number, policyType: string) => void,
  existingPolicies: Set<string>,
  confirmDelete: ConfirmDeleteFn,
  basePath: string,
): ReactNode {
  const targetName = tn.target.name || `Target ${tn.targetIndex + 1}`;
  const mcpBase = basePath.endsWith("/mcp") ? basePath : `${basePath}/mcp`;
  const targetPath = `${mcpBase}/target/${tn.targetIndex}`;

  // Build submenu for available policy types
  const availablePolicyTypes = getPolicyTypesForScope("mcpTarget").filter(
    (pt) => !existingPolicies.has(pt.key),
  );
  const policySubmenu: MenuProps["items"] = availablePolicyTypes.map((pt) => ({
    key: pt.key,
    label: pt.label,
    onClick: ({ domEvent }) => {
      domEvent.stopPropagation();
      onAddPolicy(tn.targetIndex, pt.key);
    },
  }));

  const menuItems: MenuProps["items"] = [
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        navigate(targetPath + "?edit=true");
      },
    },
    {
      key: "add-policy",
      label: "Add Policy",
      icon: <PlusOutlined />,
      disabled: policySubmenu.length === 0,
      children: policySubmenu.length > 0 ? policySubmenu : undefined,
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(
          `Delete target "${targetName}"?`,
          "This cannot be undone.",
          () => onDelete(tn.targetIndex),
        );
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(targetPath);
      }}
    >
      <ResourceIcon
        icon={<Server />}
        color={getResourceColor("mcp")}
        size="small"
      />
      <NodeLabel>{targetName}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

function buildMCPItemTitle(
  label: string,
  icon: ReactNode,
  onEdit: () => void,
  onDelete: () => void,
  onAddTarget: () => void,
  onAddPolicy: (policyType: string) => void,
  existingPolicyTypes: Set<string>,
  confirmDelete: ConfirmDeleteFn,
  navigate: (path: string) => void,
  path: string,
): ReactNode {
  const policyMenuItems: MenuProps["items"] = getPolicyTypesForScope("mcp").map((pt) => {
    const item: NonNullable<MenuProps["items"]>[number] = {
      key: pt.key,
      label: pt.label,
      disabled: existingPolicyTypes.has(pt.key),
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddPolicy(pt.key);
      },
    };
    return item;
  });

  const menuItems: MenuProps["items"] = [
    {
      key: "add-target",
      label: "Add Target",
      icon: <PlusOutlined />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onAddTarget();
      },
    },
    {
      key: "add-policy",
      label: "Add Policy",
      icon: <PlusOutlined />,
      children: policyMenuItems,
    },
    { type: "divider" },
    {
      key: "edit",
      label: "Edit",
      icon: <Pencil size={13} />,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        onEdit();
      },
    },
    { type: "divider" },
    {
      key: "delete",
      label: "Delete",
      icon: <DeleteOutlined />,
      danger: true,
      onClick: ({ domEvent }) => {
        domEvent.stopPropagation();
        confirmDelete(`Delete ${label}?`, "This cannot be undone.", onDelete);
      },
    },
  ];

  return (
    <NodeRow
      onClick={(e) => {
        e.stopPropagation();
        navigate(path);
      }}
    >
      {icon}
      <NodeLabel>{label}</NodeLabel>
      <Dropdown
        menu={{ items: menuItems }}
        trigger={["click"]}
        placement="bottomRight"
        overlayClassName="hierarchy-menu"
      >
        <MoreButton
          type="text"
          size="small"
          icon={<MoreVertical size={14} />}
          onClick={(e) => e.stopPropagation()}
        />
      </Dropdown>
    </NodeRow>
  );
}

// ---------------------------------------------------------------------------
// Tree data builder
// ---------------------------------------------------------------------------

function buildTreeData(
  hierarchy: TrafficHierarchy,
  basePath: string,
  filter: HierarchySection[] | undefined,
  navigate: (path: string) => void,
  onDeleteBind: (port: number, parentPath: string) => void,
  onDeleteListener: (port: number, li: number, parentPath: string) => void,
  onDeleteRoute: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
    parentPath: string,
  ) => void,
  onDeleteBackend: (
    port: number,
    li: number,
    ri: number,
    bi: number,
    isTcp: boolean,
    parentPath: string,
  ) => void,
  onDeleteRoutePolicy: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
    policyType: string,
    parentPath: string,
  ) => void,
  confirmDelete: ConfirmDeleteFn,
  onAddListener: (port: number) => void,
  onAddRoute: (port: number, li: number, isTcp: boolean) => void,
  onAddRouteBackend: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
  ) => void,
  onAddRoutePolicy: (
    port: number,
    li: number,
    ri: number,
    isTcp: boolean,
    policyType: string,
  ) => void,
  onEditLLM: () => void,
  onDeleteLLM: () => void,
  onAddModel: () => void,
  onDeleteModel: (modelIndex: number) => void,
  onAddLLMPolicy: (policyType: string) => void,
  onDeleteLLMPolicy: (policyType: string, parentPath: string) => void,
  onEditMCP: () => void,
  onDeleteMCP: () => void,
  onAddMCPTarget: () => void,
  onDeleteMCPTarget: (targetIndex: number) => void,
  onAddMCPTargetPolicy: (targetIndex: number, policyType: string) => void,
  onDeleteMCPTargetPolicy: (targetIndex: number, policyType: string) => void,
  onAddMCPPolicy: (policyType: string) => void,
  onDeleteMCPPolicy: (policyType: string, parentPath: string) => void,
  onEditFrontendPolicies: () => void,
  onDeleteFrontendPolicies: () => void,
): DataNode[] {
  const showLLM = !filter || filter.includes("llm");
  const showMCP = !filter || filter.includes("mcp");
  const showFrontendPolicies = !filter || filter.includes("frontendPolicies");
  const showBinds = !filter || filter.includes("binds");

  const nodes: DataNode[] = [];

  // Add top-level config items
  if (hierarchy.llm && showLLM) {
    const modelChildren: DataNode[] = hierarchy.llm.models
      .slice()
      .sort((a, b) => {
        const nameA = a.model.name || "";
        const nameB = b.model.name || "";
        return nameA.localeCompare(nameB);
      })
      .map((model) => ({
        key: `model-${model.modelIndex}`,
        title: buildModelTitle(model, navigate, onDeleteModel, confirmDelete, basePath),
        selectable: true,
      }));

    const llmPolicyChildren: DataNode[] = hierarchy.llm.policies
      .slice()
      .sort((a, b) => a.policyType.localeCompare(b.policyType))
      .map((pn) => ({
        key: `llm-policy-${pn.policyType}`,
        title: buildTopLevelPolicyTitle(pn, "llm", navigate, onDeleteLLMPolicy, confirmDelete, basePath),
        selectable: true,
      }));

    const llmChildren = [...modelChildren, ...llmPolicyChildren];
    const existingLLMPolicies = new Set(hierarchy.llm.policies.map((p) => p.policyType));

    nodes.push({
      key: "llm",
      title: buildLLMItemTitle(
        "LLM Configuration",
        <ResourceIcon icon={<Bot />} color={getResourceColor("llm")} />,
        onEditLLM,
        onDeleteLLM,
        onAddModel,
        onAddLLMPolicy,
        existingLLMPolicies,
        confirmDelete,
        navigate,
        basePath.endsWith("/llm") ? basePath : `${basePath}/llm`,
      ),
      selectable: true,
      children: llmChildren.length > 0 ? llmChildren : undefined,
    });
  }

  if (hierarchy.mcp && showMCP) {
    const mcpTargetChildren: DataNode[] = hierarchy.mcp.targets
      .slice()
      .sort((a, b) => {
        const nameA = a.target.name || "";
        const nameB = b.target.name || "";
        return nameA.localeCompare(nameB);
      })
      .map((target) => {
        // Build policy children for this target
        const targetPolicyChildren: DataNode[] = target.policies
          .slice()
          .sort((a, b) => a.policyType.localeCompare(b.policyType))
          .map((policy) => ({
            key: `mcp-target-${target.targetIndex}-policy-${policy.policyType}`,
            title: buildMCPTargetPolicyTitle(
              policy.policyType,
              policy.policy,
              target.targetIndex,
              navigate,
              onDeleteMCPTargetPolicy,
              confirmDelete,
              basePath,
            ),
            selectable: true,
          }));

        const existingTargetPolicies = new Set(target.policies.map((p) => p.policyType));

        return {
          key: `mcp-target-${target.targetIndex}`,
          title: buildMCPTargetTitle(
            target,
            navigate,
            onDeleteMCPTarget,
            onAddMCPTargetPolicy,
            existingTargetPolicies,
            confirmDelete,
            basePath,
          ),
          selectable: true,
          children: targetPolicyChildren.length > 0 ? targetPolicyChildren : undefined,
        };
      });

    const mcpPolicyChildren: DataNode[] = hierarchy.mcp.policies
      .slice()
      .sort((a, b) => a.policyType.localeCompare(b.policyType))
      .map((pn) => ({
        key: `mcp-policy-${pn.policyType}`,
        title: buildTopLevelPolicyTitle(pn, "mcp", navigate, onDeleteMCPPolicy, confirmDelete, basePath),
        selectable: true,
      }));

    const mcpChildren = [...mcpTargetChildren, ...mcpPolicyChildren];
    const existingMCPPolicies = new Set(hierarchy.mcp.policies.map((p) => p.policyType));

    nodes.push({
      key: "mcp",
      title: buildMCPItemTitle(
        "MCP Configuration",
        <ResourceIcon icon={<Headphones />} color={getResourceColor("mcp")} />,
        onEditMCP,
        onDeleteMCP,
        onAddMCPTarget,
        onAddMCPPolicy,
        existingMCPPolicies,
        confirmDelete,
        navigate,
        basePath.endsWith("/mcp") ? basePath : `${basePath}/mcp`,
      ),
      selectable: true,
      children: mcpChildren.length > 0 ? mcpChildren : undefined,
    });
  }

  if (hierarchy.frontendPolicies && showFrontendPolicies) {
    nodes.push({
      key: "frontendPolicies",
      title: buildTopLevelItemTitle(
        "Frontend Policies",
        <ResourceIcon
          icon={<Settings />}
          color={getResourceColor("frontendPolicies")}
        />,
        true,
        onEditFrontendPolicies,
        onDeleteFrontendPolicies,
        confirmDelete,
        navigate,
        `${basePath}/frontendPolicies`,
      ),
      selectable: true,
    });
  }

  // Add binds (sorted by port)
  const bindNodes = (showBinds ? hierarchy.binds
    .slice()
    .sort((a, b) => a.bind.port - b.bind.port)
    .map((bind) => ({
      key: `bind-${bind.bind.port}`,
      title: buildBindTitle(
        bind,
        navigate,
        onDeleteBind,
        confirmDelete,
        onAddListener,
        basePath,
      ),
      selectable: true,
      children: bind.listeners
        .slice()
        .sort((a, b) => {
          const nameA = a.listener.name || "";
          const nameB = b.listener.name || "";
          return nameA.localeCompare(nameB);
        })
        .map((listener, li) => ({
          key: `listener-${bind.bind.port}-${li}`,
          title: buildListenerTitle(
            listener,
            navigate,
            onDeleteListener,
            bind.bind.port,
            li,
            confirmDelete,
            onAddRoute,
            basePath,
          ),
          selectable: true,
          children: listener.routes
            .slice()
            .sort((a, b) => {
              const nameA = (a.route.name as string | undefined) || "";
              const nameB = (b.route.name as string | undefined) || "";
              return nameA.localeCompare(nameB);
            })
            .map((route) => {
              const children: DataNode[] = [];

              // Add backends (sorted by name)
              const sortedBackends = route.backends.slice().sort((a, b) => {
                const nameA = (a.backend as any)?.name || "";
                const nameB = (b.backend as any)?.name || "";
                return nameA.localeCompare(nameB);
              });

              sortedBackends.forEach((backend) => {
                children.push({
                  key: `backend-${bind.bind.port}-${li}-${route.categoryIndex}-${backend.backendIndex}`,
                  title: buildBackendTitle(
                    backend,
                    navigate,
                    onDeleteBackend,
                    bind.bind.port,
                    li,
                    route.categoryIndex,
                    confirmDelete,
                    basePath,
                  ),
                  selectable: true,
                });
              });

              // Add policies (sorted by policy type)
              const sortedPolicies = route.policies
                .slice()
                .sort((a, b) => a.policyType.localeCompare(b.policyType));

              sortedPolicies.forEach((policy) => {
                children.push({
                  key: `policy-${bind.bind.port}-${li}-${route.categoryIndex}-${policy.policyType}`,
                  title: buildPolicyTitle(
                    policy,
                    navigate,
                    onDeleteRoutePolicy,
                    bind.bind.port,
                    li,
                    route.categoryIndex,
                    confirmDelete,
                    basePath,
                  ),
                  selectable: true,
                });
              });

              return {
                key: `route-${bind.bind.port}-${li}-${route.isTcp ? "tcp" : "http"}-${route.categoryIndex}`,
                title: buildRouteTitle(
                  route,
                  navigate,
                  onDeleteRoute,
                  bind.bind.port,
                  li,
                  confirmDelete,
                  onAddRouteBackend,
                  onAddRoutePolicy,
                  basePath,
                ),
                selectable: true,
                children,
              };
            }),
        })),
    })) : []);

  return [...nodes, ...bindNodes];
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export interface AddRootHandlers {
  addBind: () => void;
  addLLM: () => void;
  addMCP: () => void;
}

interface HierarchyTreeProps {
  hierarchy: TrafficHierarchy;
  filter?: HierarchySection[];
  title?: string;
  onRegisterAddHandlers?: (handlers: AddRootHandlers) => void;
}

export function HierarchyTree({ hierarchy, filter, title, onRegisterAddHandlers }: HierarchyTreeProps) {
  const navigate = useNavigate();
  const location = useLocation();
  const basePath = useMemo(() => {
    return `/${location.pathname.split('/').at(1) ?? ''}`;
  }, [location.pathname]);
  const { modal } = App.useApp();
  const { mutate } = useConfig();

  // Define confirmDelete first
  const confirmDelete = useCallback<ConfirmDeleteFn>(
    (title, description, onConfirm) => {
      modal.confirm({
        title,
        content: description,
        okText: "Delete",
        okButtonProps: { danger: true },
        onOk: onConfirm,
      });
    },
    [modal],
  );

  const handleDeleteBind = useCallback(
    async (port: number, parentPath: string) => {
      try {
        await api.removeBind(port);
        toast.success(`Port ${port} deleted`);
        mutate();
        navigate(parentPath);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to delete bind"));
      }
    },
    [mutate, navigate],
  );

  const handleDeleteListener = useCallback(
    async (port: number, li: number, _parentPath: string) => {
      try {
        await api.removeListenerByIndex(port, li);
        toast.success("Listener deleted");
        mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(
          getErrorMessage(e, "Failed to delete listener"),
        );
      }
    },
    [basePath, mutate, navigate],
  );

  const handleDeleteRoute = useCallback(
    async (
      port: number,
      li: number,
      ri: number,
      isTcp: boolean,
      _parentPath: string,
    ) => {
      try {
        if (isTcp) {
          await api.removeTCPRouteByIndex(port, li, ri);
        } else {
          await api.removeRouteByIndex(port, li, ri);
        }
        toast.success("Route deleted");
        mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to delete route"));
      }
    },
    [basePath, mutate, navigate],
  );

  const handleDeleteBackend = useCallback(
    async (
      port: number,
      li: number,
      ri: number,
      bi: number,
      isTcp: boolean,
      _parentPath: string,
    ) => {
      try {
        if (isTcp) {
          await api.removeTCPRouteBackendByIndex(port, li, ri, bi);
        } else {
          await api.removeRouteBackendByIndex(port, li, ri, bi);
        }
        toast.success("Backend deleted");
        mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(
          getErrorMessage(e, "Failed to delete backend"),
        );
      }
    },
    [basePath, mutate, navigate],
  );

  // Handler for adding a new listener to a bind
  const handleAddListener = useCallback(
    async (port: number) => {
      try {
        const newListener = {
          name: "listener",
          protocol: "HTTP" as const,
          hostname: "*",
          routes: [],
        };
        await api.createListener(port, newListener);
        toast.success("Listener created successfully");
        ensureExpanded(`bind-${port}`);

        // Wait for config to refresh and get the updated data
        const freshConfig = await mutate();

        // Find the index of the newly created listener from fresh data
        const bind = freshConfig?.binds?.find((b: any) => b.port === port);
        const listenerIndex = bind ? bind.listeners.length - 1 : 0;
        navigate(
          `${basePath}/bind/${port}/listener/${listenerIndex}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(
          getErrorMessage(e, "Failed to create listener"),
        );
      }
    },
    [basePath, mutate, navigate],
  );

  // Handler for adding a new route to a listener
  const handleAddRoute = useCallback(
    async (port: number, li: number, isTcp: boolean) => {
      try {
        // Get the current listener to determine the route count
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) {
          throw new Error(`Bind with port ${port} not found`);
        }

        const listener = bind.bind.listeners[li];
        if (!listener) {
          throw new Error(`Listener at index ${li} not found`);
        }

        // Calculate the route index BEFORE we modify anything
        // This will be the index of the newly created route
        const routeIndex = isTcp
          ? (listener.tcpRoutes?.length ?? 0)
          : (listener.routes?.length ?? 0);

        const newRoute = {
          name: "route",
          hostnames: [],
          matches: [{ path: { pathPrefix: "/" } }],
          backends: [],
        };

        // Create updated listener with only the appropriate route array
        // Backend doesn't allow both routes and tcpRoutes to be set
        const updatedListener: any = { ...listener };

        if (isTcp) {
          // Only include tcpRoutes
          updatedListener.tcpRoutes = listener.tcpRoutes
            ? [...listener.tcpRoutes, newRoute]
            : [newRoute];
          // Remove routes field if it exists
          delete updatedListener.routes;
        } else {
          // Only include routes
          updatedListener.routes = listener.routes
            ? [...listener.routes, newRoute]
            : [newRoute];
          // Remove tcpRoutes field if it exists
          delete updatedListener.tcpRoutes;
        }

        // Update the listener
        await api.updateListenerByIndex(port, li, updatedListener);

        toast.success(
          isTcp
            ? "TCP route created successfully"
            : "Route created successfully",
        );
        ensureExpanded(`bind-${port}`, `listener-${port}-${li}`);

        // Navigate to the newly created route
        const routeSeg = isTcp ? "tcproute" : "route";

        // Navigate immediately with the creating flag to trigger polling
        navigate(
          `${basePath}/bind/${port}/listener/${li}/${routeSeg}/${routeIndex}?edit=true&creating=true`,
        );

        // Trigger a background refresh (don't await)
        mutate();
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to create route"));
      }
    },
    [basePath, hierarchy.binds, mutate, navigate],
  );

  // Handler for adding a new backend to a route
  const handleAddRouteBackend = useCallback(
    async (port: number, li: number, ri: number, isTcp: boolean) => {
      try {
        // Get the current route
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) {
          throw new Error(`Bind with port ${port} not found`);
        }

        const listener = bind.bind.listeners[li];
        if (!listener) {
          throw new Error(`Listener at index ${li} not found`);
        }

        const route = isTcp ? listener.tcpRoutes?.[ri] : listener.routes?.[ri];

        if (!route) {
          throw new Error(`Route at index ${ri} not found`);
        }

        // Create new backend WITHOUT backendType - that's a UI-only field
        // The backend needs the actual backend type field (service, host, etc.)
        // Note: service.name must be a string in format "namespace/hostname"
        const newBackend = {
          service: {
            name: {
              namespace: "default",
              hostname: "service",
            },
            port: 8080,
          },
          weight: 1,
        };

        // Construct a clean route object with only the necessary fields
        // This avoids sending any extra hierarchy metadata to the API
        const existingBackends = route.backends || [];
        const updatedBackends = [...existingBackends, newBackend];

        const updatedRoute: Record<string, unknown> = {
          hostnames: route.hostnames,
          backends: updatedBackends,
        };

        // Add optional fields if they exist
        if ("matches" in route) updatedRoute.matches = route.matches;
        if ("name" in route) updatedRoute.name = route.name;
        if ("namespace" in route) updatedRoute.namespace = route.namespace;
        if ("ruleName" in route) updatedRoute.ruleName = route.ruleName;
        if ("policies" in route) updatedRoute.policies = route.policies;

        // Update the route
        if (isTcp) {
          await api.updateTCPRouteByIndex(port, li, ri, updatedRoute);
        } else {
          await api.updateRouteByIndex(port, li, ri, updatedRoute);
        }

        toast.success("Backend created successfully");
        const routeSeg = isTcp ? "tcproute" : "route";
        ensureExpanded(
          `bind-${port}`,
          `listener-${port}-${li}`,
          `route-${port}-${li}-${isTcp ? "tcp" : "http"}-${ri}`,
        );

        // Wait for config to refresh and get the updated data
        const freshConfig = await mutate();

        // Find the index from fresh data
        const freshBind = freshConfig?.binds?.find((b: any) => b.port === port);
        const freshListener = freshBind?.listeners?.[li];
        const freshRoute = isTcp
          ? freshListener?.tcpRoutes?.[ri]
          : freshListener?.routes?.[ri];
        const backendIndex = (freshRoute?.backends?.length ?? 1) - 1;
        navigate(
          `${basePath}/bind/${port}/listener/${li}/${routeSeg}/${ri}/backend/${backendIndex}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(
          getErrorMessage(e, "Failed to create backend"),
        );
      }
    },
    [basePath, hierarchy.binds, mutate, navigate],
  );

  // Handler for adding a specific policy type to a route
  const handleAddRoutePolicy = useCallback(
    async (
      port: number,
      li: number,
      ri: number,
      isTcp: boolean,
      policyType: string,
    ) => {
      try {
        // Get the current route
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) {
          throw new Error(`Bind with port ${port} not found`);
        }

        const listener = bind.bind.listeners[li];
        if (!listener) {
          throw new Error(`Listener at index ${li} not found`);
        }

        const route = isTcp ? listener.tcpRoutes?.[ri] : listener.routes?.[ri];

        if (!route) {
          throw new Error(`Route at index ${ri} not found`);
        }

        // Get existing policies object or create new one
        const currentPolicies =
          route.policies && typeof route.policies === "object"
            ? route.policies
            : {};

        // Create a minimal policy config for the specific type
        const newPolicyConfig = {};

        // Update the route with the new policy type
        const updatedRoute = {
          ...route,
          policies: {
            ...currentPolicies,
            [policyType]: newPolicyConfig,
          },
        };

        // Update the route
        if (isTcp) {
          await api.updateTCPRouteByIndex(port, li, ri, updatedRoute as any);
        } else {
          await api.updateRouteByIndex(port, li, ri, updatedRoute as any);
        }

        // Format policy type for display
        const displayName = policyType
          .replace(/([A-Z])/g, " $1")
          .replace(/^./, (str) => str.toUpperCase())
          .trim();

        toast.success(`${displayName} policy created successfully`);
        ensureExpanded(
          `bind-${port}`,
          `listener-${port}-${li}`,
          `route-${port}-${li}-${isTcp ? "tcp" : "http"}-${ri}`,
        );

        // Wait for config to refresh
        await mutate();

        // Navigate to the policy edit page
        const routeSeg = isTcp ? "tcproute" : "route";
        navigate(
          `${basePath}/bind/${port}/listener/${li}/${routeSeg}/${ri}/policy/${policyType}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to create policy"));
      }
    },
    [basePath, hierarchy.binds, mutate, navigate],
  );

  // Handler for deleting a specific policy type from a route
  const handleDeleteRoutePolicy = useCallback(
    async (
      port: number,
      li: number,
      ri: number,
      isTcp: boolean,
      policyType: string,
      _parentPath: string,
    ) => {
      try {
        // Get the current route
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) {
          throw new Error(`Bind with port ${port} not found`);
        }

        const listener = bind.bind.listeners[li];
        if (!listener) {
          throw new Error(`Listener at index ${li} not found`);
        }

        const route = isTcp ? listener.tcpRoutes?.[ri] : listener.routes?.[ri];

        if (!route) {
          throw new Error(`Route at index ${ri} not found`);
        }

        // Get existing policies
        const currentPolicies =
          route.policies && typeof route.policies === "object"
            ? route.policies
            : {};

        // Remove the specific policy type
        const { [policyType]: _removed, ...remainingPolicies } =
          currentPolicies as Record<string, unknown>;

        // Update the route with remaining policies (or null if none left)
        const updatedRoute = {
          ...route,
          policies:
            Object.keys(remainingPolicies).length > 0
              ? remainingPolicies
              : null,
        };

        // Update the route
        if (isTcp) {
          await api.updateTCPRouteByIndex(port, li, ri, updatedRoute as any);
        } else {
          await api.updateRouteByIndex(port, li, ri, updatedRoute as any);
        }

        // Format policy type for display
        const displayName = policyType
          .replace(/([A-Z])/g, " $1")
          .replace(/^./, (str) => str.toUpperCase())
          .trim();

        toast.success(`${displayName} policy deleted`);
        mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to delete policy"));
      }
    },
    [basePath, hierarchy.binds, mutate, navigate],
  );

  // Handlers for editing top-level items
  const handleEditLLM = useCallback(() => {
    navigate(`${basePath}/llm?edit=true`);
  }, [basePath, navigate]);

  const handleEditMCP = useCallback(() => {
    navigate(`${basePath}/mcp?edit=true`);
  }, [basePath, navigate]);

  const handleEditFrontendPolicies = useCallback(() => {
    navigate(`${basePath}/frontendPolicies?edit=true`);
  }, [basePath, navigate]);

  // Handlers for deleting top-level items
  const handleDeleteLLM = useCallback(async () => {
    try {
      await api.deleteLLM();
      toast.success("LLM configuration deleted");
      mutate();
      navigate(basePath);
    } catch (e: unknown) {
      toast.error(
        getErrorMessage(e, "Failed to delete LLM config"),
      );
    }
  }, [basePath, mutate, navigate]);

  const handleAddModel = useCallback(async () => {
    try {
      const newModel = {
        name: `model-${(hierarchy.llm?.models.length ?? 0) + 1}`,
        provider: "openAI",
      };
      await api.createLLMModel(newModel);
      toast.success("Model created successfully");
      ensureExpanded("llm");

      // Wait for config to refresh and get the updated data
      const freshConfig = await mutate();

      // Find the index of the newly created model from fresh data
      const llmConfig = freshConfig?.llm as any;
      const newIndex = llmConfig?.models ? llmConfig.models.length - 1 : 0;
      const llmBase = basePath.endsWith("/llm") ? basePath : `${basePath}/llm`;
      navigate(`${llmBase}/model/${newIndex}?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to create model"));
    }
  }, [basePath, hierarchy.llm?.models.length, mutate, navigate]);

  const handleDeleteModel = useCallback(
    async (modelIndex: number) => {
      try {
        await api.removeLLMModelByIndex(modelIndex);
        toast.success("Model deleted successfully");
        await mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to delete model"));
      }
    },
    [basePath, mutate, navigate],
  );

  const handleDeleteMCP = useCallback(async () => {
    try {
      await api.deleteMCP();
      toast.success("MCP configuration deleted");
      mutate();
      navigate(basePath);
    } catch (e: unknown) {
      toast.error(
        getErrorMessage(e, "Failed to delete MCP config"),
      );
    }
  }, [basePath, mutate, navigate]);

  const handleAddMCPTarget = useCallback(async () => {
    try {
      const newTarget = {
        name: `target-${(hierarchy.mcp?.targets.length ?? 0) + 1}`,
        sse: {
          host: "localhost",
          port: 8080,
          path: "/sse",
        },
      };
      await api.createMCPTarget(newTarget);
      toast.success("Target created successfully");
      ensureExpanded("mcp");

      // Wait for config to refresh and get the updated data
      const freshConfig = await mutate();

      // Find the index of the newly created target from fresh data
      const mcpConfig = freshConfig?.mcp as any;
      const newIndex = mcpConfig?.targets ? mcpConfig.targets.length - 1 : 0;
      const mcpBase = basePath.endsWith("/mcp") ? basePath : `${basePath}/mcp`;
      navigate(`${mcpBase}/target/${newIndex}?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to create target"));
    }
  }, [basePath, hierarchy.mcp?.targets.length, mutate, navigate]);

  const handleDeleteMCPTarget = useCallback(
    async (targetIndex: number) => {
      try {
        await api.removeMCPTargetByIndex(targetIndex);
        toast.success("Target deleted successfully");
        await mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to delete target"));
      }
    },
    [basePath, mutate, navigate],
  );

  const handleAddMCPTargetPolicy = useCallback(
    async (targetIndex: number, policyType: string) => {
      try {
        await api.updateMCPTargetPolicy(targetIndex, policyType, getDefaultPolicyValue(policyType));
        toast.success(`${getPolicyLabel(policyType)} policy added`);
        ensureExpanded("mcp", `mcp-target-${targetIndex}`);
        await mutate();
        const mcpBase = basePath.endsWith("/mcp") ? basePath : `${basePath}/mcp`;
        navigate(`${mcpBase}/target/${targetIndex}/policy/${policyType}?edit=true&creating=true`);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to add policy"));
      }
    },
    [basePath, mutate, navigate],
  );

  const handleDeleteMCPTargetPolicy = useCallback(
    async (targetIndex: number, policyType: string) => {
      try {
        await api.removeMCPTargetPolicy(targetIndex, policyType);
        toast.success(`${getPolicyLabel(policyType)} policy deleted`);
        await mutate();
        navigate(basePath);
      } catch (e: unknown) {
        toast.error(getErrorMessage(e, "Failed to delete policy"));
      }
    },
    [basePath, mutate, navigate],
  );

  const handleDeleteFrontendPolicies = useCallback(async () => {
    try {
      await api.deleteFrontendPolicies();
      toast.success("Frontend policies deleted");
      mutate();
      navigate(basePath);
    } catch (e: unknown) {
      toast.error(
        getErrorMessage(e, "Failed to delete frontend policies"),
      );
    }
  }, [basePath, mutate, navigate]);

  const handleAddLLMPolicy = useCallback(async (policyType: string) => {
    try {
      if (!hierarchy.llm) return;
      const currentPolicies = hierarchy.llm.policies.reduce((acc, p) => {
        acc[p.policyType] = p.policy;
        return acc;
      }, {} as Record<string, unknown>);
      const updatedConfig = {
        ...hierarchy.llm.config,
        models: hierarchy.llm.models.map((m) => m.model),
        policies: { ...currentPolicies, [policyType]: getDefaultPolicyValue(policyType) },
      };
      await api.createOrUpdateLLM(updatedConfig as any);
      toast.success(`${getPolicyLabel(policyType)} policy added`);
      ensureExpanded("llm");
      await mutate();
      const llmBase = basePath.endsWith("/llm") ? basePath : `${basePath}/llm`;
      navigate(`${llmBase}/policy/${policyType}?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to add policy"));
    }
  }, [basePath, hierarchy.llm, mutate, navigate]);

  const handleDeleteLLMPolicy = useCallback(async (policyType: string, _parentPath: string) => {
    try {
      if (!hierarchy.llm) return;
      const currentPolicies = hierarchy.llm.policies.reduce((acc, p) => {
        acc[p.policyType] = p.policy;
        return acc;
      }, {} as Record<string, unknown>);
      delete currentPolicies[policyType];
      const updatedConfig = {
        ...hierarchy.llm.config,
        models: hierarchy.llm.models.map((m) => m.model),
        policies: Object.keys(currentPolicies).length > 0 ? currentPolicies : null,
      };
      await api.createOrUpdateLLM(updatedConfig as any);
      toast.success(`${getPolicyLabel(policyType)} policy deleted`);
      mutate();
      navigate(basePath);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to delete policy"));
    }
  }, [basePath, hierarchy.llm, mutate, navigate]);

  const handleAddMCPPolicy = useCallback(async (policyType: string) => {
    try {
      if (!hierarchy.mcp) return;
      const currentPolicies = hierarchy.mcp.policies.reduce((acc, p) => {
        acc[p.policyType] = p.policy;
        return acc;
      }, {} as Record<string, unknown>);
      const updatedConfig = {
        ...hierarchy.mcp.config,
        targets: hierarchy.mcp.targets.map((t) => t.target),
        policies: { ...currentPolicies, [policyType]: getDefaultPolicyValue(policyType) },
      };
      await api.createOrUpdateMCP(updatedConfig as any);
      toast.success(`${getPolicyLabel(policyType)} policy added`);
      ensureExpanded("mcp");
      await mutate();
      const mcpBase = basePath.endsWith("/mcp") ? basePath : `${basePath}/mcp`;
      navigate(`${mcpBase}/policy/${policyType}?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to add policy"));
    }
  }, [basePath, hierarchy.mcp, mutate, navigate]);

  const handleDeleteMCPPolicy = useCallback(async (policyType: string, _parentPath: string) => {
    try {
      if (!hierarchy.mcp) return;
      const currentPolicies = hierarchy.mcp.policies.reduce((acc, p) => {
        acc[p.policyType] = p.policy;
        return acc;
      }, {} as Record<string, unknown>);
      delete currentPolicies[policyType];
      const updatedConfig = {
        ...hierarchy.mcp.config,
        targets: hierarchy.mcp.targets.map((t) => t.target),
        policies: Object.keys(currentPolicies).length > 0 ? currentPolicies : null,
      };
      await api.createOrUpdateMCP(updatedConfig as any);
      toast.success(`${getPolicyLabel(policyType)} policy deleted`);
      mutate();
      navigate(basePath);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to delete policy"));
    }
  }, [basePath, hierarchy.mcp, mutate, navigate]);

  // Helper to collect all keys from tree data
  const getAllKeys = useCallback((nodes: DataNode[]): Key[] => {
    const keys: Key[] = [];
    const collectKeys = (nodes: DataNode[]) => {
      for (const node of nodes) {
        keys.push(node.key);
        if (node.children) collectKeys(node.children);
      }
    };
    collectKeys(nodes);
    return keys;
  }, []);

  // Build tree data after all handlers are defined
  const treeData = useMemo(() => {
    return buildTreeData(
      hierarchy,
      basePath,
      filter,
      navigate,
      handleDeleteBind,
      handleDeleteListener,
      handleDeleteRoute,
      handleDeleteBackend,
      handleDeleteRoutePolicy,
      confirmDelete,
      handleAddListener,
      handleAddRoute,
      handleAddRouteBackend,
      handleAddRoutePolicy,
      handleEditLLM,
      handleDeleteLLM,
      handleAddModel,
      handleDeleteModel,
      handleAddLLMPolicy,
      handleDeleteLLMPolicy,
      handleEditMCP,
      handleDeleteMCP,
      handleAddMCPTarget,
      handleDeleteMCPTarget,
      handleAddMCPTargetPolicy,
      handleDeleteMCPTargetPolicy,
      handleAddMCPPolicy,
      handleDeleteMCPPolicy,
      handleEditFrontendPolicies,
      handleDeleteFrontendPolicies,
    );
  }, [
    hierarchy,
    basePath,
    filter,
    navigate,
    handleDeleteBind,
    handleDeleteListener,
    handleDeleteRoute,
    handleDeleteBackend,
    handleDeleteRoutePolicy,
    confirmDelete,
    handleAddListener,
    handleAddRoute,
    handleAddRouteBackend,
    handleAddRoutePolicy,
    handleEditLLM,
    handleDeleteLLM,
    handleAddModel,
    handleDeleteModel,
    handleAddLLMPolicy,
    handleDeleteLLMPolicy,
    handleEditMCP,
    handleDeleteMCP,
    handleAddMCPTarget,
    handleDeleteMCPTarget,
    handleAddMCPTargetPolicy,
    handleDeleteMCPTargetPolicy,
    handleAddMCPPolicy,
    handleDeleteMCPPolicy,
    handleEditFrontendPolicies,
    handleDeleteFrontendPolicies,
  ]);

  // Initialize with all keys expanded
  const [expandedKeys, setExpandedKeys] = useState<Key[]>(() =>
    getAllKeys(treeData),
  );

  /** Ensure one or more tree keys are expanded (used after adding child items) */
  const ensureExpanded = useCallback((...keys: Key[]) => {
    setExpandedKeys((prev) => {
      const set = new Set(prev);
      let changed = false;
      for (const k of keys) {
        if (!set.has(k)) { set.add(k); changed = true; }
      }
      return changed ? Array.from(set) : prev;
    });
  }, []);

  const selectedKeys = useMemo(() => {
    const key = urlToSelectedKey(location.pathname, basePath);
    return key ? [key] : [];
  }, [location.pathname, basePath]);

  // Auto-expand ancestors of the selected node so it's always visible
  useEffect(() => {
    if (selectedKeys.length === 0) return;
    const key = selectedKeys[0] as string;
    const ancestors: Key[] = [];
    let m;
    if ((m = key.match(/^(?:backend|policy)-(\d+)-(\d+)-(\d+)/))) {
      // Need to find whether it's tcp or http to build the route key — check both
      ancestors.push(`bind-${m[1]}`, `listener-${m[1]}-${m[2]}`);
      // Expand both possible route keys (only the existing one will matter)
      ancestors.push(`route-${m[1]}-${m[2]}-http-${m[3]}`, `route-${m[1]}-${m[2]}-tcp-${m[3]}`);
    } else if ((m = key.match(/^route-(\d+)-(\d+)/))) {
      ancestors.push(`bind-${m[1]}`, `listener-${m[1]}-${m[2]}`);
    } else if ((m = key.match(/^listener-(\d+)/))) {
      ancestors.push(`bind-${m[1]}`);
    } else if (key.startsWith("model-") || key.startsWith("llm-policy-")) {
      ancestors.push("llm");
    } else if ((m = key.match(/^mcp-target-(\d+)-policy-/))) {
      ancestors.push("mcp", `mcp-target-${m[1]}`);
    } else if (key.startsWith("mcp-target-") || key.startsWith("mcp-policy-")) {
      ancestors.push("mcp");
    }
    if (ancestors.length > 0) {
      ensureExpanded(...ancestors);
    }
  }, [selectedKeys, ensureExpanded]);

  const handleExpandAll = useCallback(() => {
    setExpandedKeys(getAllKeys(treeData));
  }, [treeData, getAllKeys]);

  const handleCollapseAll = useCallback(() => {
    setExpandedKeys([]);
  }, []);

  // Handler for adding a new bind
  const handleAddBind = useCallback(async () => {
    try {
      const existingPorts = new Set(hierarchy.binds.map((b) => b.bind.port));
      let newPort = 8080;
      while (existingPorts.has(newPort)) {
        newPort++;
      }
      const newBind = {
        port: newPort,
        tunnelProtocol: "direct" as const,
        listeners: [],
      };
      await api.createBind(newBind);
      toast.success("Bind created successfully");
      mutate();
      navigate(`${basePath}/bind/${newPort}?edit=true`);
    } catch (e: unknown) {
      toast.error(getErrorMessage(e, "Failed to create bind"));
    }
  }, [basePath, hierarchy.binds, mutate, navigate]);

  // Handlers for creating top-level items
  const handleAddLLM = useCallback(async () => {
    try {
      const newLLM = {
        models: [],
      };
      await api.createOrUpdateLLM(newLLM);
      toast.success("LLM configuration created successfully");
      await mutate();
      navigate(`${basePath}/llm?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(
        getErrorMessage(e, "Failed to create LLM config"),
      );
    }
  }, [basePath, mutate, navigate]);

  const handleAddMCP = useCallback(async () => {
    try {
      const newMCP = {
        targets: [],
      };
      await api.createOrUpdateMCP(newMCP);
      toast.success("MCP configuration created successfully");
      await mutate();
      navigate(`${basePath}/mcp?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(
        getErrorMessage(e, "Failed to create MCP config"),
      );
    }
  }, [basePath, mutate, navigate]);

  const handleAddFrontendPolicies = useCallback(async () => {
    try {
      const newFrontendPolicies = {};
      await api.createOrUpdateFrontendPolicies(newFrontendPolicies);
      toast.success("Frontend policies created successfully");
      await mutate();
      navigate(`${basePath}/frontendPolicies?edit=true&creating=true`);
    } catch (e: unknown) {
      toast.error(
        getErrorMessage(e, "Failed to create frontend policies"),
      );
    }
  }, [basePath, mutate, navigate]);

  // Register add handlers with parent via stable refs
  const handleAddBindRef = useRef(handleAddBind);
  handleAddBindRef.current = handleAddBind;
  const handleAddLLMRef = useRef(handleAddLLM);
  handleAddLLMRef.current = handleAddLLM;
  const handleAddMCPRef = useRef(handleAddMCP);
  handleAddMCPRef.current = handleAddMCP;

  useEffect(() => {
    if (!onRegisterAddHandlers) return;
    onRegisterAddHandlers({
      addBind: () => handleAddBindRef.current(),
      addLLM: () => handleAddLLMRef.current(),
      addMCP: () => handleAddMCPRef.current(),
    });
  }, [onRegisterAddHandlers]);

  // Dropdown menu items for adding top-level resources — filtered by section
  const showBinds = !filter || filter.includes("binds");
  const showLLMSection = !filter || filter.includes("llm");
  const showMCPSection = !filter || filter.includes("mcp");
  const showFrontendPoliciesSection = !filter || filter.includes("frontendPolicies");
  const addMenuItems: MenuProps["items"] = [
    ...(showBinds ? [{ key: "bind", label: "Bind", icon: <Network size={14} />, onClick: handleAddBind } as const] : []),
    ...(showBinds && (showLLMSection || showMCPSection || showFrontendPoliciesSection) ? [{ type: "divider" as const }] : []),
    ...(showLLMSection ? [{ key: "llm", label: "LLM Config", icon: <Bot size={14} />, onClick: handleAddLLM, disabled: !!hierarchy.llm } as const] : []),
    ...(showMCPSection ? [{ key: "mcp", label: "MCP Config", icon: <Headphones size={14} />, onClick: handleAddMCP, disabled: !!hierarchy.mcp } as const] : []),
    ...(showFrontendPoliciesSection ? [{ key: "frontendPolicies", label: "Frontend Policies", icon: <Settings size={14} />, onClick: handleAddFrontendPolicies } as const] : []),
  ];

  // Determine if filtered tree has any content
  const hasFilteredContent =
    (showBinds && hierarchy.binds.length > 0) ||
    (showLLMSection && hierarchy.llm !== null) ||
    (showMCPSection && hierarchy.mcp !== null) ||
    (showFrontendPoliciesSection && hierarchy.frontendPolicies !== null);

  if (!hasFilteredContent) {
    return (
      <TreeCard>
        <Empty description="No resources configured" image={Empty.PRESENTED_IMAGE_SIMPLE}>
          <Dropdown menu={{ items: addMenuItems }} trigger={["click"]}>
            <Button type="primary" icon={<PlusOutlined />}>
              Add <DownOutlined />
            </Button>
          </Dropdown>
        </Empty>
      </TreeCard>
    );
  }

  const cardTitle = title ?? "Traffic Configuration";

  return (
    <>
      <Global
        styles={css`
          .hierarchy-menu {
            .ant-dropdown-menu {
              min-width: 160px;
            }
          }
          .ant-dropdown-menu-sub {
            max-height: 50vh;
            overflow-y: auto;
          }
        `}
      />
      <TreeCard
        title={
          <CardHeader>
            <CardTitleRow>
              <CardTitle onClick={() => navigate(basePath)}>
                {cardTitle}
              </CardTitle>
              <Space size="small">
                <Dropdown menu={{ items: addMenuItems }} trigger={["click"]}>
                  <Button type="primary" size="small" icon={<PlusOutlined />}>
                    Add <DownOutlined />
                  </Button>
                </Dropdown>
                <Tooltip
                  title={
                    expandedKeys.length > 0 ? "Collapse All" : "Expand All"
                  }
                >
                  <Button
                    type="text"
                    size="small"
                    icon={
                      expandedKeys.length > 0 ? (
                        <ChevronsDownUp size={16} />
                      ) : (
                        <ChevronsUpDown size={16} />
                      )
                    }
                    onClick={
                      expandedKeys.length > 0
                        ? handleCollapseAll
                        : handleExpandAll
                    }
                  />
                </Tooltip>
              </Space>
            </CardTitleRow>
          </CardHeader>
        }
      >
        <Tree
          treeData={treeData}
          expandedKeys={expandedKeys}
          selectedKeys={selectedKeys}
          onExpand={(keys) => setExpandedKeys(keys)}
          blockNode
          showLine={false}
          showIcon={false}
        />
      </TreeCard>
    </>
  );
}
