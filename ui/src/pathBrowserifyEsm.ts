function normalize(path: string) {
  const absolute = path.startsWith("/");
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length && parts[parts.length - 1] !== "..") parts.pop();
      else if (!absolute) parts.push(part);
    } else {
      parts.push(part);
    }
  }
  const result = `${absolute ? "/" : ""}${parts.join("/")}`;
  return result || (absolute ? "/" : ".");
}

export function isAbsolute(path: string) {
  return path.startsWith("/");
}

export function dirname(path: string) {
  if (!path) return ".";
  const trimmed = path.replace(/\/+$/, "") || "/";
  if (trimmed === "/") return "/";
  const index = trimmed.lastIndexOf("/");
  if (index < 0) return ".";
  if (index === 0) return "/";
  return trimmed.slice(0, index);
}

export function basename(path: string, ext?: string) {
  const trimmed = path.replace(/\/+$/, "");
  const base = trimmed.slice(trimmed.lastIndexOf("/") + 1);
  return ext && base.endsWith(ext) ? base.slice(0, -ext.length) : base;
}

export function extname(path: string) {
  const base = basename(path);
  const index = base.lastIndexOf(".");
  return index > 0 ? base.slice(index) : "";
}

export function join(...paths: string[]) {
  return normalize(paths.filter(Boolean).join("/"));
}

export function resolve(...paths: string[]) {
  let resolved = "";
  for (const path of paths) {
    if (!path) continue;
    resolved = isAbsolute(path) ? path : join(resolved, path);
  }
  return normalize(resolved || ".");
}

export function parse(path: string) {
  const dir = dirname(path);
  const base = basename(path);
  const ext = extname(base);
  const name = ext ? base.slice(0, -ext.length) : base;
  const root = isAbsolute(path) ? "/" : "";
  return { root, dir, base, ext, name };
}

export const sep = "/";
export const delimiter = ":";
export const posix = {
  basename,
  delimiter,
  dirname,
  extname,
  isAbsolute,
  join,
  parse,
  resolve,
  sep,
};

export const win32 = null;
export default posix;
