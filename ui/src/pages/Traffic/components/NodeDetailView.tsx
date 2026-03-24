import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import Form from "@rjsf/antd";
import type { MenuProps } from "antd";
import { Breadcrumb, Button, Dropdown, Popconfirm, Space, Spin } from "antd";
import { Edit2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import toast from "react-hot-toast";
import { useLocation, useNavigate } from "react-router-dom";
import { useConfig } from "../../../api";
import * as api from "../../../api/crud";
import {
  ArrayFieldTemplate,
  CollapsibleObjectFieldTemplate,
  FieldTemplate,
  WrapIfAdditionalTemplate,
} from "../../../components/FormTemplates";
import { ProtocolTag } from "../../../components/ProtocolTag";
import { StyledAlert } from "../../../components/StyledAlert";
import { validator } from "../../../utils/validator";
import { forms } from "../forms";
import { useNodePolling } from "../hooks/useNodePolling";
import type {
  BackendNode,
  BindNode,
  ListenerNode,
  ModelNode,
  PolicyNode,
  RouteNode,
  useTrafficHierarchy,
} from "../hooks/useTrafficHierarchy";
import type { UrlParams } from "../TrafficPage";

// ---------------------------------------------------------------------------
// Form templates configuration
// ---------------------------------------------------------------------------

const formTemplates = {
  ObjectFieldTemplate: CollapsibleObjectFieldTemplate,
  FieldTemplate,
  ArrayFieldTemplate,
  WrapIfAdditionalTemplate,
};

// ---------------------------------------------------------------------------
// Styled components
// ---------------------------------------------------------------------------

const Container = styled.div`
  padding: var(--spacing-xl);
  min-height: 100%;
`;

const Header = styled.div`
  margin-bottom: var(--spacing-xl);
  padding-bottom: var(--spacing-lg);
`;

const BreadcrumbWrapper = styled.div`
  margin-bottom: var(--spacing-md);

  .ant-breadcrumb {
    font-size: 13px;
  }

  .ant-breadcrumb-link {
    color: var(--color-text-secondary);
    cursor: pointer;

    &:hover {
      color: var(--color-primary);
    }
  }

  .ant-breadcrumb-separator {
    color: var(--color-text-tertiary);
  }
`;

const TitleRow = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-xs);
  justify-content: space-between;
`;

const TitleLeft = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
`;

const Title = styled.h2`
  margin: 0;
  font-size: 20px;
  font-weight: 600;
  color: var(--color-text-base);
`;

const TypeBadge = styled.span`
  display: inline-flex;
  align-items: center;
  padding: 3px 10px;
  height: 22px;
  font-size: 11px;
  font-weight: 600;
  line-height: 22px;
  white-space: nowrap;
  border-radius: 5px;
  border: 1px solid rgba(255, 255, 255, 0.15);
  background: rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.75);
  letter-spacing: 0.3px;
  text-transform: uppercase;

  [data-theme="light"] & {
    background: rgba(0, 0, 0, 0.04);
    border-color: rgba(0, 0, 0, 0.12);
    color: rgba(0, 0, 0, 0.65);
  }
`;

const Description = styled.p`
  margin: 0;
  color: var(--color-text-secondary);
  font-size: 14px;
`;

const SectionTitle = styled.div`
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--color-text-secondary);
  margin-bottom: var(--spacing-lg);
`;

const FormWrapper = styled.div`
  /* Style for individual form fields (not the top-level container) */
  .field-object .ant-form-item,
  fieldset .ant-form-item,
  .field-array > div {
    background: rgba(0, 0, 0, 0.15);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    padding: var(--spacing-lg);
    margin-bottom: var(--spacing-md);

    [data-theme="light"] & {
      background: rgba(0, 0, 0, 0.02);
      border-color: rgba(0, 0, 0, 0.1);
    }

    &:last-child {
      margin-bottom: 0;
    }
  }

  /* Nested fields with slightly darker background for better contrast */
  .ant-form-item .ant-form-item .ant-form-item,
  .field-object .field-object .ant-form-item {
    background: rgba(0, 0, 0, 0.2);
    border-color: rgba(255, 255, 255, 0.12);

    [data-theme="light"] & {
      background: rgba(0, 0, 0, 0.04);
      border-color: rgba(0, 0, 0, 0.12);
    }
  }

  /* Field labels */
  legend,
  .ant-form-item-label label {
    font-weight: 600;
    color: var(--color-text-base);
  }

  /* Placeholder text with better contrast */
  input::placeholder,
  textarea::placeholder {
    color: rgba(255, 255, 255, 0.45);
    opacity: 1;

    [data-theme="light"] & {
      color: rgba(0, 0, 0, 0.45);
    }
  }

  /* Ant Design specific placeholder styling */
  .ant-input::placeholder,
  .ant-input-textarea::placeholder {
    color: rgba(255, 255, 255, 0.45);

    [data-theme="light"] & {
      color: rgba(0, 0, 0, 0.45);
    }
  }
`;

// ---------------------------------------------------------------------------
// Type guards to determine node type
// ---------------------------------------------------------------------------

