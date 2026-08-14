import type { VirtualApiKey } from "./types";

export function maskKey(key: string) {
  if (key.length <= 10) return key;
  return `${key.slice(0, 7)}...${key.slice(-4)}`;
}

export function hasKeyValue<T extends { metadata?: unknown }>(
  key: T,
): key is T & { key: string } {
  return "key" in key;
}

export function keyValue(key: VirtualApiKey) {
  return hasKeyValue(key) ? key.key : key.keyHash;
}

export function keyLabel(key: VirtualApiKey) {
  const metadata =
    key.metadata &&
    typeof key.metadata === "object" &&
    !Array.isArray(key.metadata)
      ? (key.metadata as Record<string, unknown>)
      : {};
  const name =
    typeof metadata.name === "string" && metadata.name.trim()
      ? metadata.name.trim()
      : "";
  const value = keyValue(key);
  const masked = hasKeyValue(key) ? maskKey(value) : `hash ${maskKey(value)}`;
  return name ? `${name} (${masked})` : masked;
}
