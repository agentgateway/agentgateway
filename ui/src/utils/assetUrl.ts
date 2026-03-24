/**
 * Resolves a public asset path relative to the app's base URL.
 * Handles both dev (base="/") and prod (base="/ui/") correctly.
 */
export function assetUrl(path: string): string {
  const base = import.meta.env.BASE_URL;
  return base + path.replace(/^\//, "");
}
