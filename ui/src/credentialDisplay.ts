export function maskKey(key: string) {
  if (key.length <= 10) return key;
  return `${key.slice(0, 7)}...${key.slice(-4)}`;
}

export function keyLabel(key: { key: string; metadata?: unknown }) {
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
  return name ? `${name} (${maskKey(key.key)})` : maskKey(key.key);
}
