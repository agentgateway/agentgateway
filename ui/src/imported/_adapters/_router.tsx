// Shim: makes @tanstack/react-router Link available via our router
import { Link as RouterLink, useLocation as rrUseLocation } from "react-router-dom";
import type { ComponentProps } from "react";

export function useLocation() {
  return rrUseLocation();
}

type LinkProps = ComponentProps<typeof RouterLink> & {
  // tanstack/react-router passes search params as an object; we ignore it
  search?: Record<string, unknown>;
};

export function Link({ search: _search, ...props }: LinkProps) {
  return <RouterLink {...props} />;
}
