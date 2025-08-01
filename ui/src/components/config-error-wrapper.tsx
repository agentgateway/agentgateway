"use client";

import { useServer } from "@/lib/server-context";
import { ConfigError } from "@/components/config-error";

interface ConfigErrorWrapperProps {
  children: React.ReactNode;
}

export function ConfigErrorWrapper({ children }: ConfigErrorWrapperProps) {
  const { configError } = useServer();

  if (configError) {
    return <ConfigError error={configError} />;
  }

  return <>{children}</>;
}