function findSelectedNode(
  hierarchy: ReturnType<typeof useTrafficHierarchy>,
  urlParams: UrlParams,
):
  | { type: "bind"; node: BindNode }
  | { type: "listener"; node: ListenerNode; bind: BindNode }
  | { type: "route"; node: RouteNode; listener: ListenerNode; bind: BindNode }
  | {
      type: "backend";
      node: BackendNode;
      route: RouteNode;
      listener: ListenerNode;
      bind: BindNode;
    }
  | {
      type: "policy";
      node: PolicyNode;
      route: RouteNode;
      listener: ListenerNode;
      bind: BindNode;
    }
  | { type: "llm"; data: unknown }
  | { type: "model"; node: ModelNode }
  | { type: "mcp"; data: unknown }
  | { type: "frontendPolicies"; data: unknown }
  | null {
  // Handle model routes first
  if (urlParams.modelIndex !== undefined) {
    if (!hierarchy.llm) return null;
    const modelNode = hierarchy.llm.models[urlParams.modelIndex];
    if (!modelNode) return null;
    return { type: "model", node: modelNode };
  }

  // Handle top-level types
  if (urlParams.topLevelType) {
    switch (urlParams.topLevelType) {
      case "llm":
        // Return just the config part (without models - they're managed separately)
        return hierarchy.llm
          ? { type: "llm", data: hierarchy.llm.config }
          : null;
      case "mcp":
        return hierarchy.mcp ? { type: "mcp", data: hierarchy.mcp } : null;
      case "frontendPolicies":
        return hierarchy.frontendPolicies
          ? { type: "frontendPolicies", data: hierarchy.frontendPolicies }
          : null;
    }
  }

  const { port, li, ri, bi, isTcpRoute, policyType } = urlParams;
  if (port === undefined) return null;
  const bindNode = hierarchy.binds.find((b) => b.bind.port === port);
  if (!bindNode) return null;

  if (li === undefined) {
    return { type: "bind", node: bindNode };
  }

  const listenerNode = bindNode.listeners[li];
  if (!listenerNode) return null;

  if (ri === undefined) {
    return { type: "listener", node: listenerNode, bind: bindNode };
  }

  const routeNode = listenerNode.routes.find(
    (rn) => rn.isTcp === isTcpRoute && rn.categoryIndex === ri,
  );
  if (!routeNode) return null;

  if (bi === undefined && !policyType) {
    return {
      type: "route",
      node: routeNode,
      listener: listenerNode,
      bind: bindNode,
    };
  }

  if (policyType) {
    const policyNode = routeNode.policies.find(
      (p) => p.policyType === policyType,
    );
    if (!policyNode) return null;
    return {
      type: "policy",
      node: policyNode,
      route: routeNode,
      listener: listenerNode,
      bind: bindNode,
    };
  }

  const backendNode = routeNode.backends[bi!];
  if (!backendNode) return null;

  return {
    type: "backend",
    node: backendNode,
    route: routeNode,
    listener: listenerNode,
    bind: bindNode,
  };
}

// ---------------------------------------------------------------------------
// Breadcrumb generation helper
// ---------------------------------------------------------------------------

