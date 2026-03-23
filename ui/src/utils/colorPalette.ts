/**
 * Color palette utilities for automatically assigning colors to resource types
 */

/**
 * A carefully selected palette of visually distinct colors
 * that work well in both light and dark modes
 * Inspired by purple branding with complementary colors
 */
const COLOR_PALETTE = [
  "#8b5cf6", // violet-500 (bind - primary purple)
  "#06b6d4", // cyan-500 (listener - bright cyan)
  "#f59e0b", // amber-500 (route - warm amber)
  "#10b981", // emerald-500 (backend - green)
  "#ec4899", // pink-500 (policy - vibrant pink)
  "#6366f1", // indigo-500 (model - blue-purple)
  "#a855f7", // purple-500 (llm - lighter purple)
  "#14b8a6", // teal-500 (mcp - teal)
  "#f97316", // orange-500 (frontendPolicies - orange)
];

/**
 * Resource type to color index mapping
 * Each resource type gets a specific color from the palette
 */
const RESOURCE_COLOR_MAP: Record<string, number> = {
  bind: 0,
  listener: 1,
  route: 2,
  backend: 3,
  policy: 4,
  model: 5,
  llm: 6,
  mcp: 7,
  frontendPolicies: 8,
};

/**
 * Get a color from the palette based on a resource type
 * The same resource type will always return the same color
 *
 * @param resourceType - The type of resource (e.g., "bind", "listener", "route")
 * @returns A hex color code
 *
 * @example
 * getResourceColor("bind") // Always returns the same color for "bind"
 * getResourceColor("listener") // Always returns the same color for "listener"
 */
export function getResourceColor(resourceType: string): string {
  const index = RESOURCE_COLOR_MAP[resourceType];
  if (index !== undefined) {
    return COLOR_PALETTE[index];
  }
  // Fallback: use first color if resource type not mapped
  return COLOR_PALETTE[0];
}

/**
 * Get a color from the palette based on an index
 * Useful when you need sequential colors
 *
 * @param index - The index to use
 * @returns A hex color code
 */
export function getColorByIndex(index: number): string {
  return COLOR_PALETTE[index % COLOR_PALETTE.length];
}

/**
 * Get the entire color palette
 * Useful if you need all colors at once
 */
export function getColorPalette(): readonly string[] {
  return COLOR_PALETTE;
}
