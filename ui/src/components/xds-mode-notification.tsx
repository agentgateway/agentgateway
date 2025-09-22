"use client";

import { useXdsMode } from "@/hooks/use-xds-mode";

export function XdsModeNotification() {
  const xds = useXdsMode();

  if (!xds) return null;

  return (
    <div className="bg-primary text-primary-foreground text-center p-2 text-sm">
      Configuration is managed by an external source (XDS). Editing the configuration is not allowed
      via the UI.
    </div>
  );
}