function generateBreadcrumbItems(
  selected: ReturnType<typeof findSelectedNode>,
  navigate: (path: string) => void,
) {
  if (!selected) return [];

  const items: { title: React.ReactNode; onClick?: () => void }[] = [
    {
      title: "Traffic",
      onClick: () => navigate("/traffic"),
    },
  ];

  if (selected.type === "bind") {
    items.push({ title: `Port ${selected.node.bind.port}` });
  } else if (selected.type === "listener") {
    items.push({
      title: `Port ${selected.bind.bind.port}`,
      onClick: () => navigate(`/traffic/bind/${selected.bind.bind.port}`),
    });
    items.push({
      title: selected.node.listener.name ?? "(unnamed listener)",
    });
  } else if (selected.type === "route") {
    const port = selected.bind.bind.port;
    const li = selected.listener.listenerIndex;
    items.push({
      title: `Port ${port}`,
      onClick: () => navigate(`/traffic/bind/${port}`),
    });
    items.push({
      title: selected.listener.listener.name ?? "(unnamed listener)",
      onClick: () => navigate(`/traffic/bind/${port}/listener/${li}`),
    });
    items.push({
      title:
        (selected.node.route.name as string | undefined) ?? "(unnamed route)",
    });
  } else if (selected.type === "backend") {
    const port = selected.bind.bind.port;
    const li = selected.listener.listenerIndex;
    const ri = selected.route.categoryIndex;
    const routeType = selected.route.isTcp ? "tcp" : "";
    items.push({
      title: `Port ${port}`,
      onClick: () => navigate(`/traffic/bind/${port}`),
    });
    items.push({
      title: selected.listener.listener.name ?? "(unnamed listener)",
      onClick: () => navigate(`/traffic/bind/${port}/listener/${li}`),
    });
    items.push({
      title:
        (selected.route.route.name as string | undefined) ?? "(unnamed route)",
      onClick: () =>
        navigate(
          `/traffic/bind/${port}/listener/${li}/${routeType}route/${ri}`,
        ),
    });
    items.push({
      title: (selected.node.backend as any)?.name ?? "(unnamed backend)",
    });
  } else if (selected.type === "policy") {
    const port = selected.bind.bind.port;
    const li = selected.listener.listenerIndex;
    const ri = selected.route.categoryIndex;
    const routeType = selected.route.isTcp ? "tcp" : "";
    items.push({
      title: `Port ${port}`,
      onClick: () => navigate(`/traffic/bind/${port}`),
    });
    items.push({
      title: selected.listener.listener.name ?? "(unnamed listener)",
      onClick: () => navigate(`/traffic/bind/${port}/listener/${li}`),
    });
    items.push({
      title:
        (selected.route.route.name as string | undefined) ?? "(unnamed route)",
      onClick: () =>
        navigate(
          `/traffic/bind/${port}/listener/${li}/${routeType}route/${ri}`,
        ),
    });
    items.push({
      title: `${selected.node.policyType} Policy`,
    });
  } else if (selected.type === "llm") {
    items.push({ title: "LLM Configuration" });
  } else if (selected.type === "model") {
    items.push({
      title: "LLM Configuration",
      onClick: () => navigate("/traffic/llm"),
    });
    items.push({
      title: selected.node.model.name ?? `Model ${selected.node.modelIndex}`,
    });
  } else if (selected.type === "mcp") {
    items.push({ title: "MCP Configuration" });
  } else if (selected.type === "frontendPolicies") {
    items.push({ title: "Frontend Policies" });
  }

  return items;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface NodeDetailViewProps {
  hierarchy: ReturnType<typeof useTrafficHierarchy>;
  urlParams: UrlParams;
}

export function NodeDetailView({ hierarchy, urlParams }: NodeDetailViewProps) {
  const navigate = useNavigate();
  const location = useLocation();
  const { mutate } = useConfig();

  // Edit mode is URL-driven
  const searchParams = new URLSearchParams(location.search);
  const isEditing = searchParams.get("edit") === "true";
  const isCreating = searchParams.get("creating") === "true";

  const [formData, setFormData] = useState<Record<string, unknown> | null>(
    null,
  );
  const [saving, setSaving] = useState(false);

  // Poll for the node if we're in creating mode
  const { isPolling, hasTimedOut } = useNodePolling(
    hierarchy,
    urlParams,
    isCreating,
  );

  // Action handlers for creating sub-resources - defined early to satisfy hooks rules
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
        const freshConfig = await mutate();
        const bind = freshConfig?.binds?.find((b: any) => b.port === port);
        const listenerIndex = bind ? bind.listeners.length - 1 : 0;
        navigate(
          `/traffic/bind/${port}/listener/${listenerIndex}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(
          e instanceof Error ? e.message : "Failed to create listener",
        );
      }
    },
    [mutate, navigate],
  );

  const handleAddRoute = useCallback(
    async (port: number, li: number, isTcp: boolean) => {
      try {
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) throw new Error(`Bind with port ${port} not found`);
        const listener = bind.bind.listeners[li];
        if (!listener) throw new Error(`Listener at index ${li} not found`);

        const newRoute = {
          name: "route",
          hostnames: [],
          matches: [{ path: { pathPrefix: "/" } }],
          backends: [],
        };

        const updatedListener = { ...listener };
        if (isTcp) {
          if (!updatedListener.tcpRoutes) updatedListener.tcpRoutes = [];
          updatedListener.tcpRoutes.push(newRoute as any);
        } else {
          if (!updatedListener.routes) updatedListener.routes = [];
          updatedListener.routes.push(newRoute as any);
        }

        await api.updateListenerByIndex(port, li, updatedListener);
        toast.success(
          isTcp
            ? "TCP route created successfully"
            : "Route created successfully",
        );

        const freshConfig = await mutate();
        const freshBind = freshConfig?.binds?.find((b: any) => b.port === port);
        const freshListener = freshBind?.listeners?.[li];
        const routeIndex = isTcp
          ? (freshListener?.tcpRoutes?.length ?? 1) - 1
          : (freshListener?.routes?.length ?? 1) - 1;
        const routeSeg = isTcp ? "tcproute" : "route";
        navigate(
          `/traffic/bind/${port}/listener/${li}/${routeSeg}/${routeIndex}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(e instanceof Error ? e.message : "Failed to create route");
      }
    },
    [hierarchy.binds, mutate, navigate],
  );

  const handleAddBackend = useCallback(
    async (port: number, li: number, ri: number, isTcp: boolean) => {
      try {
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) throw new Error(`Bind with port ${port} not found`);
        const listener = bind.bind.listeners[li];
        if (!listener) throw new Error(`Listener at index ${li} not found`);
        const route = isTcp ? listener.tcpRoutes?.[ri] : listener.routes?.[ri];
        if (!route) throw new Error(`Route at index ${ri} not found`);

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

        const existingBackends = route.backends || [];
        const updatedBackends = [...existingBackends, newBackend];

        const updatedRoute: Record<string, unknown> = {
          hostnames: route.hostnames,
          backends: updatedBackends,
        };

        if ("matches" in route) updatedRoute.matches = route.matches;
        if ("name" in route) updatedRoute.name = route.name;
        if ("namespace" in route) updatedRoute.namespace = route.namespace;
        if ("ruleName" in route) updatedRoute.ruleName = route.ruleName;
        if ("policies" in route) updatedRoute.policies = route.policies;

        if (isTcp) {
          await api.updateTCPRouteByIndex(port, li, ri, updatedRoute);
        } else {
          await api.updateRouteByIndex(port, li, ri, updatedRoute);
        }

        toast.success("Backend created successfully");

        const freshConfig = await mutate();
        const freshBind = freshConfig?.binds?.find((b: any) => b.port === port);
        const freshListener = freshBind?.listeners?.[li];
        const freshRoute = isTcp
          ? freshListener?.tcpRoutes?.[ri]
          : freshListener?.routes?.[ri];
        const backendIndex = (freshRoute?.backends?.length ?? 1) - 1;
        const routeSeg = isTcp ? "tcproute" : "route";
        navigate(
          `/traffic/bind/${port}/listener/${li}/${routeSeg}/${ri}/backend/${backendIndex}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(
          e instanceof Error ? e.message : "Failed to create backend",
        );
      }
    },
    [hierarchy.binds, mutate, navigate],
  );

  const handleAddPolicy = useCallback(
    async (
      port: number,
      li: number,
      ri: number,
      isTcp: boolean,
      policyType: string,
    ) => {
      try {
        const bind = hierarchy.binds.find((b) => b.bind.port === port);
        if (!bind) throw new Error(`Bind with port ${port} not found`);
        const listener = bind.bind.listeners[li];
        if (!listener) throw new Error(`Listener at index ${li} not found`);
        const route = isTcp ? listener.tcpRoutes?.[ri] : listener.routes?.[ri];
        if (!route) throw new Error(`Route at index ${ri} not found`);

        const currentPolicies =
          route.policies && typeof route.policies === "object"
            ? route.policies
            : {};
        const newPolicyConfig = {};

        const updatedRoute = {
          ...route,
          policies: {
            ...currentPolicies,
            [policyType]: newPolicyConfig,
          },
        };

        if (isTcp) {
          await api.updateTCPRouteByIndex(port, li, ri, updatedRoute as any);
        } else {
          await api.updateRouteByIndex(port, li, ri, updatedRoute as any);
        }

        const displayName = policyType
          .replace(/([A-Z])/g, " $1")
          .replace(/^./, (str) => str.toUpperCase())
          .trim();

        toast.success(`${displayName} policy created successfully`);
        await mutate();

        const routeSeg = isTcp ? "tcproute" : "route";
        navigate(
          `/traffic/bind/${port}/listener/${li}/${routeSeg}/${ri}/policy/${policyType}?edit=true&creating=true`,
        );
      } catch (e: unknown) {
        toast.error(e instanceof Error ? e.message : "Failed to create policy");
      }
    },
    [hierarchy.binds, mutate, navigate],
  );

  const selected = useMemo(() => {
    const sel = findSelectedNode(hierarchy, urlParams);
    // Initialize formData when selection changes
    if (sel) {
      if (sel.type === "bind") {
        setFormData(sel.node.bind as unknown as Record<string, unknown>);
      } else if (sel.type === "listener") {
        // Filter out routes and tcpRoutes - they're managed separately via the tree
        const { routes, tcpRoutes, ...listenerData } = sel.node
          .listener as Record<string, unknown>;
        setFormData(listenerData);
      } else if (sel.type === "route") {
        // Filter out backends - they're managed separately via the tree
        const { backends, ...routeData } = sel.node.route as Record<
          string,
          unknown
        >;
        setFormData(routeData);
      } else if (sel.type === "backend") {
        // Transform backend data from API format to form format
        const form = forms.backend as any;
        const backendData = sel.node.backend as Record<string, unknown>;
        const transformedData = form?.transformForForm
          ? form.transformForForm(backendData)
          : backendData;
        setFormData(transformedData as Record<string, unknown>);
      } else if (sel.type === "policy") {
        setFormData(sel.node.policy as Record<string, unknown>);
      } else if (sel.type === "model") {
        setFormData(sel.node.model as unknown as Record<string, unknown>);
      } else if (
        sel.type === "llm" ||
        sel.type === "mcp" ||
        sel.type === "frontendPolicies"
      ) {
        setFormData(sel.data as Record<string, unknown>);
      }
    }
    return sel;
  }, [hierarchy, urlParams]);

  // Reset form data when exiting edit mode
  useEffect(() => {
    if (!isEditing && selected) {
      // Reset to original data from the node
      if (selected.type === "bind") {
        setFormData(selected.node.bind as unknown as Record<string, unknown>);
      } else if (selected.type === "listener") {
        // Filter out routes and tcpRoutes - they're managed separately via the tree
        const { routes, tcpRoutes, ...listenerData } = selected.node
          .listener as Record<string, unknown>;
        setFormData(listenerData);
      } else if (selected.type === "route") {
        // Filter out backends - they're managed separately via the tree
        const { backends, ...routeData } = selected.node.route as Record<
          string,
          unknown
        >;
        setFormData(routeData);
      } else if (selected.type === "backend") {
        // Transform backend data from API format to form format
        const form = forms.backend as any;
        const backendData = selected.node.backend as Record<string, unknown>;
        const transformedData = form?.transformForForm
          ? form.transformForForm(backendData)
          : backendData;
        setFormData(transformedData as Record<string, unknown>);
      } else if (selected.type === "policy") {
        setFormData(selected.node.policy as Record<string, unknown>);
      } else if (selected.type === "model") {
        setFormData(selected.node.model as unknown as Record<string, unknown>);
      } else if (
        selected.type === "llm" ||
        selected.type === "mcp" ||
        selected.type === "frontendPolicies"
      ) {
        setFormData(selected.data as Record<string, unknown>);
      }
    }
  }, [isEditing, selected]);

  if (hierarchy.isLoading || isPolling) {
    return (
      <Container>
        <div style={{ textAlign: "center", padding: 50 }}>
          <Spin size="large" />
          {isPolling && (
            <p style={{ marginTop: 16, color: "var(--color-text-secondary)" }}>
              Loading node...
            </p>
          )}
        </div>
      </Container>
    );
  }

  if (!selected) {
    return (
      <Container>
        <StyledAlert
          type="warning"
          message="Node Not Found"
          description={
            hasTimedOut
              ? "The node was created but could not be loaded in time. Please refresh the page or navigate to it from the tree."
              : "The selected node could not be found in the current configuration."
          }
          showIcon
        />
      </Container>
    );
  }

  const handleSave = async (fd: Record<string, unknown>) => {
    setSaving(true);
    try {
      // Top-level types are handled separately in their own render sections
      if (
        selected.type === "llm" ||
        selected.type === "mcp" ||
        selected.type === "frontendPolicies"
      ) {
        return;
      }

      // Handle model updates
      if (selected.type === "model") {
        await api.updateLLMModelByIndex(selected.node.modelIndex, fd);
        toast.success("Model updated successfully");
        await mutate();
        navigate(`/traffic/llm/model/${selected.node.modelIndex}`);
        return;
      }

      const { port, li, ri, bi, isTcpRoute } = urlParams;

      // Guard: these types require a port
      if (port === undefined) {
        throw new Error("Port is required for this operation");
      }

      // Apply form-specific transformation if available
      const form = forms[selected.type] as any;
      let dataToSave = fd;
      if (form?.transformBeforeSubmit) {
        dataToSave = form.transformBeforeSubmit(fd) as Record<string, unknown>;
      }

      if (selected.type === "bind") {
        await api.updateBind(port, dataToSave);
      } else if (selected.type === "listener") {
        // Merge back the routes/tcpRoutes that we filtered out from the form
        const listenerWithRoutes = {
          ...dataToSave,
          routes: selected.node.listener.routes,
          tcpRoutes: selected.node.listener.tcpRoutes,
        };
        await api.updateListenerByIndex(port, li!, listenerWithRoutes);
      } else if (selected.type === "route") {
        // Merge back the backends that we filtered out from the form
        const routeWithBackends = {
          ...dataToSave,
          backends: selected.node.route.backends,
        };
        if (isTcpRoute) {
          await api.updateTCPRouteByIndex(port, li!, ri!, routeWithBackends);
        } else {
          await api.updateRouteByIndex(port, li!, ri!, routeWithBackends);
        }
      } else if (selected.type === "backend") {
        if (isTcpRoute) {
          await api.updateTCPRouteBackendByIndex(
            port,
            li!,
            ri!,
            bi!,
            dataToSave,
          );
        } else {
          await api.updateRouteBackendByIndex(port, li!, ri!, bi!, dataToSave);
        }
      } else if (selected.type === "policy") {
        // For policy, update the specific policy type in the parent route's policies field
        const currentPolicies =
          selected.route.route.policies &&
          typeof selected.route.route.policies === "object"
            ? selected.route.route.policies
            : {};

        const updatedRoute = {
          ...selected.route.route,
          policies: {
            ...currentPolicies,
            [selected.node.policyType]: dataToSave,
          },
        };
        if (isTcpRoute) {
          await api.updateTCPRouteByIndex(port, li!, ri!, updatedRoute as any);
        } else {
          await api.updateRouteByIndex(port, li!, ri!, updatedRoute as any);
        }
      }

      toast.success(
        `${selected.type.charAt(0).toUpperCase() + selected.type.slice(1)} updated successfully`,
      );
      mutate();
      navigate(location.pathname); // Remove ?edit=true
    } catch (e: unknown) {
      const errorMessage =
        e &&
        typeof e === "object" &&
        "message" in e &&
        typeof e.message === "string"
          ? e.message
          : "Failed to save changes";
      toast.error(errorMessage);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    setSaving(true);
    try {
      // Top-level types are handled separately in their own render sections
      if (
        selected.type === "llm" ||
        selected.type === "mcp" ||
        selected.type === "frontendPolicies"
      ) {
        return;
      }

      // Handle model deletion
      if (selected.type === "model") {
        await api.removeLLMModelByIndex(selected.node.modelIndex);
        toast.success("Model deleted successfully");
        await mutate();
        navigate("/traffic/llm");
        return;
      }

      const { port, li, ri, bi, isTcpRoute } = urlParams;

      // Guard: these types require a port
      if (port === undefined) {
        throw new Error("Port is required for this operation");
      }

      if (selected.type === "bind") {
        await api.removeBind(port);
        navigate("/traffic");
      } else if (selected.type === "listener") {
        await api.removeListenerByIndex(port, li!);
        navigate(`/traffic/bind/${port}`);
      } else if (selected.type === "route") {
        if (isTcpRoute) {
          await api.removeTCPRouteByIndex(port, li!, ri!);
        } else {
          await api.removeRouteByIndex(port, li!, ri!);
        }
        navigate(`/traffic/bind/${port}/listener/${li}`);
      } else if (selected.type === "backend") {
        if (isTcpRoute) {
          await api.removeTCPRouteBackendByIndex(port, li!, ri!, bi!);
        } else {
          await api.removeRouteBackendByIndex(port, li!, ri!, bi!);
        }
        navigate(
          `/traffic/bind/${port}/listener/${li}/${isTcpRoute ? "tcproute" : "route"}/${ri}`,
        );
      } else if (selected.type === "policy") {
        // For policy, remove the specific policy type from the parent route
        const currentPolicies =
          selected.route.route.policies &&
          typeof selected.route.route.policies === "object"
            ? selected.route.route.policies
            : {};

        const { [selected.node.policyType]: removed, ...remainingPolicies } =
          currentPolicies as Record<string, unknown>;

        const updatedRoute = {
          ...selected.route.route,
          policies:
            Object.keys(remainingPolicies).length > 0
              ? remainingPolicies
              : null,
        };
        if (isTcpRoute) {
          await api.updateTCPRouteByIndex(port, li!, ri!, updatedRoute as any);
        } else {
          await api.updateRouteByIndex(port, li!, ri!, updatedRoute as any);
        }
        navigate(
          `/traffic/bind/${port}/listener/${li}/${isTcpRoute ? "tcproute" : "route"}/${ri}`,
        );
      }

      toast.success(
        `${selected.type.charAt(0).toUpperCase() + selected.type.slice(1)} deleted`,
      );
      mutate();
    } catch (e: unknown) {
      const errorMessage =
        e &&
        typeof e === "object" &&
        "message" in e &&
        typeof e.message === "string"
          ? e.message
          : "Failed to delete";
      toast.error(errorMessage);
    } finally {
      setSaving(false);
    }
  };

  const handleSubmit = ({
    formData: fd,
  }: {
    formData?: Record<string, unknown>;
  }) => {
    if (fd) {
      handleSave(fd);
    }
  };

  const handleError = (errors: any) => {
    // Show first validation error in toast
    if (errors && errors.length > 0) {
      const firstError = errors[0];
      const errorMessage =
        firstError.stack || firstError.message || "Validation error";
      toast.error(errorMessage);
    }
  };

  // Render based on node type
  if (selected.type === "bind") {
    const { node } = selected;
    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);
    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}Port {node.bind.port}
              </Title>
              <TypeBadge>Bind</TypeBadge>
            </TitleLeft>
            {!isEditing && (
              <Space>
                <Button
                  icon={<PlusOutlined />}
                  onClick={() => handleAddListener(node.bind.port)}
                >
                  Add Listener
                </Button>
                <Button
                  type="primary"
                  icon={<Edit2 size={14} />}
                  onClick={() => navigate(location.pathname + "?edit=true")}
                >
                  Edit
                </Button>
              </Space>
            )}
            {isEditing && (
              <Space>
                <Popconfirm
                  title="Delete this bind?"
                  description="This cannot be undone."
                  onConfirm={handleDelete}
                  okText="Delete"
                  okButtonProps={{ danger: true }}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={saving}>
                    Delete
                  </Button>
                </Popconfirm>
                <Button
                  icon={<X size={14} />}
                  onClick={() => navigate(location.pathname)}
                  disabled={saving}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  htmlType="submit"
                  form="bind-form"
                  loading={saving}
                >
                  Save
                </Button>
              </Space>
            )}
          </TitleRow>
          <Description>Port binding configuration</Description>
        </Header>

        <SectionTitle>Bind Details</SectionTitle>
        <FormWrapper>
          <Form
            id="bind-form"
            key={isEditing ? "editing" : "viewing"}
            schema={forms.bind.schema}
            uiSchema={forms.bind.uiSchema}
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={handleSubmit}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  if (selected.type === "listener") {
    const { node } = selected;
    const protocol = node.listener.protocol ?? "HTTP";
    const isTcp = protocol === "TCP" || protocol === "TLS";
    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);
    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}
                {node.listener.name ?? "(unnamed listener)"}
              </Title>
              <TypeBadge>Listener</TypeBadge>
              <ProtocolTag protocol={protocol} />
            </TitleLeft>
            {!isEditing && (
              <Space>
                <Button
                  icon={<PlusOutlined />}
                  onClick={() =>
                    handleAddRoute(
                      selected.bind.bind.port,
                      node.listenerIndex,
                      isTcp,
                    )
                  }
                >
                  {isTcp ? "Add TCP Route" : "Add Route"}
                </Button>
                <Button
                  type="primary"
                  icon={<Edit2 size={14} />}
                  onClick={() => navigate(location.pathname + "?edit=true")}
                >
                  Edit
                </Button>
              </Space>
            )}
            {isEditing && (
              <Space>
                <Popconfirm
                  title="Delete this listener?"
                  description="This cannot be undone."
                  onConfirm={handleDelete}
                  okText="Delete"
                  okButtonProps={{ danger: true }}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={saving}>
                    Delete
                  </Button>
                </Popconfirm>
                <Button
                  icon={<X size={14} />}
                  onClick={() => navigate(location.pathname)}
                  disabled={saving}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  htmlType="submit"
                  form="listener-form"
                  loading={saving}
                >
                  Save
                </Button>
              </Space>
            )}
          </TitleRow>
          <Description>
            Listener configuration on port {selected.bind.bind.port}
          </Description>
        </Header>

        <SectionTitle>Listener Details</SectionTitle>
        <FormWrapper>
          <Form
            id="listener-form"
            key={isEditing ? "editing" : "viewing"}
            schema={forms.listener.schema}
            uiSchema={forms.listener.uiSchema}
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={handleSubmit}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  if (selected.type === "route") {
    const { node } = selected;
    const isTcp = node.isTcp;
    const { port, li, ri } = urlParams;

    // Guard: route requires port, li, and ri
    if (port === undefined || li === undefined || ri === undefined) {
      return null;
    }

    // Check which policy types already exist
    const existingPolicyTypes = new Set(node.policies.map((p) => p.policyType));
    const hasCors = existingPolicyTypes.has("cors");
    const hasRequestHeaderModifier = existingPolicyTypes.has(
      "requestHeaderModifier",
    );
    const hasResponseHeaderModifier = existingPolicyTypes.has(
      "responseHeaderModifier",
    );

    const policyMenuItems: MenuProps["items"] = [
      {
        key: "addCors",
        label: hasCors ? "CORS policy already exists" : "Add CORS Policy",
        icon: <PlusOutlined />,
        disabled: hasCors,
        onClick: () => !hasCors && handleAddPolicy(port, li, ri, isTcp, "cors"),
      },
      {
        key: "addRequestHeaderModifier",
        label: hasRequestHeaderModifier
          ? "Request Header Modifier already exists"
          : "Add Request Header Modifier",
        icon: <PlusOutlined />,
        disabled: hasRequestHeaderModifier,
        onClick: () =>
          !hasRequestHeaderModifier &&
          handleAddPolicy(port, li, ri, isTcp, "requestHeaderModifier"),
      },
      {
        key: "addResponseHeaderModifier",
        label: hasResponseHeaderModifier
          ? "Response Header Modifier already exists"
          : "Add Response Header Modifier",
        icon: <PlusOutlined />,
        disabled: hasResponseHeaderModifier,
        onClick: () =>
          !hasResponseHeaderModifier &&
          handleAddPolicy(port, li, ri, isTcp, "responseHeaderModifier"),
      },
    ];

    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);
    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}
                {(node.route.name as string | undefined) ?? "(unnamed route)"}
              </Title>
              <TypeBadge>{isTcp ? "TCP Route" : "HTTP Route"}</TypeBadge>
            </TitleLeft>
            {!isEditing && (
              <Space>
                <Button
                  icon={<PlusOutlined />}
                  onClick={() => handleAddBackend(port, li, ri, isTcp)}
                >
                  Add Backend
                </Button>
                <Dropdown menu={{ items: policyMenuItems }} trigger={["click"]}>
                  <Button icon={<PlusOutlined />}>Add Policy</Button>
                </Dropdown>
                <Button
                  type="primary"
                  icon={<Edit2 size={14} />}
                  onClick={() => navigate(location.pathname + "?edit=true")}
                >
                  Edit
                </Button>
              </Space>
            )}
            {isEditing && (
              <Space>
                <Popconfirm
                  title="Delete this route?"
                  description="This cannot be undone."
                  onConfirm={handleDelete}
                  okText="Delete"
                  okButtonProps={{ danger: true }}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={saving}>
                    Delete
                  </Button>
                </Popconfirm>
                <Button
                  icon={<X size={14} />}
                  onClick={() => navigate(location.pathname)}
                  disabled={saving}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  htmlType="submit"
                  form="route-form"
                  loading={saving}
                >
                  Save
                </Button>
              </Space>
            )}
          </TitleRow>
          <Description>
            Route configuration for listener "
            {selected.listener.listener.name ?? "unnamed"}"
          </Description>
        </Header>

        <SectionTitle>Route Details</SectionTitle>
        <FormWrapper>
          <Form
            id="route-form"
            key={isEditing ? "editing" : "viewing"}
            schema={forms.route.schema}
            uiSchema={forms.route.uiSchema}
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={handleSubmit}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  if (selected.type === "backend") {
    const { node } = selected;
    const backend = node.backend;

    // Determine backend type
    let backendType = "Backend";
    if (typeof backend === "object" && backend !== null) {
      const b = backend as Record<string, unknown>;
      if ("service" in b) backendType = "Service Backend";
      else if ("host" in b) backendType = "Host Backend";
      else if ("mcp" in b) backendType = "MCP Backend";
      else if ("ai" in b) backendType = "AI Backend";
    }

    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);
    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}
                {backendType}
              </Title>
              <TypeBadge>Backend</TypeBadge>
            </TitleLeft>
            {!isEditing && (
              <Button
                type="primary"
                icon={<Edit2 size={14} />}
                onClick={() => navigate(location.pathname + "?edit=true")}
              >
                Edit
              </Button>
            )}
            {isEditing && (
              <Space>
                <Popconfirm
                  title="Delete this backend?"
                  description="This cannot be undone."
                  onConfirm={handleDelete}
                  okText="Delete"
                  okButtonProps={{ danger: true }}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={saving}>
                    Delete
                  </Button>
                </Popconfirm>
                <Button
                  icon={<X size={14} />}
                  onClick={() => navigate(location.pathname)}
                  disabled={saving}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  htmlType="submit"
                  form="backend-form"
                  loading={saving}
                >
                  Save
                </Button>
              </Space>
            )}
          </TitleRow>
          <Description>
            Backend configuration for route "
            {(selected.route.route.name as string | undefined) ?? "unnamed"}"
          </Description>
        </Header>

        <SectionTitle>Backend Details</SectionTitle>
        <FormWrapper>
          <Form
            id="backend-form"
            key={isEditing ? "editing" : "viewing"}
            schema={forms.backend.schema}
            uiSchema={forms.backend.uiSchema}
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={handleSubmit}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  if (selected.type === "policy") {
    // Format policy type for display
    const displayName = selected.node.policyType
      .replace(/([A-Z])/g, " $1")
      .replace(/^./, (str) => str.toUpperCase())
      .trim();

    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);
    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}
                {displayName}
              </Title>
              <TypeBadge>Policy</TypeBadge>
            </TitleLeft>
            {!isEditing && (
              <Button
                type="primary"
                icon={<Edit2 size={14} />}
                onClick={() => navigate(location.pathname + "?edit=true")}
              >
                Edit
              </Button>
            )}
            {isEditing && (
              <Space>
                <Popconfirm
                  title="Delete this policy?"
                  description="This cannot be undone."
                  onConfirm={handleDelete}
                  okText="Delete"
                  okButtonProps={{ danger: true }}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={saving}>
                    Delete
                  </Button>
                </Popconfirm>
                <Button
                  icon={<X size={14} />}
                  onClick={() => navigate(location.pathname)}
                  disabled={saving}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  htmlType="submit"
                  form="policy-form"
                  loading={saving}
                >
                  Save
                </Button>
              </Space>
            )}
          </TitleRow>
          <Description>
            Policy configuration for route "
            {(selected.route.route.name as string | undefined) ?? "unnamed"}"
          </Description>
        </Header>

        <SectionTitle>Policy Details</SectionTitle>
        <FormWrapper>
          <Form
            id="policy-form"
            key={isEditing ? "editing" : "viewing"}
            schema={
              selected.node.policyType === "cors"
                ? forms.corsPolicy.schema
                : selected.node.policyType === "requestHeaderModifier"
                  ? forms.requestHeaderModifierPolicy.schema
                  : selected.node.policyType === "responseHeaderModifier"
                    ? forms.responseHeaderModifierPolicy.schema
                    : forms.routePolicy.schema
            }
            uiSchema={
              selected.node.policyType === "cors"
                ? forms.corsPolicy.uiSchema
                : selected.node.policyType === "requestHeaderModifier"
                  ? forms.requestHeaderModifierPolicy.uiSchema
                  : selected.node.policyType === "responseHeaderModifier"
                    ? forms.responseHeaderModifierPolicy.uiSchema
                    : forms.routePolicy.uiSchema
            }
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={handleSubmit}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  // Handle model type
  if (selected.type === "model") {
    const { node } = selected;
    const modelName = node.model.name || `Model ${node.modelIndex + 1}`;
    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);

    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}
                {modelName}
              </Title>
              <TypeBadge>Model</TypeBadge>
            </TitleLeft>
            {!isEditing && (
              <Button
                type="primary"
                icon={<Edit2 size={14} />}
                onClick={() => navigate(location.pathname + "?edit=true")}
              >
                Edit
              </Button>
            )}
            {isEditing && (
              <Space>
                <Popconfirm
                  title={`Delete model "${modelName}"?`}
                  description="This cannot be undone."
                  onConfirm={handleDelete}
                  okText="Delete"
                  okButtonProps={{ danger: true }}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={saving}>
                    Delete
                  </Button>
                </Popconfirm>
                <Button
                  icon={<X size={14} />}
                  onClick={() => navigate(location.pathname)}
                  disabled={saving}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  htmlType="submit"
                  form="model-form"
                  loading={saving}
                >
                  Save
                </Button>
              </Space>
            )}
          </TitleRow>
          <Description>Model configuration for LLM provider</Description>
        </Header>

        <SectionTitle>Model Details</SectionTitle>
        <FormWrapper>
          <Form
            id="model-form"
            key={isEditing ? "editing" : "viewing"}
            schema={forms.model.schema}
            uiSchema={forms.model.uiSchema}
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={handleSubmit}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  // Handle top-level config types (llm, mcp, frontendPolicies)
  if (
    selected.type === "llm" ||
    selected.type === "mcp" ||
    selected.type === "frontendPolicies"
  ) {
    const typeLabels = {
      llm: "LLM Configuration",
      mcp: "MCP Configuration",
      frontendPolicies: "Frontend Policies",
    };

    const handleTopLevelSave = async (fd: Record<string, unknown>) => {
      setSaving(true);
      try {
        if (selected.type === "llm") {
          await api.createOrUpdateLLM(fd);
        } else if (selected.type === "mcp") {
          await api.createOrUpdateMCP(fd);
        } else if (selected.type === "frontendPolicies") {
          await api.createOrUpdateFrontendPolicies(fd);
        }
        toast.success(`${typeLabels[selected.type]} updated successfully`);
        mutate();
        navigate(location.pathname); // Remove ?edit=true
      } catch (e: unknown) {
        const errorMessage =
          e &&
          typeof e === "object" &&
          "message" in e &&
          typeof e.message === "string"
            ? e.message
            : "Failed to save changes";
        toast.error(errorMessage);
      } finally {
        setSaving(false);
      }
    };

    const breadcrumbItems = generateBreadcrumbItems(selected, navigate);
    return (
      <Container>
        <Header>
          <BreadcrumbWrapper>
            <Breadcrumb items={breadcrumbItems} />
          </BreadcrumbWrapper>
          <TitleRow>
            <TitleLeft>
              <Title>
                {isEditing ? "Edit: " : ""}
                {typeLabels[selected.type]}
              </Title>
              <TypeBadge>{selected.type.toUpperCase()}</TypeBadge>
            </TitleLeft>
            {!isEditing && (
              <Space>
                {selected.type === "llm" && (
                  <Button
                    icon={<PlusOutlined />}
                    onClick={async () => {
                      try {
                        const newModel = {
                          name: `model-${(hierarchy.llm?.models.length ?? 0) + 1}`,
                          provider: "openAI",
                        };
                        await api.createLLMModel(newModel);
                        toast.success("Model created successfully");

                        // Wait for config to refresh and get the updated data
                        const freshConfig = await mutate();

                        // Find the index of the newly created model from fresh data
                        const llmConfig = freshConfig?.llm as any;
                        const newIndex = llmConfig?.models
                          ? llmConfig.models.length - 1
                          : 0;
                        navigate(
                          `/traffic/llm/model/${newIndex}?edit=true&creating=true`,
                        );
                      } catch (e: unknown) {
                        toast.error(
                          e instanceof Error
                            ? e.message
                            : "Failed to create model",
                        );
                      }
                    }}
                  >
                    Add Model
                  </Button>
                )}
                <Button
                  type="primary"
                  icon={<Edit2 size={14} />}
                  onClick={() => navigate(location.pathname + "?edit=true")}
                >
                  Edit
                </Button>
              </Space>
            )}
            {isEditing && (
              <Button
                icon={<X size={14} />}
                onClick={() => navigate(location.pathname)}
              >
                Cancel
              </Button>
            )}
          </TitleRow>
          <Description>
            {selected.type === "llm" && "Configure LLM providers and models"}
            {selected.type === "mcp" &&
              "Configure Model Context Protocol settings"}
            {selected.type === "frontendPolicies" &&
              "Configure frontend-wide policies"}
          </Description>
        </Header>

        <SectionTitle>{typeLabels[selected.type]} Details</SectionTitle>
        <FormWrapper>
          <Form
            key={isEditing ? "editing" : "viewing"}
            schema={forms[selected.type].schema}
            uiSchema={forms[selected.type].uiSchema}
            formData={formData}
            validator={validator}
            disabled={!isEditing || saving}
            onChange={({ formData: fd }) => setFormData(fd)}
            onSubmit={({ formData: fd }) => {
              if (fd) handleTopLevelSave(fd);
            }}
            onError={handleError}
            templates={formTemplates}
          >
            <></>
          </Form>
        </FormWrapper>
      </Container>
    );
  }

  return null;
}
