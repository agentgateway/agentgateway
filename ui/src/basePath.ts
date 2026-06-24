export function publicAssetPath(path: string) {
  const base = import.meta.env.BASE_URL || "./";
  const normalizedBase = base.endsWith("/") ? base : `${base}/`;
  return `${normalizedBase}${path.replace(/^\/+/, "")}`;
}

export function routerBasePath() {
  const configured = import.meta.env.VITE_ROUTER_BASE_PATH;
  const base = configured || import.meta.env.BASE_URL || "";
  if (!base || base === "./" || base === "/") return undefined;
  return `/${base.replace(/^\/+|\/+$/g, "")}`;
}
