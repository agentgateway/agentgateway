import { Route as RouteType, TcpRoute, Listener, Match } from "@/lib/types";
import { DEFAULT_HTTP_ROUTE_FORM, DEFAULT_TCP_ROUTE_FORM } from "./route-constants";

// Helper function to determine if a listener protocol supports TCP routes
export const isTcpListener = (listener: Listener): boolean => {
  const protocol =
    typeof listener.protocol === "string"
      ? listener.protocol
      : listener.protocol && typeof listener.protocol === "object"
        ? Object.keys(listener.protocol as Record<string, unknown>)[0] || "HTTP"
        : "HTTP";
  return protocol === "TCP" || protocol === "TLS";
};

// Parse comma-separated string into array
export const parseStringArray = (str: string): string[] => {
  return str
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
};

// Build match object for HTTP routes
export const buildMatch = (routeForm: typeof DEFAULT_HTTP_ROUTE_FORM): Match => {
  const match: Match = {
    path:
      routeForm.pathType === "regex"
        ? { regex: [routeForm.pathPrefix, 0] }
        : { [routeForm.pathType]: routeForm.pathPrefix },
  };

  // Add headers if provided
  if (routeForm.headers) {
    const headerPairs = parseStringArray(routeForm.headers);
    match.headers = headerPairs.map((pair) => {
      const [name, value] = pair.split(":").map((s) => s.trim());
      return {
        name,
        value: { exact: value || "" },
      };
    });
  }

  // Add methods if provided
  if (routeForm.methods) {
    match.method = { method: routeForm.methods.trim() };
  }

  // Add query params if provided
  if (routeForm.queryParams) {
    const queryPairs = parseStringArray(routeForm.queryParams);
    match.query = queryPairs.map((pair) => {
      const [name, value] = pair.split("=").map((s) => s.trim());
      return {
        name,
        value: { exact: value || "" },
      };
    });
  }

  return match;
};

// Populate form with HTTP route data for editing
export const populateEditForm = (route: RouteType): typeof DEFAULT_HTTP_ROUTE_FORM => {
  const firstMatch = route.matches?.[0];

  return {
    name: route.name || "",
    ruleName: route.ruleName || "",
    hostnames: route.hostnames?.join(", ") || "",
    pathPrefix:
      firstMatch?.path.pathPrefix || firstMatch?.path.exact || firstMatch?.path.regex?.[0] || "/",
    pathType: firstMatch?.path.pathPrefix
      ? "pathPrefix"
      : firstMatch?.path.exact
        ? "exact"
        : firstMatch?.path.regex
          ? "regex"
          : "pathPrefix",
    headers: firstMatch?.headers?.map((h) => `${h.name}:${h.value.exact || ""}`).join(", ") || "",
    methods: firstMatch?.method?.method || "",
    queryParams: firstMatch?.query?.map((q) => `${q.name}=${q.value.exact || ""}`).join(", ") || "",
  };
};

// Populate form with TCP route data for editing
export const populateTcpEditForm = (tcpRoute: TcpRoute): typeof DEFAULT_TCP_ROUTE_FORM => {
  return {
    name: tcpRoute.name || "",
    ruleName: tcpRoute.ruleName || "",
    hostnames: tcpRoute.hostnames?.join(", ") || "",
  };
};

// Get path display string for table
export const getPathDisplayString = (match: Match): string => {
  if (match.path.exact) return `= ${match.path.exact}`;
  if (match.path.pathPrefix) return `${match.path.pathPrefix}*`;
  if (match.path.regex) return `~ ${match.path.regex[0]}`;
  return "/";
};

// Get route type label
export const getRouteTypeLabel = (route: RouteType | TcpRoute): string => {
  return "matches" in route ? "HTTP" : "TCP";
};

// Create new HTTP route object
export const createHttpRoute = (
  routeForm: typeof DEFAULT_HTTP_ROUTE_FORM,
  match: Match
): RouteType => {
  const newRoute: RouteType = {
    hostnames: parseStringArray(routeForm.hostnames),
    matches: [match],
    backends: [],
  };

  if (routeForm.name) newRoute.name = routeForm.name;
  if (routeForm.ruleName) newRoute.ruleName = routeForm.ruleName;

  return newRoute;
};

// Create new TCP route object
export const createTcpRoute = (tcpRouteForm: typeof DEFAULT_TCP_ROUTE_FORM): TcpRoute => {
  const newTcpRoute: TcpRoute = {
    hostnames: parseStringArray(tcpRouteForm.hostnames),
    backends: [],
  };

  if (tcpRouteForm.name) newTcpRoute.name = tcpRouteForm.name;
  if (tcpRouteForm.ruleName) newTcpRoute.ruleName = tcpRouteForm.ruleName;

  return newTcpRoute;
};

// Update HTTP route object
export const updateHttpRoute = (
  routeForm: typeof DEFAULT_HTTP_ROUTE_FORM,
  match: Match,
  existingRoute: RouteType
): RouteType => {
  const updatedRoute: RouteType = {
    hostnames: parseStringArray(routeForm.hostnames),
    matches: [match],
    backends: existingRoute.backends, // Keep existing backends
    policies: existingRoute.policies,
  };

  if (routeForm.name) updatedRoute.name = routeForm.name;
  if (routeForm.ruleName) updatedRoute.ruleName = routeForm.ruleName;

  return updatedRoute;
};

// Update TCP route object
export const updateTcpRoute = (
  tcpRouteForm: typeof DEFAULT_TCP_ROUTE_FORM,
  existingRoute: TcpRoute
): TcpRoute => {
  const updatedTcpRoute: TcpRoute = {
    hostnames: parseStringArray(tcpRouteForm.hostnames),
    backends: existingRoute.backends, // Keep existing backends
    policies: existingRoute.policies,
  };

  if (tcpRouteForm.name) updatedTcpRoute.name = tcpRouteForm.name;
  if (tcpRouteForm.ruleName) updatedTcpRoute.ruleName = tcpRouteForm.ruleName;

  return updatedTcpRoute;
};
