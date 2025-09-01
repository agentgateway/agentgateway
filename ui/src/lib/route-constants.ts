import { PathMatch } from "@/lib/types";

export const DEFAULT_HTTP_ROUTE_FORM = {
  name: "",
  ruleName: "",
  hostnames: "",
  pathPrefix: "/",
  pathType: "pathPrefix" as keyof PathMatch,
  headers: "",
  methods: "",
  queryParams: "",
};

export const DEFAULT_TCP_ROUTE_FORM = {
  name: "",
  ruleName: "",
  hostnames: "",
};

export const PATH_MATCH_TYPES = {
  pathPrefix: "Path Prefix",
  exact: "Exact Path",
  regex: "Regex Pattern",
} as const;

export const ROUTE_TABLE_HEADERS = [
  "Name",
  "Type",
  "Listener",
  "Hostnames",
  "Path",
  "Backends",
  "Actions",
] as const;

export const ROUTE_TYPE_CONFIGS = {
  http: {
    label: "HTTP",
    color: "bg-primary hover:bg-primary/90 text-primary-foreground",
    icon: "Globe",
  },
  tcp: {
    label: "TCP",
    color: "bg-primary hover:bg-primary/90 text-primary-foreground",
    icon: "Server",
  },
} as const;
